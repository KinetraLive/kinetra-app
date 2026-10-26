//! Restoring a version (CLOUD_PROJECTS.md §0 rules 2 and 5, §10 "Restoring"; the app plan's
//! decision 1). Always a user action, one at a time, in three ways:
//!
//! - **New folder** (the history's default, and the conflict notice's "Open the other version"):
//!   a separate project with its own cloudId next to the open one.
//! - **Download** (the picker's "In the cloud"): a new folder that continues the same cloud
//!   backup; when this computer already has a folder for that project, the download is a
//!   separate project instead (two folders never share a cloudId).
//! - **Replace** the open project: refused while anything is on air; the project is backed up
//!   first (or, when it can't be, copied next to it), the version is restored into a staging
//!   folder inside the project, everything is verified, and only then are the files swapped
//!   (old files moved aside first, moved back if anything fails). The next backup says
//!   `restored_from`.
//!
//! **Every file is checked** (§0 rule 5): downloaded to a temporary file, hashed while it's
//! written, and moved into place only when its SHA-256 matches the manifest; a mismatch is
//! downloaded once more, then reported damaged. A new folder keeps the other files and lists the
//! damaged ones; Replace stops before touching the project. Local files with the right hash
//! (this project, or the folder this computer has for that cloud project) are copied instead of
//! downloaded, and re-hashed after the copy. Paths come from the server: each one is checked
//! against §8.2 (no `..`, no drive letters, no reserved names) before anything is written.
//!
//! Downloads follow the on-air policy like uploads (§2.2): paused (a transfer in flight is
//! dropped and started again later) or paced. One transfer at a time in the whole app (the
//! backup slot). Nothing here runs at rest.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tokio::sync::watch;

use super::gate::{may_send, must_pause, Mode, Pacer};
use super::paths::{self, Unique};
use super::r2::{R2Error, R2};
use super::scan::{self, BackgroundPriority, HashCache};
use super::service::PRO_CLOUD;
use super::wire::{FileEntry, Presigned};
use crate::core::cloud::api::{Api, CallError};
use crate::core::model::{
    CloudRestoreMode, CloudRestorePhase, CloudRestorePlan, CloudRestoreRequest, CloudRestoreResult,
    CloudRestoreView, ProjectFile, CLOUD_REPORT_MAX,
};
use crate::core::persist;
use crate::core::remote::ids::uuid_v7;
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

/// Download URLs asked for at a time (just before they're used: they expire after 10 minutes).
const URLS_PER_CALL: usize = 50;
/// Progress events at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
/// Downloads per file: the first, and one retry after a mismatch (§0 rule 5).
const TRIES: u32 = 2;
/// Expired URLs asked for again per file, at most.
const RENEWALS: u32 = 3;
const BUF: usize = 1024 * 1024;

/// The restore running or just finished (in the service's state; the view reads `view`).
#[derive(Debug)]
pub(super) struct RestoreState {
    pub view: CloudRestoreView,
    /// The running task and its cancel flag.
    pub task: Option<(tokio::task::AbortHandle, Arc<AtomicBool>)>,
    last_emit: Option<Instant>,
}

/// Why a restore stopped.
#[derive(Debug)]
enum Stop {
    Canceled,
    Offline,
    Message(String),
}

impl From<CallError> for Stop {
    fn from(e: CallError) -> Self {
        if e.is_offline() || matches!(&e, CallError::Api { status, .. } if *status >= 500) {
            return Stop::Offline;
        }
        match e {
            CallError::Api {
                status: 404, code, ..
            } if code == "not_found" => {
                Stop::Message("That version isn't in the cloud any more.".into())
            }
            CallError::Api { code, message, .. } => {
                Stop::Message(super::backup::message_for(&code).unwrap_or(message))
            }
            CallError::Offline => Stop::Offline,
        }
    }
}

