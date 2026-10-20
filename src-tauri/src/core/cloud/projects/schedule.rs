//! When backups run by themselves (CLOUD_PROJECTS.md §2.1; the app plan's decisions 5 and 6):
//!
//! - **After changes:** `client.backup_interval_s` (default 300 s) after the last change while
//!   there are changes not in the cloud (`reason: auto`). A change is any write Kinetra makes in
//!   the open project (`fsx`'s change hook) or an outside edit the watcher reports; the backup's
//!   own excludes don't count. One sleeping task per open project while changes wait; none
//!   otherwise. "Changes not in the cloud" is kept in `cloud.json` (`pending`), so changes made
//!   before a quit are backed up after the next open.
//! - **On close:** a project closed with changes not in the cloud is backed up in the background
//!   (`reason: close`), after any running backup (one transfer at a time).
//! - **Before quitting:** the app asks "Back up before quitting?" only when there are changes not
//!   in the cloud and nothing is on air ([`AppState::cloud_exit_should_ask`]); otherwise quitting
//!   never waits.
//! - **A large first backup** (over 1 GB, automatic) asks once in the view before uploading.
//!
//! Locking: the change hook runs inside every file write, maybe with the writer's locks held, so
//! it only reads atomics and takes this service's own lock; anything more runs in a spawned task.

use std::path::{Component, Path};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::AbortHandle;

use super::backup::Target;
use super::scan;
use super::wire::{QuotaState, Reason};
use crate::core::fsx;
use crate::core::model::{CloudAccess, CloudQuestion};
use crate::core::state::AppState;

/// The open project's schedule (in the service's state).
#[derive(Debug, Default)]
pub(super) struct Sched {
    /// Changes counted since the project opened (a backup covers the count at its start).
    pub changes: u64,
    pub changed_at: Option<Instant>,
    /// The sleeping task that starts the next automatic backup.
    pub timer: Option<AbortHandle>,
    /// `cloud.json` says this project has changes not in the cloud.
    pub marked: bool,
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether a change at `rel` (relative to the project) is outside the backup's excludes.
fn counts(rel: &Path) -> bool {
    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    let last = parts.len().saturating_sub(1);
    !parts.iter().enumerate().any(|(i, name)| {
        let top = i == 0;
        scan::excluded(name, true, top) || (i == last && scan::excluded(name, false, top))
    })
}

impl AppState {
    fn cloud_interval(&self) -> Duration {
        if let Some(d) = *lock(&self.cloud_projects().test_interval) {
            return d;
        }
        let s = lock(&self.cloud_projects().inner)
            .quota
            .as_ref()
            .map(|q| q.client.backup_interval_s)
            .unwrap_or(300);
        Duration::from_secs(s.max(1))
    }

    /// Registers the change hook (once, at startup).
    pub(crate) fn cloud_schedule_init(&self) {
        let weak = self.downgrade();
        let hook: Arc<fsx::ChangeHook> = Arc::new(move |p: &Path| {
            if let Some(st) = weak.upgrade() {
                st.cloud_touch_path(p);
            }
        });
        fsx::add_change_hook(&hook);
        let _ = self.cloud_projects().hook.set(hook);
    }

    /// Whether backups may run by themselves now: signed in with cloud backup, room in storage
    /// (`ok` / `warning`: a full account is paused, §2.5, though "Back up now" still commits what
    /// storage has), today's upload allowance not used up, the open project's backup on. Kept in
    /// `armed` by every view update (`emit_cloud`).
    pub(super) fn cloud_schedule_follow(&self, view: &crate::core::model::CloudView) {
        let armed = view.access == CloudAccess::Available
            && matches!(view.state, Some(QuotaState::Ok | QuotaState::Warning))
            && view.held_until.is_none()
            && !view.offline
            && view.project.as_ref().is_some_and(|p| p.backup);
        let tracking = view.access == CloudAccess::Available
            && view.project.as_ref().is_some_and(|p| p.backup);
        self.cloud_projects()
            .tracking
            .store(tracking, Ordering::Relaxed);
        let was = self.cloud_projects().armed.swap(armed, Ordering::Relaxed);
        if armed && !was {
            // Changes waiting from before (an earlier session, or while it was off).
            let pending = {
                let s = lock(&self.cloud_projects().inner);
                s.project
                    .as_ref()
                    .is_some_and(|p| p.sched.marked && p.sched.timer.is_none())
            };
            if pending {
                self.cloud_arm();
            }
        }
    }

