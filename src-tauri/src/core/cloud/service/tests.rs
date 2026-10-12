//! The licensing service through `AppState` against a local mock of the Kinetra API (no
//! external network; secrets in memory). Covers sign-in, activation, refresh errors, the
//! on-air rule, revocation, key-set updates (and a tampered `license.json`), betas, and that
//! nothing from the frontend can set license state.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use ed25519_dalek::SigningKey;
use kinetra_license::beta::{sign_beta_status, BetaProgram, BetaState, BetaStatus};
use kinetra_license::entitlements::{CORE_APP, PRO_AI, PRO_POLLS};
use kinetra_license::keys::{key_info, sign_keyset, KeyStatus};
use kinetra_license::revocation::sign_revocations;
use kinetra_license::token::sign_license;
use kinetra_license::{KeySet, LicenseClaims, Slot};
use serde_json::{json, Value};

use super::*;
use crate::core::events::NullSink;
use crate::core::model::LiveEdit;
use crate::core::secrets::MemorySecrets;

const MACHINE: &str = "857a78b7a23b14cd718a9cbf9b90b0577225543ddcce5ed21854a4985f25ee27";
const T0: i64 = 1_791_590_400;
const DAY: i64 = 86_400;

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}

fn trusted_json() -> String {
    KeySet::from_infos(&[key_info("test-k1", &key(1), KeyStatus::Active)], 1)
        .unwrap()
        .to_json()
}

fn claims(plan: &str, ent: &[&str]) -> LicenseClaims {
    LicenseClaims {
        typ: "license".into(),
        v: 1,
        lid: "lic-1".into(),
        aid: "act-1".into(),
        sub: "acc-1".into(),
        plan: plan.into(),
        ent: ent.iter().map(|s| (*s).to_string()).collect(),
        machine: MACHINE.into(),
        slot: Slot::Main,
        iat: T0,
        grace: T0 + 30 * DAY,
        reduced: T0 + 44 * DAY,
        ends_at: None,
        updates_until: None,
        nonce: None,
    }
}

fn pro() -> LicenseClaims {
    claims("pro_yearly", &[CORE_APP, PRO_AI, PRO_POLLS])
}

fn free() -> LicenseClaims {
    claims("free", &[CORE_APP])
}

// ---- Mock API ---------------------------------------------------------------------------------

#[derive(Clone)]
enum Reply {
    Json(u16, Value),
    /// An activation answer whose token is signed now, echoing the request's nonce (unless
    /// `nonce` is set) with `claims`.
    Token {
        claims: Box<LicenseClaims>,
        aid: String,
        nonce: Option<String>,
        upgrade_blocked: Option<Value>,
    },
}

fn err(status: u16, code: &str, details: Option<Value>) -> Reply {
    let mut e = json!({"error": {"code": code, "message": format!("msg {code}")}});
    if let Some(d) = details {
        e["error"]["details"] = d;
    }
    Reply::Json(status, e)
}

fn token_reply(c: LicenseClaims) -> Reply {
    Reply::Token {
        claims: Box::new(c),
        aid: "act-1".into(),
        nonce: None,
        upgrade_blocked: None,
    }
}

#[derive(Default)]
struct MockState {
    /// Path → replies (the last one repeats).
    replies: HashMap<String, VecDeque<Reply>>,
    /// (path, body, bearer).
    calls: Vec<(String, Value, Option<String>)>,
}

type Mock = Arc<Mutex<MockState>>;

async fn mock_api(
    State(m): State<Mock>,
    uri: Uri,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    let path = uri.path().to_string();
    let body = body.map(|b| b.0).unwrap_or(Value::Null);
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    let reply = {
        let mut g = m.lock().unwrap();
        g.calls.push((path.clone(), body.clone(), bearer));
        let q = g.replies.get_mut(&path);
        match q {
            Some(q) if q.len() > 1 => q.pop_front(),
            Some(q) => q.front().cloned(),
            None => None,
        }
    };
    let Some(reply) = reply else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"code": "not_found", "message": "-"}})),
        )
            .into_response();
    };
    match reply {
        Reply::Json(s, v) => (StatusCode::from_u16(s).unwrap(), Json(v)).into_response(),
        Reply::Token {
            claims,
            aid,
            nonce,
            upgrade_blocked,
        } => {
            let mut claims = *claims;
            claims.nonce =
                Some(nonce.unwrap_or_else(|| body["nonce"].as_str().unwrap_or("").to_string()));
            claims.aid = aid.clone();
            let token = sign_license(&claims, "test-k1", &key(1));
            let mut v = json!({
                "token": token, "server_time": claims.iat, "activation_id": aid, "slot": "main",
                "license": {"id": claims.lid, "plan": claims.plan, "kind": "subscription", "status": "active",
                            "entitlements": claims.ent, "seats": 1, "seats_used": 1, "backup_used": false,
                            "activations": []}
            });
            if let Some(u) = upgrade_blocked {
                v["upgrade_blocked"] = u;
            }
            (StatusCode::OK, Json(v)).into_response()
        }
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    st: AppState,
    mock: Mock,
    secrets: Arc<MemorySecrets>,
}

