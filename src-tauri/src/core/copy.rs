//! Copying a whole project to a new folder ("Move project"): scan, copy with throttled progress
//! and cancel, verify, then switch the open project over.
//!
//! Files up to the chunk threshold go through `fs::copy` (CopyFileExW on Windows: the data never
//! enters our heap). Larger files stream through ONE 1 MiB buffer, allocated once per move, so
//! progress stays smooth and memory flat. There is no free-space pre-check: a full disk surfaces
//! as the copy's I/O error, after the partial copy has been removed.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use walkdir::WalkDir;

use crate::core::events::{names, EventSink};
use crate::core::model::{BackupReason, MovePhase, MoveProgress, Project};
use crate::core::project::BACKUPS_DIR;
use crate::core::state::AppState;
use crate::core::{fsx, recents};
use crate::error::{AppError, AppResult};

/// Files larger than this are streamed in chunks instead of `fs::copy`.
pub const DEFAULT_CHUNK_THRESHOLD: u64 = 64 * 1024 * 1024;
/// Size of the single reused copy buffer.
pub const CHUNK_SIZE: usize = 1024 * 1024;
/// Minimum gap between two progress events within one phase (≤ 10 per second).
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    #[error("Canceled")]
    Cancelled,
    #[error("{}: {source}", fsx::display_path(path))]
    Io { path: PathBuf, source: io::Error },
    #[error("The copy doesn't match the original: {0}")]
    Verify(String),
}

impl From<CopyError> for AppError {
    fn from(e: CopyError) -> Self {
        match e {
            CopyError::Cancelled => AppError::Cancelled,
            CopyError::Io { path, source } => AppError::Io(io::Error::new(
                source.kind(),
                format!("{}: {source}", fsx::display_path(&path)),
            )),
            CopyError::Verify(_) => AppError::Io(io::Error::other(e.to_string())),
        }
    }
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> CopyError + '_ {
    move |source| CopyError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEntry {
    /// Relative to the scanned root.
    pub rel: PathBuf,
    pub size: u64,
}

/// Everything under a directory: subdirectories (so empty ones are recreated) and files.
#[derive(Debug, Default)]
pub struct Scan {
    pub dirs: Vec<PathBuf>,
    pub files: Vec<ScanEntry>,
    pub bytes: u64,
}

/// One walk over `root`. Symlinks are not followed; they are skipped with a warning.
pub fn scan(root: &Path) -> Result<Scan, CopyError> {
    let mut out = Scan::default();
    let walker = WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .sort_by_file_name();
    for entry in walker {
        let entry = entry.map_err(|e| {
            let path = e.path().unwrap_or(root).to_path_buf();
            CopyError::Io {
                path,
                source: io::Error::from(e),
            }
        })?;
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap_or(entry.path())
            .to_path_buf();
        let ft = entry.file_type();
        if ft.is_symlink() {
            tracing::warn!("move: skipping symlink {}", entry.path().display());
        } else if ft.is_dir() {
            out.dirs.push(rel);
        } else if ft.is_file() {
            let size = entry
                .metadata()
                .map_err(|e| io_err(entry.path())(io::Error::from(e)))?
                .len();
            out.bytes += size;
            out.files.push(ScanEntry { rel, size });
        }
    }
    Ok(out)
}

/// Emits `MoveProgress`: always on a phase change, otherwise at most once per
/// `PROGRESS_INTERVAL`.
pub struct Reporter<'a> {
    sink: &'a dyn EventSink,
    progress: MoveProgress,
    last: Option<Instant>,
}

impl<'a> Reporter<'a> {
    pub fn new(sink: &'a dyn EventSink) -> Self {
        Self {
            sink,
            progress: MoveProgress {
                phase: MovePhase::Scanning,
                files_done: 0,
                files_total: 0,
                bytes_done: 0,
                bytes_total: 0,
            },
            last: None,
        }
    }

    pub fn progress(&self) -> &MoveProgress {
        &self.progress
    }

    pub fn set_totals(&mut self, files: usize, bytes: u64) {
        self.progress.files_total = u32::try_from(files).unwrap_or(u32::MAX);
        self.progress.bytes_total = bytes;
    }

    pub fn phase(&mut self, phase: MovePhase) {
        self.progress.phase = phase;
        self.emit();
    }