impl From<AppError> for Stop {
    fn from(e: AppError) -> Self {
        match e {
            AppError::Offline => Stop::Offline,
            other => Stop::Message(other.to_string()),
        }
    }
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The test hook: corrupts the first `KINETRA_CORRUPT_DOWNLOADS` blobs downloaded by this app
/// (one byte of each), to try the retry and "damaged" paths against the real dev API. Debug
/// builds only (`release_guard`, `scripts/check-release-overrides.mjs`).
#[cfg(debug_assertions)]
pub fn corrupt_downloads() -> u32 {
    std::env::var("KINETRA_CORRUPT_DOWNLOADS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Whether the hook corrupts this download (counts down).
fn corrupt_this_one() -> bool {
    #[cfg(debug_assertions)]
    {
        static LEFT: std::sync::OnceLock<std::sync::atomic::AtomicU32> = std::sync::OnceLock::new();
        let left = LEFT.get_or_init(|| std::sync::atomic::AtomicU32::new(corrupt_downloads()));
        left.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
    }
    #[cfg(not(debug_assertions))]
    false
}

// ---- Paths and folders (pure) -------------------------------------------------------------------

/// `rel` (a manifest path, `/`-separated) under `root`, or `None` when it may not be written
/// here: §8.2 (which rules out `..`, roots, drive letters, reserved names), or one of the names
/// a backup never holds (`.backups/`, `.git/`, temp and restore files).
pub fn target(root: &Path, rel: &str) -> Option<PathBuf> {
    if paths::problem(rel).is_some() {
        return None;
    }
    let segs: Vec<&str> = rel.split('/').collect();
    let last = segs.len() - 1;
    for (i, s) in segs.iter().enumerate() {
        if scan::excluded(s, i < last, i == 0) {
            return None;
        }
    }
    Some(segs.iter().fold(root.to_path_buf(), |p, s| p.join(s)))
}

/// "2026-10-15-1402": `rfc3339` in local time, for folder names.
pub fn stamp(rfc3339: &str) -> Option<String> {
    let t = time::OffsetDateTime::parse(rfc3339, &time::format_description::well_known::Rfc3339)
        .ok()?;
    let ms = i64::try_from(t.unix_timestamp_nanos() / 1_000_000).ok()?;
    let l = crate::core::localtime::Zone::System.to_local(ms);
    Some(format!(
        "{:04}-{:02}-{:02}-{:02}{:02}",
        l.year(),
        u8::from(l.month()),
        l.day(),
        l.hour(),
        l.minute()
    ))
}

/// `parent/leaf`, or `parent/leaf-2`, `-3`… when that exists.
pub fn free_folder(parent: &Path, leaf: &str) -> PathBuf {
    let first = parent.join(leaf);
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| parent.join(format!("{leaf}-{n}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// A folder restore may write: absolute, its parent exists, and it doesn't exist or is empty.
fn check_new_folder(folder: &Path) -> AppResult<()> {
    if !folder.is_absolute() {
        return Err(AppError::Invalid("Choose a folder for the project.".into()));
    }
    match folder.parent() {
        Some(p) if p.is_dir() => {}
        _ => return Err(AppError::Invalid("That location doesn't exist.".into())),
    }
    match std::fs::read_dir(folder).map(|mut e| e.next().is_none()) {
        Ok(true) => Ok(()),
        Ok(false) => Err(AppError::Invalid(
            "A folder with that name already exists there and isn't empty.".into(),
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) if folder.exists() => Err(AppError::Invalid(
            "A file with that name already exists there.".into(),
        )),
        Err(e) => Err(AppError::Invalid(format!(
            "Kinetra can't use that folder: {e}"
        ))),
    }
}

/// The time a file had when it was backed up (`modified_at`).
fn set_mtime(path: &Path, rfc3339: &str) {
    let Ok(t) =
        time::OffsetDateTime::parse(rfc3339, &time::format_description::well_known::Rfc3339)
    else {
        return;
    };
    let Ok(ns) = u64::try_from(t.unix_timestamp_nanos()) else {
        return;
    };
    let when = UNIX_EPOCH + Duration::from_nanos(ns);
    if let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_modified(when);
    }
}

/// Writes `id` / `cloudId` into a restored `project.json`, keeping everything else (a newer
/// version's fields too). `None` leaves a key as it is.
fn set_project_ids(
    dir: &Path,
    cloud_id: Option<&str>,
    remote_id: Option<&str>,
) -> std::io::Result<()> {
    let path = dir.join("project.json");
    let bytes = std::fs::read(&path)?;
    let mut v: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let Some(o) = v.as_object_mut() else {
        return Ok(());
    };
    if let Some(c) = cloud_id {
        o.insert("cloudId".into(), c.into());
    }
    if let Some(r) = remote_id {
        o.insert("id".into(), r.into());
    }
    crate::core::fsx::write_json_atomic(&path, &v)
}

/// Copies a project folder (everything but restore staging; links skipped) to `to`, at
/// background priority. `progress(bytes)` after each file.
fn copy_project(
    from: &Path,
    to: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> std::io::Result<()> {
    let _p = BackgroundPriority::enter();
    std::fs::create_dir_all(to)?;
    let mut walk = walkdir::WalkDir::new(from)
        .follow_links(false)
        .min_depth(1)
        .into_iter();
    while let Some(entry) = walk.next() {
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("canceled"));
        }
        let entry = entry.map_err(std::io::Error::other)?;
        let rel = entry
            .path()
            .strip_prefix(from)
            .map_err(std::io::Error::other)?;
        let name = entry.file_name().to_string_lossy();
        let ft = entry.file_type();
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if entry.depth() == 1 && name.to_lowercase().starts_with(".kinetra-restore-") {
                walk.skip_current_dir();
                continue;
            }
            std::fs::create_dir_all(to.join(rel))?;
        } else if ft.is_file() {
            let n = std::fs::copy(entry.path(), to.join(rel))?;
            progress(n);
        }
    }
    Ok(())
}

/// The bytes a project folder holds (for the copy's progress).
fn folder_bytes(dir: &Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Swaps the open project's files for the staged version (Replace). `placed`: the manifest
/// paths and where their bytes are staged; `in_place`: paths whose local file already is the
/// version's. Every other file a backup would hold is moved aside first, then the staged files
/// move in; if any move fails, everything moved goes back and the project is as it was. The
/// files moved aside are deleted at the end (the project was backed up or copied first). Files a
/// backup leaves out (`.backups/`, `.git/`, links, names it can't store) are never touched.
pub fn swap_into(
    dir: &Path,
    placed: &[(String, PathBuf)],
    in_place: &BTreeSet<String>,
    aside: &Path,
) -> std::io::Result<()> {
    let listing = scan::list(dir, None)?;
    let old: Vec<(PathBuf, PathBuf)> = listing
        .files
        .iter()
        .filter(|f| !in_place.contains(&f.path))
        .filter_map(|f| target(aside, &f.path).map(|to| (f.abs.clone(), to)))
        .collect();
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let undo = |moved: &[(PathBuf, PathBuf)]| {
        for (from, to) in moved.iter().rev() {
            if let Err(e) = std::fs::rename(to, from) {
                tracing::error!("restore: a file couldn't be moved back: {e}");
            }
        }
    };
    for (from, to) in &old {
        let r = to
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::rename(from, to));
        if let Err(e) = r {
            undo(&moved);
            return Err(e);
        }
        moved.push((from.clone(), to.clone()));
    }
    let mut new: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (rel, staged) in placed {
        let Some(to) = target(dir, rel) else { continue };
        let r = to
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::rename(staged, &to));
        if let Err(e) = r {
            undo(&new);
            undo(&moved);
            return Err(e);
        }
        new.push((staged.clone(), to));
    }
    if let Err(e) = std::fs::remove_dir_all(aside) {
        tracing::warn!("restore: the old files weren't removed: {e}");
    }
    Ok(())
}

/// Removes the restore's own folders when it ends (kept: the staging folder became the result).
struct Scratch {
    dirs: Vec<PathBuf>,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for d in &self.dirs {
            if d.exists() {
                if let Err(e) = std::fs::remove_dir_all(d) {
                    tracing::warn!("restore: a temporary folder wasn't removed: {e}");
                }
            }
        }
    }
}

// ---- The run ----------------------------------------------------------------------------------

/// Everything one run needs.
struct Run {
    api: Api,
    session: String,
    r2: R2,
    on_air: watch::Receiver<Mode>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicU64>,
    /// Temporary downloads (removed at the end).
    tmp: PathBuf,
}

/// One download: what arrived, or why not.
enum Got {
    /// The bytes match.
    Ok,
    /// They don't (or the size doesn't): try again, then it's damaged.
    Mismatch,
    /// The URL expired: ask for a new one.
    Expired,
}

/// What the run ended with (beyond the view).
#[derive(Default)]
struct Outcome {
    damaged: Vec<String>,
    copy_folder: Option<String>,
    reused: u32,
}

impl AppState {
    fn restore_state(&self, f: impl FnOnce(&mut RestoreState)) {
        if let Some(r) = lock(&self.cloud_projects().inner).restore.as_mut() {
            f(r);
        }
    }

    fn restore_phase(&self, phase: CloudRestorePhase, total: u64, run_done: &AtomicU64) {
        run_done.store(0, Ordering::Relaxed);
        self.restore_state(|r| {
            r.view.phase = phase;
            r.view.done_bytes = 0;
            r.view.total_bytes = total;
            r.view.paused = false;
            r.last_emit = None;
        });
        self.emit_cloud();
    }

    /// Progress (≤ 10 events/s) and the paused flag (at once).
    fn restore_progress(&self, done: u64, paused: Option<bool>) {
        let mut send = false;
        let limited = matches!(self.cloud_gate().mode(), Mode::Limit(_));
        self.restore_state(|r| {
            r.view.done_bytes = done;
            send |= r.view.limited != limited;
            r.view.limited = limited;
            if let Some(p) = paused {
                send |= r.view.paused != p;
                r.view.paused = p;
            }
            if send || r.last_emit.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
                r.last_emit = Some(Instant::now());
                send = true;
            }
        });
        if send {
            self.emit_cloud();
        }
    }

    /// Whether Replace backs the project up first (else it copies it).
    fn cloud_replace_backs_up(&self) -> bool {
        self.require_new_pro_work(PRO_CLOUD).is_ok()
            && self.cloud_backup_on()
            && self.cloud_writable()
            && self.cloud_record().is_some_and(|r| r.server_id.is_some())
    }

    /// Where a restore would go, and (Replace) what happens first. `name`: the cloud project's
    /// name (Download); `at`: the version's time (a new folder is named after it).
    pub fn cloud_restore_plan(
        &self,
        mode: CloudRestoreMode,
        name: &str,
        at: Option<&str>,
    ) -> AppResult<CloudRestorePlan> {
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        match mode {
            CloudRestoreMode::Replace => {
                let dir = self.project_dir()?;
                let backup_first = self.cloud_replace_backs_up();
                Ok(CloudRestorePlan {
                    folder: dir.to_string_lossy().into_owned(),
                    backup_first,
                    copy_folder: (!backup_first)
                        .then(|| copy_folder_for(&dir, &now))
                        .flatten()
                        .map(|p| p.to_string_lossy().into_owned()),
                })
            }
            CloudRestoreMode::NewFolder => {
                let dir = self.project_dir()?;
                let parent = dir.parent().ok_or(AppError::NoProject)?;
                let leaf = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "project".into());
                let when = stamp(at.unwrap_or(&now)).unwrap_or_else(|| "restored".into());
                Ok(CloudRestorePlan {
                    folder: free_folder(parent, &format!("{leaf}-{when}"))
                        .to_string_lossy()
                        .into_owned(),
                    backup_first: false,
                    copy_folder: None,
                })
            }
            CloudRestoreMode::Download => {
                let parent = self
                    .recents()
                    .iter()
                    .map(|r| PathBuf::from(&r.path))
                    .find(|p| p.is_dir())
                    .and_then(|p| p.parent().map(Path::to_path_buf))
                    .or_else(documents_dir)
                    .ok_or_else(|| AppError::Invalid("Choose a folder for the project.".into()))?;
                let leaf = crate::core::project::slug(name);
                Ok(CloudRestorePlan {
                    folder: free_folder(&parent, &leaf).to_string_lossy().into_owned(),
                    backup_first: false,
                    copy_folder: None,
                })
            }
        }
    }

