//! One backup of the open project (CLOUD_PROJECTS.md §2, §6, §7, §13):
//!
//! 1. the quota (state, `max_file_bytes`), registration if needed;
//! 2. flush the project's pending writes, list the folder, hash (`scan`: cached, background
//!    priority, large files wait while on air);
//! 3. `blobs/check` in batches of 1 000, then upload what's missing **one transfer at a time**:
//!    a single `PUT` up to the threshold, else multipart (MD5 per part, a URL per part just
//!    before it's sent, parts recorded in `cloud.json` so a restart resumes them);
//! 4. check every file's size and mtime again; a file that changed is hashed and sent again;
//! 5. commit the version (`Idempotency-Key` reused on retries; `missing_blobs` → upload those,
//!    commit again; a path the server refuses → that file is dropped and listed).
//!
//! **Never on air** (§0 rule 1): before every file and part the run waits until nothing is on
//! air, and a transfer in flight is dropped the moment something goes on air (a part is at
//! most one part size; it's sent again later). Nothing here blocks saving, playout or opening.
//! A full account (`storage_full`, `daily_upload_limit`) sends nothing new and commits the files
//! already stored. Bodies stream from disk on a blocking thread in 256 KiB chunks (no file is
//! held in memory). At most one backup per project; closing the project cancels it.
//!
//! Not yet (later slices): the automatic schedule, the `limit` speed cap (pauses for now), the
//! conflict notice's Dismiss, restore.

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use md5::{Digest, Md5};
use tokio::sync::watch;

use super::api::BLOBS_PER_CALL;
use super::gate::{may_send, must_pause, Mode, Pacer};
use super::local::MultipartRecord;
use super::r2::{R2Error, R2};
use super::scan::{self, BackgroundPriority, Found, HashCache, Hashed, Skipped};
use super::service::PRO_CLOUD;
use super::wire::{
    BlobRef, FileEntry, IfTrashed, Manifest, NewVersion, PartDone, PartReq, Presigned, QuotaState,
    Reason, Upload,
};
use crate::core::cloud::api::{Api, CallError};
use crate::core::model::{
    CloudAccess, CloudConflictView, CloudPhase, CloudRunView, CLOUD_REPORT_MAX,
};
use crate::core::remote::ids::uuid_v7;
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

/// Bytes per chunk of a streamed body (small enough to pace smoothly under `limit`).
const CHUNK: usize = 64 * 1024;
/// Progress events at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
/// Tries per file / part before the backup gives up for now (expired URLs, changed files).
const TRIES: u32 = 3;
/// Commit attempts (each `missing_blobs` or refused path is one).
const COMMITS: u32 = 4;
/// Waits before committing again with the same key after the server wasn't reached.
#[cfg(not(test))]
const COMMIT_RETRIES: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];
#[cfg(test)]
const COMMIT_RETRIES: [Duration; 2] = [Duration::from_millis(10), Duration::from_millis(20)];

/// The backup state of the open project (in the service's state; the view reads it).
#[derive(Debug, Clone, Default)]
pub(super) struct BackupState {
    pub run: Option<CloudRunView>,
    pub report: Vec<Skipped>,
    pub skipped: u32,
    pub error: Option<String>,
    pub conflict: Option<CloudConflictView>,
    /// The running task (abort on close) and its cancel flag (blocking hashing).
    pub task: Option<(tokio::task::AbortHandle, Arc<AtomicBool>)>,
    /// The open project's change count this backup covers (taken after its flush).
    pub covers: u64,
    pub last_emit: Option<Instant>,
}

/// What a backup backs up: a project folder and its cloud id. `open`: the open project (the view
/// follows it, it registers when needed, its pending writes are flushed first).
#[derive(Debug, Clone)]
pub(super) struct Target {
    pub dir: PathBuf,
    pub cloud_id: String,
    pub open: bool,
}

/// A first automatic backup larger than this asks first (owner decision 5: 1 GB).
#[cfg(not(test))]
pub const FIRST_BACKUP_ASK_BYTES: u64 = 1_000_000_000;
#[cfg(test)]
pub const FIRST_BACKUP_ASK_BYTES: u64 = 100_000;

/// Why a backup stopped.
#[derive(Debug)]
enum Fail {
    /// A question waits for the user (shown in the view, not as an error).
    Waiting,
    /// Not now (offline, server busy): the next backup carries on (uploaded blobs stay).
    Offline,
    /// Canceled (project closed, app exiting).
    Canceled,
    /// Shown as it is.
    Message(String),
}

impl From<CallError> for Fail {
    fn from(e: CallError) -> Self {
        if e.is_offline() || matches!(&e, CallError::Api { status, .. } if *status >= 500) {
            return Fail::Offline;
        }
        if matches!(&e, CallError::Api { status: 429, .. }) {
            return Fail::Message(
                "Cloud backup is busy for this account. Kinetra will try again later.".into(),
            );
        }
        match e {
            CallError::Api { code, message, .. } => Fail::Message(
                crate::core::cloud::projects::backup::message_for(&code).unwrap_or(message),
            ),
            CallError::Offline => Fail::Offline,
        }
    }
}