    pub fn add(&mut self, files: u32, bytes: u64) {
        self.progress.files_done = self.progress.files_done.saturating_add(files);
        self.progress.bytes_done = self.progress.bytes_done.saturating_add(bytes);
        self.tick();
    }

    /// Emits if the last event is at least `PROGRESS_INTERVAL` old.
    pub fn tick(&mut self) {
        if self.last.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL) {
            self.emit();
        }
    }

    fn emit(&mut self) {
        self.last = Some(Instant::now());
        match serde_json::to_value(&self.progress) {
            Ok(v) => self.sink.emit(names::MOVE_PROGRESS, v),
            Err(e) => tracing::warn!("move progress payload: {e}"),
        }
    }
}

fn check(cancel: &AtomicBool) -> Result<(), CopyError> {
    if cancel.load(Ordering::SeqCst) {
        Err(CopyError::Cancelled)
    } else {
        Ok(())
    }
}

/// Copy settings for [`copy_tree`].
pub struct CopyOptions<'a> {
    /// Files larger than this are streamed through the reused buffer.
    pub chunk_threshold: u64,
    pub cancel: &'a AtomicBool,
    /// Test hook, called after each file.
    pub after_file: Option<&'a (dyn Fn() + Sync)>,
}

/// Copies everything in `scan` from `src` to `dst` (which must exist). Checks `cancel` between
/// files and between chunks. `scan` sizes are updated to the bytes actually copied; a file under
/// `.backups/` that vanished meanwhile (pruned) is dropped from `scan` instead of failing.
pub fn copy_tree(
    src: &Path,
    dst: &Path,
    scan: &mut Scan,
    opts: &CopyOptions<'_>,
    rep: &mut Reporter<'_>,
) -> Result<(), CopyError> {
    for rel in &scan.dirs {
        let d = dst.join(rel);
        fs::create_dir_all(&d).map_err(io_err(&d))?;
    }
    let mut buf: Vec<u8> = Vec::new();
    let mut vanished = Vec::new();
    for (i, entry) in scan.files.iter_mut().enumerate() {
        check(opts.cancel)?;
        let from = src.join(&entry.rel);
        let to = dst.join(&entry.rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        let copied = if entry.size > opts.chunk_threshold {
            if buf.is_empty() {
                buf = vec![0; CHUNK_SIZE];
            }
            copy_chunked(&from, &to, &mut buf, opts.cancel, rep)
        } else {
            fs::copy(&from, &to)
                .map_err(io_err(&from))
                .inspect(|n| rep.add(0, *n))
        };
        match copied {
            Ok(n) => entry.size = n,
            Err(CopyError::Io { source, .. })
                if source.kind() == io::ErrorKind::NotFound
                    && entry.rel.starts_with(BACKUPS_DIR) =>
            {
                vanished.push(i);
            }
            Err(e) => return Err(e),
        }
        rep.add(1, 0);
        if let Some(hook) = opts.after_file {
            hook();
        }
    }
    for i in vanished.into_iter().rev() {
        scan.files.remove(i);
    }
    Ok(())
}

/// Streams one file through `buf`, reporting bytes as they go. Keeps the modification time.
fn copy_chunked(
    from: &Path,
    to: &Path,
    buf: &mut [u8],
    cancel: &AtomicBool,
    rep: &mut Reporter<'_>,
) -> Result<u64, CopyError> {
    let mut input = File::open(from).map_err(io_err(from))?;
    let modified = input.metadata().and_then(|m| m.modified()).ok();
    let mut output = File::create(to).map_err(io_err(to))?;
    let mut total = 0u64;
    loop {
        check(cancel)?;
        let n = match input.read(buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io_err(from)(e)),
        };
        output.write_all(&buf[..n]).map_err(io_err(to))?;
        total += n as u64;
        rep.add(0, n as u64);
    }
    output.flush().map_err(io_err(to))?;
    if let Some(t) = modified {
        let _ = output.set_modified(t);
    }
    Ok(total)
}