    /// Starts a restore (returns at once; `cloud-changed` follows it). See the module docs.
    pub fn cloud_restore_start(&self, req: CloudRestoreRequest) -> AppResult<()> {
        self.cloud_history_access()?;
        if !super::api::is_id(&req.project_id)
            || req
                .version_id
                .as_deref()
                .is_some_and(|v| !super::api::is_id(v))
        {
            return Err(AppError::Invalid("Not a cloud version".into()));
        }
        let folder = match req.mode {
            CloudRestoreMode::Replace => {
                let dir = self.project_dir()?;
                let server = self.cloud_record().and_then(|r| r.server_id);
                if server.as_deref() != Some(req.project_id.as_str()) {
                    return Err(AppError::Invalid(
                        "That version belongs to another project.".into(),
                    ));
                }
                if self.cloud_on_air() {
                    return Err(AppError::Invalid(
                        "The project can't be replaced while anything is on air.".into(),
                    ));
                }
                dir
            }
            CloudRestoreMode::NewFolder | CloudRestoreMode::Download => {
                let f =
                    PathBuf::from(req.folder.as_deref().ok_or_else(|| {
                        AppError::Invalid("Choose a folder for the project.".into())
                    })?);
                check_new_folder(&f)?;
                f
            }
        };
        let rt = self
            .live()
            .runtime_handle()
            .ok_or_else(|| AppError::Internal("No runtime".into()))?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut s = lock(&self.cloud_projects().inner);
            if s.restore.as_ref().is_some_and(|r| r.task.is_some()) {
                return Err(AppError::Invalid("A restore is already running.".into()));
            }
            s.restore = Some(RestoreState {
                view: CloudRestoreView {
                    mode: req.mode,
                    version_id: req.version_id.clone().unwrap_or_default(),
                    folder: folder.to_string_lossy().into_owned(),
                    phase: CloudRestorePhase::Preparing,
                    done_bytes: 0,
                    total_bytes: 0,
                    paused: false,
                    limited: false,
                    result: None,
                },
                task: None,
                last_emit: None,
            });
        }
        let weak = self.downgrade();
        let flag = cancel.clone();
        let task = rt.spawn(async move {
            let Some(st) = weak.upgrade() else { return };
            st.cloud_gate_active(true);
            let mut out = Outcome::default();
            let result = st.cloud_restore_run(&req, &folder, flag, &mut out).await;
            let damaged_count = out.damaged.len() as u32;
            let mut damaged = out.damaged;
            damaged.truncate(CLOUD_REPORT_MAX);
            let error = match result {
                Ok(()) => None,
                Err(Stop::Canceled) => Some("The restore was canceled.".to_string()),
                Err(Stop::Offline) => Some(
                    "Kinetra couldn't reach cloud backup. Try again when you're online.".into(),
                ),
                Err(Stop::Message(m)) => Some(m),
            };
            st.restore_state(|r| {
                r.task = None;
                r.view.paused = false;
                r.view.result = Some(CloudRestoreResult {
                    ok: error.is_none(),
                    error,
                    damaged,
                    damaged_count,
                    copy_folder: out.copy_folder,
                    reused: out.reused,
                });
            });
            st.cloud_gate_active(st.cloud_backups_running());
            st.emit_cloud();
        });
        self.restore_state(|r| r.task = Some((task.abort_handle(), cancel)));
        self.emit_cloud();
        Ok(())
    }

    /// Stops a running restore (nothing in the project changes; its temporary files go).
    pub fn cloud_restore_cancel(&self) {
        if let Some((_, flag)) = lock(&self.cloud_projects().inner)
            .restore
            .as_ref()
            .and_then(|r| r.task.as_ref())
        {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Forgets a finished restore's result.
    pub fn cloud_restore_dismiss(&self) {
        {
            let mut s = lock(&self.cloud_projects().inner);
            if s.restore.as_ref().is_some_and(|r| r.task.is_none()) {
                s.restore = None;
            }
        }
        self.emit_cloud();
    }

    /// Whether a restore runs (tests).
    pub fn cloud_restore_running(&self) -> bool {
        lock(&self.cloud_projects().inner)
            .restore
            .as_ref()
            .is_some_and(|r| r.task.is_some())
    }

    async fn cloud_restore_run(
        &self,
        req: &CloudRestoreRequest,
        folder: &Path,
        cancel: Arc<AtomicBool>,
        out: &mut Outcome,
    ) -> Result<(), Stop> {
        let (api, session, _) = self.cloud_session_parts()?;
        let project = api.cloud_project(&session, &req.project_id).await?;
        let vid = match &req.version_id {
            Some(v) => v.clone(),
            None => project
                .head
                .as_ref()
                .map(|h| h.id.clone())
                .ok_or_else(|| Stop::Message("This project has no versions yet.".into()))?,
        };
        self.restore_state(|r| r.view.version_id = vid.clone());
        let manifest = api.cloud_manifest(&session, &req.project_id, &vid).await?;
        let open = self.project_cloud_id();
        let replace = req.mode == CloudRestoreMode::Replace;
        let done: Arc<AtomicU64> = Arc::default();

        // Which paths may be written, each once (ignoring case); the rest is damaged.
        let mut unique = Unique::default();
        let mut entries: Vec<FileEntry> = Vec::new();
        for f in manifest.manifest.files {
            if target(folder, &f.path).is_some() && unique.insert(&f.path) {
                entries.push(f);
            } else {
                out.damaged.push(f.path);
            }
        }
        if replace && !out.damaged.is_empty() {
            return Err(Stop::Message(damaged_message(out.damaged.len(), true)));
        }

        // Replace: the project as it is goes to the cloud (or next to it) first.
        if replace {
            self.restore_keep_current(folder, &cancel, &done, out)
                .await?;
        }

        // One transfer at a time in the whole app.
        let _slot = self.cloud_projects().slot.lock().await;
        if cancel.load(Ordering::Relaxed) {
            return Err(Stop::Canceled);
        }
        let id8: String = uuid_v7().chars().rev().take(8).collect();
        let base = if replace {
            folder.to_path_buf()
        } else {
            folder
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| Stop::Message("That location doesn't exist.".into()))?
        };
        let staging = base.join(format!(".kinetra-restore-{id8}"));
        let tmp = base.join(format!(".kinetra-restore-{id8}-tmp"));
        let mut scratch = Scratch {
            dirs: vec![staging.clone(), tmp.clone()],
        };
        std::fs::create_dir_all(&staging)
            .and_then(|()| std::fs::create_dir_all(&tmp))
            .map_err(|e| Stop::Message(format!("Kinetra can't write there: {e}")))?;

        // Local files with the right hash (re-hashed when used): this project, or the folder this
        // computer has for that cloud project.
        let mut sources: Vec<(PathBuf, String)> = Vec::new();
        if let Some((dir, Some(cid))) = &open {
            if replace || req.mode == CloudRestoreMode::NewFolder {
                sources.push((dir.clone(), cid.clone()));
            }
        }
        let recorded = self
            .cloud_file()
            .folders
            .get(&project.app_project_id)
            .map(PathBuf::from)
            .filter(|d| {
                super::identity::cloud_id_at(d).as_deref() == Some(project.app_project_id.as_str())
            });
        if let Some(d) = &recorded {
            if !sources.iter().any(|(s, _)| super::local::same_folder(s, d)) {
                sources.push((d.clone(), project.app_project_id.clone()));
            }
        }
        let have = local_hashes(self.config_dir(), &sources);

        // Group the paths by blob; in Replace, a path whose local file already is the version's
        // stays as it is.
        let mut by_sha: BTreeMap<String, (u64, Vec<FileEntry>)> = BTreeMap::new();
        for e in entries {
            by_sha
                .entry(e.sha256.clone())
                .or_insert((e.size, Vec::new()))
                .1
                .push(e);
        }
        let mut in_place: BTreeSet<String> = BTreeSet::new();
        if replace {
            let candidates: Vec<(String, String, u64)> = by_sha
                .iter()
                .flat_map(|(sha, (size, es))| {
                    es.iter().map(move |e| (e.path.clone(), sha.clone(), *size))
                })
                .collect();
            let dir = folder.to_path_buf();
            let flag = cancel.clone();
            in_place = tokio::task::spawn_blocking(move || same_in_place(&dir, &candidates, &flag))
                .await
                .map_err(|_| Stop::Canceled)?;
        }
        let todo: Vec<(String, u64, Vec<FileEntry>)> = by_sha
            .into_iter()
            .filter_map(|(sha, (size, es))| {
                let es: Vec<FileEntry> = es
                    .into_iter()
                    .filter(|e| !in_place.contains(&e.path))
                    .collect();
                (!es.is_empty()).then_some((sha, size, es))
            })
            .collect();

        let r2 = R2::new().map_err(Stop::Message)?;
        let run = Run {
            api,
            session,
            r2,
            on_air: self.cloud_projects().gate.subscribe(),
            cancel: cancel.clone(),
            done,
            tmp,
        };
        let total: u64 = todo.iter().map(|(_, size, _)| *size).sum();
        self.restore_phase(CloudRestorePhase::Downloading, total, &run.done);
        let mut placed: Vec<(String, PathBuf)> = Vec::new();
        let mut urls: HashMap<String, Presigned> = HashMap::new();
        let mut missing_on_server: BTreeSet<String> = BTreeSet::new();
        for (i, (sha, size, es)) in todo.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(Stop::Canceled);
            }
            let first = staged_path(&staging, &es[0].path)?;
            let base_done = run.done.load(Ordering::Relaxed);
            let mut ok = false;
            // A copy of a local file with this hash, checked after copying.
            if let Some(srcs) = have.get(sha) {
                let (srcs, dst, sha2, size2, flag) = (
                    srcs.clone(),
                    first.clone(),
                    sha.clone(),
                    *size,
                    cancel.clone(),
                );
                ok = tokio::task::spawn_blocking(move || {
                    srcs.iter()
                        .any(|s| copy_verified(s, &dst, &sha2, size2, &flag))
                })
                .await
                .unwrap_or(false);
                if ok {
                    out.reused += 1;
                }
            }
            if !ok {
                if !urls.contains_key(sha) && !missing_on_server.contains(sha) {
                    // The URLs for this blob and the next few (just in time).
                    let batch: Vec<String> = todo[i..]
                        .iter()
                        .map(|(s, ..)| s.clone())
                        .filter(|s| !have.contains_key(s) || s == sha)
                        .take(URLS_PER_CALL)
                        .collect();
                    let d = run.api.cloud_downloads(&run.session, &batch).await?;
                    missing_on_server.extend(d.missing);
                    for u in d.urls {
                        urls.insert(u.sha256, u.get);
                    }
                }
                ok = self
                    .restore_fetch(&run, sha, *size, &first, &mut urls, &missing_on_server)
                    .await?;
            }
            run.done.store(base_done + size, Ordering::Relaxed);
            self.restore_progress(base_done + size, None);
            if !ok {
                out.damaged.extend(es.iter().map(|e| e.path.clone()));
                continue;
            }
            set_mtime(&first, &es[0].modified_at);
            placed.push((es[0].path.clone(), first.clone()));
            for e in &es[1..] {
                let p = staged_path(&staging, &e.path)?;
                std::fs::copy(&first, &p)
                    .map_err(|e| Stop::Message(format!("Kinetra can't write there: {e}")))?;
                set_mtime(&p, &e.modified_at);
                placed.push((e.path.clone(), p));
            }
        }
        out.damaged.sort();

        if replace {
            if !out.damaged.is_empty() {
                return Err(Stop::Message(damaged_message(out.damaged.len(), true)));
            }
            return self
                .restore_swap(folder, &staging, &placed, &in_place, &vid, open, &cancel)
                .await;
        }

        // A new folder: the staging folder becomes it.
        let continue_same = req.mode == CloudRestoreMode::Download && recorded.is_none();
        let cid = if continue_same {
            project.app_project_id.clone()
        } else {
            uuid_v7()
        };
        if staging.join("project.json").is_file() {
            if let Err(e) = set_project_ids(&staging, Some(&cid), None) {
                tracing::warn!("restore: project.json not updated: {e}");
            }
        }
        if folder.is_dir() {
            std::fs::remove_dir(folder)
                .map_err(|e| Stop::Message(format!("Kinetra can't use that folder: {e}")))?;
        }
        std::fs::rename(&staging, folder)
            .map_err(|e| Stop::Message(format!("Kinetra can't write there: {e}")))?;
        scratch.dirs.retain(|d| d != &staging);
        self.cloud_record_folder(&cid, folder);
        if continue_same {
            let (pid, head) = (project.id.clone(), vid.clone());
            self.cloud_record_update_of(&cid, |r| {
                r.server_id = Some(pid);
                r.head = Some(head);
            });
        }
        Ok(())
    }

    /// Replace's first step: a backup of the project as it is (waits for one that runs), or,
    /// when that can't happen, a copy of the folder next to it.
    async fn restore_keep_current(
        &self,
        dir: &Path,
        cancel: &AtomicBool,
        done: &AtomicU64,
        out: &mut Outcome,
    ) -> Result<(), Stop> {
        if self.cloud_replace_backs_up() {
            self.restore_phase(CloudRestorePhase::BackingUp, 0, done);
            self.restore_wait_backups(cancel).await?;
            // A new version from here is the proof (its id becomes the head this computer knows).
            let before = self.cloud_record().and_then(|r| r.head);
            if self.cloud_backup_now().is_ok() {
                self.restore_wait_backups(cancel).await?;
                let after = self.cloud_record().and_then(|r| r.head);
                let failed = self.cloud_view().project.and_then(|p| p.error).is_some();
                if !failed && after.is_some() && after != before {
                    return Ok(());
                }
            }
            tracing::info!("restore: the backup didn't finish; the project is copied instead");
        }
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let to = copy_folder_for(dir, &now)
            .ok_or_else(|| Stop::Message("Kinetra can't copy the project.".into()))?;
        let from = dir.to_path_buf();
        self.restore_phase(CloudRestorePhase::Copying, folder_bytes(&from), done);
        let st = self.clone();
        let flag = Arc::new(AtomicBool::new(false));
        let watch_flag = flag.clone();
        let to2 = to.clone();
        // The copy runs on the blocking pool; cancel reaches it through its own flag.
        let copy = tokio::task::spawn_blocking(move || {
            let mut n = 0u64;
            copy_project(&from, &to2, &watch_flag, &mut |b| {
                n += b;
                st.restore_progress(n, None);
            })
        });
        let mut copy = std::pin::pin!(copy);
        let r = loop {
            tokio::select! {
                r = &mut copy => break r,
                () = tokio::time::sleep(Duration::from_millis(100)) => {
                    if cancel.load(Ordering::Relaxed) {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
            }
        };
        match r {
            Ok(Ok(())) => {
                out.copy_folder = Some(to.to_string_lossy().into_owned());
                Ok(())
            }
            _ => {
                let _ = std::fs::remove_dir_all(&to);
                if cancel.load(Ordering::Relaxed) {
                    Err(Stop::Canceled)
                } else {
                    Err(Stop::Message(
                        "Kinetra couldn't copy the project first, so nothing was replaced.".into(),
                    ))
                }
            }
        }
    }

    async fn restore_wait_backups(&self, cancel: &AtomicBool) -> Result<(), Stop> {
        while self.cloud_backup_running() {
            if cancel.load(Ordering::Relaxed) {
                return Err(Stop::Canceled);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// Replace's last step: checks the staged `project.json`, keeps the project's identity, and
    /// swaps the files (see [`swap_into`]); the next backup says `restored_from`.
    #[allow(clippy::too_many_arguments)]
    async fn restore_swap(
        &self,
        dir: &Path,
        staging: &Path,
        placed: &[(String, PathBuf)],
        in_place: &BTreeSet<String>,
        vid: &str,
        open: Option<(PathBuf, Option<String>)>,
        cancel: &AtomicBool,
    ) -> Result<(), Stop> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Stop::Canceled);
        }
        if !in_place.contains("project.json") {
            if !staging.join("project.json").is_file() {
                return Err(Stop::Message(
                    "That version has no project file, so nothing was replaced.".into(),
                ));
            }
            match persist::load_strict::<ProjectFile>(staging) {
                Ok(_) => {}
                Err(persist::PersistError::NewerVersion { .. }) => {
                    return Err(Stop::Message(
                        "That version was saved by a newer Kinetra. Update Kinetra to restore it."
                            .into(),
                    ))
                }
                Err(_) => {
                    return Err(Stop::Message(
                        "That version's project file can't be read, so nothing was replaced."
                            .into(),
                    ))
                }
            }
            let cid = open.as_ref().and_then(|(_, c)| c.clone());
            let remote = self.project_remote_id();
            if let Err(e) = set_project_ids(staging, cid.as_deref(), remote.as_deref()) {
                return Err(Stop::Message(format!("Kinetra can't write there: {e}")));
            }
        }
        if self.cloud_on_air() {
            return Err(Stop::Message(
                "Something went on air, so nothing was replaced. Try again after the show.".into(),
            ));
        }
        self.restore_phase(CloudRestorePhase::Replacing, 0, &AtomicU64::new(0));
        let st = self.clone();
        let (dir2, placed, in_place) = (dir.to_path_buf(), placed.to_vec(), in_place.clone());
        let aside = dir.join(format!(
            ".kinetra-restore-{}-old",
            uuid_v7().chars().rev().take(8).collect::<String>()
        ));
        let swapped = tokio::task::spawn_blocking(move || {
            st.replace_project_files(&dir2, |d| {
                swap_into(d, &placed, &in_place, &aside).map_err(|e| {
                    AppError::Invalid(format!(
                        "A file is in use or can't be moved ({e}), so nothing was replaced. Close other programs using the project and try again."
                    ))
                })
            })
        })
        .await
        .map_err(|_| Stop::Canceled)?;
        swapped.map_err(Stop::from)?;
        if let Some((_, Some(cid))) = open {
            self.cloud_record_update_of(&cid, |r| {
                r.restored_from = Some(vid.to_string());
                r.pending = true;
            });
        }
        // A change like any other: the schedule backs it up (with `restored_from`).
        self.cloud_touch();
        Ok(())
    }

    /// Downloads one blob into `to` (through a temporary file), checked; retried once after a
    /// mismatch. `false` = damaged (or missing on the server).
    async fn restore_fetch(
        &self,
        run: &Run,
        sha: &str,
        size: u64,
        to: &Path,
        urls: &mut HashMap<String, Presigned>,
        missing: &BTreeSet<String>,
    ) -> Result<bool, Stop> {
        if missing.contains(sha) {
            return Ok(false);
        }
        let mut tries = 0;
        let mut renewals = 0;
        while tries < TRIES {
            let url = match urls.remove(sha) {
                Some(u) => u,
                None => {
                    let d = run
                        .api
                        .cloud_downloads(&run.session, &[sha.to_string()])
                        .await?;
                    match d.urls.into_iter().find(|u| u.sha256 == sha) {
                        Some(u) => u.get,
                        None => return Ok(false),
                    }
                }
            };
            let tmp = run
                .tmp
                .join(format!("{}.tmp-{}", &sha[..16.min(sha.len())], uuid_v7()));
            match self.restore_download(run, &url, sha, size, &tmp).await? {
                Got::Ok => {
                    std::fs::rename(&tmp, to)
                        .map_err(|e| Stop::Message(format!("Kinetra can't write there: {e}")))?;
                    return Ok(true);
                }
                Got::Mismatch => {
                    let _ = std::fs::remove_file(&tmp);
                    tracing::warn!("restore: a downloaded file didn't match its hash");
                    tries += 1;
                }
                Got::Expired => {
                    let _ = std::fs::remove_file(&tmp);
                    renewals += 1;
                    if renewals > RENEWALS {
                        return Err(Stop::Offline);
                    }
                }
            }
        }
        Ok(false)
    }

    /// One `GET` into `tmp`, hashed as it's written. A transfer in flight is dropped when
    /// something goes on air (`pause`) and started again once nothing is.
    async fn restore_download(
        &self,
        run: &Run,
        url: &Presigned,
        sha: &str,
        size: u64,
        tmp: &Path,
    ) -> Result<Got, Stop> {
        let base = run.done.load(Ordering::Relaxed);
        let corrupt = corrupt_this_one();
        loop {
            self.restore_wait_off_air(run).await?;
            let mut rx = run.on_air.clone();
            let resp = tokio::select! {
                r = run.r2.get(url) => r,
                () = must_pause(&mut rx) => continue,
            };
            let mut resp = match resp {
                Ok(r) => r,
                Err(R2Error::Expired) => return Ok(Got::Expired),
                Err(R2Error::Offline) => return Err(Stop::Offline),
                Err(R2Error::BadDigest | R2Error::Status(_)) => return Ok(Got::Mismatch),
            };
            let (tx, writer) = writer(tmp.to_path_buf());
            let mut pacer = Pacer::default();
            let mut got = 0u64;
            let mut first = true;
            let mut dropped = false;
            loop {
                if run.cancel.load(Ordering::Relaxed) {
                    drop(tx);
                    let _ = writer.await;
                    return Err(Stop::Canceled);
                }
                let chunk = tokio::select! {
                    c = resp.chunk() => c,
                    () = must_pause(&mut rx) => { dropped = true; break; }
                };
                let chunk = match chunk {
                    Ok(Some(c)) => c,
                    Ok(None) => break,
                    Err(_) => {
                        drop(tx);
                        let _ = writer.await;
                        return Err(Stop::Offline);
                    }
                };
                let mut bytes = chunk.to_vec();
                if first && corrupt && !bytes.is_empty() {
                    bytes[0] ^= 0xff;
                }
                first = false;
                let wait =
                    pacer.delay(self.cloud_gate().mode(), bytes.len() as u64, Instant::now());
                if !wait.is_zero() {
                    self.restore_progress(base + got.min(size), None);
                    tokio::time::sleep(wait).await;
                }
                got += bytes.len() as u64;
                if tx.send(bytes).await.is_err() {
                    break; // the writer failed: its error comes below
                }
                run.done.store(base + got.min(size), Ordering::Relaxed);
                self.restore_progress(base + got.min(size), None);
            }
            drop(tx);
            let written = writer.await.map_err(|_| Stop::Canceled)?;
            if dropped {
                let _ = std::fs::remove_file(tmp);
                run.done.store(base, Ordering::Relaxed);
                continue;
            }
            let (hash, len) =
                written.map_err(|e| Stop::Message(format!("Kinetra can't write there: {e}")))?;
            return Ok(if hash == sha && len == size {
                Got::Ok
            } else {
                Got::Mismatch
            });
        }
    }

    async fn restore_wait_off_air(&self, run: &Run) -> Result<(), Stop> {
        if run.cancel.load(Ordering::Relaxed) {
            return Err(Stop::Canceled);
        }
        let mut rx = run.on_air.clone();
        if *rx.borrow() == Mode::Pause {
            self.restore_progress(run.done.load(Ordering::Relaxed), Some(true));
            loop {
                tokio::select! {
                    () = may_send(&mut rx) => break,
                    () = tokio::time::sleep(Duration::from_millis(250)) => {
                        if run.cancel.load(Ordering::Relaxed) {
                            return Err(Stop::Canceled);
                        }
                    }
                }
            }
            self.restore_progress(run.done.load(Ordering::Relaxed), Some(false));
        }
        if run.cancel.load(Ordering::Relaxed) {
            return Err(Stop::Canceled);
        }
        Ok(())
    }
}

/// "Nothing was replaced: 3 files didn't arrive intact." / the new-folder wording.
fn damaged_message(n: usize, replace: bool) -> String {
    let files = if n == 1 {
        "1 file".to_string()
    } else {
        format!("{n} files")
    };
    if replace {
        format!("Nothing was replaced: {files} couldn't be restored intact.")
    } else {
        format!("{files} couldn't be restored intact.")
    }
}

/// Where a file of the version is staged (folders made).
fn staged_path(staging: &Path, rel: &str) -> Result<PathBuf, Stop> {
    let p =
        target(staging, rel).ok_or_else(|| Stop::Message("A file name can't be used.".into()))?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)
            .map_err(|e| Stop::Message(format!("Kinetra can't write there: {e}")))?;
    }
    Ok(p)
}

