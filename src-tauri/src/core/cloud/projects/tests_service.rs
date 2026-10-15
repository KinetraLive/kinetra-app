//! Cloud backup through `AppState` against the local mock (no external network; secrets in
//! memory): who may ask the server, the quota and its states, offline retries, settings, and
//! sign-out. Slice 3 adds project identity and registration.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::tests::{err, ok, quota_json, Mock, Reply};
use crate::core::cloud::service::S_SESSION;
use crate::core::events::EventSink;
use crate::core::model::{CloudAccess, CloudOnAir, CloudState};
use crate::core::secrets::{MemorySecrets, SecretStore};
use crate::core::state::AppState;

pub(super) const MACHINE: &str = "857a78b7a23b14cd718a9cbf9b90b0577225543ddcce5ed21854a4985f25ee27";
pub(super) const ACCOUNT: &str = "0199e2a0-aaaa-7000-8000-00000000000a";

#[derive(Default)]
pub(super) struct Rec(Mutex<Vec<(String, Value)>>);

impl EventSink for Rec {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

impl Rec {
    pub fn of(&self, event: &str) -> Vec<Value> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(e, _)| e == event)
            .map(|(_, p)| p.clone())
            .collect()
    }
}

pub(super) struct Env {
    pub tmp: tempfile::TempDir,
    pub rt: tokio::runtime::Runtime,
    pub st: AppState,
    pub mock: Mock,
    pub rec: Arc<Rec>,
    pub secrets: Arc<MemorySecrets>,
}

pub(super) fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Rec::default());
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        rec.clone(),
        Duration::from_secs(60),
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    let mock = Mock::start(&rt);
    mock.on(
        "GET /v1/me",
        vec![ok(200, json!({"id": ACCOUNT, "email": "a@b.c"}))],
    );
    let secrets = Arc::new(MemorySecrets::default());
    st.cloud_service()
        .set_test_env(&mock.base, secrets.clone(), MACHINE);
    st.cloud_init();
    st.cloud_service().set_test_now(1_791_590_400);
    Env {
        tmp,
        rt,
        st,
        mock,
        rec,
        secrets,
    }
}

impl Env {
    pub fn signed_in(&self) {
        self.secrets.set(S_SESSION, "kntr_s_test").unwrap();
    }

    pub fn pro(&self, on: bool) {
        let ent: &[&str] = if on {
            &["core.app", super::service::PRO_CLOUD]
        } else {
            &["core.app"]
        };
        self.st.cloud_service().set_test_entitlements(ent, true);
    }

    pub fn refresh(
        &self,
        on_demand: bool,
    ) -> crate::error::AppResult<crate::core::model::CloudView> {
        self.rt.block_on(self.st.cloud_refresh(on_demand))
    }

    pub fn quota_calls(&self) -> usize {
        self.mock.calls_to("GET /v1/cloud/quota").len()
    }
}

pub(super) fn wait_until(what: &str, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(5), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---- Access ----------------------------------------------------------------------------------

#[test]
fn signed_out_and_free_users_make_no_request() {
    let e = env();
    e.mock
        .on("GET /v1/cloud/quota", vec![ok(200, quota_json("ok"))]);
    assert_eq!(e.refresh(false).unwrap().access, CloudAccess::SignedOut);
    e.signed_in();
    e.pro(false);
    let v = e.refresh(false).unwrap();
    assert_eq!(v.access, CloudAccess::Free);
    assert_eq!(v.state, None);
    // A license sync doesn't ask either.
    e.st.cloud_projects_after_sync(true);
    std::thread::sleep(Duration::from_millis(50));
    assert!(e.mock.calls().is_empty(), "{:?}", e.mock.calls());
}

#[test]
fn pro_asks_for_the_quota_and_the_view_follows_it() {
    let e = env();
    e.signed_in();
    e.pro(true);
    e.mock
        .on("GET /v1/cloud/quota", vec![ok(200, quota_json("warning"))]);
    let v = e.refresh(false).unwrap();
    assert_eq!(v.access, CloudAccess::Available);
    assert_eq!(v.state, Some(CloudState::Warning));
    let q = v.quota.unwrap();
    assert_eq!(
        (q.used_bytes, q.limit_bytes, q.percent),
        (7_340_032_000, 20_000_000_000, 36)
    );
    assert_eq!((q.projects_used, q.projects_limit), (12, 100));
    assert_eq!(v.settings.interval_s, 300);
    assert_eq!(v.settings.on_air, CloudOnAir::Pause);
    assert!(!v.offline);
    // The UI gets camelCase states.
    let sent = e.rec.of("cloud-changed");
    assert_eq!(sent.last().unwrap()["state"], "warning");
    // The same answer again sends nothing new.
    let n = sent.len();
    e.refresh(false).unwrap();
    assert_eq!(e.rec.of("cloud-changed").len(), n);
    // The account was looked up once (this session has no stored account id).
    assert_eq!(e.mock.calls_to("GET /v1/me").len(), 1);
}

#[test]
fn a_license_sync_asks_in_the_background() {
    let e = env();
    e.signed_in();
    e.pro(true);
    e.mock
        .on("GET /v1/cloud/quota", vec![ok(200, quota_json("ok"))]);
    e.st.cloud_projects_after_sync(true);
    wait_until("quota asked", || e.quota_calls() == 1);
    wait_until("view", || e.st.cloud_view().state == Some(CloudState::Ok));
    // Not after a sync that didn't reach the server.
    e.st.cloud_projects_after_sync(false);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(e.quota_calls(), 1);
}

#[test]
fn a_lapsed_account_keeps_asking_without_pro_but_not_in_plan_does_not() {
    let e = env();
    e.signed_in();
    e.pro(true);
    e.mock.on(
        "GET /v1/cloud/quota",
        vec![
            ok(200, quota_json("not_in_plan")),
            ok(200, quota_json("lapsed")),
        ],
    );
    assert_eq!(e.refresh(false).unwrap().state, Some(CloudState::NotInPlan));
    e.pro(false);
    // not_in_plan isn't "seen cloud backup": Free makes no request.
    assert_eq!(e.refresh(false).unwrap().access, CloudAccess::Free);
    assert_eq!(e.quota_calls(), 1);
    // "Check for backups" asks once regardless; a lapsed answer remembers the account.
    let mut lapsed = quota_json("lapsed");
    lapsed["lapse"] = json!({"since": "2026-10-01T00:00:00Z", "purge_at": "2026-12-30T00:00:00Z"});
    e.mock.on("GET /v1/cloud/quota", vec![ok(200, lapsed)]);
    let v = e.refresh(true).unwrap();
    assert_eq!(v.state, Some(CloudState::Lapsed));
    assert_eq!(
        v.quota.unwrap().purge_at.as_deref(),
        Some("2026-12-30T00:00:00Z")
    );
    assert_eq!(v.access, CloudAccess::Available);
    e.refresh(false).unwrap();
    assert_eq!(e.quota_calls(), 3);
    // Remembered on disk, under the account.
    let path = e.tmp.path().join("config").join("cloud.json");
    wait_until("cloud.json", || path.exists());
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["accounts"][ACCOUNT]["known"], true);
    assert_eq!(saved["schemaVersion"], 1);
}

