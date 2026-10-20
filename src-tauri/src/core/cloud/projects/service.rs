//! Cloud backup on `AppState`: access, the quota, this machine's records (`cloud.json`) and the
//! view the UI mirrors (`cloud_view`, `cloud-changed`).
//!
//! # Access (the server decides; the core decides when to ask)
//! Availability comes from `GET /v1/cloud/quota`, never from the license token (§4). The app
//! asks only when signed in and either the license has `pro.cloud_sync` or this computer has
//! seen cloud backup for the account before (`cloud.json` `known`), so a never-Pro Free user
//! makes zero calls and a lapsed one can still download. "Check for backups" asks once
//! regardless (`cloud_refresh(true)`).
//!
//! # When the quota is asked
//! After every successful license sync (startup, sign-in, every 12 h), on "Check now", and
//! (later slices) before a backup and after errors that change the state. Offline answers
//! retry with the backoff (one sleeping task, none otherwise).
//!
//! Locking: the service lock is a data lock, held briefly, never across I/O; events go out
//! after it is released.

use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::task::AbortHandle;

use super::backoff::{self, Retry};
use super::local::CloudFile;
use super::wire::{Quota, QuotaState};
use crate::core::cloud::api::Api;
use crate::core::events::names;
use crate::core::model::{CloudAccess, CloudQuotaView, CloudSettingsView, CloudView};
use crate::core::persist;
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

/// The entitlement cloud backup needs (`kinetra_license::entitlements::PRO_CLOUD_SYNC`).
pub const PRO_CLOUD: &str = kinetra_license::entitlements::PRO_CLOUD_SYNC;

#[derive(Default)]
pub(super) struct Svc {
    pub(super) quota: Option<Quota>,
    /// Unix seconds of the last answer.
    checked_at: Option<i64>,
    /// The last request didn't reach the server.
    offline: bool,
    /// Failed quota requests in a row.
    fails: u32,
    /// The sleeping retry after an offline answer.
    retry: Option<AbortHandle>,
    /// The account id from `/v1/me` when the license file has none (this session).
    account: Option<String>,
    /// `cloud.json`, read once.
    file: Option<CloudFile>,
    /// The last view sent (`cloud-changed` goes out on change only).
    sent: Option<CloudView>,
    /// The open project (see `identity`).
    pub(super) project: Option<super::identity::OpenProject>,
    /// Today's upload allowance is used (`daily_upload_limit`): automatic backups wait until this
    /// time (RFC 3339, the quota's `upload_today.resets_at`), and the sleeping task that ends the
    /// wait.
    pub(super) held_until: Option<String>,
    hold: Option<AbortHandle>,
    /// The restore running or just finished (see `restore`).
    pub(super) restore: Option<super::restore::RestoreState>,
    /// Backups of projects that were closed with changes not in the cloud (folder, task, cancel).
    pub(super) closing: Vec<(
        std::path::PathBuf,
        AbortHandle,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
    )>,
}