/// The copy of a project kept before Replace when it can't be backed up: next to it.
fn copy_folder_for(dir: &Path, now: &str) -> Option<PathBuf> {
    let parent = dir.parent()?;
    let leaf = dir.file_name()?.to_string_lossy().into_owned();
    let when = stamp(now).unwrap_or_else(|| "copy".into());
    Some(free_folder(
        parent,
        &format!("{leaf}-before-restore-{when}"),
    ))
}

/// The user's Documents folder (where a download goes when there are no recent projects).
fn documents_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let docs = PathBuf::from(home).join("Documents");
    docs.is_dir().then_some(docs)
}

/// Blob → local files that had it when last hashed (the hash caches of `sources`), still with the
/// same size and mtime. Used only after a re-hash.
fn local_hashes(config_dir: &Path, sources: &[(PathBuf, String)]) -> HashMap<String, Vec<PathBuf>> {
    let mut out: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for (dir, cid) in sources {
        let cache = HashCache::load(config_dir, cid);
        for (rel, c) in cache.files {
            let Some(abs) = target(dir, &rel) else {
                continue;
            };
            let same = std::fs::metadata(&abs).is_ok_and(|m| {
                m.len() == c.size
                    && m.modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .is_some_and(|d| d.as_nanos().to_string() == c.mtime)
            });
            if same {
                out.entry(c.sha256).or_default().push(abs);
            }
        }
    }
    out
}