/// Every scanned file exists under `dst` with the expected size.
pub fn verify(dst: &Path, scan: &Scan) -> Result<(), CopyError> {
    for f in &scan.files {
        let p = dst.join(&f.rel);
        match fs::metadata(&p) {
            Ok(m) if m.is_file() && m.len() == f.size => {}
            Ok(_) => {
                return Err(CopyError::Verify(format!(
                    "{} was not copied completely",
                    fsx::display_path(&p)
                )))
            }
            Err(_) => {
                return Err(CopyError::Verify(format!(
                    "{} is missing",
                    fsx::display_path(&p)
                )))
            }
        }
    }
    Ok(())
}

/// Options for [`move_project`].
pub struct MoveOptions {
    pub chunk_threshold: u64,
    /// Test hook, called after each copied file.
    pub after_file: Option<Box<dyn Fn() + Send + Sync>>,
}

impl Default for MoveOptions {
    fn default() -> Self {
        Self {
            chunk_threshold: DEFAULT_CHUNK_THRESHOLD,
            after_file: None,
        }
    }
}

/// True when `child` is strictly inside `parent` (both normalized with `recents::normalize`).
fn is_within(child: &str, parent: &str) -> bool {
    let sep = if cfg!(windows) { '\\' } else { '/' };
    child
        .strip_prefix(parent)
        .is_some_and(|rest| !rest.is_empty() && (rest.starts_with(sep) || parent.ends_with(sep)))
}

fn check_paths(src: &Path, dest: &Path) -> AppResult<()> {
    let s = recents::normalize(&fsx::display_path(src));
    let d = recents::normalize(&fsx::display_path(dest));
    if s == d {
        return Err(AppError::Invalid(
            "The destination is the project folder itself".into(),
        ));
    }
    if is_within(&d, &s) {
        return Err(AppError::Invalid(
            "The destination cannot be inside the project folder".into(),
        ));
    }
    if is_within(&s, &d) {
        return Err(AppError::Invalid(
            "The project folder cannot be inside the destination".into(),
        ));
    }
    Ok(())
}

/// Ensures `dest` is an empty directory. Returns whether it was created here.
fn prepare_dest(dest: &Path) -> AppResult<bool> {
    match fs::metadata(dest) {
        Ok(m) if m.is_dir() => {
            if fs::read_dir(dest)?.next().is_some() {
                return Err(AppError::NotEmpty(fsx::display_path(dest)));
            }
            Ok(false)
        }
        Ok(_) => Err(AppError::NotEmpty(fsx::display_path(dest))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(dest)?;
            Ok(true)
        }
        Err(e) => Err(e.into()),
    }
}

/// Removes what a failed move copied: the whole folder if we created it, else its contents.
fn cleanup(dest: &Path, created: bool) {
    let result = if created {
        fs::remove_dir_all(dest)
    } else {
        fs::read_dir(dest).and_then(|rd| {
            for entry in rd {
                let entry = entry?;
                let p = entry.path();
                if entry.file_type()?.is_dir() {
                    fs::remove_dir_all(&p)?;
                } else {
                    fs::remove_file(&p)?;
                }
            }
            Ok(())
        })
    };
    if let Err(e) = result {
        tracing::error!("could not clean up {}: {e}", dest.display());
    }
}

