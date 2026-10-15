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
use crate::error::AppResult;

/// The entitlement cloud backup needs (`kinetra_license::entitlements::PRO_CLOUD_SYNC`).
pub const PRO_CLOUD: &str = kinetra_license::entitlements::PRO_CLOUD_SYNC;

#[derive(Default)]
struct Svc {
    quota: Option<Quota>,
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
}

/// Cloud backup's state (held by `AppState`; see the module docs).
#[derive(Default)]
pub struct CloudProjects {
    inner: Mutex<Svc>,
    /// Tests: the wait before a retry instead of the backoff.
    test_retry: Mutex<Option<std::time::Duration>>,
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

    /// Changes `cloud.json` and writes it (on the blocking pool) when anything changed.
    pub(crate) fn cloud_file_update(&self, f: impl FnOnce(&mut CloudFile)) {
        let mut file = self.cloud_file();
        let before = file.clone();
        f(&mut file);
        if file == before {
            return;
        }
        lock(&self.cloud_projects().inner).file = Some(file.clone());
        let dir = self.config_dir().to_path_buf();
        let save = move || {
            if let Err(e) = persist::save(&dir, &file) {
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

    /// What the app may do with cloud backup now (see the module docs).
    pub fn cloud_access(&self) -> CloudAccess {
        if !self.cloud_signed_in() {
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
        let s = lock(&self.cloud_projects().inner);
        let client = s
            .quota
            .as_ref()
            .map(|q| q.client.clone())
            .unwrap_or_default();
        let shown = access != CloudAccess::SignedOut;
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
        }
    }

    /// Sends `cloud-changed` when the view differs from the last one sent.
    pub(crate) fn emit_cloud(&self) {
        let view = self.cloud_view();
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
        if access == CloudAccess::SignedOut || (access == CloudAccess::Free && !on_demand) {
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
            s.checked_at = None;
            s.offline = false;
            s.fails = 0;
            if let Some(t) = s.retry.take() {
                t.abort();
            }
        }
        self.emit_cloud();
    }

    /// Stops background work (exit).
    pub fn cloud_projects_stop(&self) {
        if let Some(t) = lock(&self.cloud_projects().inner).retry.take() {
            t.abort();
        }
    }
}
