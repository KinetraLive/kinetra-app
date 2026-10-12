//! The licensing service on [`AppState`]: stored state, sign-in (device flow), activation,
//! refresh, keys / revocations / beta status, the on-air rule, and the read-only
//! [`LicenseView`] the UI shows.
//!
//! - **Startup is local:** the keychain token is verified and evaluated with no network. The
//!   background loop ([`AppState::cloud_start`]) then fetches keys, revocations and beta
//!   status and refreshes (activates when signed in without an activation): at start, every
//!   12 h, backoff 1 min · 5 min · 30 min · 2 h · 12 h after failures. One sleeping task.
//! - **The core decides:** [`AppState::require`] checks the *effective* entitlements. The UI
//!   only reads [`LicenseView`]; no command sets license state.
//! - **Never on air:** losing access waits while anything is on air (`license::settle`);
//!   the change listener applies it once nothing is.
//! - **Secrets** (session, license token, activation id, a copy of the last server time) live
//!   only in the keychain ([`SecretStore`]); `license.json` holds what isn't secret, and every
//!   signed document in it is verified again when loaded.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use kinetra_license::beta::{self, BetaProgram, BetaState};
use kinetra_license::evaluate::{Access, Status};
use kinetra_license::key_format::{self, KeyFormatError};
use kinetra_license::revocation::{self, RevocationList};
use kinetra_license::KeySet;
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;

use super::api::{self, Api, CallError, SlotReq, UpgradeBlocked};
use super::config;
use super::license::{self, Context, Eval};
use super::machine;
use crate::core::events::names;
use crate::core::fsx;
use crate::core::secrets::SecretStore;
use crate::core::state::{AppState, StateChange};
use crate::error::{AppError, AppResult};