/// Replace: the manifest paths whose local file already has the version's bytes (hashed now).
fn same_in_place(
    dir: &Path,
    candidates: &[(String, String, u64)],
    cancel: &AtomicBool,
) -> BTreeSet<String> {
    let _p = BackgroundPriority::enter();
    let mut buf = vec![0u8; BUF];
    let mut out = BTreeSet::new();
    for (rel, sha, size) in candidates {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Some(abs) = target(dir, rel) else {
            continue;
        };
        if !std::fs::metadata(&abs).is_ok_and(|m| m.is_file() && m.len() == *size) {
            continue;
        }
        if scan::sha256_file(&abs, *size, &mut buf, &|| true, cancel).is_ok_and(|h| &h == sha) {
            out.insert(rel.clone());
        }
    }
    out
}

/// Copies `src` to `dst` and checks the copy's hash and size; a mismatch removes it.
fn copy_verified(src: &Path, dst: &Path, sha: &str, size: u64, cancel: &AtomicBool) -> bool {
    let _p = BackgroundPriority::enter();
    if std::fs::copy(src, dst).is_err() {
        return false;
    }
    let mut buf = vec![0u8; BUF];
    let ok = std::fs::metadata(dst).is_ok_and(|m| m.len() == size)
        && scan::sha256_file(dst, size, &mut buf, &|| true, cancel).is_ok_and(|h| h == sha);
    if !ok {
        let _ = std::fs::remove_file(dst);
    }
    ok
}