    /// The change hook: Kinetra wrote `path`.
    fn cloud_touch_path(&self, path: &Path) {
        let c = self.cloud_projects();
        if !c.tracking.load(Ordering::Relaxed) {
            return;
        }
        let in_project = {
            let s = lock(&c.inner);
            s.project
                .as_ref()
                .and_then(|p| path.strip_prefix(&p.dir).ok().map(|rel| rel.to_path_buf()))
        };
        match in_project {
            Some(rel) if counts(&rel) => self.cloud_touch(),
            _ => {}
        }
    }

    /// Something in the open project changed (an own write, or an outside edit). Recorded while
    /// the project's backup is on; the timer starts only while backups may run (`armed`).
    pub(crate) fn cloud_touch(&self) {
        let c = self.cloud_projects();
        if !c.tracking.load(Ordering::Relaxed) {
            return;
        }
        let armed = c.armed.load(Ordering::Relaxed);
        let (mark, arm, cloud_id) = {
            let mut s = lock(&c.inner);
            let Some(p) = s.project.as_mut() else { return };
            p.sched.changes += 1;
            p.sched.changed_at = Some(Instant::now());
            let mark = !p.sched.marked;
            p.sched.marked = true;
            (mark, armed && p.sched.timer.is_none(), p.cloud_id.clone())
        };
        if !(mark || arm) {
            return;
        }
        let Some(rt) = self.live().runtime_handle() else {
            return;
        };
        let weak = self.downgrade();
        rt.spawn(async move {
            let Some(st) = weak.upgrade() else { return };
            if let (true, Some(cid)) = (mark, cloud_id) {
                st.cloud_record_update_of(&cid, |r| r.pending = true);
            }
            if arm {
                st.cloud_arm();
            }
        });
    }

    /// Starts the sleeping task that runs a backup `interval` after the last change (unless one
    /// waits already).
    pub(super) fn cloud_arm(&self) {
        let Some(rt) = self.live().runtime_handle() else {
            return;
        };
        let mut s = lock(&self.cloud_projects().inner);
        let Some(p) = s.project.as_mut() else { return };
        if p.sched.timer.is_some() {
            return;
        }
        p.sched.changed_at.get_or_insert_with(Instant::now);
        let weak = self.downgrade();
        let task = rt.spawn(async move {
            loop {
                let Some(st) = weak.upgrade() else { return };
                let interval = st.cloud_interval();
                let wait = {
                    let s = lock(&st.cloud_projects().inner);
                    let Some(p) = s.project.as_ref() else { return };
                    p.sched
                        .changed_at
                        .map(|t| (t + interval).saturating_duration_since(Instant::now()))
                        .unwrap_or(interval)
                };
                drop(st);
                if wait.is_zero() {
                    break;
                }
                tokio::time::sleep(wait).await;
            }
            let Some(st) = weak.upgrade() else { return };
            if let Some(p) = lock(&st.cloud_projects().inner).project.as_mut() {
                p.sched.timer = None;
            }
            st.cloud_auto_backup();
        });
        p.sched.timer = Some(task.abort_handle());
    }

    /// The timer fired: an automatic backup, unless it can't run now (a backup runs: its end
    /// re-arms; a question waits: the answer starts it).
    fn cloud_auto_backup(&self) {
        if !self.cloud_projects().armed.load(Ordering::Relaxed) {
            return;
        }
        let blocked = {
            let s = lock(&self.cloud_projects().inner);
            match s.project.as_ref() {
                Some(p) => p.backup.task.is_some() || p.question.is_some(),
                None => true,
            }
        };
        if blocked {
            return;
        }
        if let Err(e) = self.cloud_backup_start(Reason::Auto) {
            tracing::debug!("automatic cloud backup not started: {e}");
        }
    }