/// Copies the open project to `dest` (absolute; missing or an empty folder), verifies it and
/// switches the open project to it. With `delete_original`, the old folder is removed afterwards;
/// failing to remove it does not fail the move (a `background-error` is emitted instead).
///
/// On cancel or any failure before the switch, the partial copy is removed and the source
/// project stays open and untouched (apart from its `pre-move` snapshot).
pub fn move_project(
    state: &AppState,
    dest: &Path,
    delete_original: bool,
    opts: &MoveOptions,
) -> AppResult<Project> {
    let _guard = state.begin_move()?;
    let src = state.project_dir()?;
    if !dest.is_absolute() {
        return Err(AppError::Invalid(
            "The destination must be an absolute path".into(),
        ));
    }
    let dest = fsx::absolute(dest);
    check_paths(&src, &dest)?;
    let created = prepare_dest(&dest)?;

    let events = state.events().clone();
    let mut rep = Reporter::new(&*events);
    let cancel = state.move_cancel();

    // The timer must not prune or add snapshots while we copy.
    state.stop_backup_worker();
    let copied = (|| -> AppResult<()> {
        rep.phase(MovePhase::Snapshot);
        check(cancel)?;
        state.create_snapshot(BackupReason::PreMove)?;

        rep.phase(MovePhase::Scanning);
        check(cancel)?;
        let mut tree = scan(&src)?;
        rep.set_totals(tree.files.len(), tree.bytes);

        rep.phase(MovePhase::Copying);
        let copy_opts = CopyOptions {
            chunk_threshold: opts.chunk_threshold,
            cancel,
            after_file: opts.after_file.as_deref().map(|f| f as &(dyn Fn() + Sync)),
        };
        copy_tree(&src, &dest, &mut tree, &copy_opts, &mut rep)?;

        rep.phase(MovePhase::Verifying);
        check(cancel)?;
        verify(&dest, &tree)?;
        check(cancel)?;
        Ok(())
    })();
    if let Err(e) = copied {
        cleanup(&dest, created);
        state.start_backup_worker(None);
        return Err(e);
    }

    rep.phase(MovePhase::Switching);
    let project = match state.relocate_project(&src, dest.clone()) {
        Ok(p) => p,
        Err(e) => {
            cleanup(&dest, created);
            state.start_backup_worker(None);
            return Err(e);
        }
    };
    state.start_backup_worker(None);

    if delete_original {
        rep.phase(MovePhase::Deleting);
        if let Err(e) = fs::remove_dir_all(&src) {
            tracing::error!("could not delete {}: {e}", src.display());
            events.emit(
                names::BACKGROUND_ERROR,
                serde_json::json!({
                    "message": format!(
                        "The project was moved, but the old folder {} could not be deleted ({e}). Delete it yourself.",
                        fsx::display_path(&src)
                    )
                }),
            );
        }
    }
    rep.phase(MovePhase::Done);
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::backup;
    use serde_json::Value;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Recorder(Mutex<Vec<(Instant, String, Value)>>);

    impl EventSink for Recorder {
        fn emit(&self, event: &str, payload: Value) {
            self.0
                .lock()
                .unwrap()
                .push((Instant::now(), event.to_string(), payload));
        }
    }

    impl Recorder {
        fn progress(&self) -> Vec<(Instant, MoveProgress)> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, n, _)| n == names::MOVE_PROGRESS)
                .map(|(t, _, v)| (*t, serde_json::from_value(v.clone()).unwrap()))
                .collect()
        }
    }

    fn setup() -> (tempfile::TempDir, AppState, Arc<Recorder>, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let rec = Arc::new(Recorder::default());
        let st = AppState::load_with_debounce(
            tmp.path().join("config"),
            rec.clone(),
            Duration::from_secs(60),
        )
        .unwrap();
        let p = st.create_project(tmp.path(), "Show").unwrap();
        let dir = PathBuf::from(&p.path);
        fs::write(dir.join("screens/main.json"), "{}").unwrap();
        fs::create_dir_all(dir.join("templates/bar/nested")).unwrap();
        fs::write(dir.join("templates/bar/nested/x.css"), "a{}").unwrap();
        fs::write(dir.join("assets/clip.mp4"), vec![7u8; 300_000]).unwrap();
        fs::create_dir_all(dir.join("assets/empty-dir")).unwrap();
        (tmp, st, rec, dir)
    }

    fn rel_files(root: &Path) -> Vec<(PathBuf, u64)> {
        let s = scan(root).unwrap();
        s.files.into_iter().map(|f| (f.rel, f.size)).collect()
    }

    #[test]
    fn rejects_bad_destinations() {
        let (tmp, st, _, src) = setup();
        let full = tmp.path().join("full");
        fs::create_dir(&full).unwrap();
        fs::write(full.join("x"), "x").unwrap();
        let opts = MoveOptions::default();
        assert!(matches!(
            move_project(&st, &full, false, &opts),
            Err(AppError::NotEmpty(_))
        ));
        assert!(matches!(
            move_project(&st, &src.join("inner"), false, &opts),
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            move_project(&st, &src, false, &opts),
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            move_project(&st, tmp.path(), false, &opts),
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            move_project(&st, Path::new("relative/dir"), false, &opts),
            Err(AppError::Invalid(_))
        ));
        assert!(!src.join("inner").exists());
        assert_eq!(st.project_dir().unwrap(), src);
    }

    #[cfg(windows)]
    #[test]
    fn inside_check_is_case_insensitive_on_windows() {
        let src = Path::new(r"C:\Shows\Main");
        assert!(check_paths(src, Path::new(r"c:\shows\main\sub")).is_err());
        assert!(check_paths(src, Path::new(r"C:\SHOWS")).is_err());
        assert!(check_paths(src, Path::new(r"C:\Shows\Main2")).is_ok());
    }

    #[test]
    fn move_copies_everything_and_switches() {
        let (tmp, st, rec, src) = setup();
        let old_path = st.current_project().unwrap().path;
        let dest = tmp.path().join("moved").join("show");
        let p = move_project(&st, &dest, false, &MoveOptions::default()).unwrap();

        assert_eq!(st.project_dir().unwrap(), dest);
        assert_eq!(p.path, fsx::display_path(&dest));
        let recents: Vec<String> = st.recents().into_iter().map(|r| r.path).collect();
        assert_eq!(recents[0], p.path);
        assert!(!recents.iter().any(|r| recents::same_path(r, &old_path)));

        // Everything, including assets/ and .backups/ (with the pre-move snapshot) and empty dirs.
        assert!(dest.join("assets/clip.mp4").is_file());
        assert!(dest.join("assets/empty-dir").is_dir());
        assert!(dest.join("templates/bar/nested/x.css").is_file());
        let snaps = backup::list(&dest).unwrap();
        assert!(snaps.iter().any(|b| b.reason == BackupReason::PreMove));
        // delete_original = false leaves the source.
        assert!(src.join("project.json").is_file());
        let src_files: Vec<_> = rel_files(&src)
            .into_iter()
            .filter(|(r, _)| !r.starts_with(BACKUPS_DIR))
            .filter(|(r, _)| !r.ends_with("project.json") && !r.ends_with("ui-state.json"))
            .collect();
        for (rel, size) in src_files {
            assert_eq!(
                fs::metadata(dest.join(&rel)).unwrap().len(),
                size,
                "{rel:?}"
            );
        }

        // Progress: final Done, and within a phase events are at least ~100 ms apart.
        let events = rec.progress();
        assert_eq!(events.last().unwrap().1.phase, MovePhase::Done);
        assert_throttled(&events);
        let copying = events
            .iter()
            .rev()
            .find(|(_, p)| p.phase == MovePhase::Verifying)
            .unwrap();
        assert_eq!(copying.1.files_done, copying.1.files_total);
        assert_eq!(copying.1.bytes_done, copying.1.bytes_total);
        assert!(st.backup_worker_running());
    }

    fn assert_throttled(events: &[(Instant, MoveProgress)]) {
        for w in events.windows(2) {
            if w[0].1.phase == w[1].1.phase {
                assert!(
                    w[1].0.duration_since(w[0].0) >= Duration::from_millis(90),
                    "progress events too close together"
                );
            }
        }
    }

    #[test]
    fn delete_original_removes_source() {
        let (tmp, st, _, src) = setup();
        let dest = tmp.path().join("dest");
        fs::create_dir(&dest).unwrap();
        move_project(&st, &dest, true, &MoveOptions::default()).unwrap();
        assert!(!src.exists());
        assert!(dest.join("project.json").is_file());
        assert_eq!(st.project_dir().unwrap(), dest);
    }

    #[test]
    fn cancel_removes_copy_and_keeps_source_open() {
        let (tmp, st, _, src) = setup();
        let before = st.current_project().unwrap();
        let recents_before = st.recents();
        let dest = tmp.path().join("dest");
        let st2 = st.clone();
        let opts = MoveOptions {
            chunk_threshold: DEFAULT_CHUNK_THRESHOLD,
            after_file: Some(Box::new(move || st2.cancel_move())),
        };
        let err = move_project(&st, &dest, true, &opts).unwrap_err();
        assert!(matches!(err, AppError::Cancelled));
        assert!(!dest.exists());
        assert_eq!(st.current_project().unwrap(), before);
        assert_eq!(st.recents(), recents_before);
        assert!(src.join("assets/clip.mp4").is_file());
        assert!(st.backup_worker_running());

        // An existing empty destination is emptied back, not removed.
        fs::create_dir(&dest).unwrap();
        let err = move_project(&st, &dest, false, &opts).unwrap_err();
        assert!(matches!(err, AppError::Cancelled));
        assert!(dest.is_dir());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);

        // The flag does not leak into the next move.
        move_project(&st, &dest, false, &MoveOptions::default()).unwrap();
        assert_eq!(st.project_dir().unwrap(), dest);
    }

    #[test]
    fn busy_while_moving() {
        let (tmp, st, _, _) = setup();
        let _g = st.begin_move().unwrap();
        let err =
            move_project(&st, &tmp.path().join("d"), false, &MoveOptions::default()).unwrap_err();
        assert!(matches!(err, AppError::Busy(_)));
    }

    #[test]
    fn chunked_copy_is_identical() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(src.join("a")).unwrap();
        fs::create_dir(&dst).unwrap();
        let data: Vec<u8> = (0..(2 * CHUNK_SIZE + 12_345))
            .map(|i| (i * 31 % 251) as u8)
            .collect();
        fs::write(src.join("a/big.bin"), &data).unwrap();
        fs::write(src.join("small.txt"), "hi").unwrap();

        let mut tree = scan(&src).unwrap();
        let cancel = AtomicBool::new(false);
        let rec = Recorder::default();
        let mut rep = Reporter::new(&rec);
        let opts = CopyOptions {
            chunk_threshold: 64 * 1024,
            cancel: &cancel,
            after_file: None,
        };
        copy_tree(&src, &dst, &mut tree, &opts, &mut rep).unwrap();
        assert_eq!(fs::read(dst.join("a/big.bin")).unwrap(), data);
        assert_eq!(fs::read(dst.join("small.txt")).unwrap(), b"hi");
        assert_eq!(rep.progress().bytes_done, data.len() as u64 + 2);
        verify(&dst, &tree).unwrap();
    }

    #[test]
    fn chunked_copy_honours_cancel() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("big.bin"), vec![1u8; 200_000]).unwrap();
        let mut tree = scan(&src).unwrap();
        let cancel = AtomicBool::new(true);
        let rec = Recorder::default();
        let mut rep = Reporter::new(&rec);
        let opts = CopyOptions {
            chunk_threshold: 1024,
            cancel: &cancel,
            after_file: None,
        };
        let dst = tmp.path().join("dst");
        fs::create_dir(&dst).unwrap();
        assert!(matches!(
            copy_tree(&src, &dst, &mut tree, &opts, &mut rep),
            Err(CopyError::Cancelled)
        ));
    }

    #[test]
    fn verify_catches_truncated_and_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(src.join("d")).unwrap();
        fs::write(src.join("d/a.bin"), vec![0u8; 1000]).unwrap();
        fs::write(src.join("b.bin"), vec![0u8; 10]).unwrap();
        fs::create_dir(&dst).unwrap();
        let mut tree = scan(&src).unwrap();
        let cancel = AtomicBool::new(false);
        let rec = Recorder::default();
        let mut rep = Reporter::new(&rec);
        let opts = CopyOptions {
            chunk_threshold: DEFAULT_CHUNK_THRESHOLD,
            cancel: &cancel,
            after_file: None,
        };
        copy_tree(&src, &dst, &mut tree, &opts, &mut rep).unwrap();
        verify(&dst, &tree).unwrap();

        fs::write(dst.join("d/a.bin"), vec![0u8; 999]).unwrap();
        assert!(matches!(verify(&dst, &tree), Err(CopyError::Verify(_))));
        fs::write(dst.join("d/a.bin"), vec![0u8; 1000]).unwrap();
        fs::remove_file(dst.join("b.bin")).unwrap();
        assert!(matches!(verify(&dst, &tree), Err(CopyError::Verify(_))));
    }

    #[test]
    fn reporter_throttles_ticks_but_not_phases() {
        let rec = Recorder::default();
        let mut rep = Reporter::new(&rec);
        rep.phase(MovePhase::Copying);
        for _ in 0..10_000 {
            rep.add(0, 1);
        }
        assert_eq!(rec.progress().len(), 1);
        rep.phase(MovePhase::Verifying);
        rep.phase(MovePhase::Done);
        assert_eq!(rec.progress().len(), 3);
        std::thread::sleep(PROGRESS_INTERVAL + Duration::from_millis(10));
        rep.tick();
        assert_eq!(rec.progress().len(), 4);
    }
}