/// The writer's result: the file's SHA-256 and length.
type Written = tokio::task::JoinHandle<std::io::Result<(String, u64)>>;

/// A file written on a blocking thread from chunks, hashed as it goes (synced to disk).
fn writer(path: PathBuf) -> (tokio::sync::mpsc::Sender<Vec<u8>>, Written) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
    let handle = tokio::task::spawn_blocking(move || {
        let _p = BackgroundPriority::enter();
        let mut f = std::fs::File::create(&path)?;
        let mut h = Sha256::new();
        let mut len = 0u64;
        while let Some(b) = rx.blocking_recv() {
            f.write_all(&b)?;
            h.update(&b);
            len += b.len() as u64;
        }
        f.sync_all()?;
        Ok((scan::hex(&h.finalize()), len))
    });
    (tx, handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_paths_stay_inside_the_folder() {
        let root = Path::new("/shows/gala");
        assert_eq!(
            target(root, "assets/logo.png"),
            Some(root.join("assets").join("logo.png"))
        );
        for bad in [
            "../evil.txt",
            "assets/../../evil",
            "/etc/passwd",
            "C:/Windows/x",
            "a\\b",
            "",
            "assets//x",
            "CON",
            "nul.txt",
            ".backups/x/project.json",
            ".git/config",
            ".kinetra-restore-1/x",
            "assets/x.tmp-1",
            "Thumbs.db",
        ] {
            assert_eq!(target(root, bad), None, "{bad}");
        }
    }

    #[test]
    fn new_folders_never_take_an_existing_name() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(free_folder(tmp.path(), "gala"), tmp.path().join("gala"));
        std::fs::create_dir(tmp.path().join("gala")).unwrap();
        std::fs::create_dir(tmp.path().join("gala-2")).unwrap();
        assert_eq!(free_folder(tmp.path(), "gala"), tmp.path().join("gala-3"));
    }

    #[test]
    fn a_new_folder_must_be_new_or_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(check_new_folder(&tmp.path().join("x")).is_ok());
        std::fs::create_dir(tmp.path().join("empty")).unwrap();
        assert!(check_new_folder(&tmp.path().join("empty")).is_ok());
        std::fs::create_dir(tmp.path().join("full")).unwrap();
        std::fs::write(tmp.path().join("full/a"), b"a").unwrap();
        assert!(check_new_folder(&tmp.path().join("full")).is_err());
        assert!(check_new_folder(&tmp.path().join("no/such/x")).is_err());
        assert!(check_new_folder(Path::new("relative")).is_err());
    }

    #[test]
    fn stamps_name_folders_by_local_time() {
        let s = stamp("2026-10-15T14:02:00Z").unwrap();
        assert_eq!(s.len(), "2026-10-15-1402".len(), "{s}");
        assert!(s.starts_with("2026-10-1"), "{s}");
        assert_eq!(stamp("nonsense"), None);
    }

    #[test]
    fn swapping_moves_old_files_aside_and_the_new_ones_in() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("gala");
        let staging = dir.join(".kinetra-restore-1");
        let w = |p: &Path, b: &[u8]| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b).unwrap();
        };
        w(&dir.join("project.json"), b"old project");
        w(&dir.join("assets/same.png"), b"same");
        w(&dir.join("assets/gone.png"), b"gone");
        w(&dir.join(".backups/1/manifest.json"), b"snapshot");
        w(&staging.join("project.json"), b"new project");
        w(&staging.join("screens/a.json"), b"screen");
        let placed = vec![
            ("project.json".to_string(), staging.join("project.json")),
            ("screens/a.json".to_string(), staging.join("screens/a.json")),
        ];
        let in_place: BTreeSet<String> = ["assets/same.png".to_string()].into();
        swap_into(
            &dir,
            &placed,
            &in_place,
            &dir.join(".kinetra-restore-1-old"),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dir.join("project.json")).unwrap(),
            b"new project"
        );
        assert_eq!(
            std::fs::read(dir.join("screens/a.json")).unwrap(),
            b"screen"
        );
        assert_eq!(std::fs::read(dir.join("assets/same.png")).unwrap(), b"same");
        assert!(!dir.join("assets/gone.png").exists());
        // What a backup never holds stays.
        assert!(dir.join(".backups/1/manifest.json").exists());
        assert!(!dir.join(".kinetra-restore-1-old").exists());
    }

    #[test]
    fn a_failed_swap_puts_everything_back() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("gala");
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("project.json"), b"old").unwrap();
        std::fs::write(dir.join("assets/a.png"), b"a").unwrap();
        let staging = dir.join(".kinetra-restore-1");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("project.json"), b"new").unwrap();
        // The second staged file doesn't exist: its move fails after the first one went in.
        let placed = vec![
            ("project.json".to_string(), staging.join("project.json")),
            ("assets/b.png".to_string(), staging.join("missing.png")),
        ];
        let r = swap_into(
            &dir,
            &placed,
            &BTreeSet::new(),
            &dir.join(".kinetra-restore-1-old"),
        );
        assert!(r.is_err());
        assert_eq!(std::fs::read(dir.join("project.json")).unwrap(), b"old");
        assert_eq!(std::fs::read(dir.join("assets/a.png")).unwrap(), b"a");
        assert_eq!(std::fs::read(staging.join("project.json")).unwrap(), b"new");
    }

    #[test]
    fn project_ids_are_written_and_other_fields_kept() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("project.json"),
            br#"{"schemaVersion":9,"name":"Gala","future":true,"cloudId":"old"}"#,
        )
        .unwrap();
        set_project_ids(tmp.path(), Some("new"), Some("remote")).unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&std::fs::read(tmp.path().join("project.json")).unwrap())
                .unwrap();
        assert_eq!(v["cloudId"], "new");
        assert_eq!(v["id"], "remote");
        assert_eq!(v["future"], true);
        assert_eq!(v["schemaVersion"], 9);
    }
}