const FILE: &str = "license.json";
const SCHEMA: u32 = 1;
/// Keychain ids (accounts `secret:<id>` under service `Kinetra`).
pub(crate) const S_SESSION: &str = "kinetra-session";
const S_TOKEN: &str = "kinetra-license-token";
const S_ACTIVATION: &str = "kinetra-activation";
const S_SERVER_TIME: &str = "kinetra-server-time";
/// Keychain values are split into pieces this long (Windows Credential Manager caps one blob
/// at 2560 bytes).
const CHUNK: usize = 1200;
/// Signed key-set updates kept (enough to replay several rotations).
const KEY_UPDATES_MAX: usize = 8;
const REFRESH_EVERY: Duration = Duration::from_secs(12 * 3600);
const BACKOFF: [Duration; 5] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(30 * 60),
    Duration::from_secs(2 * 3600),
    Duration::from_secs(12 * 3600),
];

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- Persisted, not secret ------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountInfo {
    pub email: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LicenseFile {
    schema_version: u32,
    /// Signed key-set updates accepted so far (oldest first, at most [`KEY_UPDATES_MAX`]).
    /// Never trusted as such: on load they are applied again, in order, to the embedded set
    /// with `KeySet::apply_update`, so only updates a trusted key vouched for count (editing
    /// this file can't add a key).
    key_updates: Vec<kinetra_license::SignedKeySet>,
    /// The last signed revocation list (verified again on load).
    revocations: Option<String>,
    /// Highest server time seen (unix s). Also in the keychain; the max of both counts.
    last_server_time: i64,
    last_check_at: Option<i64>,
    /// The last signed beta status (verified again on load).
    beta_status: Option<String>,
    account: Option<AccountInfo>,
    /// `GET /v1/license` / activation responses, for display only.
    licenses: Vec<api::License>,
    slot: Option<String>,
    upgrade_blocked: Option<UpgradeBlocked>,
    /// A sign-in has succeeded on this computer once: the welcome screen never shows again.
    welcome_done: bool,
}

// ---- The view the UI reads ------------------------------------------------------------------

/// Sign-in progress (device flow).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SigninView {
    #[default]
    Idle,
    /// Waiting for approval in the browser.
    Waiting {
        #[serde(rename = "userCode")]
        user_code: String,
        url: String,
        #[serde(rename = "expiresAt")]
        expires_at: i64,
    },
    Denied,
    Expired,
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaView {
    pub name: String,
    pub state: &'static str,
    pub ends_at: i64,
    /// The signed end message, one entry per line.
    pub end_message: Vec<String>,
    /// The program has ended (signed state, its end date, or the beta license ran out):
    /// show the end screen once nothing is on air.
    pub ended: bool,
    /// A beta or canary build: the end screen blocks the app (with "Download the stable
    /// version") instead of offering Continue.
    pub blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Notice {
    /// The computer's clock is off by `minutes` (`clock_skew`).
    ClockWrong { minutes: i64 },
    /// The last activation/refresh hit `seats_full`, `backup_in_use` or `backup_not_included`
    /// (the plan has no backup computer): the computers using the license, and whether
    /// "Use as backup" may be offered.
    SeatsFull {
        code: String,
        activations: Vec<api::Activation>,
        backup: bool,
    },
}

/// Pro features the app has, unlocked or not (from the effective entitlements, decided here
/// so the UI never handles entitlement names). Display only: commands check `require`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Features {
    /// AI template generation (`pro.ai`).
    pub ai: bool,
    /// Live polls (`pro.polls`).
    pub polls: bool,
}

impl Features {
    fn from(effective: &[String]) -> Self {
        let has = |e: &str| effective.iter().any(|x| x == e);
        Self {
            ai: has(kinetra_license::entitlements::PRO_AI),
            polls: has(kinetra_license::entitlements::PRO_POLLS),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseView {
    pub signed_in: bool,
    pub account: Option<AccountInfo>,
    /// none | active | reduced | offlineExpired | expired | notCovered | revoked | clockTampered
    pub status: &'static str,
    /// What applies now (effective): full | reduced | free.
    pub access: &'static str,
    /// A stable code, for display decisions only.
    pub plan: Option<String>,
    /// The plan's name to show (`plan_name` of this computer's license), when known.
    pub plan_name: Option<String>,
    /// Effective entitlements (what the core gates on now).
    pub entitlements: Vec<String>,
    /// The app's Pro features, unlocked or not (see [`Features`]).
    pub features: Features,
    /// A loss of access waiting for the show to end: the entitlements that will remain.
    pub pending: Option<Vec<String>>,
    pub ends_at: Option<i64>,
    pub updates_until: Option<i64>,
    pub full_until: Option<i64>,
    pub reduced_until: Option<i64>,
    pub offline_days_left: i64,
    pub last_check_at: Option<i64>,
    /// The last background check reached the server (`None` before the first one).
    pub online: Option<bool>,
    pub machine_name: String,
    pub slot: Option<String>,
    pub licenses: Vec<api::License>,
    pub upgrade_blocked: Option<UpgradeBlocked>,
    pub beta: Option<BetaView>,
    pub notice: Option<Notice>,
    /// Show the first-launch welcome screen: until one sign-in or license key has succeeded on
    /// this computer, the app can't be used.
    pub welcome: bool,
    /// Something is on air (Program layers or a showing output): screens like Beta Ended wait.
    pub on_air: bool,
    pub signin: SigninView,
}

// ---- State ----------------------------------------------------------------------------------

struct St {
    file: LicenseFile,
    keys: KeySet,
    revocations: Option<RevocationList>,
    beta: Option<BetaProgram>,
    /// The newest evaluation (`latest`) and what gates commands now (`effective`).
    eval: Eval,
    effective: Vec<String>,
    /// The access `effective` stands for (the previous one while a loss is pending).
    effective_access: Access,
    online: Option<bool>,
    signin: SigninView,
    notice: Option<Notice>,
    /// The server said this account's plan has no backup computer (`backup_not_included`):
    /// "Use as backup" isn't offered again this session.
    no_backup: bool,
}

/// The licensing service (held by `AppState`; see the module docs).
pub struct Cloud {
    st: Mutex<Option<St>>,
    base: RwLock<String>,
    secrets: RwLock<Arc<dyn SecretStore>>,
    config_dir: RwLock<Option<PathBuf>>,
    /// Tests: a fixed "now" (unix s); 0 = the system clock.
    now: AtomicI64,
    /// A loss of access is waiting (cheap check on every live change).
    pending: AtomicBool,
    /// The beta program has ended: the end screen waits for the show to end, so on-air
    /// changes re-send the view. `last_on_air` is what the last view said.
    beta_ended: AtomicBool,
    last_on_air: AtomicBool,
    loop_task: Mutex<Option<AbortHandle>>,
    wake: Arc<tokio::sync::Notify>,
    signin_task: Mutex<Option<AbortHandle>>,
    /// Tests: the machine fingerprint (production reads `MachineGuid`).
    machine_override: RwLock<Option<String>>,
    /// AI jobs of this session (see `core::cloud::ai`).
    ai_jobs: super::ai::AiJobs,
    /// Tests only: the embedded key set and the beta program slug.
    #[cfg(test)]
    test_keys: RwLock<Option<String>>,
    #[cfg(test)]
    test_beta: RwLock<Option<String>>,
}

impl Default for Cloud {
    fn default() -> Self {
        #[cfg(test)]
        let secrets: Arc<dyn SecretStore> =
            Arc::new(crate::core::secrets::MemorySecrets::default());
        #[cfg(not(test))]
        let secrets: Arc<dyn SecretStore> = Arc::new(crate::core::secrets::KeyringSecrets);
        Self {
            st: Mutex::new(None),
            base: RwLock::new(config::api_base()),
            secrets: RwLock::new(secrets),
            config_dir: RwLock::new(None),
            now: AtomicI64::new(0),
            pending: AtomicBool::new(false),
            beta_ended: AtomicBool::new(false),
            last_on_air: AtomicBool::new(false),
            loop_task: Mutex::new(None),
            wake: Arc::new(tokio::sync::Notify::new()),
            signin_task: Mutex::new(None),
            machine_override: RwLock::new(None),
            ai_jobs: Default::default(),
            #[cfg(test)]
            test_keys: RwLock::new(None),
            #[cfg(test)]
            test_beta: RwLock::new(None),
        }
    }
}

impl Cloud {
    fn now(&self) -> i64 {
        match self.now.load(Ordering::Relaxed) {
            0 => (fsx::now_epoch_ms() / 1000) as i64,
            t => t,
        }
    }

    fn secrets(&self) -> Arc<dyn SecretStore> {
        self.secrets
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn api(&self) -> Result<Api, CallError> {
        let base = self
            .base
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Api::new(base).map_err(|_| CallError::Offline)
    }

    fn machine(&self) -> Option<String> {
        self.machine_override
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .or_else(machine::fingerprint)
    }

    /// A long value in the keychain, in pieces (see [`CHUNK`]).
    fn put_long(&self, id: &str, value: &str) -> AppResult<()> {
        let s = self.secrets();
        let old = self.get_long(id).ok().flatten();
        let pieces: Vec<&str> = value
            .as_bytes()
            .chunks(CHUNK)
            .map(|c| std::str::from_utf8(c).unwrap_or(""))
            .collect();
        s.set(
            id,
            &format!("{}:{}", pieces.len(), pieces.first().copied().unwrap_or("")),
        )?;
        for (i, p) in pieces.iter().enumerate().skip(1) {
            s.set(&format!("{id}-{i}"), p)?;
        }
        // Drop pieces a longer old value left behind.
        if let Some(old) = old {
            for i in pieces.len()..old.len().div_ceil(CHUNK).max(1) {
                let _ = s.delete(&format!("{id}-{i}"));
            }
        }
        Ok(())
    }

    fn get_long(&self, id: &str) -> AppResult<Option<String>> {
        let s = self.secrets();
        let Some(head) = s.get(id)? else {
            return Ok(None);
        };
        let Some((n, first)) = head.split_once(':') else {
            return Ok(None);
        };
        let n: usize = n.parse().unwrap_or(0);
        let mut out = first.to_string();
        for i in 1..n {
            out.push_str(&s.get(&format!("{id}-{i}"))?.unwrap_or_default());
        }
        Ok(Some(out))
    }

    fn delete_long(&self, id: &str) {
        let s = self.secrets();
        let n = s
            .get(id)
            .ok()
            .flatten()
            .and_then(|h| h.split_once(':').and_then(|(n, _)| n.parse::<usize>().ok()))
            .unwrap_or(1);
        let _ = s.delete(id);
        for i in 1..n {
            let _ = s.delete(&format!("{id}-{i}"));
        }
    }

    // ---- Test hooks ----

    #[doc(hidden)]
    pub fn set_test_env(&self, base: &str, secrets: Arc<dyn SecretStore>, machine: &str) {
        *self.base.write().unwrap_or_else(PoisonError::into_inner) =
            base.trim_end_matches('/').to_string();
        *self.secrets.write().unwrap_or_else(PoisonError::into_inner) = secrets;
        *self
            .machine_override
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(machine.to_string());
    }

    pub(crate) fn ai_jobs(&self) -> &super::ai::AiJobs {
        &self.ai_jobs
    }

    #[doc(hidden)]
    pub fn set_test_now(&self, now: i64) {
        self.now.store(now, Ordering::Relaxed);
    }

    /// Beta / canary build (tests can pretend).
    fn prerelease(&self) -> bool {
        #[cfg(test)]
        if self
            .test_beta
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
        {
            return true;
        }
        config::prerelease_build()
    }

    /// Tests: these effective entitlements, full access (or Reduced), without a token.
    #[cfg(test)]
    pub fn set_test_entitlements(&self, effective: &[&str], full: bool) {
        if let Some(s) = lock(&self.st).as_mut() {
            s.effective = effective.iter().map(|e| (*e).to_string()).collect();
            s.effective_access = if full { Access::Full } else { Access::Reduced };
        }
    }

    #[cfg(test)]
    pub fn set_test_trust(&self, keys_json: &str, beta: Option<&str>) {
        *self
            .test_keys
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(keys_json.to_string());
        *self
            .test_beta
            .write()
            .unwrap_or_else(PoisonError::into_inner) = beta.map(str::to_string);
    }

    /// The key set this build trusts from the start.
    fn embedded_keys(&self) -> String {
        #[cfg(test)]
        if let Some(k) = self
            .test_keys
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return k;
        }
        config::TRUSTED_KEYS.to_string()
    }

    /// The beta program of this build.
    fn beta_slug(&self) -> Option<String> {
        #[cfg(test)]
        if let Some(b) = self
            .test_beta
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return Some(b);
        }
        config::beta_program().map(str::to_string)
    }
}

fn status_str(s: Option<Status>) -> &'static str {
    match s {
        None => "none",
        Some(Status::Active) => "active",
        Some(Status::Reduced) => "reduced",
        Some(Status::OfflineExpired) => "offlineExpired",
        Some(Status::Expired) => "expired",
        Some(Status::NotCovered) => "notCovered",
        Some(Status::Revoked) => "revoked",
        Some(Status::ClockTampered) => "clockTampered",
    }
}

fn access_str(a: Access) -> &'static str {
    match a {
        Access::Full => "full",
        Access::Reduced => "reduced",
        Access::Free => "free",
    }
}

fn key_error(e: &KeyFormatError) -> AppError {
    AppError::Invalid(match e {
        KeyFormatError::WrongLength => "A license key has 12 characters after KNTR-.".into(),
        KeyFormatError::InvalidChar(c) => format!("“{c}” can’t be in a license key."),
        KeyFormatError::Checksum => "This key has a typo. Check it against the original.".into(),
    })
}

impl AppState {
    fn cloud(&self) -> &Cloud {
        self.cloud_service()
    }

    /// Loads the stored license (no network): keychain token → verify → evaluate. Call once at
    /// startup, before anything gates on it. Registers the on-air listener.
    pub fn cloud_init(&self) {
        let c = self.cloud();
        *c.config_dir.write().unwrap_or_else(PoisonError::into_inner) =
            Some(self.config_dir().to_path_buf());
        let file: LicenseFile = std::fs::read(self.config_dir().join(FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // The embedded set, then every stored signed update replayed through apply_update.
        let mut keys = KeySet::from_trusted_json(&c.embedded_keys()).unwrap_or_default();
        for u in &file.key_updates {
            if let Err(e) = keys.apply_update(u) {
                tracing::debug!("stored key-set update skipped: {e}");
            }
        }
        let revocations = file
            .revocations
            .as_deref()
            .and_then(|d| revocation::verify_revocations(d, &keys).ok());
        let beta = file
            .beta_status
            .as_deref()
            .and_then(|d| beta::verify_beta_status(d, &keys).ok())
            .and_then(|s| c.beta_slug().and_then(|slug| s.program(&slug).cloned()));
        *lock(&c.st) = Some(St {
            file,
            keys,
            revocations,
            beta,
            eval: Eval::none(),
            effective: Vec::new(),
            effective_access: Access::Free,
            online: None,
            signin: SigninView::Idle,
            notice: None,
            no_backup: false,
        });
        self.license_reevaluate(true);
        // Losing access waits for the show to end: apply once nothing is on air.
        self.add_change_listener(Arc::new(|st: &AppState, ch: StateChange<'_>| {
            if !matches!(ch, StateChange::Live) {
                return;
            }
            let c = st.cloud();
            if c.pending.load(Ordering::Relaxed) {
                st.license_settle();
            }
            if c.beta_ended.load(Ordering::Relaxed)
                && st.license_on_air() != c.last_on_air.load(Ordering::Relaxed)
            {
                st.emit_license();
            }
        }));
    }

    /// Starts the background loop (keys, revocations, beta, refresh). No-op without a runtime.
    pub fn cloud_start(&self) {
        let Some(rt) = self.live().runtime_handle() else {
            return;
        };
        let weak = self.downgrade();
        let wake = self.cloud().wake.clone();
        let task = rt.spawn(async move {
            let mut fails = 0usize;
            loop {
                let Some(st) = weak.upgrade() else { return };
                let ok = st.cloud_sync().await;
                drop(st);
                let wait = if ok {
                    fails = 0;
                    REFRESH_EVERY
                } else {
                    let w = BACKOFF[fails.min(BACKOFF.len() - 1)];
                    fails += 1;
                    w
                };
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = wake.notified() => {}
                }
            }
        });
        if let Some(old) = lock(&self.cloud().loop_task).replace(task.abort_handle()) {
            old.abort();
        }
    }

    /// Stops background work (exit).
    pub fn cloud_stop(&self) {
        if let Some(t) = lock(&self.cloud().loop_task).take() {
            t.abort();
        }
        if let Some(t) = lock(&self.cloud().signin_task).take() {
            t.abort();
        }
    }

    /// The core's Pro gate: `Ok` when the effective entitlements include `entitlement` (use
    /// `kinetra_license::entitlements` constants). Never trusts anything from the UI.
    pub fn require(&self, entitlement: &str) -> AppResult<()> {
        let g = lock(&self.cloud().st);
        let ok = g
            .as_ref()
            .is_some_and(|s| s.effective.iter().any(|e| e == entitlement));
        if ok {
            Ok(())
        } else {
            Err(AppError::ProRequired)
        }
    }

    /// The gate for starting *new* Pro work (an AI generation, a poll, a sync): the entitlement
    /// and full access (Reduced mode keeps existing Pro work running but starts nothing new).
    pub fn require_new_pro_work(&self, entitlement: &str) -> AppResult<()> {
        self.require(entitlement)?;
        let full = lock(&self.cloud().st)
            .as_ref()
            .is_some_and(|s| s.effective_access == Access::Full);
        if full {
            Ok(())
        } else {
            Err(AppError::ProLimited)
        }
    }

    /// The API client, the session and this machine's fingerprint, for signed-in calls.
    pub(crate) fn cloud_session_parts(&self) -> AppResult<(Api, String, String)> {
        let c = self.cloud();
        let session = self.session()?;
        let machine = c
            .machine()
            .ok_or_else(|| AppError::Internal("This computer has no machine id".into()))?;
        Ok((c.api()?, session, machine))
    }

    /// Whether anything is on air (Program layers or a showing output): losing access waits.
    fn license_on_air(&self) -> bool {
        self.live().any_on_air() || self.any_output_showing()
    }

    /// Re-evaluates the stored token; `startup` applies the result at once (nothing is on air
    /// before the app has started).
    fn license_reevaluate(&self, startup: bool) {
        let c = self.cloud();
        let token = c.get_long(S_TOKEN).ok().flatten();
        let kc_time = c
            .secrets()
            .get(S_SERVER_TIME)
            .ok()
            .flatten()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let machine = c.machine();
        let now = c.now();
        {
            let mut g = lock(&c.st);
            let Some(s) = g.as_mut() else { return };
            s.file.last_server_time = s.file.last_server_time.max(kc_time);
            s.eval = match machine.as_deref() {
                Some(m) => license::evaluate_token(
                    token.as_deref(),
                    &Context {
                        keys: &s.keys,
                        machine: m,
                        now,
                        last_server_time: s.file.last_server_time,
                        release_date: config::release_date(),
                        revocations: s.revocations.as_ref(),
                    },
                ),
                None => Eval::none(),
            };
            if startup {
                s.effective = s.eval.entitlements.clone();
                s.effective_access = s.eval.access;
            }
        }
        if !startup {
            self.license_settle();
        } else {
            c.pending.store(false, Ordering::Relaxed);
        }
    }

    /// Applies the on-air rule and tells the UI when anything changed.
    pub(crate) fn license_settle(&self) {
        let on_air = self.license_on_air();
        let c = self.cloud();
        let changed = {
            let mut g = lock(&c.st);
            let Some(s) = g.as_mut() else { return };
            let next = license::settle(&s.effective, &s.eval.entitlements, on_air);
            let pending = license::pending(&next, &s.eval.entitlements);
            c.pending.store(pending, Ordering::Relaxed);
            let changed = next != s.effective;
            if !pending {
                s.effective_access = s.eval.access;
            }
            s.effective = next;
            changed
        };
        if changed {
            self.emit_license();
        }
    }

    /// The read-only view for the UI (`license_view`, `license-changed`).
    pub fn license_view(&self) -> LicenseView {
        let c = self.cloud();
        let signed_in = c.secrets().get(S_SESSION).ok().flatten().is_some();
        let now = c.now();
        let on_air = self.license_on_air();
        let g = lock(&c.st);
        let Some(s) = g.as_ref() else {
            return LicenseView {
                signed_in,
                account: None,
                status: "none",
                access: "free",
                plan: None,
                plan_name: None,
                entitlements: kinetra_license::evaluate::no_license(),
                features: Features::default(),
                pending: None,
                ends_at: None,
                updates_until: None,
                full_until: None,
                reduced_until: None,
                offline_days_left: 0,
                last_check_at: None,
                online: None,
                machine_name: machine::machine_name(),
                slot: None,
                licenses: Vec::new(),
                upgrade_blocked: None,
                beta: None,
                notice: None,
                welcome: false,
                on_air: false,
                signin: SigninView::Idle,
            };
        };
        let pending = license::pending(&s.effective, &s.eval.entitlements);
        let claims = s.eval.claims.as_ref();
        let beta = s.beta.as_ref().map(|p| {
            let license_ran_out = claims.is_some_and(|cl| cl.plan == "beta")
                && matches!(
                    s.eval.status,
                    Some(Status::Expired | Status::OfflineExpired)
                );
            BetaView {
                name: p.name.clone(),
                state: match p.state {
                    BetaState::Upcoming => "upcoming",
                    BetaState::Active => "active",
                    BetaState::Ended => "ended",
                },
                ends_at: p.ends_at,
                end_message: p.end_message.split('\n').map(str::to_string).collect(),
                ended: p.state == BetaState::Ended || now >= p.ends_at || license_ran_out,
                blocking: c.prerelease(),
            }
        });
        LicenseView {
            signed_in,
            account: s.file.account.clone(),
            status: status_str(s.eval.status),
            access: access_str(s.effective_access),
            plan: claims.map(|c| c.plan.clone()),
            plan_name: claims.and_then(|c| {
                s.file
                    .licenses
                    .iter()
                    .find(|l| l.id == c.lid)
                    .and_then(|l| l.plan_name.clone())
            }),
            entitlements: s.effective.clone(),
            features: Features::from(&s.effective),
            pending: pending.then(|| s.eval.entitlements.clone()),
            ends_at: claims.and_then(|c| c.ends_at),
            updates_until: claims.and_then(|c| c.updates_until),
            full_until: claims.map(|_| s.eval.full_until),
            reduced_until: claims.map(|_| s.eval.reduced_until),
            offline_days_left: s.eval.offline_days_left,
            last_check_at: s.file.last_check_at,
            online: s.online,
            machine_name: machine::machine_name(),
            slot: s.file.slot.clone(),
            licenses: s.file.licenses.clone(),
            upgrade_blocked: s.file.upgrade_blocked.clone(),
            beta,
            notice: s.notice.clone(),
            welcome: !s.file.welcome_done && claims.is_none(),
            on_air,
            signin: s.signin.clone(),
        }
    }

    fn emit_license(&self) {
        let view = self.license_view();
        let c = self.cloud();
        c.beta_ended.store(
            view.beta.as_ref().is_some_and(|b| b.ended),
            Ordering::Relaxed,
        );
        c.last_on_air.store(view.on_air, Ordering::Relaxed);
        if let Ok(v) = serde_json::to_value(view) {
            self.events().emit(names::LICENSE_CHANGED, v);
        }
        // Signing in or out, or Pro changing, decides which polls may stream.
        self.polls_reconcile();
    }

    fn save_license_file(&self) {
        let c = self.cloud();
        let file = lock(&c.st).as_ref().map(|s| s.file.clone());
        let Some(mut file) = file else { return };
        file.schema_version = SCHEMA;
        let path = self.config_dir().join(FILE);
        if let Err(e) = fsx::write_json_atomic(&path, &file) {
            tracing::warn!("saving {FILE}: {e}");
        }
        let _ = c
            .secrets()
            .set(S_SERVER_TIME, &file.last_server_time.to_string());
    }

    fn see_server_time(&self, t: i64) {
        if let Some(s) = lock(&self.cloud().st).as_mut() {
            s.file.last_server_time = s.file.last_server_time.max(t);
        }
    }

    // ---- Background sync ----

    /// One round: keys, revocations, beta status, then refresh / activate when signed in.
    /// `true` when the server was reached (the license may still be unchanged).
    pub async fn cloud_sync(&self) -> bool {
        let c = self.cloud();
        let Ok(api) = c.api() else { return false };
        let reached = self.sync_public(&api).await;
        let session = c.secrets().get(S_SESSION).ok().flatten();
        let mut ok = reached;
        if let Some(session) = session {
            ok = match self.refresh_or_activate(&api, &session).await {
                Ok(()) => true,
                Err(e) => !e.is_offline() && reached,
            };
        }
        let now = c.now();
        if let Some(s) = lock(&c.st).as_mut() {
            s.online = Some(ok);
            if ok {
                s.file.last_check_at = Some(now);
            }
        }
        self.save_license_file();
        self.license_reevaluate(false);
        self.emit_license();
        ok
    }

    /// Keys, revocations and beta status (public). `false` when the server can't be reached.
    async fn sync_public(&self, api: &Api) -> bool {
        let c = self.cloud();
        match api.keys().await {
            Ok(update) => {
                let mut g = lock(&c.st);
                if let Some(s) = g.as_mut() {
                    let mut k = s.keys.clone();
                    match k.apply_update(&update) {
                        Ok(()) => {
                            if k != s.keys {
                                s.file.key_updates.push(update);
                                let n = s.file.key_updates.len();
                                if n > KEY_UPDATES_MAX {
                                    s.file.key_updates.drain(..n - KEY_UPDATES_MAX);
                                }
                            }
                            s.keys = k;
                        }
                        Err(e) => tracing::info!("key set update not applied: {e}"),
                    }
                }
            }
            Err(e) if e.is_offline() => return false,
            Err(_) => {}
        }
        if let Ok(doc) = api.revocations().await {
            let mut g = lock(&c.st);
            if let Some(s) = g.as_mut() {
                match revocation::verify_revocations(&doc, &s.keys) {
                    Ok(list) => {
                        s.revocations = Some(list);
                        s.file.revocations = Some(doc);
                    }
                    Err(e) => tracing::info!("revocation list not accepted: {e}"),
                }
            }
        }
        if let Some(slug) = c.beta_slug() {
            let session = c.secrets().get(S_SESSION).ok().flatten();
            if let Ok(doc) = api.beta_status(session.as_deref(), &slug).await {
                let mut g = lock(&c.st);
                if let Some(s) = g.as_mut() {
                    if let Ok(status) = beta::verify_beta_status(&doc, &s.keys) {
                        s.beta = status.program(&slug).cloned();
                        s.file.beta_status = Some(doc);
                        s.file.last_server_time = s.file.last_server_time.max(status.iat);
                    }
                }
            }
        }
        true
    }

    async fn refresh_or_activate(&self, api: &Api, session: &str) -> Result<(), CallError> {
        let c = self.cloud();
        let Some(machine) = c.machine() else {
            return Err(CallError::Offline);
        };
        let activation = c.secrets().get(S_ACTIVATION).ok().flatten();
        let nonce = license::nonce().map_err(|_| CallError::Offline)?;
        let result = match &activation {
            Some(aid) => api.refresh(session, aid, &machine, &nonce, c.now()).await,
            None => {
                api.activate(
                    session,
                    &machine,
                    &machine::machine_name(),
                    SlotReq::Main,
                    None,
                    &nonce,
                    c.now(),
                )
                .await
            }
        };
        match result {
            Ok(a) => self.apply_activated(a, &nonce, &machine),
            Err(CallError::Api { code, .. })
                if code == "activation_revoked" && activation.is_some() =>
            {
                // Deactivated elsewhere or replaced: activate again (free and unlimited).
                c.secrets().delete(S_ACTIVATION).ok();
                let nonce = license::nonce().map_err(|_| CallError::Offline)?;
                let a = api
                    .activate(
                        session,
                        &machine,
                        &machine::machine_name(),
                        SlotReq::Main,
                        None,
                        &nonce,
                        c.now(),
                    )
                    .await;
                match a {
                    Ok(a) => self.apply_activated(a, &nonce, &machine),
                    Err(e) => self.note_error(e),
                }
            }
            Err(e) => self.note_error(e),
        }
    }

    /// Keeps state on errors; notes what the UI should offer. Session errors sign out.
    fn note_error(&self, e: CallError) -> Result<(), CallError> {
        let c = self.cloud();
        if let CallError::Api { code, details, .. } = &e {
            match code.as_str() {
                "invalid_token" | "unauthorized" => {
                    // The session is gone: sign out locally; the license token stays valid
                    // per policy.
                    let _ = c.secrets().delete(S_SESSION);
                }
                "clock_skew" => {
                    let now = c.now();
                    if let Some(server) = license::skew_server_time(details.as_ref()) {
                        if let Some(s) = lock(&c.st).as_mut() {
                            s.notice = Some(Notice::ClockWrong {
                                minutes: (now - server) / 60,
                            });
                        }
                    }
                }
                "seats_full" | "backup_in_use" | "backup_not_included" => {
                    let mut acts: Vec<api::Activation> = details
                        .as_ref()
                        .and_then(|d| d.get("activations"))
                        .and_then(|a| serde_json::from_value(a.clone()).ok())
                        .unwrap_or_default();
                    if let Some(s) = lock(&c.st).as_mut() {
                        if code == "backup_not_included" {
                            s.no_backup = true;
                        }
                        // `backup_not_included` lists no computers: keep the ones the seats
                        // choice showed, so the operator can still free a seat.
                        if acts.is_empty() {
                            if let Some(Notice::SeatsFull { activations, .. }) = &s.notice {
                                acts = activations.clone();
                            }
                        }
                        // The license these computers use says whether it has a backup slot.
                        let included = s
                            .file
                            .licenses
                            .iter()
                            .find(|l| {
                                l.activations
                                    .iter()
                                    .any(|a| acts.iter().any(|b| b.id == a.id))
                            })
                            .and_then(|l| l.backup_included);
                        s.notice = Some(Notice::SeatsFull {
                            backup: code == "seats_full" && !s.no_backup && included != Some(false),
                            code: code.clone(),
                            activations: acts,
                        });
                    }
                }
                _ => {}
            }
        }
        Err(e)
    }

    /// Stores a fresh token after checking it: signature, machine, and the echoed nonce.
    fn apply_activated(
        &self,
        a: api::Activated,
        nonce: &str,
        machine: &str,
    ) -> Result<(), CallError> {
        let c = self.cloud();
        let verified = {
            let g = lock(&c.st);
            let keys = g.as_ref().map(|s| s.keys.clone()).unwrap_or_default();
            kinetra_license::verify_license(&a.token, &keys, machine)
        };
        let claims = match verified {
            Ok(v) => v.claims,
            Err(e) => {
                tracing::warn!("activation token rejected: {e}");
                return Err(CallError::Offline);
            }
        };
        if claims.nonce.as_deref().is_some_and(|n| n != nonce) {
            tracing::warn!("activation token rejected: nonce mismatch");
            return Err(CallError::Offline);
        }
        if c.put_long(S_TOKEN, &a.token).is_err()
            || c.secrets().set(S_ACTIVATION, &a.activation_id).is_err()
        {
            tracing::warn!("the keychain refused the license token");
            return Err(CallError::Offline);
        }
        self.see_server_time(a.server_time.max(claims.iat));
        if let Some(s) = lock(&c.st).as_mut() {
            s.file.slot = Some(a.slot.clone());
            s.file.upgrade_blocked = a.upgrade_blocked.clone();
            if a.upgrade_blocked
                .as_ref()
                .is_some_and(|b| b.reason == "backup_not_included")
            {
                s.no_backup = true;
            }
            s.file.welcome_done = true;
            s.notice = None;
            match s.file.licenses.iter_mut().find(|l| l.id == a.license.id) {
                Some(l) => *l = a.license.clone(),
                None => s.file.licenses.push(a.license.clone()),
            }
        }
        Ok(())
    }

    // ---- Operations (commands) ----

    /// Starts the device flow: returns at once with the code; a task polls until approved,
    /// denied or expired (`license-changed` carries the progress). On approval the session goes
    /// to the keychain and this computer activates.
    pub async fn sign_in_start(&self) -> AppResult<SigninView> {
        let c = self.cloud();
        let api = c.api()?;
        let start = api.device_start(Some(&machine::machine_name())).await?;
        let now = c.now();
        let view = SigninView::Waiting {
            user_code: start.user_code.clone(),
            url: start.verification_uri_complete.clone(),
            expires_at: now + start.expires_in as i64,
        };
        if let Some(s) = lock(&c.st).as_mut() {
            s.signin = view.clone();
        }
        self.emit_license();
        let weak = self.downgrade();
        let rt = self.live().runtime_handle();
        let fut = async move {
            let mut interval = start.interval.max(1);
            let deadline = now + start.expires_in as i64;
            loop {
                tokio::time::sleep(Duration::from_secs(interval)).await;
                let Some(st) = weak.upgrade() else { return };
                let res = api.device_token(&start.device_code).await;
                let next = match res {
                    Ok(tok) => {
                        let c = st.cloud();
                        if c.secrets().set(S_SESSION, &tok.session_token).is_err() {
                            SigninView::Failed {
                                message: "The system keychain refused the sign-in.".into(),
                            }
                        } else {
                            let name = [tok.user.first_name.clone(), tok.user.last_name.clone()]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" ");
                            if let Some(s) = lock(&c.st).as_mut() {
                                s.file.account = Some(AccountInfo {
                                    email: tok.user.email.clone(),
                                    name: (!name.trim().is_empty()).then_some(name),
                                });
                                s.file.welcome_done = true;
                                s.signin = SigninView::Idle;
                            }
                            st.save_license_file();
                            st.emit_license();
                            // Activate now (puts this computer on the best license).
                            st.cloud_sync().await;
                            return;
                        }
                    }
                    Err(CallError::Api { code, .. }) => match code.as_str() {
                        "authorization_pending" => continue,
                        "slow_down" => {
                            interval += 5;
                            continue;
                        }
                        "access_denied" => SigninView::Denied,
                        "expired_token" | "invalid_token" => SigninView::Expired,
                        "rate_limited" => {
                            interval += 5;
                            continue;
                        }
                        _ => SigninView::Failed {
                            message: "Sign-in failed. Try again.".into(),
                        },
                    },
                    Err(CallError::Offline) => {
                        if st.cloud().now() < deadline {
                            continue;
                        }
                        SigninView::Expired
                    }
                };
                if let Some(s) = lock(&st.cloud().st).as_mut() {
                    s.signin = next;
                }
                st.emit_license();
                return;
            }
        };
        let handle = match rt {
            Some(rt) => rt.spawn(fut),
            None => tokio::spawn(fut),
        };
        if let Some(old) = lock(&c.signin_task).replace(handle.abort_handle()) {
            old.abort();
        }
        Ok(view)
    }

    pub fn sign_in_cancel(&self) {
        if let Some(t) = lock(&self.cloud().signin_task).take() {
            t.abort();
        }
        if let Some(s) = lock(&self.cloud().st).as_mut() {
            s.signin = SigninView::Idle;
        }
        self.emit_license();
    }

    /// Signs out (ends the session on the server when reachable). The license token stays and
    /// keeps its offline validity; nothing on air changes.
    pub async fn sign_out(&self) -> AppResult<()> {
        let c = self.cloud();
        if let Some(session) = c.secrets().get(S_SESSION)? {
            if let Ok(api) = c.api() {
                let _ = api.logout(&session).await;
            }
        }
        c.secrets().delete(S_SESSION)?;
        if let Some(s) = lock(&c.st).as_mut() {
            s.file.account = None;
            s.file.upgrade_blocked = None;
        }
        self.save_license_file();
        self.emit_license();
        Ok(())
    }

    fn session(&self) -> AppResult<String> {
        self.cloud()
            .secrets()
            .get(S_SESSION)?
            .ok_or_else(|| AppError::Cloud {
                api_code: "unauthorized".into(),
                message: "Sign in first.".into(),
                details: None,
            })
    }

    /// Activates this computer on a chosen license / slot ("Use as backup", "Use this
    /// license").
    pub async fn license_activate(
        &self,
        slot: SlotReq,
        license_id: Option<String>,
    ) -> AppResult<()> {
        let c = self.cloud();
        let session = self.session()?;
        let machine = c
            .machine()
            .ok_or_else(|| AppError::Internal("This computer has no machine id".into()))?;
        let nonce = license::nonce().map_err(AppError::Internal)?;
        let res = c
            .api()?
            .activate(
                &session,
                &machine,
                &machine::machine_name(),
                slot,
                license_id.as_deref(),
                &nonce,
                c.now(),
            )
            .await;
        let out = match res {
            Ok(a) => self.apply_activated(a, &nonce, &machine),
            Err(e) => self.note_error(e),
        };
        self.save_license_file();
        self.license_reevaluate(false);
        self.emit_license();
        out.map_err(Into::into)
    }

    /// Deactivates a computer of the account (this one included). Deactivating this computer
    /// drops its token (the change waits for the show to end).
    pub async fn license_deactivate(&self, activation_id: &str) -> AppResult<()> {
        let c = self.cloud();
        let session = self.session()?;
        c.api()?.deactivate(&session, activation_id).await?;
        let own = c.secrets().get(S_ACTIVATION)?.as_deref() == Some(activation_id);
        if own {
            c.delete_long(S_TOKEN);
            c.secrets().delete(S_ACTIVATION)?;
            if let Some(s) = lock(&c.st).as_mut() {
                s.file.slot = None;
            }
        }
        if let Some(s) = lock(&c.st).as_mut() {
            for l in &mut s.file.licenses {
                l.activations.retain(|a| a.id != activation_id);
            }
        }
        self.save_license_file();
        self.license_reevaluate(false);
        self.emit_license();
        Ok(())
    }

    /// "Pro is active on <computer>. Move Pro here?": deactivate there, then refresh here.
    pub async fn license_move_here(&self, activation_id: &str) -> AppResult<()> {
        let session = self.session()?;
        self.cloud()
            .api()?
            .deactivate(&session, activation_id)
            .await?;
        self.cloud_sync().await;
        Ok(())
    }

    /// Redeems a license key (any formatting), then refreshes onto the best license.
    pub async fn license_redeem(&self, key: &str) -> AppResult<()> {
        let key = key_format::normalize(key).map_err(|e| key_error(&e))?;
        let session = self.session()?;
        self.cloud().api()?.redeem(&session, &key).await?;
        self.cloud_sync().await;
        Ok(())
    }

    /// The account's licenses and computers (`GET /v1/license`), for Preferences.
    pub async fn license_list(&self) -> AppResult<Vec<api::License>> {
        let c = self.cloud();
        let session = self.session()?;
        let machine = c
            .machine()
            .ok_or_else(|| AppError::Internal("This computer has no machine id".into()))?;
        let l = c.api()?.licenses(&session, &machine).await?;
        self.see_server_time(l.server_time);
        if let Some(s) = lock(&c.st).as_mut() {
            s.file.licenses = l.licenses.clone();
        }
        self.save_license_file();
        self.emit_license();
        Ok(l.licenses)
    }

    /// Wakes the background loop now ("Check now", after sign-in).
    pub fn license_check_now(&self) {
        self.cloud().wake.notify_one();
    }
}

#[cfg(test)]
mod tests;