impl Env {
    fn reply(&self, path: &str, replies: Vec<Reply>) {
        self.mock
            .lock()
            .unwrap()
            .replies
            .insert(path.into(), replies.into());
    }

    fn calls(&self, path: &str) -> Vec<(Value, Option<String>)> {
        self.mock
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|c| c.0 == path)
            .map(|c| (c.1.clone(), c.2.clone()))
            .collect()
    }

    fn sync(&self) -> bool {
        self.rt.block_on(self.st.cloud_sync())
    }

    fn pro_ok(&self) -> bool {
        self.st.require(PRO_AI).is_ok()
    }

    /// Signed in, with the public endpoints answering.
    fn signed_in(&self) {
        self.secrets.set(S_SESSION, "kntr_s_test").unwrap();
    }

    /// Reloads the service from what is stored (a restart).
    fn restart(&self) {
        self.st.cloud_init();
    }
}

fn env() -> Env {
    env_with(None)
}

fn env_with(beta: Option<&str>) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        Arc::new(NullSink),
        Duration::from_secs(60),
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    let mock: Mock = Arc::default();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .fallback(mock_api)
        .with_state(mock.clone());
    rt.spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let secrets = Arc::new(MemorySecrets::default());
    st.cloud_service()
        .set_test_env(&base, secrets.clone(), MACHINE);
    st.cloud_service().set_test_trust(&trusted_json(), beta);
    st.cloud_service().set_test_now(T0 + DAY);
    st.cloud_init();
    let e = Env {
        _tmp: tmp,
        rt,
        st,
        mock,
        secrets,
    };
    // Public endpoints: an unchanged key set (signed by the trusted key) and an empty list.
    e.reply(
        "/v1/license/keys",
        vec![Reply::Json(200, keyset_update(&[("test-k1", 1)], 1))],
    );
    e.reply(
        "/v1/license/revocations",
        vec![Reply::Json(200, revocations(&[]))],
    );
    e
}

fn keyset_update(keys: &[(&str, u8)], iat: i64) -> Value {
    let infos: Vec<_> = keys
        .iter()
        .map(|(k, n)| key_info(k, &key(*n), KeyStatus::Active))
        .collect();
    let signed = sign_keyset(&infos, iat, &[("test-k1", &key(1))]);
    serde_json::to_value(signed).unwrap()
}

fn revocations(activations: &[&str]) -> Value {
    let list = kinetra_license::revocation::RevocationList {
        typ: "revocations".into(),
        v: 1,
        iat: T0,
        licenses: vec![],
        activations: activations.iter().map(|s| (*s).to_string()).collect(),
    };
    json!({"revocations": sign_revocations(&list, "test-k1", &key(1))})
}

fn on_air(e: &Env) {
    e.st.live()
        .apply_screen(
            crate::core::media::test_plan(crate::core::model::MediaEndAction::Hold, None),
            LiveEdit::Immediate,
        )
        .unwrap();
    e.st.live().show("main", "clip").unwrap();
    assert!(e.st.live().any_on_air());
}