    /// A backup of `t` ended. The open project's changes it covered are in the cloud when it went
    /// through; later changes (or a failure) arm the next one.
    pub(super) fn cloud_backup_finished(&self, t: &Target, ok: bool) {
        if !t.open {
            return;
        }
        let (clear, rearm) = {
            let mut s = lock(&self.cloud_projects().inner);
            let Some(p) = s.project.as_mut().filter(|p| p.dir == t.dir) else {
                return;
            };
            let waiting = p.question.is_some();
            if ok && p.sched.changes == p.backup.covers {
                p.sched.marked = false;
                p.sched.changed_at = None;
                (true, false)
            } else {
                // Paused (full, held, offline…): the change waits, and arming follows the state.
                let armed = self.cloud_projects().armed.load(Ordering::Relaxed);
                (false, p.sched.marked && !waiting && armed)
            }
        };
        if clear {
            self.cloud_record_update_of(&t.cloud_id, |r| r.pending = false);
        }
        if rearm {
            if let Some(p) = lock(&self.cloud_projects().inner).project.as_mut() {
                p.sched.changed_at = Some(Instant::now());
            }
            self.cloud_arm();
        }
    }

    /// The project opened: changes left from before are scheduled.
    pub(super) fn cloud_schedule_opened(&self) {
        let pending = self.cloud_record().is_some_and(|r| r.pending);
        if !pending {
            return;
        }
        if let Some(p) = lock(&self.cloud_projects().inner).project.as_mut() {
            p.sched.marked = true;
            p.sched.changed_at = Some(Instant::now());
        }
        if self.cloud_projects().armed.load(Ordering::Relaxed) {
            self.cloud_arm();
        }
    }

    /// The project is closing: its timer stops, a running backup stops (blobs stay), and with
    /// changes not in the cloud a background backup of the folder starts (`reason: close`).
    pub(super) fn cloud_schedule_closing(&self) {
        let armed = self.cloud_projects().armed.load(Ordering::Relaxed);
        let target = {
            let mut s = lock(&self.cloud_projects().inner);
            let Some(p) = s.project.as_mut() else { return };
            if let Some(t) = p.sched.timer.take() {
                t.abort();
            }
            let pending = p.sched.marked;
            match (&p.cloud_id, pending && armed) {
                (Some(cid), true) => Some(Target {
                    dir: p.dir.clone(),
                    cloud_id: cid.clone(),
                    open: false,
                }),
                _ => None,
            }
        };
        self.cloud_backup_cancel();
        if let Some(t) = target {
            self.cloud_backup_closed(t);
        }
    }

    /// "Back up before quitting?" is asked only when the open project has changes not in the
    /// cloud, backups may run, it's registered, no question waits, and nothing is on air.
    pub fn cloud_exit_should_ask(&self) -> bool {
        if !self.cloud_projects().armed.load(Ordering::Relaxed) {
            return false;
        }
        let ready = {
            let s = lock(&self.cloud_projects().inner);
            s.project
                .as_ref()
                .is_some_and(|p| p.sched.marked && p.question.is_none())
        };
        ready && self.cloud_record().is_some_and(|r| r.server_id.is_some()) && !self.cloud_on_air()
    }

    /// A first automatic backup over 1 GB waits for the user (shown in the view).
    pub(super) fn cloud_ask_first_backup(&self, dir: &Path, bytes: u64) {
        if let Some(p) = lock(&self.cloud_projects().inner)
            .project
            .as_mut()
            .filter(|p| p.dir == dir)
        {
            p.question = Some(CloudQuestion::FirstBackup { bytes });
        }
        self.emit_cloud();
    }

    /// Tests: the wait after the last change.
    #[doc(hidden)]
    pub fn set_test_cloud_interval(&self, d: Option<Duration>) {
        *lock(&self.cloud_projects().test_interval) = d;
    }

    /// Whether the next automatic backup is waiting (tests: no task at rest).
    pub fn cloud_timer_waiting(&self) -> bool {
        lock(&self.cloud_projects().inner)
            .project
            .as_ref()
            .is_some_and(|p| p.sched.timer.is_some())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn excluded_paths_dont_count_as_changes() {
        assert!(super::counts(Path::new("screens/a.json")));
        assert!(super::counts(Path::new("assets/logo.png")));
        assert!(super::counts(Path::new("project.json")));
        assert!(!super::counts(Path::new(".backups/2026/project.json")));
        assert!(!super::counts(Path::new(".git/index")));
        assert!(!super::counts(Path::new("screens/a.json.tmp-x1")));
        assert!(!super::counts(Path::new("assets/Thumbs.db")));
        assert!(!super::counts(Path::new(".kinetra-restore-ab/x")));
        // A user folder named .backups below the top level is backed up.
        assert!(super::counts(Path::new("assets/.backups/x.png")));
    }
}
