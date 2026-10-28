//! Config snapshots under `<project>/.backups/<id>/`, plus the per-project backup timer.
//!
//! A snapshot holds the config set only (`project.json`, `ui-state.json`, `macros.json`,
//! `screens/`, `templates/`, `sources/`, `recipes/`, `flows/`, `chat-themes/`), never `assets/` or
//! `.backups/`, and a `manifest.json` listing every file with its size and modification time. It
//! is written into `.backups/.tmp-<nanoid>` and renamed into place when complete, so a crash never
//! leaves a half snapshot that looks real.
//!
//! Change detection compares fingerprints: the sorted `(path, size, mtime)` of the config set.
//! File contents are never read for it.
//!
//! Callers serialize snapshot/restore/prune through `AppState`'s `io` lock; the functions here do
//! no locking of their own.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::core::events::{names, EventSink};
use crate::core::fsx;
use crate::core::model::{BackupInfo, BackupPrefs, BackupReason};
use crate::core::project::BACKUPS_DIR;
use crate::error::AppError;

/// Top-level files of the config set.
pub const CONFIG_FILES: [&str; 5] = [
    "project.json",
    "ui-state.json",
    "macros.json",
    "controls.json",
    "panel.json",
];
/// Top-level directories of the config set (copied recursively).
pub const CONFIG_DIRS: [&str; 6] = [
    "screens",
    "templates",
    "sources",
    "recipes",
    "flows",
    "chat-themes",
];
pub const MANIFEST_FILE: &str = "manifest.json";
pub const MANIFEST_VERSION: u32 = 1;
/// Prefix of in-progress snapshot directories.
const TMP_PREFIX: &str = ".tmp-";
/// One "minute" of `BackupPrefs::interval_minutes` in production.
pub const INTERVAL_UNIT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("Backup not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{}: {source}", fsx::display_path(path))]
    Io { path: PathBuf, source: io::Error },
}

impl From<BackupError> for AppError {
    fn from(e: BackupError) -> Self {
        match e {
            BackupError::NotFound(id) => AppError::NotFound(format!("backup {id}")),
            BackupError::Invalid(m) => AppError::Invalid(m),
            BackupError::Io { path, source } => AppError::Io(io::Error::new(
                source.kind(),
                format!("{}: {source}", fsx::display_path(&path)),
            )),
        }
    }
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> BackupError + '_ {
    move |source| BackupError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn walk_err(root: &Path, e: walkdir::Error) -> BackupError {
    let path = e.path().unwrap_or(root).to_path_buf();
    BackupError::Io {
        path,
        source: io::Error::from(e),
    }
}

/// One file of the config set: path relative to the project (forward slashes), size in bytes and
/// modification time in nanoseconds since the Unix epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStat {
    pub path: String,
    pub size: u64,
    pub mtime: u64,
}

/// Sorted stats of the config set. Equal fingerprints mean "nothing changed".
pub type Fingerprint = Vec<FileStat>;

/// Contents of `.backups/<id>/manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub created_at: String,
    pub reason: BackupReason,
    pub app_version: String,
    pub files: Vec<FileStat>,
}

impl Manifest {
    fn info(&self, id: String) -> BackupInfo {
        BackupInfo {
            id,
            created_at: self.created_at.clone(),
            reason: self.reason,
            files: u32::try_from(self.files.len()).unwrap_or(u32::MAX),
            bytes: self.files.iter().map(|f| f.size).sum(),
        }
    }
}

/// Kebab-case name of a reason, as used in snapshot ids.
pub fn reason_slug(r: BackupReason) -> &'static str {
    match r {
        BackupReason::Open => "open",
        BackupReason::Timer => "timer",
        BackupReason::Manual => "manual",
        BackupReason::PreImport => "pre-import",
        BackupReason::PreMove => "pre-move",
        BackupReason::PreRestore => "pre-restore",
        BackupReason::PreReset => "pre-reset",
        BackupReason::PreMigrate => "pre-migrate",
        BackupReason::PreUpdate => "pre-update",
    }
}