impl From<AppError> for Fail {
    fn from(e: AppError) -> Self {
        match e {
            AppError::Offline => Fail::Offline,
            other => Fail::Message(other.to_string()),
        }
    }
}

/// Words for the codes a backup can stop on (the UI's `messages.ts` has the same).
pub fn message_for(code: &str) -> Option<String> {
    Some(
        match code {
            "storage_full" => "Cloud storage is full. Your project is saved on this computer; cloud backup is paused.",
            "daily_upload_limit" => "Today's cloud upload allowance is used. Backup continues tomorrow.",
            "cloud_read_only" => "Your plan no longer includes cloud backup. Your backups can still be downloaded.",
            "cloud_not_in_plan" => "Cloud backup is part of Kinetra Pro.",
            "too_many_files" => "This project has more than 50,000 files. Cloud backup can't hold it.",
            "manifest_too_large" => "This project has too many files for one cloud backup.",
            "project_in_trash" => "This project's cloud backup is in the trash.",
            "invalid_token" => "You were signed out. Sign in again.",
            _ => return None,
        }
        .to_string(),
    )
}

/// Everything one run needs.
struct Run {
    api: Api,
    session: String,
    r2: R2,
    on_air: watch::Receiver<Mode>,
    cancel: Arc<AtomicBool>,
    dir: PathBuf,
    cloud_id: String,
    server_id: String,
    /// Bytes sent (uploading) or read (hashing) so far.
    done: Arc<AtomicU64>,
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl AppState {
    fn cloud_backup_state(&self, dir: &Path, f: impl FnOnce(&mut BackupState)) {
        if let Some(p) = lock(&self.cloud_projects().inner)
            .project
            .as_mut()
            .filter(|p| p.dir == dir)
        {
            f(&mut p.backup);
        }
    }

    fn cloud_phase(&self, dir: &Path, phase: CloudPhase, total: u64, run: &Run) {
        run.done.store(0, Ordering::Relaxed);
        self.cloud_backup_state(dir, |b| {
            b.run = Some(CloudRunView {
                phase,
                done_bytes: 0,
                total_bytes: total,
                paused: false,
                limited: false,
            });
        });
        self.emit_cloud();
    }

    /// Progress and the paused flag, sent at most every 100 ms (`force` for pause changes).
    fn cloud_progress(&self, dir: &Path, done: u64, paused: Option<bool>) {
        let mut send = false;
        self.cloud_backup_state(dir, |b| {
            if let Some(r) = b.run.as_mut() {
                r.done_bytes = done.min(r.total_bytes);
                let limited = matches!(self.cloud_gate().mode(), Mode::Limit(_));
                send |= r.limited != limited;
                r.limited = limited;
                if let Some(p) = paused {
                    send |= r.paused != p;
                    r.paused = p;
                }
            }
            let due = b.last_emit.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
            if send || due {
                b.last_emit = Some(Instant::now());
                send = true;
            }
        });
        if send {
            self.emit_cloud();
        }
    }

    /// "Back up now": starts a backup of the open project unless one runs. Returns at once.
    pub fn cloud_backup_now(&self) -> AppResult<()> {
        self.cloud_backup_start(Reason::Manual)
    }

    /// Before quitting ("Back up and quit"): a backup with `reason: close`.
    pub fn cloud_backup_before_exit(&self) -> AppResult<()> {
        self.cloud_backup_start(Reason::Close)
    }

    /// Starts a backup of the open project (`reason`) unless one runs. Returns at once.
    pub(super) fn cloud_backup_start(&self, reason: Reason) -> AppResult<()> {
        self.require_new_pro_work(PRO_CLOUD)?;
        if !self.cloud_backup_on() {
            return Err(AppError::Invalid(
                "Cloud backup is off for this project. Turn it on first.".into(),
            ));
        }
        let (dir, cloud_id) = self.cloud_ensure_id()?;
        let rt = self
            .live()
            .runtime_handle()
            .ok_or_else(|| AppError::Internal("No runtime".into()))?;
        let cancel = Arc::new(AtomicBool::new(false));
        let started = {
            let mut s = lock(&self.cloud_projects().inner);
            let Some(p) = s.project.as_mut().filter(|p| p.dir == dir) else {
                return Err(AppError::NoProject);
            };
            if p.backup.task.is_some() {
                None
            } else {
                p.backup.error = None;
                p.backup.run = Some(CloudRunView {
                    phase: CloudPhase::Scanning,
                    done_bytes: 0,
                    total_bytes: 0,
                    paused: false,
                    limited: false,
                });
                p.backup.covers = p.sched.changes;
                Some(())
            }
        };
        let Some(()) = started else {
            return Ok(());
        };
        let weak = self.downgrade();
        let flag = cancel.clone();
        let target = Target {
            dir: dir.clone(),
            cloud_id,
            open: true,
        };
        let task = rt.spawn(async move {
            let Some(st) = weak.upgrade() else { return };
            st.cloud_gate_active(true);
            let result = st.cloud_backup_run(&target, reason, flag.clone()).await;
            st.cloud_gate_active(st.cloud_backups_running());
            st.cloud_backup_state(&target.dir, |b| {
                b.run = None;
                b.task = None;
                match &result {
                    Ok(()) | Err(Fail::Waiting) | Err(Fail::Canceled) => b.error = None,
                    Err(Fail::Offline) => {
                        b.error = Some("Cloud backup: offline, will retry.".into())
                    }
                    Err(Fail::Message(m)) => b.error = Some(m.clone()),
                }
            });
            st.cloud_backup_finished(&target, result.is_ok());
            st.emit_cloud();
        });
        self.cloud_backup_state(&dir, |b| b.task = Some((task.abort_handle(), cancel)));
        self.emit_cloud();
        Ok(())
    }

    /// A project just closed with changes not in the cloud: backs it up in the background
    /// (`reason: close`; CLOUD_PROJECTS.md §2.1). Only a project already registered; it waits for
    /// any other backup (one transfer at a time).
    pub(super) fn cloud_backup_closed(&self, target: Target) {
        let Some(rt) = self.live().runtime_handle() else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let weak = self.downgrade();
        let flag = cancel.clone();
        let dir = target.dir.clone();
        let task = rt.spawn(async move {
            let Some(st) = weak.upgrade() else { return };
            st.cloud_gate_active(true);
            let result = st.cloud_backup_run(&target, Reason::Close, flag).await;
            {
                let mut s = lock(&st.cloud_projects().inner);
                s.closing.retain(|c| c.0 != target.dir);
            }
            st.cloud_gate_active(st.cloud_backups_running());
            if result.is_ok() {
                st.cloud_record_update_of(&target.cloud_id, |r| r.pending = false);
            }
        });
        lock(&self.cloud_projects().inner)
            .closing
            .push((dir, task.abort_handle(), cancel));
    }

    /// Whether any backup runs (the open project's or a closed one's).
    fn cloud_backups_running(&self) -> bool {
        let s = lock(&self.cloud_projects().inner);
        !s.closing.is_empty() || s.project.as_ref().is_some_and(|p| p.backup.task.is_some())
    }

    /// Cancels the open project's backup (project closing, app exiting). Uploaded blobs stay;
    /// the next backup carries on.
    pub(crate) fn cloud_backup_cancel(&self) {
        let mut s = lock(&self.cloud_projects().inner);
        if let Some(p) = s.project.as_mut() {
            if let Some((task, flag)) = p.backup.task.take() {
                flag.store(true, Ordering::Relaxed);
                task.abort();
            }
            p.backup.run = None;
        }
        drop(s);
        self.cloud_gate_active(self.cloud_backups_running());
    }

    /// Cancels every backup, closed projects' too (app exiting).
    pub(crate) fn cloud_backup_cancel_all(&self) {
        self.cloud_backup_cancel();
        let closing = std::mem::take(&mut lock(&self.cloud_projects().inner).closing);
        for (_, task, flag) in closing {
            flag.store(true, Ordering::Relaxed);
            task.abort();
        }
        self.cloud_gate_active(false);
    }

    /// Whether a closed project's backup runs (tests).
    pub fn cloud_closed_backups(&self) -> usize {
        lock(&self.cloud_projects().inner).closing.len()
    }

    /// Whether the open project's registration is running.
    fn cloud_registering(&self) -> bool {
        lock(&self.cloud_projects().inner)
            .project
            .as_ref()
            .is_some_and(|p| p.registering)
    }

    /// Whether a backup runs (tests).
    pub fn cloud_backup_running(&self) -> bool {
        lock(&self.cloud_projects().inner)
            .project
            .as_ref()
            .is_some_and(|p| p.backup.task.is_some())
    }

    async fn cloud_backup_run(
        &self,
        t: &Target,
        reason: Reason,
        cancel: Arc<AtomicBool>,
    ) -> Result<(), Fail> {
        // One transfer at a time in the whole app (a closed project's backup, then this one).
        let _slot = self.cloud_projects().slot.lock().await;
        if cancel.load(Ordering::Relaxed) {
            return Err(Fail::Canceled);
        }
        let dir = t.dir.as_path();
        // 1. Quota and registration.
        let view = self.cloud_refresh(false).await.map_err(Fail::from)?;
        if view.access != CloudAccess::Available {
            return Err(Fail::Message("Cloud backup is part of Kinetra Pro.".into()));
        }
        if view.offline {
            return Err(Fail::Offline);
        }
        let quota = self.cloud_quota().ok_or(Fail::Offline)?;
        match quota.state {
            QuotaState::Ok | QuotaState::Warning | QuotaState::Full => {}
            QuotaState::Lapsed => return Err(CallError::api(403, "cloud_read_only").into()),
            QuotaState::NotInPlan => return Err(CallError::api(403, "cloud_not_in_plan").into()),
        }
        let server_id_of =
            |st: &AppState| st.cloud_record_of(&t.cloud_id).and_then(|r| r.server_id);
        if t.open {
            if server_id_of(self).is_none() {
                // A background registration may be running: wait for it (seconds at most).
                for _ in 0..100 {
                    if !self.cloud_registering() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                if server_id_of(self).is_none() {
                    self.cloud_register(IfTrashed::Error).await?;
                }
            }
            if self
                .cloud_view()
                .project
                .is_some_and(|p| p.question.is_some())
            {
                return Err(Fail::Waiting);
            }
        }
        // A closed project is backed up only when it was registered while open.
        let server_id = server_id_of(self).ok_or(if t.open {
            Fail::Message("This project isn't registered for cloud backup.".into())
        } else {
            Fail::Canceled
        })?;
        let cloud_id = t.cloud_id.clone();
        let (api, session, _) = self.cloud_session_parts()?;
        let r2 = R2::new().map_err(Fail::Message)?;
        let run = Run {
            api,
            session,
            r2,
            on_air: self.cloud_projects().gate.subscribe(),
            cancel: cancel.clone(),
            dir: dir.to_path_buf(),
            cloud_id: cloud_id.clone(),
            server_id,
            done: Arc::default(),
        };

        // 2. List and hash.
        self.cloud_phase(dir, CloudPhase::Scanning, 0, &run);
        let st = self.clone();
        let max = Some(quota.max_file_bytes).filter(|m| *m > 0);
        let d = dir.to_path_buf();
        let open = t.open;
        let listing = tokio::task::spawn_blocking(move || {
            if open {
                st.flush_all();
            }
            scan::list(&d, max)
        })
        .await
        .map_err(|_| Fail::Canceled)?
        .map_err(|e| Fail::Message(format!("Kinetra can't read the project folder: {e}")))?;
        // Everything changed so far (the flush's own writes included) is in this backup.
        if t.open {
            if let Some(p) = lock(&self.cloud_projects().inner)
                .project
                .as_mut()
                .filter(|p| p.dir == t.dir)
            {
                p.backup.covers = p.sched.changes;
            }
        }
        if listing.files.len() > scan::MAX_FILES {
            return Err(CallError::api(400, "too_many_files").into());
        }
        let mut report = listing.skipped;
        let total: u64 = listing.files.iter().map(|f| f.size).sum();
        // The first automatic backup of a large project asks once (owner decision 5); a backup
        // the user started doesn't.
        let record = self.cloud_record_of(&cloud_id).unwrap_or_default();
        if reason != Reason::Manual
            && record.head.is_none()
            && !record.big_ok
            && total > FIRST_BACKUP_ASK_BYTES
        {
            if t.open {
                self.cloud_ask_first_backup(dir, total);
            }
            return Err(Fail::Waiting);
        }
        self.cloud_phase(dir, CloudPhase::Hashing, total, &run);
        let (mut files, unread) = self
            .cloud_hash(
                &run,
                listing.files,
                HashCache::load(self.config_dir(), &cloud_id),
            )
            .await?;
        report.extend(unread);

        // 3–4. Upload what's missing; files that changed meanwhile are hashed and sent again.
        let mut refused: Option<String> = None;
        let mut stored_ok: HashSet<String> = HashSet::new();
        for round in 0..TRIES {
            // The total grows as the checks say what storage still needs.
            self.cloud_phase(dir, CloudPhase::Uploading, 0, &run);
            match self
                .cloud_upload_all(&run, &files, &mut report, &mut stored_ok)
                .await
            {
                Ok(()) => {}
                Err(UploadStop::Refused(code)) => refused = Some(code),
                Err(UploadStop::Fail(f)) => return Err(f),
            }
            let changed: Vec<Found> = files
                .iter()
                .filter(|h| !scan::unchanged(&h.file))
                .filter_map(|h| scan::restat(&h.file).ok())
                .collect();
            if changed.is_empty() || refused.is_some() {
                break;
            }
            if round + 1 == TRIES {
                return Err(Fail::Message(
                    "Files kept changing during the backup. Kinetra will try again.".into(),
                ));
            }
            let paths: HashSet<String> = changed.iter().map(|f| f.path.clone()).collect();
            let mut cache = HashCache::load(self.config_dir(), &cloud_id);
            for f in &changed {
                cache.files.remove(&f.path);
            }
            let (rehashed, unread) = self.cloud_hash(&run, changed, cache).await?;
            report.extend(unread);
            files.retain(|h| !paths.contains(&h.file.path));
            files.extend(rehashed);
            files.sort_by(|a, b| a.file.path.cmp(&b.file.path));
        }

        // A full account commits only what storage already has.
        if let Some(code) = &refused {
            stored_ok.extend(HashCache::load(self.config_dir(), &cloud_id).committed);
            let why = message_for(code).unwrap_or_default();
            let (keep, left): (Vec<Hashed>, Vec<Hashed>) = files
                .into_iter()
                .partition(|h| stored_ok.contains(&h.sha256));
            files = keep;
            report.extend(left.into_iter().map(|h| Skipped {
                path: h.file.path,
                reason: why.clone(),
            }));
        }

        // 5. Commit.
        self.cloud_phase(dir, CloudPhase::Committing, 0, &run);
        self.cloud_commit(&run, reason, files, &mut report).await?;
        let skipped = report.len() as u32;
        report.truncate(CLOUD_REPORT_MAX);
        self.cloud_backup_state(dir, |b| {
            b.report = report;
            b.skipped = skipped;
        });
        match refused {
            Some(code) => Err(CallError::api(403, &code).into()),
            None => Ok(()),
        }
    }

    /// Hashes on the blocking pool (large files wait while on air); saves the cache.
    async fn cloud_hash(
        &self,
        run: &Run,
        files: Vec<Found>,
        mut cache: HashCache,
    ) -> Result<(Vec<Hashed>, Vec<Skipped>), Fail> {
        let st = self.clone();
        let cancel = run.cancel.clone();
        let done = run.done.clone();
        let dir = run.dir.clone();
        let cloud_id = run.cloud_id.clone();
        tokio::task::spawn_blocking(move || {
            let gate = &st.cloud_projects().gate;
            let wait = || {
                if gate.mode() == Mode::Pause {
                    st.cloud_progress(&dir, done.load(Ordering::Relaxed), Some(true));
                    let go = gate.wait_blocking(&cancel);
                    st.cloud_progress(&dir, done.load(Ordering::Relaxed), Some(false));
                    go
                } else {
                    true
                }
            };
            let mut progress = |n: u64, _read: bool| {
                let d = done.fetch_add(n, Ordering::Relaxed) + n;
                st.cloud_progress(&dir, d, None);
            };
            let r = scan::hash_all(files, &mut cache, &wait, &cancel, &mut progress);
            if r.is_ok() {
                if let Err(e) = cache.save(st.config_dir(), &cloud_id) {
                    tracing::warn!("cloud hash cache not saved: {e}");
                }
            }
            r
        })
        .await
        .map_err(|_| Fail::Canceled)?
        .map_err(|_| Fail::Canceled)
    }

    /// `blobs/check` in batches, then each missing blob, one at a time. `stored` collects the
    /// blobs storage has (already, or sent now).
    async fn cloud_upload_all(
        &self,
        run: &Run,
        files: &[Hashed],
        report: &mut Vec<Skipped>,
        stored: &mut HashSet<String>,
    ) -> Result<(), UploadStop> {
        let mut blobs: BTreeMap<String, (u64, PathBuf)> = BTreeMap::new();
        for h in files {
            blobs
                .entry(h.sha256.clone())
                .or_insert((h.file.size, h.file.abs.clone()));
        }
        let shas: Vec<String> = blobs.keys().cloned().collect();
        for batch in shas.chunks(BLOBS_PER_CALL) {
            let mut refs: Vec<BlobRef> = batch
                .iter()
                .map(|s| BlobRef {
                    sha256: s.clone(),
                    size: blobs[s].0,
                })
                .collect();
            let checked = loop {
                match run.api.cloud_check(&run.session, &refs).await {
                    Ok(c) => break c,
                    Err(CallError::Api { code, details, .. }) if code == "file_too_large" => {
                        // That blob is skipped and listed; the rest are asked again.
                        let sha = details
                            .as_ref()
                            .and_then(|d| d.get("sha256"))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        if !refs.iter().any(|r| r.sha256 == sha) {
                            return Err(UploadStop::Fail(Fail::Message(
                                "A file is too large for cloud backup.".into(),
                            )));
                        }
                        refs.retain(|r| r.sha256 != sha);
                        report.extend(files.iter().filter(|h| h.sha256 == sha).map(|h| Skipped {
                            path: h.file.path.clone(),
                            reason: "It's larger than cloud backup allows for one file.".into(),
                        }));
                        if refs.is_empty() {
                            break super::wire::Checked {
                                missing: Vec::new(),
                                reserved_until: None,
                            };
                        }
                    }
                    Err(e @ CallError::Api { .. })
                        if matches!(e.code(), Some("storage_full" | "daily_upload_limit")) =>
                    {
                        // Nothing new goes up; what storage has is still committed.
                        return Err(UploadStop::Refused(e.code().unwrap_or_default().into()));
                    }
                    Err(e) => return Err(UploadStop::Fail(e.into())),
                }
            };
            let missing: HashSet<String> =
                checked.missing.iter().map(|m| m.sha256.clone()).collect();
            let more: u64 = checked.missing.iter().map(|m| m.size).sum();
            self.cloud_backup_state(&run.dir, |b| {
                if let Some(r) = b.run.as_mut() {
                    r.total_bytes += more;
                }
            });
            for r in &refs {
                if !missing.contains(&r.sha256) {
                    stored.insert(r.sha256.clone());
                }
            }
            for m in checked.missing {
                let (size, abs) = blobs[&m.sha256].clone();
                self.cloud_upload_blob(run, &m.sha256, size, &abs, m.upload)
                    .await
                    .map_err(UploadStop::Fail)?;
                stored.insert(m.sha256.clone());
            }
        }
        Ok(())
    }

    /// One blob: waits until nothing is on air, then sends it (single or multipart).
    async fn cloud_upload_blob(
        &self,
        run: &Run,
        sha: &str,
        size: u64,
        abs: &Path,
        mut upload: Upload,
    ) -> Result<(), Fail> {
        for _ in 0..TRIES {
            let r = match &upload {
                Upload::Put { put } => self.cloud_put_single(run, size, abs, put).await,
                Upload::Multipart {
                    upload_id,
                    part_size,
                    part_count,
                } => {
                    self.cloud_put_multipart(
                        run,
                        sha,
                        size,
                        abs,
                        upload_id,
                        *part_size,
                        *part_count,
                    )
                    .await
                }
            };
            match r {
                Ok(()) => return Ok(()),
                // Expired URL or a restarted multipart: ask again for this blob.
                Err(Step::Again) => {
                    let checked = run
                        .api
                        .cloud_check(
                            &run.session,
                            &[BlobRef {
                                sha256: sha.to_string(),
                                size,
                            }],
                        )
                        .await?;
                    match checked.missing.into_iter().next() {
                        Some(m) => upload = m.upload,
                        None => return Ok(()), // stored meanwhile
                    }
                }
                // The file changed under us: the re-check after the uploads picks it up.
                Err(Step::Changed) => return Ok(()),
                Err(Step::Fail(f)) => return Err(f),
            }
        }
        Err(Fail::Offline)
    }

    async fn cloud_put_single(
        &self,
        run: &Run,
        size: u64,
        abs: &Path,
        put: &Presigned,
    ) -> Result<(), Step> {
        let base = run.done.load(Ordering::Relaxed);
        loop {
            self.cloud_wait_off_air(run).await?;
            let body = self.cloud_body(run, abs, 0, size, base);
            let mut rx = run.on_air.clone();
            tokio::select! {
                r = run.r2.put(put, body) => {
                    return match r {
                        Ok(_) => {
                            run.done.store(base + size, Ordering::Relaxed);
                            Ok(())
                        }
                        Err(R2Error::Expired) => Err(Step::Again),
                        Err(R2Error::BadDigest) => Err(Step::Changed),
                        Err(R2Error::Offline) => Err(Step::Fail(Fail::Offline)),
                        Err(R2Error::Status(s)) => Err(Step::Fail(Fail::Message(format!(
                            "Cloud storage refused a file ({s}). Kinetra will try again."
                        )))),
                    };
                }
                _ = must_pause(&mut rx) => {
                    // Dropped mid-transfer: sent again once nothing is on air.
                    run.done.store(base, Ordering::Relaxed);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn cloud_put_multipart(
        &self,
        run: &Run,
        sha: &str,
        size: u64,
        abs: &Path,
        upload_id: &str,
        part_size: u64,
        part_count: u32,
    ) -> Result<(), Step> {
        // What storage already has (a restart resumes here).
        let status = run
            .api
            .cloud_upload_status(&run.session, upload_id)
            .await
            .map_err(|e| match &e {
                CallError::Api { status: 404, .. } => Step::Again,
                _ => Step::Fail(e.into()),
            })?;
        let mut done: BTreeMap<u32, String> = status
            .parts_done
            .into_iter()
            .map(|p| (p.number, p.etag))
            .collect();
        self.cloud_multipart_record(run, sha, upload_id, part_size, part_count, &done);
        let base = run.done.load(Ordering::Relaxed);
        let sent_before: u64 = done
            .keys()
            .map(|n| part_len(size, part_size, part_count, *n))
            .sum();
        run.done.store(base + sent_before, Ordering::Relaxed);
        for n in 1..=part_count {
            if done.contains_key(&n) {
                continue;
            }
            let offset = u64::from(n - 1) * part_size;
            let len = part_len(size, part_size, part_count, n);
            let md5 = part_md5(abs, offset, len)
                .await
                .map_err(|_| Step::Changed)?;
            let mut tries = 0;
            let etag = loop {
                tries += 1;
                if tries > TRIES {
                    return Err(Step::Fail(Fail::Offline));
                }
                self.cloud_wait_off_air(run).await?;
                let url = run
                    .api
                    .cloud_part_urls(
                        &run.session,
                        upload_id,
                        &[PartReq {
                            number: n,
                            md5: md5.clone(),
                        }],
                    )
                    .await
                    .map_err(|e| Step::Fail(e.into()))?
                    .into_iter()
                    .find(|p| p.number == n)
                    .ok_or(Step::Fail(Fail::Offline))?;
                let before = run.done.load(Ordering::Relaxed);
                let body = self.cloud_body(run, abs, offset, len, before);
                let mut rx = run.on_air.clone();
                tokio::select! {
                    r = run.r2.put(&url.put, body) => match r {
                        Ok(Some(etag)) => {
                            run.done.store(before + len, Ordering::Relaxed);
                            break etag;
                        }
                        Ok(None) => return Err(Step::Fail(Fail::Message(
                            "Cloud storage didn't confirm a part. Kinetra will try again.".into()))),
                        Err(R2Error::Expired) => continue,
                        Err(R2Error::BadDigest) => return Err(Step::Changed),
                        Err(R2Error::Offline) => return Err(Step::Fail(Fail::Offline)),
                        Err(R2Error::Status(s)) => return Err(Step::Fail(Fail::Message(format!(
                            "Cloud storage refused a part ({s}). Kinetra will try again.")))),
                    },
                    _ = must_pause(&mut rx) => {
                        run.done.store(before, Ordering::Relaxed);
                        tries -= 1; // on air isn't a failure
                        continue;
                    }
                }
            };
            done.insert(n, etag);
            self.cloud_multipart_record(run, sha, upload_id, part_size, part_count, &done);
        }
        let parts: Vec<PartDone> = done
            .into_iter()
            .map(|(number, etag)| PartDone { number, etag })
            .collect();
        match run
            .api
            .cloud_complete(&run.session, upload_id, &parts)
            .await
        {
            Ok(()) => {
                self.cloud_multipart_forget(run, sha);
                Ok(())
            }
            Err(CallError::Api { code, .. }) if code == "upload_mismatch" => {
                let _ = run.api.cloud_abort_upload(&run.session, upload_id).await;
                self.cloud_multipart_forget(run, sha);
                Err(Step::Again)
            }
            Err(e) => Err(Step::Fail(e.into())),
        }
    }

    fn cloud_multipart_record(
        &self,
        run: &Run,
        sha: &str,
        upload_id: &str,
        part_size: u64,
        part_count: u32,
        done: &BTreeMap<u32, String>,
    ) {
        let Some(account) = self.cloud_account() else {
            return;
        };
        let rec = MultipartRecord {
            upload_id: upload_id.to_string(),
            part_size,
            part_count,
            parts: done
                .iter()
                .map(|(n, e)| PartDone {
                    number: *n,
                    etag: e.clone(),
                })
                .collect(),
        };
        let cid = run.cloud_id.clone();
        self.cloud_file_update(|f| {
            f.accounts
                .entry(account)
                .or_default()
                .projects
                .entry(cid)
                .or_default()
                .uploads
                .insert(sha.to_string(), rec);
        });
    }

    fn cloud_multipart_forget(&self, run: &Run, sha: &str) {
        let Some(account) = self.cloud_account() else {
            return;
        };
        let cid = run.cloud_id.clone();
        self.cloud_file_update(|f| {
            if let Some(p) = f
                .accounts
                .get_mut(&account)
                .and_then(|a| a.projects.get_mut(&cid))
            {
                p.uploads.remove(sha);
            }
        });
    }

    /// Waits while transfers must pause (shown as paused), or fails when canceled.
    async fn cloud_wait_off_air(&self, run: &Run) -> Result<(), Step> {
        if run.cancel.load(Ordering::Relaxed) {
            return Err(Step::Fail(Fail::Canceled));
        }
        let mut rx = run.on_air.clone();
        if *rx.borrow() == Mode::Pause {
            self.cloud_progress(&run.dir, run.done.load(Ordering::Relaxed), Some(true));
            may_send(&mut rx).await;
            self.cloud_progress(&run.dir, run.done.load(Ordering::Relaxed), Some(false));
        }
        if run.cancel.load(Ordering::Relaxed) {
            return Err(Step::Fail(Fail::Canceled));
        }
        Ok(())
    }

    /// A streamed request body: `len` bytes of `abs` from `offset`, read on a blocking thread at
    /// background priority in 256 KiB chunks. Progress is `base` + bytes handed to the request.
    fn cloud_body(&self, run: &Run, abs: &Path, offset: u64, len: u64, base: u64) -> reqwest::Body {
        let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<Vec<u8>>>(4);
        let path = abs.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let _p = BackgroundPriority::enter();
            let mut f = match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                    return;
                }
            };
            if let Err(e) = f.seek(SeekFrom::Start(offset)) {
                let _ = tx.blocking_send(Err(e));
                return;
            }
            let mut left = len;
            while left > 0 {
                let mut buf = vec![0u8; CHUNK.min(left as usize)];
                if let Err(e) = f.read_exact(&mut buf) {
                    let _ = tx.blocking_send(Err(e));
                    return;
                }
                left -= buf.len() as u64;
                if tx.blocking_send(Ok(buf)).is_err() {
                    return; // the request was dropped (on air, canceled)
                }
            }
        });
        let st = self.clone();
        let dir = run.dir.clone();
        let done = run.done.clone();
        let sent = Arc::new(AtomicU64::new(0));
        // Paced to the limit while on air in `limit` mode (the state carries the pacer).
        let stream =
            futures_util::stream::unfold((rx, Pacer::default()), move |(mut rx, mut pacer)| {
                let (st, dir, done, sent) = (st.clone(), dir.clone(), done.clone(), sent.clone());
                async move {
                    let item = rx.recv().await?;
                    if let Ok(b) = &item {
                        let mode = st.cloud_gate().mode();
                        let wait = pacer.delay(mode, b.len() as u64, Instant::now());
                        if !wait.is_zero() {
                            tokio::time::sleep(wait).await;
                        }
                        let s = sent.fetch_add(b.len() as u64, Ordering::Relaxed) + b.len() as u64;
                        done.store(base + s, Ordering::Relaxed);
                        st.cloud_progress(&dir, base + s, None);
                    }
                    Some((item, (rx, pacer)))
                }
            });
        reqwest::Body::wrap_stream(stream)
    }

    /// Commits the version; `missing_blobs` sends those and commits again (same key); a path
    /// the server refuses drops that file (new key).
    async fn cloud_commit(
        &self,
        run: &Run,
        reason: Reason,
        mut files: Vec<Hashed>,
        report: &mut Vec<Skipped>,
    ) -> Result<(), Fail> {
        let activation_id = self
            .cloud_activation()
            .ok_or_else(|| Fail::Message("This computer isn't activated. Sign in again.".into()))?;
        let (base, restored_from) = self.cloud_commit_base(&run.cloud_id);
        // A commit whose answer was lost is retried with its key: the server returns that version
        // instead of a second one (which would look like a conflict with ourselves).
        let mut key = self
            .cloud_pending_commit(&run.cloud_id)
            .unwrap_or_else(uuid_v7);
        self.cloud_set_pending_commit(&run.cloud_id, Some(key.clone()));
        let mut offline = 0u32;
        let mut tries = 0u32;
        while tries < COMMITS {
            tries += 1;
            let version = NewVersion {
                base_version_id: base.clone(),
                restored_from: restored_from.clone(),
                reason,
                label: None,
                activation_id: activation_id.clone(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                manifest: Manifest {
                    format: 1,
                    files: files
                        .iter()
                        .map(|h| FileEntry {
                            path: h.file.path.clone(),
                            sha256: h.sha256.clone(),
                            size: h.file.size,
                            modified_at: scan::rfc3339(h.file.mtime_ns),
                        })
                        .collect(),
                },
            };
            match run
                .api
                .cloud_commit(&run.session, &run.server_id, &key, &version)
                .await
            {
                Ok(c) => {
                    self.cloud_set_pending_commit(&run.cloud_id, None);
                    let conflict = c.conflict.map(|x| CloudConflictView {
                        machine: x.with_version.machine_name,
                        at: Some(x.with_version.created_at),
                        version_id: c.version.id.clone(),
                    });
                    let head = c.version.id.clone();
                    let at = c.version.created_at.clone();
                    self.cloud_record_committed(&run.cloud_id, &head, &at);
                    let mut cache = HashCache::load(self.config_dir(), &run.cloud_id);
                    cache.committed = files.iter().map(|h| h.sha256.clone()).collect();
                    if let Err(e) = cache.save(self.config_dir(), &run.cloud_id) {
                        tracing::warn!("cloud hash cache not saved: {e}");
                    }
                    self.cloud_backup_state(&run.dir, |b| {
                        if conflict.is_some() {
                            b.conflict = conflict;
                        }
                    });
                    return Ok(());
                }
                Err(CallError::Api { code, details, .. }) if code == "missing_blobs" => {
                    let missing: HashSet<String> = details
                        .as_ref()
                        .and_then(|d| d.get("missing"))
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    let need: Vec<Hashed> = files
                        .iter()
                        .filter(|h| missing.contains(&h.sha256))
                        .cloned()
                        .collect();
                    let mut stored = HashSet::new();
                    match self.cloud_upload_all(run, &need, report, &mut stored).await {
                        Ok(()) => {}
                        Err(UploadStop::Refused(code)) => {
                            return Err(CallError::api(403, &code).into())
                        }
                        Err(UploadStop::Fail(f)) => return Err(f),
                    }
                }
                Err(CallError::Api { code, details, .. }) if code == "invalid_path" => {
                    let index = details
                        .as_ref()
                        .and_then(|d| d.get("index"))
                        .and_then(|v| v.as_u64())
                        .map(|i| i as usize);
                    let Some(i) = index.filter(|i| *i < files.len()) else {
                        return Err(Fail::Message("Cloud backup refused a file name.".into()));
                    };
                    let h = files.remove(i);
                    report.push(Skipped {
                        path: h.file.path,
                        reason: "Cloud backup can't store this name.".into(),
                    });
                    key = uuid_v7();
                    self.cloud_set_pending_commit(&run.cloud_id, Some(key.clone()));
                }
                // Not reached / the server is busy: the same key again, a little later.
                Err(e)
                    if e.is_offline()
                        || matches!(&e, CallError::Api { status, .. } if *status >= 500) =>
                {
                    offline += 1;
                    if offline > COMMIT_RETRIES.len() as u32 {
                        return Err(Fail::Offline);
                    }
                    tries -= 1;
                    tokio::time::sleep(COMMIT_RETRIES[offline as usize - 1]).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(Fail::Offline)
    }
}

/// Why an upload stopped.
enum UploadStop {
    /// `storage_full` / `daily_upload_limit`: nothing new goes up.
    Refused(String),
    Fail(Fail),
}

/// The next step for one blob.
enum Step {
    /// Ask the server again (expired URL, restarted multipart).
    Again,
    /// The file changed: the re-check after the uploads handles it.
    Changed,
    Fail(Fail),
}

impl From<CallError> for Step {
    fn from(e: CallError) -> Self {
        Step::Fail(e.into())
    }
}

/// Bytes in part `n` (1-based).
pub fn part_len(size: u64, part_size: u64, part_count: u32, n: u32) -> u64 {
    if n < part_count {
        part_size
    } else {
        size.saturating_sub(part_size * u64::from(part_count.saturating_sub(1)))
    }
}

/// MD5 (lowercase hex) of `len` bytes of `abs` from `offset`, read at background priority.
async fn part_md5(abs: &Path, offset: u64, len: u64) -> std::io::Result<String> {
    let path = abs.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let _p = BackgroundPriority::enter();
        let mut f = std::fs::File::open(&path)?;
        f.seek(SeekFrom::Start(offset))?;
        let mut h = Md5::new();
        let mut buf = vec![0u8; CHUNK];
        let mut left = len;
        while left > 0 {
            let n = CHUNK.min(left as usize);
            f.read_exact(&mut buf[..n])?;
            h.update(&buf[..n]);
            left -= n as u64;
        }
        Ok(scan::hex(&h.finalize()))
    })
    .await
    .map_err(|_| std::io::Error::other("canceled"))?
}