/// Cloud backup's state (held by `AppState`; see the module docs).
#[derive(Default)]
pub struct CloudProjects {
    pub(super) inner: Mutex<Svc>,
    /// On air for transfers (see `gate`).
    pub(super) gate: super::gate::Gate,
    /// One transfer at a time in the whole app (backups wait for it).
    pub(super) slot: tokio::sync::Mutex<()>,
    /// Backups may run by themselves now (see `schedule`); read by the change hook.
    pub(super) armed: std::sync::atomic::AtomicBool,
    /// Changes are recorded (the open project's backup is on for a signed-in account), also while
    /// automatic backups are paused (full, held, offline, lapsed), so they go up once they may.
    pub(super) tracking: std::sync::atomic::AtomicBool,
    /// The change hook (registered weakly with `fsx`; kept alive here).
    pub(super) hook: std::sync::OnceLock<std::sync::Arc<crate::core::fsx::ChangeHook>>,
    /// Tests: the wait after the last change instead of the server's interval.
    pub(super) test_interval: Mutex<Option<std::time::Duration>>,
    /// `cloud.json` writes, one at a time.
    save: Mutex<()>,
    /// Tests: the wait before a retry instead of the backoff.
    test_retry: Mutex<Option<std::time::Duration>>,
    /// Tests: act as a build without cloud backup.
    test_unavailable: std::sync::atomic::AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl CloudProjects {
    /// Whether a quota retry is waiting (tests: no task at rest).
    pub fn retry_pending(&self) -> bool {
        lock(&self.inner).retry.is_some()
    }

    #[doc(hidden)]
    pub fn set_test_retry(&self, wait: Option<std::time::Duration>) {
        *lock(&self.test_retry) = wait;
    }

    #[doc(hidden)]
    pub fn set_test_unavailable(&self, on: bool) {
        self.test_unavailable
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The quota as the UI shows it.
fn quota_view(q: &Quota) -> CloudQuotaView {
    CloudQuotaView {
        plan: q.plan.clone(),
        used_bytes: q.used_bytes,
        pending_bytes: q.pending_bytes,
        limit_bytes: q.limit_bytes,
        percent: q.percent,
        warn_percent: q.warn_percent,
        projects_used: q.projects.used,
        projects_limit: q.projects.limit,
        max_file_bytes: q.max_file_bytes,
        retention_days: q.retention_days,
        kept_per_project: q.kept_per_project,
        upload_today_used: q.upload_today.used_bytes,
        upload_today_limit: q.upload_today.limit_bytes,
        upload_today_resets_at: q.upload_today.resets_at.clone(),
        purge_at: q.lapse.as_ref().and_then(|l| l.purge_at.clone()),
        retention_days_next: q.retention_change.as_ref().map(|r| r.retention_days),
        retention_from: q.retention_change.as_ref().and_then(|r| r.from.clone()),
    }
}

impl AppState {
    // ---- cloud.json ----

    /// `cloud.json` (read once, then from memory).
    pub(crate) fn cloud_file(&self) -> CloudFile {
        if let Some(f) = &lock(&self.cloud_projects().inner).file {
            return f.clone();
        }
        let f = persist::load_or_recover::<CloudFile>(self.config_dir())
            .map(|(f, _)| f)
            .unwrap_or_default();
        lock(&self.cloud_projects().inner)
            .file
            .get_or_insert(f)
            .clone()
    }

    /// Changes `cloud.json` (in memory, under the lock, so concurrent changes never lose one)
    /// and writes it on the blocking pool when anything changed. Each write takes the newest
    /// content, one at a time, so an older write never lands last.
    pub(crate) fn cloud_file_update(&self, f: impl FnOnce(&mut CloudFile)) {
        self.cloud_file(); // loaded
        let changed = {
            let mut s = lock(&self.cloud_projects().inner);
            let file = s.file.get_or_insert_with(CloudFile::default);
            let before = file.clone();
            f(file);
            *file != before
        };
        if !changed {
            return;
        }
        let weak = self.downgrade();
        let save = move || {
            let Some(st) = weak.upgrade() else { return };
            let svc = st.cloud_projects();
            let _one_at_a_time = lock(&svc.save);
            let Some(file) = lock(&svc.inner).file.clone() else {
                return;
            };
            if let Err(e) = persist::save(st.config_dir(), &file) {
                tracing::warn!("cloud.json: {e}");
            }
        };
        match self.live().runtime_handle() {
            Some(h) => drop(h.spawn_blocking(save)),
            None => save(),
        }
    }

    // ---- Access ----

    /// The signed-in account's id: the license file's, else the one fetched this session.
    pub(crate) fn cloud_account(&self) -> Option<String> {
        self.cloud_account_id()
            .or_else(|| lock(&self.cloud_projects().inner).account.clone())
    }

    /// Whether this computer has seen cloud backup for the signed-in account.
    fn cloud_known(&self) -> bool {
        self.cloud_account()
            .is_some_and(|id| self.cloud_file().accounts.get(&id).is_some_and(|a| a.known))
    }

    /// Whether this build has cloud backup ([`super::CLOUD_BUILD`]; tests can pretend not).
    pub fn cloud_available(&self) -> bool {
        super::CLOUD_BUILD
            && !self
                .cloud_projects()
                .test_unavailable
                .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// An error for commands that act on cloud backup in a build without it.
    pub(crate) fn cloud_build_check(&self) -> AppResult<()> {
        if self.cloud_available() {
            Ok(())
        } else {
            Err(AppError::Invalid(
                "Cloud backup isn't in this version of Kinetra.".into(),
            ))
        }
    }

    /// What the app may do with cloud backup now (see the module docs).
    pub fn cloud_access(&self) -> CloudAccess {
        if !self.cloud_available() {
            CloudAccess::Unavailable
        } else if !self.cloud_signed_in() {
            CloudAccess::SignedOut
        } else if self.require(PRO_CLOUD).is_ok() || self.cloud_known() {
            CloudAccess::Available
        } else {
            CloudAccess::Free
        }
    }

    // ---- The view ----

    pub fn cloud_view(&self) -> CloudView {
        let access = self.cloud_access();
        let prefs = self.prefs().cloud;
        let project = self.cloud_project_view(access);
        let s = lock(&self.cloud_projects().inner);
        let client = s
            .quota
            .as_ref()
            .map(|q| q.client.clone())
            .unwrap_or_default();
        let shown = !matches!(access, CloudAccess::SignedOut | CloudAccess::Unavailable);
        CloudView {
            access,
            state: s.quota.as_ref().filter(|_| shown).map(|q| q.state),
            offline: shown && s.offline,
            checked_at: s.checked_at.filter(|_| shown),
            quota: s.quota.as_ref().filter(|_| shown).map(quota_view),
            settings: CloudSettingsView {
                on_air: prefs.on_air.unwrap_or(client.on_air),
                limit_kbps: prefs.limit_kbps.unwrap_or(client.on_air_limit_kbps),
                interval_s: client.backup_interval_s,
                new_projects: prefs.new_projects,
            },
            project,
            restore: s.restore.as_ref().map(|r| r.view.clone()),
            held_until: s.held_until.clone().filter(|_| shown),
        }
    }

    /// The last quota answer.
    pub(super) fn cloud_quota(&self) -> Option<Quota> {
        lock(&self.cloud_projects().inner).quota.clone()
    }

    /// The last quota answer's state.
    pub(super) fn cloud_quota_state(&self) -> Option<QuotaState> {
        lock(&self.cloud_projects().inner)
            .quota
            .as_ref()
            .map(|q| q.state)
    }

    /// Sends `cloud-changed` when the view differs from the last one sent.
    pub(crate) fn emit_cloud(&self) {
        let view = self.cloud_view();
        self.cloud_schedule_follow(&view);
        // Transfers follow the on-air setting at once (also mid-backup).
        self.cloud_gate()
            .set_policy(view.settings.on_air, view.settings.limit_kbps);
        {
            let mut s = lock(&self.cloud_projects().inner);
            if s.sent.as_ref() == Some(&view) {
                return;
            }
            s.sent = Some(view.clone());
        }
        match serde_json::to_value(&view) {
            Ok(v) => self.events().emit(names::CLOUD_CHANGED, v),
            Err(e) => tracing::warn!("cloud-changed not sent: {e}"),
        }
    }

    // ---- Quota ----

    /// The account id for `cloud.json`, fetched once (`/v1/me`) for sign-ins that predate it.
    async fn cloud_ensure_account_id(&self, api: &Api, session: &str) -> Option<String> {
        if let Some(id) = self.cloud_account() {
            return Some(id);
        }
        let me = api.me(session).await.ok()?;
        self.cloud_set_account_id(&me.id);
        lock(&self.cloud_projects().inner).account = Some(me.id.clone());
        Some(me.id)
    }

    /// Asks the server for the quota (when access allows, or always with `on_demand`) and
    /// returns the view. Offline answers keep the last quota, show "offline" and retry later.
    pub async fn cloud_refresh(&self, on_demand: bool) -> AppResult<CloudView> {
        let access = self.cloud_access();
        if matches!(access, CloudAccess::Unavailable | CloudAccess::SignedOut)
            || (access == CloudAccess::Free && !on_demand)
        {
            self.emit_cloud();
            return Ok(self.cloud_view());
        }
        let (api, session, _) = self.cloud_session_parts()?;
        let account = self.cloud_ensure_account_id(&api, &session).await;
        let result = api.cloud_quota(&session).await;
        let now = self.cloud_now();
        let outcome = {
            let mut s = lock(&self.cloud_projects().inner);
            match result {
                Ok(q) => {
                    let known = q.state != QuotaState::NotInPlan;
                    s.quota = Some(q);
                    s.checked_at = Some(now);
                    s.offline = false;
                    s.fails = 0;
                    if let Some(t) = s.retry.take() {
                        t.abort();
                    }
                    Ok(known)
                }
                Err(e) => {
                    s.fails += 1;
                    match backoff::classify(&e, s.fails, backoff::jitter()) {
                        Retry::After(wait) => {
                            s.offline = true;
                            Err((e, Some(wait)))
                        }
                        Retry::No => Err((e, None)),
                    }
                }
            }
        };
        match outcome {
            Ok(known) => {
                if let (true, Some(id)) = (known, account) {
                    self.cloud_file_update(|f| f.accounts.entry(id).or_default().known = true);
                }
                // The open project registers once the quota allows it.
                self.cloud_maybe_register();
            }
            Err((_, Some(wait))) => self.cloud_retry_quota(wait),
            Err((e, None)) => {
                self.emit_cloud();
                return Err(e.into());
            }
        }
        self.emit_cloud();
        Ok(self.cloud_view())
    }

    /// One sleeping task that asks again after `wait` (replaces a waiting one).
    fn cloud_retry_quota(&self, wait: std::time::Duration) {
        let wait = lock(&self.cloud_projects().test_retry).unwrap_or(wait);
        let Some(rt) = self.live().runtime_handle() else {
            return;
        };
        let weak = self.downgrade();
        let task = rt.spawn(async move {
            tokio::time::sleep(wait).await;
            let Some(st) = weak.upgrade() else { return };
            lock(&st.cloud_projects().inner).retry = None;
            let _ = st.cloud_refresh(false).await;
        });
        if let Some(old) = lock(&self.cloud_projects().inner)
            .retry
            .replace(task.abort_handle())
        {
            old.abort();
        }
    }

    /// A backup was refused because today's upload allowance is used: automatic backups wait until
    /// it resets (one sleeping task; the quota says when). A manual backup still commits what
    /// storage has.
    pub(super) async fn cloud_hold_uploads(&self) {
        let _ = self.cloud_refresh(false).await;
        let until = self
            .cloud_quota()
            .and_then(|q| q.upload_today.resets_at)
            .and_then(|t| {
                time::OffsetDateTime::parse(&t, &time::format_description::well_known::Rfc3339)
                    .ok()
                    .map(|at| (t, at))
            });
        // Without a time from the server: an hour.
        let (text, at) = until.unwrap_or_else(|| {
            let at = time::OffsetDateTime::now_utc() + time::Duration::hours(1);
            let text = at
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default();
            (text, at)
        });
        let wait = lock(&self.cloud_projects().test_retry).unwrap_or_else(|| {
            let left = at - time::OffsetDateTime::now_utc();
            std::time::Duration::try_from(left).unwrap_or_default()
        });
        let Some(rt) = self.live().runtime_handle() else {
            return;
        };
        let weak = self.downgrade();
        let task = rt.spawn(async move {
            tokio::time::sleep(wait).await;
            let Some(st) = weak.upgrade() else { return };
            {
                let mut s = lock(&st.cloud_projects().inner);
                s.held_until = None;
                s.hold = None;
            }
            // A fresh quota (a new day's allowance), then the schedule carries on.
            let _ = st.cloud_refresh(false).await;
        });
        {
            let mut s = lock(&self.cloud_projects().inner);
            s.held_until = Some(text);
            if let Some(old) = s.hold.replace(task.abort_handle()) {
                old.abort();
            }
        }
        self.emit_cloud();
    }

    /// Whether automatic backups wait for tomorrow's upload allowance.
    pub fn cloud_held(&self) -> bool {
        lock(&self.cloud_projects().inner).held_until.is_some()
    }

    /// Asks for the quota again in the background (after the account's storage changed: a
    /// version or project deleted, a backup refused).
    pub(super) fn cloud_refresh_soon(&self) {
        let Some(rt) = self.live().runtime_handle() else {
            return;
        };
        let weak = self.downgrade();
        rt.spawn(async move {
            if let Some(st) = weak.upgrade() {
                let _ = st.cloud_refresh(false).await;
            }
        });
    }

    /// After a license sync: ask for the quota when signed in and allowed (in the background).
    pub(crate) fn cloud_projects_after_sync(&self, reached: bool) {
        if !reached || self.cloud_access() != CloudAccess::Available {
            self.emit_cloud();
            return;
        }
        let Some(rt) = self.live().runtime_handle() else {
            return;
        };
        let weak = self.downgrade();
        rt.spawn(async move {
            if let Some(st) = weak.upgrade() {
                let _ = st.cloud_refresh(false).await;
            }
        });
    }

    /// Signed out: forget the quota (records stay with their account) and stop retrying.
    pub(crate) fn cloud_projects_signed_out(&self) {
        {
            let mut s = lock(&self.cloud_projects().inner);
            s.quota = None;
            s.account = None;
            if let Some(p) = s.project.as_mut() {
                p.question = None;
                p.pending = None;
                p.issue = None;
            }
            s.checked_at = None;
            s.offline = false;
            s.fails = 0;
            if let Some(t) = s.retry.take() {
                t.abort();
            }
            s.held_until = None;
            if let Some(t) = s.hold.take() {
                t.abort();
            }
        }
        self.emit_cloud();
    }

    /// Stops background work (exit).
    pub fn cloud_projects_stop(&self) {
        self.cloud_restore_cancel();
        self.cloud_backup_cancel_all();
        let mut s = lock(&self.cloud_projects().inner);
        if let Some(t) = s.retry.take() {
            t.abort();
        }
        if let Some(t) = s.hold.take() {
            t.abort();
        }
    }
}