fn mtime_ns(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// `a/b/c` for `path` relative to `base`.
fn rel_slash(base: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Forward-slash relative path to a native one.
fn rel_path(s: &str) -> PathBuf {
    s.split('/').collect()
}

/// Validates a manifest path: relative, no `.`/`..`/empty parts, and inside the config set.
fn safe_rel(path: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains(['\\', ':']))
    {
        return None;
    }
    let ok = match parts.as_slice() {
        [one] => CONFIG_FILES.contains(one),
        [first, _, ..] => CONFIG_DIRS.contains(first),
        [] => false,
    };
    ok.then(|| rel_path(path))
}

/// Stats of every file in the config set, sorted by path. Symlinks are skipped; missing optional
/// entries are simply absent.
pub fn fingerprint(project_dir: &Path) -> Result<Fingerprint, BackupError> {
    let mut out = Vec::new();
    for name in CONFIG_FILES {
        let p = project_dir.join(name);
        match fs::symlink_metadata(&p) {
            Ok(m) if m.is_file() => out.push(FileStat {
                path: name.to_string(),
                size: m.len(),
                mtime: mtime_ns(&m),
            }),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(&p)(e)),
        }
    }
    for name in CONFIG_DIRS {
        let root = project_dir.join(name);
        match fs::symlink_metadata(&root) {
            Ok(m) if m.is_dir() => {}
            _ => continue,
        }
        for entry in WalkDir::new(&root).follow_links(false) {
            let entry = entry.map_err(|e| walk_err(&root, e))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let meta = entry.metadata().map_err(|e| walk_err(&root, e))?;
            out.push(FileStat {
                path: rel_slash(project_dir, entry.path()),
                size: meta.len(),
                mtime: mtime_ns(&meta),
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn backups_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(BACKUPS_DIR)
}

/// Writes a snapshot (always, changed or not). Does not prune.
pub fn snapshot(project_dir: &Path, reason: BackupReason) -> Result<BackupInfo, BackupError> {
    snapshot_with_fingerprint(project_dir, reason).map(|(info, _)| info)
}

/// [`snapshot`], also returning the fingerprint recorded in its manifest.
pub fn snapshot_with_fingerprint(
    project_dir: &Path,
    reason: BackupReason,
) -> Result<(BackupInfo, Fingerprint), BackupError> {
    snapshot_at(project_dir, reason, &fsx::now_rfc3339())
}

/// [`snapshot`] with an explicit `createdAt` (RFC 3339, second precision).
pub(crate) fn snapshot_at(
    project_dir: &Path,
    reason: BackupReason,
    created_at: &str,
) -> Result<(BackupInfo, Fingerprint), BackupError> {
    let backups = backups_dir(project_dir);
    fs::create_dir_all(&backups).map_err(io_err(&backups))?;
    remove_stale_tmp(&backups);
    let tmp = backups.join(format!("{TMP_PREFIX}{}", nanoid::nanoid!(10)));
    let result = write_snapshot(project_dir, &tmp, reason, created_at)
        .and_then(|manifest| Ok((publish(&backups, &tmp, created_at, reason)?, manifest)));
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    let (id, manifest) = result?;
    Ok((manifest.info(id), manifest.files))
}

/// Leftovers of a crashed snapshot. Callers hold the `io` lock, so nothing is in progress.
fn remove_stale_tmp(backups: &Path) {
    let Ok(rd) = fs::read_dir(backups) else {
        return;
    };
    for entry in rd.flatten() {
        if entry.file_name().to_string_lossy().starts_with(TMP_PREFIX) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn write_snapshot(
    project_dir: &Path,
    tmp: &Path,
    reason: BackupReason,
    created_at: &str,
) -> Result<Manifest, BackupError> {
    fs::create_dir(tmp).map_err(io_err(tmp))?;
    let mut files = Vec::new();
    for stat in fingerprint(project_dir)? {
        let rel = rel_path(&stat.path);
        let src = project_dir.join(&rel);
        let dst = tmp.join(&rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        match fs::copy(&src, &dst) {
            Ok(_) => files.push(stat),
            // Deleted between the scan and the copy: it is simply not in this snapshot.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err(&src)(e)),
        }
    }
    let manifest = Manifest {
        schema_version: MANIFEST_VERSION,
        created_at: created_at.to_string(),
        reason,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        files,
    };
    let mpath = tmp.join(MANIFEST_FILE);
    fsx::write_json_atomic(&mpath, &manifest).map_err(io_err(&mpath))?;
    Ok(manifest)
}

/// Renames `tmp` to `<compact time>-<reason>[-N]` and returns the id.
fn publish(
    backups: &Path,
    tmp: &Path,
    created_at: &str,
    reason: BackupReason,
) -> Result<String, BackupError> {
    let compact = created_at.replace(['-', ':'], "");
    let base = format!("{compact}-{}", reason_slug(reason));
    for n in 1..=1000u32 {
        let id = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let target = backups.join(&id);
        if target.exists() {
            continue;
        }
        match fsx::rename_retrying(tmp, &target) {
            Ok(()) => return Ok(id),
            Err(_) if target.exists() => continue,
            Err(e) => return Err(io_err(&target)(e)),
        }
    }
    Err(BackupError::Invalid(format!(
        "Too many snapshots named {base}"
    )))
}

fn read_manifest(dir: &Path) -> Result<Manifest, BackupError> {
    let p = dir.join(MANIFEST_FILE);
    let bytes = fs::read(&p).map_err(io_err(&p))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| BackupError::Invalid(format!("{}: {e}", fsx::display_path(&p))))
}

/// Every valid snapshot with its manifest, newest first.
fn entries(project_dir: &Path) -> Result<Vec<(String, Manifest)>, BackupError> {
    let backups = backups_dir(project_dir);
    let rd = match fs::read_dir(&backups) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err(&backups)(e)),
    };
    let mut out: Vec<(SystemTime, String, Manifest)> = Vec::new();
    for entry in rd {
        let entry = entry.map_err(io_err(&backups))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.starts_with('.') || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(manifest) = read_manifest(&entry.path()) else {
            continue;
        };
        // Same-second snapshots are ordered by when their manifest was written.
        let written = fs::metadata(entry.path().join(MANIFEST_FILE))
            .and_then(|m| m.modified())
            .unwrap_or(UNIX_EPOCH);
        out.push((written, name, manifest));
    }
    out.sort_by(|a, b| {
        (b.2.created_at.as_str(), b.0, b.1.as_str()).cmp(&(
            a.2.created_at.as_str(),
            a.0,
            a.1.as_str(),
        ))
    });
    Ok(out.into_iter().map(|(_, id, m)| (id, m)).collect())
}

/// Snapshots, newest first. Reads only `manifest.json` files; directories without a valid
/// manifest and in-progress `.tmp-*` directories are ignored.
pub fn list(project_dir: &Path) -> Result<Vec<BackupInfo>, BackupError> {
    Ok(entries(project_dir)?
        .into_iter()
        .map(|(id, m)| m.info(id))
        .collect())
}

/// Fingerprint recorded in the newest snapshot's manifest, if any.
pub fn latest_fingerprint(project_dir: &Path) -> Result<Option<Fingerprint>, BackupError> {
    Ok(entries(project_dir)?
        .into_iter()
        .next()
        .map(|(_, m)| m.files))
}

/// Accepts only a plain directory name (no separators, no `..`, not hidden).
fn validate_id(id: &str) -> Result<(), BackupError> {
    let plain = !id.is_empty()
        && id.len() <= 255
        && !id.starts_with('.')
        && !id.contains(['/', '\\', ':'])
        && Path::new(id).components().count() == 1;
    if plain {
        Ok(())
    } else {
        Err(BackupError::Invalid(format!("Invalid backup id: {id}")))
    }
}

/// Directory of snapshot `id`, which must exist.
pub fn backup_path(project_dir: &Path, id: &str) -> Result<PathBuf, BackupError> {
    validate_id(id)?;
    let dir = backups_dir(project_dir).join(id);
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(BackupError::NotFound(id.to_string()))
    }
}

pub fn delete(project_dir: &Path, id: &str) -> Result<(), BackupError> {
    let dir = backup_path(project_dir, id)?;
    fs::remove_dir_all(&dir).map_err(io_err(&dir))
}

/// Deletes all but the newest `keep` snapshots. Returns how many were deleted.
pub fn prune(project_dir: &Path, keep: usize) -> Result<usize, BackupError> {
    let backups = backups_dir(project_dir);
    let mut removed = 0;
    for (id, _) in entries(project_dir)?.into_iter().skip(keep) {
        let dir = backups.join(&id);
        fs::remove_dir_all(&dir).map_err(io_err(&dir))?;
        removed += 1;
    }
    Ok(removed)
}

/// Makes the config set exactly match snapshot `id`: config-set files that are not in the
/// snapshot are removed (empty subfolders too), the snapshot's files are copied back. Never
/// touches anything outside the config set. The caller takes a `pre-restore` snapshot first and
/// reloads the project afterwards.
pub fn restore(project_dir: &Path, id: &str) -> Result<(), BackupError> {
    let snap = backup_path(project_dir, id)?;
    let manifest = read_manifest(&snap)?;
    let mut files = Vec::with_capacity(manifest.files.len());
    let mut keep = HashSet::new();
    for f in &manifest.files {
        let rel = safe_rel(&f.path).ok_or_else(|| {
            BackupError::Invalid(format!("Backup {id} lists an invalid path: {}", f.path))
        })?;
        if !snap.join(&rel).is_file() {
            return Err(BackupError::Invalid(format!(
                "Backup {id} is incomplete: {} is missing",
                f.path
            )));
        }
        keep.insert(f.path.clone());
        files.push(rel);
    }

    for cur in fingerprint(project_dir)? {
        if !keep.contains(&cur.path) {
            let p = project_dir.join(rel_path(&cur.path));
            fs::remove_file(&p).map_err(io_err(&p))?;
        }
    }
    for name in CONFIG_DIRS {
        remove_empty_dirs(&project_dir.join(name));
    }
    for rel in files {
        let src = snap.join(&rel);
        let dst = project_dir.join(&rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        fs::copy(&src, &dst).map_err(io_err(&dst))?;
    }
    Ok(())
}

/// Removes empty subdirectories below `root` (not `root` itself), deepest first.
fn remove_empty_dirs(root: &Path) {
    let walker = WalkDir::new(root)
        .min_depth(1)
        .contents_first(true)
        .follow_links(false);
    for entry in walker.into_iter().flatten() {
        if entry.file_type().is_dir() {
            // Fails (and is ignored) when the directory is not empty.
            let _ = fs::remove_dir(entry.path());
        }
    }
}

/// One step of change detection: snapshots `reason` unless the config set matches `last` or the
/// newest manifest. Prunes to `keep` after writing. Returns the snapshot written, if any.
pub(crate) fn snapshot_if_changed(
    project_dir: &Path,
    reason: BackupReason,
    last: &mut Option<Fingerprint>,
    keep: usize,
) -> Result<Option<BackupInfo>, BackupError> {
    let fp = fingerprint(project_dir)?;
    if last.as_ref() == Some(&fp) {
        return Ok(None);
    }
    if latest_fingerprint(project_dir)?.as_ref() == Some(&fp) {
        *last = Some(fp);
        return Ok(None);
    }
    let (info, recorded) = snapshot_with_fingerprint(project_dir, reason)?;
    *last = Some(recorded);
    if let Err(e) = prune(project_dir, keep) {
        tracing::warn!("pruning backups failed: {e}");
    }
    Ok(Some(info))
}

// ---- Timer worker -------------------------------------------------------------------------

/// Runs the closure with the owner's I/O lock held, after flushing pending store writes, and
/// only while the worker's project is still the open one. Returns `false` when the owner is gone
/// or the project changed, which ends the worker.
pub type Host = Box<dyn Fn(&mut dyn FnMut()) -> bool + Send>;

enum Msg {
    Reconfigure(BackupPrefs),
    Stop,
}

pub struct WorkerConfig {
    pub dir: PathBuf,
    pub prefs: BackupPrefs,
    /// Length of one interval "minute" (`INTERVAL_UNIT` in production).
    pub unit: Duration,
    /// Snapshot taken right after start (skipped if unchanged), e.g. `Open`. `None` only
    /// records the newest manifest's fingerprint as the baseline.
    pub initial: Option<BackupReason>,
}

/// Handle to one backup timer thread. Dropping it stops the thread promptly (it never joins).
pub struct Worker {
    dir: PathBuf,
    tx: Sender<Msg>,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(cfg: WorkerConfig, events: Arc<dyn EventSink>, host: Host) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let ctx = Ctx {
            dir: cfg.dir.clone(),
            prefs: cfg.prefs,
            unit: cfg.unit,
            events,
            host,
            stopped: stopped.clone(),
            last: None,
        };
        let initial = cfg.initial;
        let thread = std::thread::Builder::new()
            .name("backup-timer".into())
            .spawn(move || ctx.run(initial, rx))?;
        Ok(Self {
            dir: cfg.dir,
            tx,
            stopped,
            thread: Some(thread),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// New interval / retention take effect without restarting; `enabled: false` ends the thread.
    pub fn reconfigure(&self, prefs: BackupPrefs) {
        let _ = self.tx.send(Msg::Reconfigure(prefs));
    }

    /// Stops the worker and hands back its thread so a caller (tests) can wait for it.
    pub fn stop(mut self) -> Option<JoinHandle<()>> {
        self.thread.take()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Msg::Stop);
    }
}

struct Ctx {
    dir: PathBuf,
    prefs: BackupPrefs,
    unit: Duration,
    events: Arc<dyn EventSink>,
    host: Host,
    stopped: Arc<AtomicBool>,
    last: Option<Fingerprint>,
}

impl Ctx {
    fn interval(&self) -> Duration {
        self.unit.saturating_mul(self.prefs.interval_minutes.max(1))
    }

    fn run(mut self, initial: Option<BackupReason>, rx: Receiver<Msg>) {
        if !self.prefs.enabled || !self.step(initial) {
            return;
        }
        let mut last_tick = Instant::now();
        loop {
            let due = last_tick + self.interval();
            match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(Msg::Reconfigure(p)) => {
                    if !p.enabled {
                        return;
                    }
                    self.prefs = p;
                }
                Err(RecvTimeoutError::Timeout) => {
                    if !self.step(Some(BackupReason::Timer)) {
                        return;
                    }
                    last_tick = Instant::now();
                }
            }
        }
    }

    /// Runs one check through the host. Returns whether the worker should keep going.
    fn step(&mut self, reason: Option<BackupReason>) -> bool {
        let keep = self.prefs.retention.max(1) as usize;
        let dir = &self.dir;
        let stopped = &self.stopped;
        let last = &mut self.last;
        let mut keep_going = true;
        let mut outcome: Option<Result<Option<BackupInfo>, BackupError>> = None;
        let ran = (self.host)(&mut || {
            if stopped.load(Ordering::SeqCst) {
                keep_going = false;
                return;
            }
            match reason {
                None => *last = latest_fingerprint(dir).ok().flatten(),
                Some(r) => outcome = Some(snapshot_if_changed(dir, r, last, keep)),
            }
        });
        match outcome {
            Some(Ok(Some(info))) => match serde_json::to_value(&info) {
                Ok(v) => self.events.emit(names::BACKUP_CREATED, v),
                Err(e) => tracing::warn!("backup-created payload: {e}"),
            },
            Some(Err(e)) => {
                tracing::error!("automatic backup failed: {e}");
                self.events.emit(
                    names::BACKGROUND_ERROR,
                    serde_json::json!({ "message": format!("Automatic backup failed: {e}") }),
                );
            }
            Some(Ok(None)) | None => {}
        }
        ran && keep_going
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<(String, Value)>>);

    impl EventSink for Recorder {
        fn emit(&self, event: &str, payload: Value) {
            self.0.lock().unwrap().push((event.to_string(), payload));
        }
    }

    fn write(p: &Path, text: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    /// A project with the full config set plus assets and an existing backup dir.
    fn project() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        write(&d.join("project.json"), r#"{"name":"P"}"#);
        write(&d.join("ui-state.json"), "{}");
        write(&d.join("screens/main.json"), "{\"a\":1}");
        write(&d.join("templates/bar/template.html"), "<div></div>");
        write(&d.join("templates/bar/nested/deep.css"), "a{}");
        write(&d.join("sources/s.json"), "{}");
        write(&d.join("flows/f.json"), "{}");
        write(&d.join("chat-themes/bubble.css"), "b{}");
        write(&d.join("assets/logo.png"), "PNG");
        write(&d.join(".backups/old-stuff/file.txt"), "x");
        write(&d.join("notes.txt"), "not config");
        t
    }

    fn paths(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = WalkDir::new(dir)
            .into_iter()
            .map(|e| e.unwrap())
            .filter(|e| e.file_type().is_file())
            .map(|e| rel_slash(dir, e.path()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn snapshot_contains_exactly_the_config_set() {
        let t = project();
        let info = snapshot(t.path(), BackupReason::Manual).unwrap();
        assert!(info.id.ends_with("-manual"), "{}", info.id);
        assert_eq!(info.id.len(), "20261003T171200Z-manual".len());
        let snap = t.path().join(".backups").join(&info.id);
        assert_eq!(
            paths(&snap),
            [
                "chat-themes/bubble.css",
                "flows/f.json",
                "manifest.json",
                "project.json",
                "screens/main.json",
                "sources/s.json",
                "templates/bar/nested/deep.css",
                "templates/bar/template.html",
                "ui-state.json",
            ]
        );
        let m = read_manifest(&snap).unwrap();
        assert_eq!(m.schema_version, 1);
        assert_eq!(m.reason, BackupReason::Manual);
        assert_eq!(m.app_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(m.files.len(), 8);
        let main = m
            .files
            .iter()
            .find(|f| f.path == "screens/main.json")
            .unwrap();
        assert_eq!(main.size, 7);
        assert_eq!(info.files, 8);
        assert_eq!(info.bytes, m.files.iter().map(|f| f.size).sum::<u64>());
        // Manifest JSON uses camelCase and the kebab reason.
        let raw: Value =
            serde_json::from_slice(&fs::read(snap.join(MANIFEST_FILE)).unwrap()).unwrap();
        assert_eq!(raw["schemaVersion"], 1);
        assert_eq!(raw["reason"], "manual");
        assert!(raw["createdAt"].is_string() && raw["appVersion"].is_string());
        // No temp dirs left.
        assert!(fs::read_dir(t.path().join(".backups")).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tmp-")));
    }

    #[test]
    fn missing_optional_entries_are_skipped() {
        let t = tempfile::tempdir().unwrap();
        write(&t.path().join("project.json"), "{}");
        let info = snapshot(t.path(), BackupReason::Open).unwrap();
        assert_eq!(info.files, 1);
    }

    #[test]
    fn id_collision_gets_suffix() {
        let t = project();
        let at = "2026-10-03T17:12:00Z";
        let (a, _) = snapshot_at(t.path(), BackupReason::Manual, at).unwrap();
        let (b, _) = snapshot_at(t.path(), BackupReason::Manual, at).unwrap();
        let (c, _) = snapshot_at(t.path(), BackupReason::Manual, at).unwrap();
        assert_eq!(a.id, "20261003T171200Z-manual");
        assert_eq!(b.id, "20261003T171200Z-manual-2");
        assert_eq!(c.id, "20261003T171200Z-manual-3");
    }

    #[test]
    fn list_newest_first_ignoring_tmp_and_invalid() {
        let t = project();
        snapshot_at(t.path(), BackupReason::Open, "2026-10-01T10:00:00Z").unwrap();
        snapshot_at(t.path(), BackupReason::Timer, "2026-10-03T10:00:00Z").unwrap();
        snapshot_at(t.path(), BackupReason::Manual, "2026-10-02T10:00:00Z").unwrap();
        // A crashed snapshot and a manifest-less dir.
        write(
            &t.path().join(".backups/.tmp-abc/manifest.json"),
            &fs::read_to_string(
                t.path()
                    .join(".backups/20261003T100000Z-timer/manifest.json"),
            )
            .unwrap(),
        );
        write(
            &t.path().join(".backups/garbage/manifest.json"),
            "{not json",
        );
        let ids: Vec<String> = list(t.path()).unwrap().into_iter().map(|b| b.id).collect();
        assert_eq!(
            ids,
            [
                "20261003T100000Z-timer",
                "20261002T100000Z-manual",
                "20261001T100000Z-open"
            ]
        );
    }

    #[test]
    fn list_without_backups_dir_is_empty() {
        let t = tempfile::tempdir().unwrap();
        assert!(list(t.path()).unwrap().is_empty());
        assert!(latest_fingerprint(t.path()).unwrap().is_none());
    }

    #[test]
    fn prune_keeps_newest() {
        let t = project();
        for day in 1..=5 {
            let at = format!("2026-10-0{day}T10:00:00Z");
            snapshot_at(t.path(), BackupReason::Timer, &at).unwrap();
        }
        assert_eq!(prune(t.path(), 2).unwrap(), 3);
        let ids: Vec<String> = list(t.path()).unwrap().into_iter().map(|b| b.id).collect();
        assert_eq!(ids, ["20261005T100000Z-timer", "20261004T100000Z-timer"]);
        // Unrelated directories are left alone.
        assert!(t.path().join(".backups/old-stuff/file.txt").exists());
        assert_eq!(prune(t.path(), 2).unwrap(), 0);
    }

    #[test]
    fn delete_validates_id() {
        let t = project();
        let info = snapshot(t.path(), BackupReason::Manual).unwrap();
        for bad in ["..", "../x", "a/b", r"a\b", "", ".tmp-x", "C:x"] {
            assert!(
                matches!(delete(t.path(), bad), Err(BackupError::Invalid(_))),
                "{bad}"
            );
        }
        assert!(matches!(
            delete(t.path(), "20990101T000000Z-manual"),
            Err(BackupError::NotFound(_))
        ));
        delete(t.path(), &info.id).unwrap();
        assert!(list(t.path()).unwrap().is_empty());
        let e: AppError = BackupError::NotFound("x".into()).into();
        assert_eq!(e.code(), "notFound");
    }

    #[test]
    fn restore_makes_config_set_match() {
        let t = project();
        let d = t.path();
        let info = snapshot(d, BackupReason::Manual).unwrap();
        let before: Vec<String> = fingerprint(d)
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect();

        write(&d.join("screens/added.json"), "{}");
        write(&d.join("templates/new/x/y.html"), "<p>");
        write(&d.join("screens/main.json"), "{\"a\":2,\"b\":3}");
        fs::remove_file(d.join("sources/s.json")).unwrap();
        write(&d.join("assets/new.png"), "PNG2");

        restore(d, &info.id).unwrap();
        let after: Vec<String> = fingerprint(d)
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect();
        assert_eq!(after, before);
        assert_eq!(
            fs::read_to_string(d.join("screens/main.json")).unwrap(),
            "{\"a\":1}"
        );
        assert!(!d.join("templates/new").exists());
        assert!(d.join("templates").is_dir());
        // Outside the config set: untouched.
        assert!(d.join("assets/new.png").exists());
        assert!(d.join("assets/logo.png").exists());
        assert!(d.join("notes.txt").exists());
        assert!(d.join(".backups/old-stuff/file.txt").exists());
        assert!(matches!(
            restore(d, "../escape"),
            Err(BackupError::Invalid(_))
        ));
    }

    #[test]
    fn restore_removes_files_absent_from_snapshot() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        write(&d.join("project.json"), "{}");
        let info = snapshot(d, BackupReason::Manual).unwrap();
        write(&d.join("ui-state.json"), "{}");
        restore(d, &info.id).unwrap();
        assert!(!d.join("ui-state.json").exists());
    }

    #[test]
    fn restore_rejects_paths_outside_config_set() {
        let t = project();
        let info = snapshot(t.path(), BackupReason::Manual).unwrap();
        let mpath = t.path().join(".backups").join(&info.id).join(MANIFEST_FILE);
        let mut m = read_manifest(mpath.parent().unwrap()).unwrap();
        m.files.push(FileStat {
            path: "assets/logo.png".into(),
            size: 3,
            mtime: 0,
        });
        fsx::write_json_atomic(&mpath, &m).unwrap();
        assert!(matches!(
            restore(t.path(), &info.id),
            Err(BackupError::Invalid(_))
        ));
    }

    #[test]
    fn fingerprint_tracks_size_and_mtime() {
        let t = project();
        let d = t.path();
        let a = fingerprint(d).unwrap();
        assert_eq!(a, fingerprint(d).unwrap());
        assert!(a.iter().all(|f| !f.path.starts_with("assets")));

        let f = fs::OpenOptions::new()
            .write(true)
            .open(d.join("screens/main.json"))
            .unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();
        drop(f);
        let b = fingerprint(d).unwrap();
        assert_ne!(a, b);

        write(&d.join("screens/main.json"), "{\"a\":12345}");
        assert_ne!(b, fingerprint(d).unwrap());
    }

    #[test]
    fn unchanged_state_skips_snapshot() {
        let t = project();
        let d = t.path();
        let mut last = None;
        let first = snapshot_if_changed(d, BackupReason::Timer, &mut last, 10).unwrap();
        assert!(first.is_some());
        assert!(snapshot_if_changed(d, BackupReason::Timer, &mut last, 10)
            .unwrap()
            .is_none());
        // Fresh baseline (app restart): the newest manifest still matches.
        let mut fresh = None;
        assert!(snapshot_if_changed(d, BackupReason::Open, &mut fresh, 10)
            .unwrap()
            .is_none());
        write(&d.join("sources/s.json"), "{\"changed\":true}");
        assert!(snapshot_if_changed(d, BackupReason::Timer, &mut last, 10)
            .unwrap()
            .is_some());
        assert_eq!(list(d).unwrap().len(), 2);
    }

    fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn spawn_test_worker(dir: &Path, enabled: bool, rec: Arc<Recorder>) -> Worker {
        let host: Host = Box::new(|f: &mut dyn FnMut()| {
            f();
            true
        });
        Worker::spawn(
            WorkerConfig {
                dir: dir.to_path_buf(),
                prefs: BackupPrefs {
                    enabled,
                    interval_minutes: 1,
                    retention: 10,
                },
                unit: Duration::from_millis(50),
                initial: Some(BackupReason::Open),
            },
            rec,
            host,
        )
        .unwrap()
    }

    #[test]
    fn timer_worker_snapshots_only_on_change_and_stops_promptly() {
        let t = project();
        let d = t.path();
        let rec = Arc::new(Recorder::default());
        let w = spawn_test_worker(d, true, rec.clone());

        wait_until("open snapshot", || list(d).unwrap().len() == 1);
        assert_eq!(list(d).unwrap()[0].reason, BackupReason::Open);
        std::thread::sleep(Duration::from_millis(200)); // ~4 ticks, nothing changed
        assert_eq!(list(d).unwrap().len(), 1);

        write(&d.join("screens/main.json"), "{\"a\":1,\"changed\":1}");
        wait_until("timer snapshot", || list(d).unwrap().len() == 2);
        assert_eq!(list(d).unwrap()[0].reason, BackupReason::Timer);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(list(d).unwrap().len(), 2);

        let created = || -> Vec<Value> {
            rec.0
                .lock()
                .unwrap()
                .iter()
                .filter(|(n, _)| n == names::BACKUP_CREATED)
                .map(|(_, v)| v.clone())
                .collect()
        };
        // The event follows the snapshot folder.
        wait_until("both events", || created().len() >= 2);
        let created = created();
        assert_eq!(created.len(), 2);
        assert_eq!(created[0]["reason"], "open");
        assert_eq!(created[1]["reason"], "timer");

        let handle = w.stop().unwrap();
        wait_until("worker exit", || handle.is_finished());
    }

    #[test]
    fn reconfigure_disabled_ends_worker() {
        let t = project();
        let rec = Arc::new(Recorder::default());
        let mut w = spawn_test_worker(t.path(), true, rec);
        let handle = w.thread.take().unwrap();
        w.reconfigure(BackupPrefs {
            enabled: false,
            ..Default::default()
        });
        wait_until("worker exit", || handle.is_finished());
    }

    #[test]
    fn disabled_worker_takes_no_snapshots() {
        let t = project();
        let rec = Arc::new(Recorder::default());
        let w = spawn_test_worker(t.path(), false, rec.clone());
        let handle = w.stop().unwrap();
        wait_until("worker exit", || handle.is_finished());
        std::thread::sleep(Duration::from_millis(100));
        assert!(list(t.path()).unwrap().is_empty());
        assert!(rec.0.lock().unwrap().is_empty());
    }
}