// ---- Offline ---------------------------------------------------------------------------------

#[test]
fn offline_keeps_the_last_quota_and_retries_with_one_task() {
    let e = env();
    e.signed_in();
    e.pro(true);
    e.st.cloud_projects()
        .set_test_retry(Some(Duration::from_millis(80)));
    e.mock.on(
        "GET /v1/cloud/quota",
        vec![
            ok(200, quota_json("ok")),
            err(503, "cloud_unavailable", None),
            ok(200, quota_json("full")),
        ],
    );
    e.refresh(false).unwrap();
    // Switched off on the server: quietly offline, the last answer stays, one retry waits.
    let v = e.refresh(false).unwrap();
    assert!(v.offline);
    assert_eq!(v.state, Some(CloudState::Ok));
    assert!(e.st.cloud_projects().retry_pending());
    wait_until("retried", || {
        e.st.cloud_view().state == Some(CloudState::Full)
    });
    assert!(!e.st.cloud_view().offline);
    wait_until("no task at rest", || !e.st.cloud_projects().retry_pending());
    assert_eq!(e.quota_calls(), 3);
}

#[test]
fn a_refusal_is_an_error_not_offline() {
    let e = env();
    e.signed_in();
    e.pro(true);
    e.mock
        .on("GET /v1/cloud/quota", vec![err(400, "bad_request", None)]);
    let er = e.refresh(false).unwrap_err();
    assert!(matches!(er, crate::error::AppError::Cloud { .. }), "{er:?}");
    assert!(!e.st.cloud_view().offline);
    assert!(!e.st.cloud_projects().retry_pending());
}

// ---- Settings and sign-out ---------------------------------------------------------------------

#[test]
fn this_computers_settings_win_over_the_servers_defaults() {
    let e = env();
    e.signed_in();
    e.pro(true);
    let mut q = quota_json("ok");
    q["client"]["on_air"] = json!("limit");
    q["client"]["on_air_limit_kbps"] = json!(1500);
    e.mock.on("GET /v1/cloud/quota", vec![ok(200, q)]);
    let v = e.refresh(false).unwrap();
    assert_eq!(
        (v.settings.on_air, v.settings.limit_kbps),
        (CloudOnAir::Limit, 1500)
    );
    assert!(v.settings.new_projects);
    let mut prefs = e.st.prefs();
    prefs.cloud.on_air = Some(CloudOnAir::Pause);
    prefs.cloud.limit_kbps = Some(5); // clamped to the minimum
    prefs.cloud.new_projects = false;
    let stored = e.st.set_prefs(prefs);
    assert_eq!(stored.cloud.limit_kbps, Some(64));
    let v = e.st.cloud_view();
    assert_eq!(
        (v.settings.on_air, v.settings.limit_kbps),
        (CloudOnAir::Pause, 64)
    );
    assert!(!v.settings.new_projects);
    // The change went out.
    let last = e.rec.of("cloud-changed").pop().unwrap();
    assert_eq!(last["settings"]["onAir"], "pause");
}

#[test]
fn signing_out_forgets_the_quota_and_stops_retrying() {
    let e = env();
    e.signed_in();
    e.pro(true);
    e.st.cloud_projects()
        .set_test_retry(Some(Duration::from_secs(60)));
    e.mock.on(
        "GET /v1/cloud/quota",
        vec![
            ok(200, quota_json("ok")),
            Reply {
                status: 502,
                body: None,
                headers: Vec::new(),
            },
        ],
    );
    e.refresh(false).unwrap();
    e.refresh(false).unwrap();
    assert!(e.st.cloud_projects().retry_pending());
    e.mock.on("POST /v1/auth/logout", vec![ok(204, json!({}))]);
    e.rt.block_on(e.st.sign_out()).unwrap();
    let v = e.st.cloud_view();
    assert_eq!(v.access, CloudAccess::SignedOut);
    assert!(v.quota.is_none() && v.state.is_none() && !v.offline);
    assert!(!e.st.cloud_projects().retry_pending());
}