fn wait_until(what: &str, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(10), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---- Tests ------------------------------------------------------------------------------------

#[test]
fn first_launch_needs_one_sign_in() {
    let e = env();
    let v = e.st.license_view();
    assert_eq!(
        (v.status, v.access, v.welcome, v.signed_in),
        ("none", "free", true, false)
    );
    assert_eq!(v.entitlements, vec![CORE_APP.to_string()]);
    assert!(!e.pro_ok());
    // Still required after a restart until a sign-in has succeeded once.
    e.restart();
    assert!(e.st.license_view().welcome);
    // Once activated, never again (also after signing out and restarting).
    e.signed_in();
    e.reply("/v1/license/activate", vec![token_reply(free())]);
    e.sync();
    assert!(!e.st.license_view().welcome);
    e.rt.block_on(e.st.sign_out()).unwrap();
    e.restart();
    assert!(!e.st.license_view().welcome);
}

#[test]
fn device_flow_signs_in_and_activates_pro() {
    let e = env();
    e.reply(
        "/v1/auth/device",
        vec![Reply::Json(
            200,
            json!({"device_code": "kntr_dc_x", "user_code": "RKPH-XHVM",
                   "verification_uri": "https://account.kinetra.live/device",
                   "verification_uri_complete": "https://account.kinetra.live/device?code=RKPH-XHVM",
                   "expires_in": 600, "interval": 1}),
        )],
    );
    e.reply(
        "/v1/auth/device/token",
        vec![
            err(400, "authorization_pending", None),
            Reply::Json(
                200,
                json!({"session_token": "kntr_s_abc", "token_type": "Bearer", "expires_at": "2027-10-09T20:27:47Z",
                       "user": {"id": "u1", "email": "ola@example.com", "first_name": "Ola", "last_name": "Nordmann"}}),
            ),
        ],
    );
    e.reply("/v1/license/activate", vec![token_reply(pro())]);
    let v = e.rt.block_on(e.st.sign_in_start()).unwrap();
    assert!(matches!(v, SigninView::Waiting { ref user_code, .. } if user_code == "RKPH-XHVM"));
    wait_until("activated", || e.pro_ok());
    let v = e.st.license_view();
    assert!(v.signed_in && !v.welcome);
    assert_eq!(v.account.unwrap().email, "ola@example.com");
    assert_eq!(
        (v.status, v.access, v.plan.as_deref()),
        ("active", "full", Some("pro_yearly"))
    );
    assert!(
        v.features.ai,
        "the view says AI is unlocked (decided in the core)"
    );
    // The session and the token are in the keychain, never in license.json.
    assert_eq!(
        e.secrets.get(S_SESSION).unwrap().as_deref(),
        Some("kntr_s_abc")
    );
    let file = std::fs::read_to_string(e.st.config_dir().join(FILE)).unwrap();
    let token = e.st.cloud_service().get_long(S_TOKEN).unwrap().unwrap();
    assert!(!file.contains("kntr_s_abc") && !file.contains(&token));
    // The activation sent a fresh nonce, a timestamp and the fingerprint, with the session.
    let (body, bearer) = e.calls("/v1/license/activate").remove(0);
    assert_eq!(bearer.as_deref(), Some("kntr_s_abc"));
    assert_eq!(body["machine"], MACHINE);
    assert_eq!(body["nonce"].as_str().unwrap().len(), 32);
    assert_eq!(body["timestamp"], T0 + DAY);
}

#[test]
fn device_flow_denied_and_expired() {
    let e = env();
    let start = Reply::Json(
        200,
        json!({"device_code": "d", "user_code": "AAAA-BBBB", "verification_uri": "u",
               "verification_uri_complete": "u?c", "expires_in": 600, "interval": 1}),
    );
    e.reply("/v1/auth/device", vec![start.clone()]);
    e.reply(
        "/v1/auth/device/token",
        vec![err(400, "access_denied", None)],
    );
    e.rt.block_on(e.st.sign_in_start()).unwrap();
    wait_until("denied", || {
        e.st.license_view().signin == SigninView::Denied
    });
    e.reply(
        "/v1/auth/device/token",
        vec![err(400, "expired_token", None)],
    );
    e.rt.block_on(e.st.sign_in_start()).unwrap();
    wait_until("expired", || {
        e.st.license_view().signin == SigninView::Expired
    });
    assert!(!e.st.license_view().signed_in);
}

#[test]
fn refresh_after_deactivation_elsewhere_activates_again() {
    let e = env();
    e.signed_in();
    e.reply("/v1/license/activate", vec![token_reply(pro())]);
    assert!(e.sync() && e.pro_ok());
    e.reply(
        "/v1/license/refresh",
        vec![err(409, "activation_revoked", None)],
    );
    e.reply(
        "/v1/license/activate",
        vec![Reply::Token {
            claims: Box::new(pro()),
            aid: "act-2".into(),
            nonce: None,
            upgrade_blocked: None,
        }],
    );
    assert!(e.sync());
    assert_eq!(
        e.secrets.get(S_ACTIVATION).unwrap().as_deref(),
        Some("act-2")
    );
    assert!(e.pro_ok());
}

#[test]
fn a_token_with_another_nonce_is_rejected() {
    let e = env();
    e.signed_in();
    e.reply(
        "/v1/license/activate",
        vec![Reply::Token {
            claims: Box::new(pro()),
            aid: "act-1".into(),
            nonce: Some("replayed-nonce-from-an-old-request".into()),
            upgrade_blocked: None,
        }],
    );
    e.sync();
    assert!(!e.pro_ok());
    assert!(e.secrets.get(S_TOKEN).unwrap().is_none());
}

#[test]
fn clock_skew_keeps_the_license_and_says_by_how_much() {
    let e = env();
    e.signed_in();
    e.reply("/v1/license/activate", vec![token_reply(pro())]);
    e.sync();
    e.reply(
        "/v1/license/refresh",
        vec![err(
            400,
            "clock_skew",
            Some(json!({"server_time": T0 + DAY - 3600})),
        )],
    );
    e.sync();
    assert!(e.pro_ok());
    assert_eq!(
        e.st.license_view().notice,
        Some(Notice::ClockWrong { minutes: 60 })
    );
}

#[test]
fn seats_full_lists_the_computers() {
    let e = env();
    e.signed_in();
    e.reply(
        "/v1/license/activate",
        vec![err(
            409,
            "seats_full",
            Some(
                json!({"activations": [{"id": "a9", "machine_name": "STUDIO-PC", "slot": "main"}]}),
            ),
        )],
    );
    e.sync();
    match e.st.license_view().notice {
        Some(Notice::SeatsFull {
            code,
            activations,
            backup,
        }) => {
            assert_eq!(code, "seats_full");
            assert!(backup);
            assert_eq!(activations[0].machine_name.as_deref(), Some("STUDIO-PC"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_plan_without_a_backup_keeps_the_computers_to_free() {
    let e = env();
    e.signed_in();
    e.reply(
        "/v1/license/activate",
        vec![
            err(
                409,
                "seats_full",
                Some(
                    json!({"activations": [{"id": "a9", "machine_name": "STUDIO-PC", "slot": "main"}]}),
                ),
            ),
            err(409, "backup_not_included", None),
        ],
    );
    e.sync();
    assert!(e
        .rt
        .block_on(e.st.license_activate(api::SlotReq::Backup, None))
        .is_err());
    match e.st.license_view().notice {
        Some(Notice::SeatsFull {
            code,
            activations,
            backup,
        }) => {
            assert_eq!(code, "backup_not_included");
            assert_eq!(activations[0].id, "a9");
            assert!(!backup);
        }
        other => panic!("{other:?}"),
    }
    // A later seats choice no longer offers a backup.
    e.reply(
        "/v1/license/activate",
        vec![err(
            409,
            "seats_full",
            Some(json!({"activations": [{"id": "a9", "slot": "main"}]})),
        )],
    );
    let _ =
        e.rt.block_on(e.st.license_activate(api::SlotReq::Main, None));
    assert!(matches!(
        e.st.license_view().notice,
        Some(Notice::SeatsFull { backup: false, .. })
    ));
}

#[test]
fn a_dead_session_signs_out_but_keeps_the_license() {
    let e = env();
    e.signed_in();
    e.reply("/v1/license/activate", vec![token_reply(pro())]);
    e.sync();
    e.reply("/v1/license/refresh", vec![err(401, "invalid_token", None)]);
    e.sync();
    assert!(!e.st.license_view().signed_in);
    assert!(e.pro_ok(), "the token stays valid offline");
}

#[test]
fn offline_keeps_the_state_and_startup_needs_no_network() {
    let e = env();
    e.signed_in();
    e.reply("/v1/license/activate", vec![token_reply(pro())]);
    e.sync();
    // Point the service at a port nobody listens on: offline.
    e.st.cloud_service()
        .set_test_env("http://127.0.0.1:9", e.secrets.clone(), MACHINE);
    assert!(!e.sync());
    assert!(e.pro_ok());
    assert_eq!(e.st.license_view().online, Some(false));
    // A restart evaluates locally: still Pro, no request needed.
    e.restart();
    assert!(e.pro_ok());
    // 31 days later offline: reduced (Pro listed, banner), 45 days: Free.
    e.st.cloud_service().set_test_now(T0 + 31 * DAY);
    e.restart();
    assert_eq!(e.st.license_view().status, "reduced");
    e.st.cloud_service().set_test_now(T0 + 45 * DAY);
    e.restart();
    assert!(!e.pro_ok());
}

#[test]
fn losing_pro_waits_until_nothing_is_on_air() {
    let e = env();
    e.signed_in();
    e.reply("/v1/license/activate", vec![token_reply(pro())]);
    e.sync();
    on_air(&e);
    // Pro ended: the refresh puts this computer on Free.
    e.reply("/v1/license/refresh", vec![token_reply(free())]);
    e.sync();
    assert!(e.pro_ok(), "nothing changes on air");
    let v = e.st.license_view();
    assert!(v.entitlements.contains(&PRO_AI.to_string()));
    assert_eq!(v.pending, Some(vec![CORE_APP.to_string()]));
    // The show ends: the change applies.
    e.st.live().hide("main", "clip").unwrap();
    wait_until("applied after the show", || !e.pro_ok());
    assert_eq!(e.st.license_view().pending, None);
}

#[test]
fn gaining_pro_on_air_applies_at_once() {
    let e = env();
    e.signed_in();
    e.reply("/v1/license/activate", vec![token_reply(free())]);
    e.sync();
    on_air(&e);
    e.reply("/v1/license/refresh", vec![token_reply(pro())]);
    e.sync();
    assert!(e.pro_ok());
}

#[test]
fn revoked_on_the_list_even_when_refresh_fails() {
    let e = env();
    e.signed_in();
    e.reply("/v1/license/activate", vec![token_reply(pro())]);
    e.sync();
    e.reply(
        "/v1/license/revocations",
        vec![Reply::Json(200, revocations(&["act-1"]))],
    );
    e.reply(
        "/v1/license/refresh",
        vec![err(503, "license_service_unavailable", None)],
    );
    e.sync();
    assert_eq!(e.st.license_view().status, "revoked");
    assert!(!e.pro_ok());
}

#[test]
fn key_rotation_applies_and_a_tampered_file_adds_nothing() {
    let e = env();
    // The server rotates to test-k2 (vouched for by test-k1).
    e.reply(
        "/v1/license/keys",
        vec![Reply::Json(
            200,
            keyset_update(&[("test-k1", 1), ("test-k2", 2)], 2),
        )],
    );
    e.sync();
    // A token signed by the new key now verifies, also after a restart (the update is replayed).
    let mut c = pro();
    c.aid = "act-k2".into();
    let t = sign_license(&c, "test-k2", &key(2));
    e.st.cloud_service().put_long(S_TOKEN, &t).unwrap();
    e.restart();
    assert!(e.pro_ok());
    // An attacker edits license.json to trust their own key: ignored.
    let path = e.st.config_dir().join(FILE);
    let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let evil = KeySet::from_infos(&[key_info("evil", &key(66), KeyStatus::Active)], 99).unwrap();
    let evil_update = sign_keyset(&evil.infos(), 99, &[("evil", &key(66))]);
    file["keyUpdates"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(evil_update).unwrap());
    std::fs::write(&path, file.to_string()).unwrap();
    let forged = sign_license(&pro(), "evil", &key(66));
    e.st.cloud_service().put_long(S_TOKEN, &forged).unwrap();
    e.restart();
    assert!(
        !e.pro_ok(),
        "a key nobody trusted vouched for can't sign tokens"
    );
}

#[test]
fn the_last_server_time_survives_a_deleted_file() {
    let e = env();
    e.signed_in();
    let mut c = pro();
    c.iat = T0 + 10 * DAY;
    c.grace = c.iat + 30 * DAY;
    c.reduced = c.iat + 44 * DAY;
    e.reply("/v1/license/activate", vec![token_reply(c)]);
    e.st.cloud_service().set_test_now(T0 + 10 * DAY);
    e.sync();
    assert!(e.pro_ok());
    std::fs::remove_file(e.st.config_dir().join(FILE)).unwrap();
    // The clock is turned back a week: the keychain copy still catches it.
    e.st.cloud_service().set_test_now(T0 + 3 * DAY);
    e.restart();
    assert_eq!(e.st.license_view().status, "clockTampered");
    assert!(!e.pro_ok());
}

#[test]
fn long_tokens_are_split_in_the_keychain() {
    let e = env();
    let long = "x".repeat(CHUNK * 3 + 17);
    let c = e.st.cloud_service();
    c.put_long(S_TOKEN, &long).unwrap();
    assert_eq!(c.get_long(S_TOKEN).unwrap().as_deref(), Some(long.as_str()));
    c.put_long(S_TOKEN, "short").unwrap();
    assert_eq!(c.get_long(S_TOKEN).unwrap().as_deref(), Some("short"));
    assert!(
        e.secrets.get(&format!("{S_TOKEN}-2")).unwrap().is_none(),
        "old pieces removed"
    );
    c.delete_long(S_TOKEN);
    assert!(c.get_long(S_TOKEN).unwrap().is_none());
}

#[test]
fn move_pro_here_deactivates_there_then_refreshes() {
    let e = env();
    e.signed_in();
    let blocked = json!({"license_id": "lic-pro", "plan": "pro_yearly", "reason": "seats_full",
                         "activations": [{"id": "a-studio", "machine_name": "STUDIO-PC", "slot": "main"}]});
    e.reply(
        "/v1/license/activate",
        vec![Reply::Token {
            claims: Box::new(free()),
            aid: "act-1".into(),
            nonce: None,
            upgrade_blocked: Some(blocked),
        }],
    );
    e.sync();
    let v = e.st.license_view();
    assert_eq!(
        v.upgrade_blocked.unwrap().activations[0]
            .machine_name
            .as_deref(),
        Some("STUDIO-PC")
    );
    assert!(!e.pro_ok());
    e.reply(
        "/v1/license/deactivate",
        vec![Reply::Json(204, Value::Null)],
    );
    e.reply("/v1/license/refresh", vec![token_reply(pro())]);
    e.rt.block_on(e.st.license_move_here("a-studio")).unwrap();
    assert_eq!(
        e.calls("/v1/license/deactivate")[0].0["activation_id"],
        "a-studio"
    );
    assert!(e.pro_ok());
}

#[test]
fn redeem_checks_the_key_before_sending() {
    let e = env();
    e.signed_in();
    let err =
        e.rt.block_on(e.st.license_redeem("KNTR-AAAA-AAAA-AAAB"))
            .unwrap_err();
    assert!(
        err.to_string().contains("typo") || err.to_string().contains("can’t be"),
        "{err}"
    );
    assert!(e.calls("/v1/license/redeem").is_empty());
}

#[test]
fn beta_ended_shows_the_signed_message() {
    let e = env_with(Some("spring-26"));
    let status = BetaStatus {
        typ: "beta_status".into(),
        v: 1,
        iat: T0,
        programs: vec![BetaProgram {
            id: "b1".into(),
            slug: "spring-26".into(),
            name: "Spring beta".into(),
            state: BetaState::Ended,
            starts_at: T0 - 30 * DAY,
            ends_at: T0,
            end_message: "Beta Ended\nThanks for testing!\nWatch out for a new beta soon.".into(),
        }],
    };
    e.reply(
        "/v1/beta/status",
        vec![Reply::Json(
            200,
            json!({"status": sign_beta_status(&status, "test-k1", &key(1))}),
        )],
    );
    e.sync();
    let b = e.st.license_view().beta.unwrap();
    assert!(b.ended);
    assert!(b.blocking, "a beta build stops when its beta ends");
    assert_eq!(
        b.end_message,
        vec![
            "Beta Ended",
            "Thanks for testing!",
            "Watch out for a new beta soon."
        ]
    );
    // A forged status (unknown key) is ignored.
    let e2 = env_with(Some("spring-26"));
    e2.reply(
        "/v1/beta/status",
        vec![Reply::Json(
            200,
            json!({"status": sign_beta_status(&status, "test-k1", &key(9))}),
        )],
    );
    e2.sync();
    assert!(e2.st.license_view().beta.is_none());
}

/// No command takes license state or entitlements from the UI.
#[test]
fn no_command_accepts_license_state() {
    let src = include_str!("../../../commands/license.rs");
    for (i, line) in src.lines().enumerate() {
        if line.trim_start().starts_with("pub async fn") || line.trim_start().starts_with("pub fn")
        {
            let sig = src.lines().skip(i).take(6).collect::<Vec<_>>().join(" ");
            for bad in [
                "LicenseView",
                "entitlement",
                "Eval",
                "Access",
                "LicenseClaims",
                "token",
            ] {
                assert!(
                    !sig.contains(&format!(": {bad}")) && !sig.contains(&format!("{bad}:")),
                    "{sig}"
                );
            }
        }
    }
}

/// Features are gated on catalog constants, never on string literals.
#[test]
fn no_entitlement_literals_in_the_app() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    for f in files {
        let name = f.to_string_lossy().replace('\\', "/");
        if name.contains("/tests") || name.ends_with("tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        for lit in ["\"pro.", "\"business.", "\"core.app\""] {
            assert!(
                !text.contains(lit),
                "{name} has the literal {lit}: use kinetra_license::entitlements"
            );
        }
    }
}

// ---- AI templates (part 3; mock server only) ---------------------------------------------------

fn package(html: &str) -> Value {
    json!({
        "manifest": {"schemaVersion": 2, "name": "AI lower third", "description": "", "preset": null,
                     "fields": [{"key": "name", "label": "Name", "type": "text", "default": "Ada"}],
                     "variables": [], "autoOutMs": null, "defaultVariant": "default",
                     "variants": [{"id": "default", "name": "Default", "vars": {}}]},
        "files": {"default": {"html": html, "css": ".lt{color:white}"}}
    })
}

/// A Pro user with an open project.
fn pro_env() -> Env {
    let e = env();
    e.st.create_project(e._tmp.path(), "Show").unwrap();
    e.signed_in();
    e.reply("/v1/license/activate", vec![token_reply(pro())]);
    e.sync();
    assert!(e.pro_ok());
    e
}

fn job(e: &Env, id: &str) -> super::super::ai::AiJobView {
    e.st.ai_jobs_list()
        .into_iter()
        .find(|j| j.id == id)
        .unwrap()
}

fn generate(e: &Env) -> Result<super::super::ai::AiJobView, crate::error::AppError> {
    e.rt.block_on(e.st.ai_generate(super::super::ai::AiGenerateInput {
        prompt: "Lower third".into(),
        ..Default::default()
    }))
}

#[test]
fn ai_is_gated_in_the_core_before_any_request() {
    // Free: refused, nothing sent (a forged UI can't get past this).
    let e = env();
    e.st.create_project(e._tmp.path(), "Show").unwrap();
    e.signed_in();
    e.reply("/v1/license/activate", vec![token_reply(free())]);
    e.sync();
    assert!(matches!(
        generate(&e),
        Err(crate::error::AppError::ProRequired)
    ));
    assert!(matches!(
        e.rt.block_on(e.st.ai_tweak("x".into(), "Make it blue".into())),
        Err(crate::error::AppError::ProRequired)
    ));
    assert!(e.calls("/v1/ai/templates/generate").is_empty());
    // Reduced mode: existing Pro keeps running, new generations wait.
    let r = pro_env();
    r.st.cloud_service().set_test_now(T0 + 31 * DAY);
    r.restart();
    assert!(r.pro_ok());
    assert!(matches!(
        generate(&r),
        Err(crate::error::AppError::ProLimited)
    ));
}

#[test]
fn a_generated_template_is_imported_as_a_normal_folder() {
    let e = pro_env();
    e.reply(
        "/v1/ai/templates/generate",
        vec![Reply::Json(
            202,
            json!({"job_id": "job-1", "status": "pending", "poll_after_ms": 10}),
        )],
    );
    e.reply(
        "/v1/ai/jobs/job-1",
        vec![
            Reply::Json(200, json!({"job_id": "job-1", "task": "generate_template", "status": "pending", "poll_after_ms": 10})),
            Reply::Json(200, json!({"job_id": "job-1", "task": "generate_template", "status": "succeeded",
                                    "template": package("<div class=\"lt\">{{name}}</div>"), "model": "m", "quota": {}})),
        ],
    );
    let j = generate(&e).unwrap();
    assert_eq!(j.status, "pending");
    wait_until("imported", || job(&e, "job-1").status != "pending");
    let j = job(&e, "job-1");
    assert_eq!((j.status, j.code.as_deref()), ("succeeded", None));
    let id = j.template_id.unwrap();
    let dir = e.st.project_dir().unwrap();
    let doc = crate::core::templates::get(&dir, &id).unwrap();
    assert_eq!(doc.meta.name, "AI lower third");
    assert!(doc.meta.preset.is_none() && !doc.meta.updated_at.is_empty());
    // The request carried the machine and the session.
    let (body, bearer) = e.calls("/v1/ai/templates/generate").remove(0);
    assert_eq!(
        (body["machine"].as_str(), bearer.as_deref()),
        (Some(MACHINE), Some("kntr_s_test"))
    );
}

#[test]
fn an_unsafe_or_failed_result_imports_nothing() {
    let e = pro_env();
    let dir = e.st.project_dir().unwrap();
    let before = std::fs::read_dir(dir.join("templates"))
        .map(|d| d.count())
        .unwrap_or(0);
    e.reply(
        "/v1/ai/templates/generate",
        vec![Reply::Json(
            202,
            json!({"job_id": "job-x", "poll_after_ms": 10}),
        )],
    );
    e.reply(
        "/v1/ai/jobs/job-x",
        vec![Reply::Json(200, json!({"status": "succeeded", "template": package("<div><img src=x onerror=alert(1)>{{name}}</div>")}))],
    );
    generate(&e).unwrap();
    wait_until("refused", || job(&e, "job-x").status != "pending");
    let j = job(&e, "job-x");
    assert_eq!((j.status, j.code.as_deref()), ("failed", Some("import")));
    assert!(j.message.unwrap().contains("event handler"));
    // A declined job: the API's code.
    e.reply(
        "/v1/ai/templates/generate",
        vec![Reply::Json(
            202,
            json!({"job_id": "job-d", "poll_after_ms": 10}),
        )],
    );
    e.reply(
        "/v1/ai/jobs/job-d",
        vec![Reply::Json(200, json!({"status": "declined", "error": {"code": "ai_declined", "message": "Couldn't generate this template, try rephrasing"}}))],
    );
    generate(&e).unwrap();
    wait_until("declined", || job(&e, "job-d").status != "pending");
    assert_eq!(job(&e, "job-d").code.as_deref(), Some("ai_declined"));
    assert_eq!(
        std::fs::read_dir(dir.join("templates"))
            .map(|d| d.count())
            .unwrap_or(0),
        before
    );
}

#[test]
fn start_errors_come_back_with_their_codes() {
    let e = pro_env();
    e.reply(
        "/v1/ai/templates/generate",
        vec![err(
            429,
            "ai_quota_exhausted",
            Some(json!({"scope": "month", "resets_at": T0 + 30 * DAY})),
        )],
    );
    match generate(&e) {
        Err(crate::error::AppError::Cloud {
            api_code, details, ..
        }) => {
            assert_eq!(api_code, "ai_quota_exhausted");
            assert_eq!(details.unwrap()["scope"], "month");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_canceled_job_is_never_imported() {
    let e = pro_env();
    e.reply(
        "/v1/ai/templates/generate",
        vec![Reply::Json(
            202,
            json!({"job_id": "job-c", "poll_after_ms": 300}),
        )],
    );
    e.reply(
        "/v1/ai/jobs/job-c",
        vec![Reply::Json(
            200,
            json!({"status": "succeeded", "template": package("<div>{{name}}</div>")}),
        )],
    );
    generate(&e).unwrap();
    e.st.ai_cancel("job-c");
    std::thread::sleep(Duration::from_millis(600));
    let j = job(&e, "job-c");
    assert_eq!((j.status, j.template_id), ("canceled", None));
    assert!(e.calls("/v1/ai/jobs/job-c").is_empty());
}

#[test]
fn tweak_sends_the_template_and_keeps_the_original() {
    let e = pro_env();
    let dir = e.st.project_dir().unwrap();
    let orig =
        e.st.templates()
            .import_package(
                &dir,
                &package("<div class=\"lt\">{{name}}</div>"),
                Some("Match lower third"),
                false,
            )
            .unwrap();
    e.reply(
        "/v1/ai/templates/tweak",
        vec![Reply::Json(
            202,
            json!({"job_id": "job-t", "poll_after_ms": 10}),
        )],
    );
    e.reply(
        "/v1/ai/jobs/job-t",
        vec![Reply::Json(200, json!({"status": "succeeded", "template": package("<div class=\"lt blue\">{{name}}</div>")}))],
    );
    e.rt.block_on(e.st.ai_tweak(orig.id.clone(), "Make it blue".into()))
        .unwrap();
    wait_until("tweaked", || job(&e, "job-t").status != "pending");
    let (body, _) = e.calls("/v1/ai/templates/tweak").remove(0);
    assert_eq!(body["instruction"], "Make it blue");
    assert_eq!(body["template"]["manifest"]["schemaVersion"], 2);
    assert_eq!(
        body["template"]["files"]["default"]["html"],
        "<div class=\"lt\">{{name}}</div>"
    );
    let new = crate::core::templates::get(&dir, &job(&e, "job-t").template_id.unwrap()).unwrap();
    assert_eq!(new.meta.name, "Match lower third (tweaked)");
    assert!(
        crate::core::templates::get(&dir, &orig.id).is_ok(),
        "the original stays"
    );
}
