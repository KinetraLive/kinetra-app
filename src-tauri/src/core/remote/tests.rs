//! Remote control through `AppState` against an in-process mock of the relay (axum WebSocket on
//! 127.0.0.1, no external network): the handshake, state and diffs, every command check, Studio
//! mode, the lifecycle (off at start, end, ended, stop codes, reconnect with `resume_run`,
//! `show_in_use`, a new show), Test connection (probe mode), diagnostics without values, exact
//! `on_air`, and that a stalled link never slows the output.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::conn::{set_test_timing, Timing};
use super::*;
use crate::core::cloud::service::S_SESSION;
use crate::core::events::EventSink;
use crate::core::model::{
    FlowEdge, FlowNode, NewFlow, NewScreen, NodeKind, NodePos, PortRef, PreviewAfterTake, Rect,
    Retrigger, ScreenDoc, ScreenLayer, ScreenLayerKind, SourceBinding, StudioSettings,
    TakeTransition, TakeTransitionKind, TemplateSource,
};
use crate::core::protocol::Fields;
use crate::core::secrets::{MemorySecrets, SecretStore};

const MACHINE: &str = "857a78b7a23b14cd718a9cbf9b90b0577225543ddcce5ed21854a4985f25ee27";
const SESSION: &str = "kntr_s_secret_session_value";
const TOKEN: &str = "license-token-secret-value";
/// Values and names a scripted session sends; none may appear in the diagnostics export.
const SECRET_VALUE: &str = "Ingrid Vale Secretvalue";
const ACTOR_NAME: &str = "Kari Nordmann-Secretname";

// ---- Mock relay -------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Ev {
    Connected {
        auth: String,
        proto: String,
        ua: String,
        probe: bool,
    },
    Msg {
        n: usize,
        v: Value,
    },
    Gone,
}

#[derive(Debug)]
enum ToApp {
    Text(String),
    Close(u16, String),
}

#[derive(Clone)]
struct Cfg {
    /// Answer the upgrade with this HTTP status (and Retry-After, error code) instead.
    http: Option<(u16, Option<u64>, &'static str)>,
    auto_welcome: bool,
    welcome_mode: Option<&'static str>,
    auto_register: bool,
    auto_pong: bool,
    /// Stop reading the app's messages (a stalled link).
    stall: bool,
}

impl Default for Cfg {
    fn default() -> Self {
        Self {
            http: None,
            auto_welcome: true,
            welcome_mode: None,
            auto_register: true,
            auto_pong: true,
            stall: false,
        }
    }
}

#[derive(Default)]
struct Shared {
    cfg: Mutex<Cfg>,
    conns: Mutex<usize>,
    to_app: Mutex<Option<mpsc::UnboundedSender<ToApp>>>,
    events: Mutex<Vec<Ev>>,
}

#[derive(serde::Deserialize)]
struct Q {
    mode: Option<String>,
}

async fn upgrade(
    State(sh): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(q): Query<Q>,
    ws: WebSocketUpgrade,
) -> Response {
    let cfg = sh.cfg.lock().unwrap().clone();
    if let Some((status, retry, code)) = cfg.http {
        let mut r = (
            StatusCode::from_u16(status).unwrap(),
            json!({"error": {"code": code, "message": "no"}}).to_string(),
        )
            .into_response();
        if let Some(s) = retry {
            r.headers_mut()
                .insert("retry-after", s.to_string().parse().unwrap());
        }
        return r;
    }
    let h = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    let n = {
        let mut c = sh.conns.lock().unwrap();
        *c += 1;
        *c
    };
    sh.events.lock().unwrap().push(Ev::Connected {
        auth: h("authorization"),
        proto: h("sec-websocket-protocol"),
        ua: h("user-agent"),
        probe: q.mode.as_deref() == Some("probe"),
    });
    ws.protocols(["kinetra-control.v1"])
        .on_upgrade(move |s| session(s, n, sh))
}

async fn session(socket: WebSocket, n: usize, sh: Arc<Shared>) {
    use futures_util::{SinkExt, StreamExt};
    let (mut tx, mut rx) = socket.split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<ToApp>();
    *sh.to_app.lock().unwrap() = Some(out.clone());
    let writer = tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            let msg = match m {
                ToApp::Text(t) => Message::Text(t.into()),
                ToApp::Close(code, reason) => Message::Close(Some(CloseFrame {
                    code,
                    reason: reason.into(),
                })),
            };
            let close = matches!(msg, Message::Close(_));
            if tx.send(msg).await.is_err() || close {
                break;
            }
        }
    });
    loop {
        while sh.cfg.lock().unwrap().stall {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let Some(Ok(m)) = rx.next().await else { break };
        let Message::Text(t) = m else {
            if matches!(m, Message::Close(_)) {
                break;
            }
            continue;
        };
        let v: Value = serde_json::from_str(t.as_str()).unwrap();
        sh.events.lock().unwrap().push(Ev::Msg { n, v: v.clone() });
        let cfg = sh.cfg.lock().unwrap().clone();
        let send = |v: Value| {
            let _ = out.send(ToApp::Text(v.to_string()));
        };
        match v["t"].as_str().unwrap_or_default() {
            "hello" if cfg.auto_welcome => send(json!({
                "t": "welcome", "protocol": 1,
                "mode": cfg.welcome_mode.unwrap_or("run"),
                "server_version": "mock 1", "server_time_ms": crate::core::fsx::now_epoch_ms(),
                "heartbeat_s": 20, "limits": {}
            })),
            "register" if cfg.auto_register => send(json!({
                "t": "registered", "corr": v["corr"], "show_id": "show-1",
                "run_id": format!("run-{n}"), "resumed": !v["resume_run"].is_null(),
                "crew_url": "https://control.kinetra.live/s/show-1/crew",
                "controller_url": "https://control.kinetra.live/s/show-1"
            })),
            "ping" if cfg.auto_pong => send(json!({
                "t": "pong", "corr": v["corr"], "ping_sent_ms": v["sent_ms"],
                "received_ms": crate::core::fsx::now_epoch_ms(),
                "sent_ms": crate::core::fsx::now_epoch_ms()
            })),
            "end" => {
                let _ = out.send(ToApp::Close(1000, "ended".into()));
            }
            _ => {}
        }
    }
    sh.events.lock().unwrap().push(Ev::Gone);
    writer.abort();
}

// ---- Environment ------------------------------------------------------------------------------

#[derive(Default)]
struct Rec(Mutex<Vec<(String, Value)>>);

impl EventSink for Rec {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    st: AppState,
    sh: Arc<Shared>,
    secrets: Arc<MemorySecrets>,
    screen: String,
    rec: Arc<Rec>,
}

fn fast_timing() -> Timing {
    Timing {
        handshake: Duration::from_secs(3),
        backoff: [Duration::from_millis(20); 5],
        backoff_steady: Duration::from_millis(40),
        end_wait: Duration::from_millis(500),
        ..Timing::LIVE
    }
}

fn layer(id: &str, template_id: &str, bindings: &[&str]) -> ScreenLayer {
    ScreenLayer {
        id: id.into(),
        name: format!("Layer {id}"),
        rect: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        kind: ScreenLayerKind::Template {
            template_id: template_id.into(),
            fields: Fields::new(),
            vars: Fields::new(),
            bindings: bindings
                .iter()
                .map(|k| {
                    (
                        k.to_string(),
                        SourceBinding {
                            source_id: "nosource".into(),
                            column: "c".into(),
                            sort: None,
                            limit: None,
                            row: None,
                        },
                    )
                })
                .collect(),
            variant: None,
            sizing: None,
            opacity: None,
        },
        anchor: None,
    }
}

/// A project with screen "Main": `l1` (lower third: text + image fields, `title` bound to a
/// source) and `l2` (scoreboard: a timer), both off air; signed in with `pro.control`.
fn env() -> Env {
    set_test_timing(Some(fast_timing()));
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Rec::default());
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        rec.clone(),
        Duration::from_millis(20),
    )
    .unwrap();
    st.set_content_flush_delay(Duration::from_secs(60));
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(3)
        .enable_all()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    let sh = Arc::new(Shared::default());
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/v1/control/app", get(upgrade))
        .with_state(sh.clone());
    rt.spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let secrets = Arc::new(MemorySecrets::default());
    st.cloud_service()
        .set_test_env(&base, secrets.clone(), MACHINE);
    st.cloud_init();
    secrets.set(S_SESSION, SESSION).unwrap();
    secrets
        .set("kinetra-license-token", &format!("1:{TOKEN}"))
        .unwrap();
    st.cloud_service()
        .set_test_entitlements(&["core.app", PRO_CONTROL], true);

    st.create_project(tmp.path(), "Nordlys Live").unwrap();
    let dir = st.project_dir().unwrap();
    let t = |p: &str| {
        st.templates()
            .create(
                &dir,
                p,
                &TemplateSource::Preset {
                    preset_id: p.into(),
                },
            )
            .unwrap()
            .id
    };
    let lt = t("lower-third");
    let sb = t("scoreboard");
    let mut doc: ScreenDoc = st
        .create_screen(&NewScreen {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap()
        .doc;
    doc.meta.layers.push(layer("l1", &lt, &["title"]));
    doc.meta.layers.push(layer("l2", &sb, &[]));
    let screen = st.save_screen(doc).unwrap().doc.id;
    crate::core::assets::add_bytes(&dir, "ingrid.png", PNG).unwrap();
    st.remote_assets_changed();
    Env {
        _tmp: tmp,
        rt,
        st,
        sh,
        secrets,
        screen,
        rec,
    }
}

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

impl Env {
    fn msgs(&self) -> Vec<(usize, Value)> {
        self.sh
            .events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Ev::Msg { n, v } => Some((*n, v.clone())),
                _ => None,
            })
            .collect()
    }

    fn of(&self, t: &str) -> Vec<Value> {
        self.msgs()
            .into_iter()
            .filter(|(_, v)| v["t"] == t)
            .map(|(_, v)| v)
            .collect()
    }

    fn connects(&self) -> Vec<Ev> {
        self.sh
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, Ev::Connected { .. }))
            .cloned()
            .collect()
    }

    fn cfg(&self, f: impl FnOnce(&mut Cfg)) {
        f(&mut self.sh.cfg.lock().unwrap());
    }

    fn push(&self, v: Value) {
        let tx = self.sh.to_app.lock().unwrap().clone().expect("connected");
        tx.send(ToApp::Text(v.to_string())).unwrap();
    }

    fn close(&self, code: u16, reason: &str) {
        let tx = self.sh.to_app.lock().unwrap().clone().expect("connected");
        tx.send(ToApp::Close(code, reason.into())).unwrap();
    }

    /// On, live, with the first snapshot received. Returns it.
    fn live(&self) -> Value {
        self.st.remote_set(true).unwrap();
        wait_until("snapshot", || !self.of("snapshot").is_empty());
        wait_until("live", || {
            matches!(self.st.remote_view().phase, Phase::Live { .. })
        });
        self.of("snapshot").last().unwrap()["state"].clone()
    }

    /// The tree as the mock has it: the last snapshot plus every diff after it.
    fn tree(&self) -> Value {
        let mut tree = Value::Null;
        for (_, m) in self.msgs() {
            match m["t"].as_str() {
                Some("snapshot") => tree = m["state"].clone(),
                Some("diff") => {
                    for op in m["ops"].as_array().unwrap() {
                        apply(&mut tree, op);
                    }
                }
                _ => {}
            }
        }
        tree
    }

    fn seq(&self) -> u64 {
        self.msgs()
            .iter()
            .filter_map(|(_, m)| m["seq"].as_u64())
            .max()
            .unwrap_or(0)
    }

    /// Sends a command and waits for its result.
    fn command(&self, corr: &str, action: &str, args: Value, role: &str, seen: u64) -> Value {
        self.push(json!({
            "t": "command", "corr": corr, "action": action, "args": args,
            "actor": {"id": "0193aaaa-0000-7000-8000-000000000001", "name": ACTOR_NAME, "role": role},
            "seen_seq": seen, "sent_ms": crate::core::fsx::now_epoch_ms()
        }));
        let mut found = None;
        wait_until("result", || {
            found = self.of("result").into_iter().rfind(|r| r["corr"] == corr);
            found.is_some()
        });
        found.unwrap()
    }

    fn on_air(&self, l: &str) -> bool {
        self.st.live().layer_view(&self.screen, l).unwrap().on_air
    }

    fn task_running(&self) -> bool {
        self.st
            .remote()
            .active
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Applies one JSON Pointer op, as the server would.
fn apply(tree: &mut Value, op: &Value) {
    let path = op["path"].as_str().unwrap();
    let (parent, key) = path.rsplit_once('/').unwrap();
    let key = key.replace("~1", "/").replace("~0", "~");
    let target = if parent.is_empty() {
        tree
    } else {
        tree.pointer_mut(parent).expect("parent exists")
    };
    let obj = target.as_object_mut().expect("parent is an object");
    match op["op"].as_str().unwrap() {
        "put" => {
            obj.insert(key, op["value"].clone());
        }
        "del" => {
            assert!(obj.remove(&key).is_some(), "del of a missing {path}");
        }
        o => panic!("op {o}"),
    }
}

fn cue_flow(e: &Env, name: &str) -> String {
    let n = |id: &str, kind| FlowNode {
        id: id.into(),
        position: NodePos::default(),
        kind,
    };
    e.st.create_flow(NewFlow {
        name: name.into(),
        armed: true,
        retrigger: Retrigger::Restart,
        origin: None,
        nodes: vec![
            n("c", NodeKind::Cue { label: None }),
            n(
                "t",
                NodeKind::Take {
                    screen_id: e.screen.clone(),
                    layer_id: "l2".into(),
                    for_ms: None,
                },
            ),
        ],
        edges: vec![FlowEdge {
            id: "e1".into(),
            from: PortRef {
                node: "c".into(),
                port: "fired".into(),
            },
            to: PortRef {
                node: "t".into(),
                port: "in".into(),
            },
            kind: crate::core::model::WireKind::Event,
        }],
    })
    .unwrap()
    .flow
    .id
}

// ---- Tests ------------------------------------------------------------------------------------

#[test]
fn off_at_start_and_off_costs_nothing() {
    let e = env();
    std::thread::sleep(Duration::from_millis(100));
    assert!(e.connects().is_empty());
    assert!(!e.task_running());
    assert_eq!(e.st.remote_view().phase, Phase::Off);
    assert!(!e.st.remote_view().on);
    // A take with remote control off wakes nothing.
    e.st.live().show(&e.screen, "l1").unwrap();
    std::thread::sleep(Duration::from_millis(50));
    assert!(e.connects().is_empty());
    assert!(e.st.project_remote_id().is_none());
}

#[test]
fn handshake_register_and_snapshot() {
    let e = env();
    let snap = e.live();
    let id = e.st.project_remote_id().expect("project id written");
    assert!(ids::is_uuid(&id) && &id[14..15] == "7");
    let Ev::Connected {
        auth,
        proto,
        ua,
        probe,
        ..
    } = &e.connects()[0]
    else {
        unreachable!()
    };
    assert_eq!(auth, &format!("Bearer {SESSION}"));
    assert_eq!(proto, "kinetra-control.v1");
    assert!(ua.starts_with("Kinetra/"), "{ua}");
    assert!(!probe);
    let hello = &e.of("hello")[0];
    assert_eq!(hello["protocol"], 1);
    assert_eq!(hello["machine"], MACHINE);
    assert_eq!(hello["license_token"], TOKEN);
    assert_eq!(hello["nonce"].as_str().unwrap().len(), 32);
    assert!(hello["timestamp"].as_i64().unwrap() > 1_700_000_000);
    let reg = &e.of("register")[0];
    assert_eq!(reg["project_id"], id);
    assert_eq!(reg["name"], "Nordlys Live");
    assert!(reg["resume_run"].is_null());
    assert_eq!(reg["takeover"], false);
    assert!(ids::is_uuid(reg["corr"].as_str().unwrap()));

    assert_eq!(e.of("snapshot")[0]["seq"], 0);
    state::check_refs(&snap).unwrap();
    assert_eq!(snap["screen_order"], json!([e.screen]));
    // Top first.
    assert_eq!(snap["screens"][&e.screen]["layers"], json!(["l2", "l1"]));
    let l1 = &snap["layers"]["l1"];
    assert_eq!(l1["on_air"], false);
    assert_eq!(l1["bound"], json!(["title"]));
    assert_eq!(l1["remote_editable"], true);
    assert!(l1["values"].get("name").is_some());
    // Clocks have no value; timers do.
    assert!(snap["layers"]["l2"]["values"].get("clock").is_some());
    let fields = snap["templates"][l1["template"].as_str().unwrap()]["fields"]
        .as_array()
        .unwrap();
    assert!(fields
        .iter()
        .any(|f| f["key"] == "photo" && f["type"] == "image"));
    assert_eq!(
        snap["assets"],
        json!([{"path": "/assets/ingrid.png", "name": "ingrid.png", "kind": "image"}])
    );
    assert_eq!(snap["remote"]["take_locked"], false);
    assert_eq!(snap["cue_list_order"], json!(["cues"]));
    assert_eq!(snap["studio_mode"], false);
    // Never in the state: HTML, CSS, variables, paths, secrets.
    let text = snap.to_string();
    for bad in ["<div", "var(--", "template.css", SESSION, TOKEN, MACHINE] {
        assert!(!text.contains(bad), "{bad} in the state");
    }
    // On again while on: the same connection.
    e.st.remote_set(true).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(e.connects().len(), 1);
    let v = e.st.remote_view();
    assert_eq!(v.show_id.as_deref(), Some("show-1"));
    assert!(v.on && v.has_show_id);
    assert!(e
        .rec
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|(n, _)| n == names::REMOTE_CONTROL));
}

#[test]
fn local_changes_go_out_as_diffs_quickly() {
    let e = env();
    e.live();
    let t0 = Instant::now();
    e.st.live().show(&e.screen, "l1").unwrap();
    wait_until("on_air diff", || e.tree()["layers"]["l1"]["on_air"] == true);
    let took = t0.elapsed();
    assert!(
        took < Duration::from_millis(100),
        "on air went out after {took:?}"
    );
    let diff = e.of("diff").last().unwrap().clone();
    assert_eq!(diff["seq"], 1);
    assert!(diff.get("cause").is_none());
    // A local field edit.
    let mut f = Fields::new();
    f.insert("name".into(), "Ola".into());
    e.st.live().hide(&e.screen, "l1").unwrap();
    e.st.set_layer_fields(&e.screen, "l2", {
        let mut g = Fields::new();
        g.insert("home".into(), "Molde".into());
        g
    })
    .unwrap();
    wait_until("field diff", || {
        e.tree()["layers"]["l2"]["values"]["home"] == "Molde"
    });
    // Seq numbers have no gaps.
    let seqs: Vec<u64> = e
        .msgs()
        .iter()
        .filter_map(|(_, m)| m["seq"].as_u64())
        .collect();
    for w in seqs.windows(2) {
        assert_eq!(w[1], w[0] + 1);
    }
    // The mock's copy equals the app's tree.
    let input = e.st.remote_build_input();
    let mut mine = state::build(&e.st, &input);
    let mut theirs = e.tree();
    // on_air of l1 may still be in its out-animation window in either: compare the rest.
    mine["layers"]["l1"]["on_air"] = Value::Null;
    theirs["layers"]["l1"]["on_air"] = Value::Null;
    assert_eq!(mine, theirs);
}

#[test]
fn commands_run_through_the_local_paths() {
    let e = env();
    e.live();
    // Take, as the local Take button.
    let r = e.command(
        "c-take",
        "take",
        json!({"layer": "l1"}),
        "operator",
        e.seq(),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(r["received_ms"].as_u64().unwrap() <= r["done_ms"].as_u64().unwrap());
    assert!(e.on_air("l1"));
    wait_until("caused diff", || {
        e.of("diff").iter().any(|d| d["cause"] == "c-take")
    });
    // A data editor edits an off-air layer: saved through the live rules.
    let r = e.command(
        "c-edit",
        "update_fields",
        json!({"layer": "l2", "values": {"home": "Rosenborg"}}),
        "data_editor",
        e.seq(),
    );
    assert_eq!(r["ok"], true, "{r}");
    let v = e.st.live().layer_view(&e.screen, "l2").unwrap();
    assert_eq!(v.fields["home"], "Rosenborg");
    // The same corr again: the same result, nothing done twice.
    let before = e.of("result").len();
    e.push(
        json!({"t":"command","corr":"c-edit","action":"update_fields",
        "args":{"layer":"l2","values":{"home":"Other"}},
        "actor":{"id":null,"name":"x","role":"data_editor"},"seen_seq":0,"sent_ms":0}),
    );
    wait_until("dup result", || e.of("result").len() == before + 1);
    assert_eq!(e.of("result").last().unwrap()["ok"], true);
    assert_eq!(
        e.st.live().layer_view(&e.screen, "l2").unwrap().fields["home"],
        "Rosenborg"
    );
    // Clear.
    let r = e.command(
        "c-clear",
        "clear",
        json!({"layer": "l1"}),
        "operator",
        e.seq(),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(!e.on_air("l1"));
    // Activity, in memory.
    let a = e.st.remote_activity();
    assert_eq!(a.len(), 3);
    assert!(a[1].text.contains("updated"), "{:?}", a[1]);
    assert!(a[2].text.starts_with(ACTOR_NAME) && a[2].text.contains("took"));
}

#[test]
fn every_refusal_in_the_protocols_order() {
    let e = env();
    e.live();
    let code = |r: &Value| r["code"].as_str().unwrap_or("ok").to_string();
    let s = || e.seq();
    // unsupported, invalid
    assert_eq!(
        code(&e.command("a", "clear_all", json!({}), "operator", s())),
        "unsupported"
    );
    assert_eq!(
        code(&e.command("b", "take", json!({}), "operator", s())),
        "invalid"
    );
    assert_eq!(
        code(&e.command(
            "b2",
            "update_fields",
            json!({"layer":"l1","values":{}}),
            "operator",
            s()
        )),
        "invalid"
    );
    // forbidden: a viewer; a data editor taking.
    assert_eq!(
        code(&e.command("c", "take", json!({"layer":"l1"}), "viewer", s())),
        "forbidden"
    );
    assert_eq!(
        code(&e.command("d", "take", json!({"layer":"l1"}), "data_editor", s())),
        "forbidden"
    );
    // not_found
    assert_eq!(
        code(&e.command("e", "take", json!({"layer":"nope"}), "operator", s())),
        "not_found"
    );
    assert_eq!(
        code(&e.command(
            "e2",
            "run_cue",
            json!({"cue_list":"cues","cue":"nope"}),
            "operator",
            s()
        )),
        "not_found"
    );
    // A field the template doesn't have; a bound field; a timer.
    let r = e.command(
        "f",
        "update_fields",
        json!({"layer":"l1","values":{"zzz":"1"}}),
        "operator",
        s(),
    );
    assert_eq!(
        (code(&r), r["details"]["field"].clone()),
        ("invalid".into(), json!("zzz"))
    );
    let r = e.command(
        "g",
        "update_fields",
        json!({"layer":"l1","values":{"title":"x"}}),
        "operator",
        s(),
    );
    assert_eq!(code(&r), "bound");
    let r = e.command(
        "h",
        "update_fields",
        json!({"layer":"l2","values":{"clock":"{}"}}),
        "operator",
        s(),
    );
    assert_eq!(code(&r), "read_only");
    // An image must be one of the assets sent (or empty).
    let r = e.command(
        "i",
        "update_fields",
        json!({"layer":"l1","values":{"photo":"https://evil.example/x.png"}}),
        "operator",
        s(),
    );
    assert_eq!(code(&r), "invalid");
    let r = e.command(
        "i2",
        "update_fields",
        json!({"layer":"l1","values":{"photo":"/assets/ingrid.png"}}),
        "operator",
        s(),
    );
    assert_eq!(code(&r), "ok", "{r}");
    // Value validation as local (a number field).
    let r = e.command(
        "j",
        "update_fields",
        json!({"layer":"l2","values":{"home_score":"x".repeat(70_000)}}),
        "operator",
        s(),
    );
    assert_eq!(code(&r), "invalid");
    // stale: the layer went on air after what the sender saw.
    let seen = s();
    e.st.live().show(&e.screen, "l2").unwrap();
    wait_until("diff", || e.seq() > seen);
    assert_eq!(
        code(&e.command("k", "clear", json!({"layer":"l2"}), "operator", seen)),
        "stale"
    );
    // on_air: a data editor on an on-air layer (checked before the lock and the target).
    let r = e.command(
        "l",
        "update_fields",
        json!({"layer":"l2","values":{"home":"x"}}),
        "data_editor",
        s(),
    );
    assert_eq!(code(&r), "on_air");
    // locked: take, clear, and an on-air edit; an off-air data edit still works.
    e.st.remote_lock(true).unwrap();
    wait_until("lock diff", || e.tree()["remote"]["take_locked"] == true);
    assert_eq!(
        code(&e.command("m", "take", json!({"layer":"l1"}), "operator", s())),
        "locked"
    );
    assert_eq!(
        code(&e.command("n", "clear", json!({"layer":"l2"}), "operator", s())),
        "locked"
    );
    assert_eq!(
        code(&e.command(
            "o",
            "update_fields",
            json!({"layer":"l2","values":{"home":"y"}}),
            "operator",
            s()
        )),
        "locked"
    );
    assert_eq!(
        code(&e.command(
            "p",
            "update_fields",
            json!({"layer":"l1","values":{"name":"Ok"}}),
            "data_editor",
            s()
        )),
        "ok"
    );
    assert!(e.on_air("l2"));
    // disabled: the switch went off while a command was on its way (checked first).
    e.st.remote_lock(false).unwrap();
    let refused: Vec<String> =
        e.st.remote()
            .diag()
            .refused
            .iter()
            .map(|r| r.code.clone())
            .collect();
    assert!(refused.contains(&"stale".to_string()) && refused.contains(&"locked".to_string()));
}

#[test]
fn a_data_editor_can_never_edit_a_layer_taken_meanwhile() {
    let e = env();
    e.live();
    // The atomic guard: refused under the hub's lock once the layer shows.
    e.st.live().show(&e.screen, "l1").unwrap();
    let mut f = Fields::new();
    f.insert("name".into(), "Late".into());
    assert_eq!(
        e.st.set_layer_fields_unless_shown(&e.screen, "l1", f.clone())
            .unwrap(),
        None
    );
    let doc = e.st.get_screen(&e.screen).unwrap().doc;
    let ScreenLayerKind::Template { fields, .. } = &doc.meta.layers[0].kind else {
        unreachable!()
    };
    assert_ne!(fields.get("name").map(String::as_str), Some("Late"));
    // Off air (and the out-animation over: no renderer counts here) it applies.
    e.st.live().hide(&e.screen, "l1").unwrap();
    assert!(e
        .st
        .set_layer_fields_unless_shown(&e.screen, "l1", f)
        .unwrap()
        .is_some());
}

#[test]
fn studio_mode_take_cues_into_preview_and_preview_is_read_only_state() {
    let e = env();
    e.st.set_studio(
        StudioSettings {
            enabled: true,
            preview_after_take: PreviewAfterTake::Keep,
            transition: TakeTransition {
                kind: TakeTransitionKind::Cut,
                ms: 300,
            },
        },
        None,
    )
    .unwrap();
    let snap = e.live();
    assert_eq!(snap["studio_mode"], true);
    assert_eq!(snap["preview"][&e.screen], json!([]));
    let r = e.command("t", "take", json!({"layer":"l1"}), "operator", e.seq());
    assert_eq!(r["ok"], true, "{r}");
    // Cued into Preview, like the local Take button; Program unchanged.
    assert!(!e.on_air("l1"));
    wait_until("preview diff", || {
        e.tree()["preview"][&e.screen] == json!(["l1"])
    });
    // A data editor can't edit a layer cued in Preview.
    let r = e.command(
        "u",
        "update_fields",
        json!({"layer":"l1","values":{"name":"x"}}),
        "data_editor",
        e.seq(),
    );
    assert_eq!(r["code"], "on_air");
}

#[test]
fn cue_list_follows_every_run_and_two_gos_run_one_cue() {
    let e = env();
    let a = cue_flow(&e, "Open");
    let b = cue_flow(&e, "Close");
    let snap = e.live();
    let list = &snap["cue_lists"]["cues"];
    assert_eq!(list["order"], json!([a, b]));
    assert_eq!(list["current"], Value::Null);
    assert_eq!(list["next"], json!(a));
    // A local GO moves it too.
    e.st.run_flow(&a, None).unwrap();
    wait_until("current", || {
        e.tree()["cue_lists"]["cues"]["current"] == json!(a)
    });
    assert_eq!(e.tree()["cue_lists"]["cues"]["next"], json!(b));
    assert_eq!(
        e.st.remote_cue_position(),
        CuePosition {
            current: Some(a.clone()),
            next: Some(b.clone())
        }
    );
    let announced: Vec<Value> = e
        .rec
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(n, _)| n == names::CUE_POSITION)
        .map(|(_, v)| v.clone())
        .collect();
    assert_eq!(announced.last().unwrap(), &json!({"current": a, "next": b}));
    // Two people press GO having seen the same state: one runs, the other is stale.
    let seen = e.seq();
    let r1 = e.command(
        "g1",
        "run_cue",
        json!({"cue_list":"cues","cue":b}),
        "operator",
        seen,
    );
    let r2 = e.command(
        "g2",
        "run_cue",
        json!({"cue_list":"cues","cue":b}),
        "operator",
        seen,
    );
    assert_eq!(r1["ok"], true, "{r1}");
    assert_eq!(r2["code"], "stale");
    wait_until("last", || e.tree()["cue_lists"]["cues"]["next"].is_null());
    // A new cue appears in the list without anything else changing.
    let c = cue_flow(&e, "Encore");
    wait_until("new cue", || {
        e.tree()["cue_lists"]["cues"]["order"] == json!([a, b, c])
    });
    assert_eq!(e.tree()["cue_lists"]["cues"]["next"], json!(c));
}

#[test]
fn resync_answers_with_a_snapshot_echoing_its_corr() {
    let e = env();
    e.live();
    e.push(json!({"t":"resync","corr":"rs-1","reason":"seq_gap"}));
    wait_until("snapshot", || {
        e.of("snapshot").iter().any(|s| s["corr"] == "rs-1")
    });
    let s = e.of("snapshot").last().unwrap().clone();
    assert!(s["seq"].as_u64().unwrap() >= 1);
    assert_eq!(e.st.remote().diag().resyncs, 1);
}

#[test]
fn a_drop_reconnects_resumes_and_replays_nothing() {
    let e = env();
    e.live();
    e.close(1006, "");
    wait_until("second connection", || e.connects().len() == 2);
    wait_until("second register", || e.of("register").len() == 2);
    assert_eq!(e.of("register")[1]["resume_run"], "run-1");
    wait_until("fresh snapshot", || e.of("snapshot").len() == 2);
    wait_until("resumed", || {
        matches!(e.st.remote_view().phase, Phase::Live { resumed: true, .. })
    });
    // Nothing from before the drop is sent again (no results, no replays).
    assert!(e.of("result").is_empty());
    assert!(e.st.remote().diag().reconnects.len() == 1);
}

#[test]
fn server_restart_reconnects_after_its_wait() {
    let e = env();
    e.live();
    e.push(json!({"t":"server_restart","retry_after_ms":50}));
    std::thread::sleep(Duration::from_millis(30));
    e.close(1001, "server_restart");
    wait_until("reconnected", || e.of("snapshot").len() == 2);
}

#[test]
fn stopping_codes_stop_and_never_reconnect() {
    for (code, reason, want) in [
        (4401, "unauthorized", "unauthorized"),
        (4403, "entitlement_required", "entitlement_required"),
        (4409, "replaced", "replaced"),
        (4412, "clock_skew", "clock_skew"),
        (4410, "show_deleted", "show_deleted"),
        (4001, "unsupported_protocol", "unsupported_protocol"),
    ] {
        let e = env();
        e.live();
        if code == 4409 {
            e.push(json!({"t":"error","code":"replaced","fatal":true,
                "message":"Remote control moved to Studio B."}));
        }
        e.close(code, reason);
        wait_until("stopped", || {
            matches!(e.st.remote_view().phase, Phase::Stopped { .. })
        });
        let Phase::Stopped { code: c, message } = e.st.remote_view().phase else {
            unreachable!()
        };
        assert_eq!(c, want);
        if code == 4409 {
            assert_eq!(message, "Remote control moved to Studio B.");
        }
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(e.connects().len(), 1, "{reason} reconnected");
        assert!(!e.task_running());
        // The switch stays on: Start again.
        assert!(e.st.remote_view().on);
    }
}

#[test]
fn ended_shows_start_again_and_never_reconnects_by_itself() {
    let e = env();
    e.live();
    e.close(1000, "ended");
    wait_until("ended", || e.st.remote_view().phase == Phase::Ended);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(e.connects().len(), 1);
    e.st.remote_start_again().unwrap();
    wait_until("live again", || e.of("snapshot").len() == 2);
}

#[test]
fn end_on_off_and_on_project_close() {
    let e = env();
    e.live();
    e.st.remote_set(false).unwrap();
    wait_until("end", || !e.of("end").is_empty());
    assert_eq!(e.of("end")[0]["reason"], "disabled");
    assert_eq!(e.st.remote_view().phase, Phase::Off);
    wait_until("task gone", || !e.task_running());
    e.live();
    e.st.close_project();
    wait_until("end 2", || e.of("end").len() == 2);
    assert_eq!(e.of("end")[1]["reason"], "project_closed");
    assert!(!e.st.remote_view().on);
}

#[test]
fn end_remote_control_keeps_the_switch_on() {
    let e = env();
    e.live();
    let v = e.st.remote_end().unwrap();
    assert_eq!(v.phase, Phase::Ended);
    assert!(v.on);
    wait_until("end", || !e.of("end").is_empty());
    assert_eq!(e.of("end")[0]["reason"], "disabled");
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(e.st.remote_view().phase, Phase::Ended);
    assert_eq!(e.connects().len(), 1);
    e.st.remote_start_again().unwrap();
    wait_until("live again", || e.of("snapshot").len() == 2);
}

#[test]
fn rename_goes_out_during_a_run() {
    let e = env();
    e.live();
    e.st.remote_project_changed(Some(&Project {
        name: "Nordlys Finale".into(),
        ..e.st.current_project().unwrap()
    }));
    wait_until("rename", || !e.of("rename").is_empty());
    assert_eq!(e.of("rename")[0]["name"], "Nordlys Finale");
}

#[test]
fn show_in_use_asks_and_takes_over_or_makes_a_separate_show() {
    let e = env();
    e.cfg(|c| c.auto_register = false);
    e.st.remote_set(true).unwrap();
    wait_until("register", || !e.of("register").is_empty());
    e.push(json!({"t":"error","code":"show_in_use","fatal":false,
        "corr": e.of("register")[0]["corr"], "details":{"machine_name":"Studio PC"}}));
    wait_until("asks", || {
        e.st.remote_view().phase
            == Phase::ShowInUse {
                machine: "Studio PC".into(),
            }
    });
    e.st.remote_takeover().unwrap();
    wait_until("takeover register", || e.of("register").len() == 2);
    wait_until("connecting", || {
        e.st.remote_view().phase == Phase::Connecting
    });
    assert_eq!(e.of("register")[1]["takeover"], true);
    // The other choice: a separate show (a new ID) on the same connection.
    let old = e.st.project_remote_id().unwrap();
    e.push(json!({"t":"error","code":"show_in_use","fatal":false,
        "details":{"machine_name":"Studio PC"}}));
    wait_until("asks again", || {
        matches!(e.st.remote_view().phase, Phase::ShowInUse { .. })
    });
    e.st.remote_new_show().unwrap();
    wait_until("new register", || e.of("register").len() == 3);
    let reg = &e.of("register")[2];
    assert_ne!(reg["project_id"], json!(old));
    assert_eq!(reg["project_id"], json!(e.st.project_remote_id().unwrap()));
    assert_eq!(reg["takeover"], false);
}

#[test]
fn a_new_show_while_live_ends_the_run_and_registers_the_new_id() {
    let e = env();
    e.live();
    let old = e.st.project_remote_id().unwrap();
    e.st.live().show(&e.screen, "l1").unwrap();
    e.st.remote_new_show().unwrap();
    wait_until("end", || !e.of("end").is_empty());
    wait_until("register 2", || e.of("register").len() == 2);
    let reg = &e.of("register")[1];
    assert_ne!(reg["project_id"], json!(old));
    assert!(reg["resume_run"].is_null());
    // Nothing on air changed.
    assert!(e.on_air("l1"));
    // Written to project.json.
    e.st.flush_all();
    let text = std::fs::read_to_string(e.st.project_dir().unwrap().join("project.json")).unwrap();
    assert!(text.contains(&e.st.project_remote_id().unwrap()));
}

#[test]
fn license_warning_crew_and_round_trip_reach_the_view() {
    let e = env();
    e.live();
    e.push(json!({"t":"crew","connected":[
        {"id":"0193aaaa-0000-7000-8000-000000000001","name":"Kari N.","role":"operator","since_ms":5},
        {"id":null,"name":"Guest 3","role":"viewer","anonymous":true,"since_ms":6}]}));
    e.push(json!({"t":"license_warning","ends_ms":99,"message":"Your license lapsed."}));
    wait_until("crew", || e.st.remote_view().crew.len() == 2);
    wait_until("warning", || e.st.remote_view().license_warning.is_some());
    // A server ping is answered at once with timestamps.
    e.push(json!({"t":"ping","corr":"p1","sent_ms":1}));
    wait_until("pong", || !e.of("pong").is_empty());
    let pong = &e.of("pong")[0];
    assert_eq!(pong["corr"], "p1");
    assert_eq!(pong["ping_sent_ms"], 1);
    assert!(pong["received_ms"].as_u64().unwrap() <= pong["sent_ms"].as_u64().unwrap());
    // Test connection while live: a ping on the connection, no probe.
    let r = e.rt.block_on(e.st.remote_test());
    assert!(r.ok && r.live, "{r:?}");
    assert!(r.rtt_ms.is_some());
    assert_eq!(e.connects().len(), 1);
    assert!(e.st.remote_view().rtt_ms.is_some());
}

#[test]
fn probe_mode_tests_without_registering() {
    let e = env();
    e.cfg(|c| c.welcome_mode = Some("probe"));
    let r = e.rt.block_on(e.st.remote_test());
    assert!(r.ok, "{r:?}");
    assert_eq!(r.message, "Connected");
    assert_eq!(r.server_version.as_deref(), Some("mock 1"));
    assert!(r.connect_ms.is_some() && r.welcome_ms.is_some() && r.rtt_ms.is_some());
    let Ev::Connected { probe, .. } = &e.connects()[0] else {
        unreachable!()
    };
    assert!(probe);
    let sent: Vec<String> = e
        .msgs()
        .iter()
        .map(|(_, m)| m["t"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(sent, ["hello", "ping", "end"]);
    assert_eq!(e.of("end")[0]["reason"], "probe_done");
    assert!(e.st.project_remote_id().is_none());
    assert_eq!(e.st.remote().diag().probe.as_ref().unwrap().outcome, "ok");

    // A welcome without probe mode: leave at once, register nothing.
    e.cfg(|c| c.welcome_mode = Some("run"));
    let r = e.rt.block_on(e.st.remote_test());
    assert_eq!(r.code, "bad_mode");
    assert!(e.of("register").is_empty());

    // The §4a table.
    e.cfg(|c| c.http = Some((429, Some(42), "rate_limited")));
    let r = e.rt.block_on(e.st.remote_test());
    assert_eq!(
        (r.code.as_str(), r.retry_after_s),
        ("rate_limited", Some(42))
    );
    assert_eq!(r.message, "Tested too often, try again in 42 s");
    e.cfg(|c| c.http = Some((401, None, "unauthorized")));
    assert_eq!(e.rt.block_on(e.st.remote_test()).message, "Sign in again");
    e.cfg(|c| c.http = Some((503, Some(5), "server_busy")));
    assert_eq!(e.rt.block_on(e.st.remote_test()).code, "server_busy");
    // Signed out: nothing is sent.
    e.cfg(|c| c.http = None);
    e.secrets.delete(S_SESSION).unwrap();
    let before = e.connects().len();
    let r = e.rt.block_on(e.st.remote_test());
    assert_eq!(r.code, "signed_out");
    assert_eq!(e.connects().len(), before);
}

#[test]
fn probe_maps_fatal_handshake_errors() {
    let e = env();
    e.cfg(|c| c.auto_welcome = false);
    let st = e.st.clone();
    let task = e.rt.spawn(async move { st.remote_test().await });
    wait_until("hello", || !e.of("hello").is_empty());
    e.push(
        json!({"t":"error","code":"entitlement_required","fatal":true,
        "message":"Remote control is part of Kinetra Team."}),
    );
    e.close(4403, "entitlement_required");
    let r = e.rt.block_on(task).unwrap();
    assert_eq!(r.code, "entitlement_required");
    assert_eq!(r.message, "Remote control is part of Kinetra Team.");
}

#[test]
fn probe_without_a_server_cant_reach() {
    let e = env();
    // A port nothing listens on.
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    drop(l);
    e.st.cloud_service()
        .set_test_env(&base, e.secrets.clone(), MACHINE);
    let r = e.rt.block_on(e.st.remote_test());
    assert!(!r.ok);
    assert_eq!(r.message, "Can't reach Kinetra cloud");
    assert!(r.detail.is_some());
}

#[test]
fn diagnostics_export_has_no_values_names_or_secrets() {
    let e = env();
    e.live();
    e.command(
        "x1",
        "update_fields",
        json!({"layer":"l2","values":{"home": SECRET_VALUE}}),
        "operator",
        e.seq(),
    );
    e.command("x2", "take", json!({"layer":"nope"}), "viewer", e.seq());
    e.push(
        json!({"t":"crew","connected":[{"id":"0193aaaa-0000-7000-8000-000000000001",
        "name": ACTOR_NAME,"role":"operator","since_ms":5}]}),
    );
    e.st.remote_lock(true).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let dir = e.st.project_dir().unwrap();
    let path = dir.join("diag.json");
    e.st.remote_export_diagnostics(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["kind"], "kinetra/remote-diagnostics");
    assert!(v["messages"].as_array().unwrap().len() > 5);
    assert_eq!(v["refused"][0]["code"], "forbidden");
    let dir_text = dir.to_string_lossy().replace('\\', "\\\\");
    for bad in [
        SECRET_VALUE,
        ACTOR_NAME,
        "Kari",
        "Nordlys",
        "Layer l1",
        "Main",
        SESSION,
        TOKEN,
        MACHINE,
        "0193aaaa-0000-7000-8000-000000000001",
        "127.0.0.1",
        &dir_text,
    ] {
        assert!(!text.contains(bad), "{bad} in the export");
    }
    for h in e.of("hello") {
        assert!(!text.contains(h["nonce"].as_str().unwrap()));
    }
}

#[test]
fn a_stalled_link_never_slows_the_output() {
    let e = env();
    e.live();
    e.cfg(|c| c.stall = true);
    let mut rx = e.st.live().subscribe();
    let mut worst = Duration::ZERO;
    for i in 0..100 {
        let t0 = Instant::now();
        if i % 2 == 0 {
            e.st.live().show(&e.screen, "l1").unwrap();
        } else {
            e.st.live().hide(&e.screen, "l1").unwrap();
        }
        let mut f = Fields::new();
        f.insert("home".into(), format!("Team {i}"));
        e.st.set_layer_fields(&e.screen, "l2", f).unwrap();
        // The renderer message is out by the time the call returns.
        let mut got = false;
        while let Ok(m) = rx.try_recv() {
            got |= &*m.screen_id == e.screen.as_str();
        }
        assert!(got, "no renderer message for change {i}");
        worst = worst.max(t0.elapsed());
    }
    assert!(
        worst < Duration::from_millis(50),
        "slowest change {worst:?}"
    );
    // After the link moves again, everything arrives as a coalesced update.
    e.cfg(|c| c.stall = false);
    wait_until("caught up", || {
        e.tree()["layers"]["l2"]["values"]["home"] == "Team 99"
    });
    state::check_refs(&e.tree()).unwrap();
}

#[test]
fn exact_on_air_waits_for_the_out_animation() {
    let e = env();
    e.live();
    let hub = e.st.live().clone();
    let reg = hub.renderer_attach(&e.screen, true);
    hub.show(&e.screen, "l1").unwrap();
    wait_until("on", || e.tree()["layers"]["l1"]["on_air"] == true);
    hub.hide(&e.screen, "l1").unwrap();
    // Off air in Program, still visible: the renderer hasn't finished the out-animation.
    assert!(!e.on_air("l1"));
    assert!(hub.visible(&e.screen, "l1"));
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(e.tree()["layers"]["l1"]["on_air"], true);
    // The renderer reports: off.
    hub.renderer_gone(&reg, "l1");
    assert!(!hub.visible(&e.screen, "l1"));
    wait_until("off", || e.tree()["layers"]["l1"]["on_air"] == false);
    // A re-take during the out-animation keeps it on; the late report is ignored.
    hub.show(&e.screen, "l1").unwrap();
    hub.hide(&e.screen, "l1").unwrap();
    hub.show(&e.screen, "l1").unwrap();
    hub.renderer_gone(&reg, "l1");
    assert!(hub.visible(&e.screen, "l1"));
    // PANIC: off at once.
    hub.panic();
    assert!(!hub.visible(&e.screen, "l1"));
    // A renderer that never reports: off after the fallback.
    hub.show(&e.screen, "l1").unwrap();
    wait_until("on again", || e.tree()["layers"]["l1"]["on_air"] == true);
    hub.hide(&e.screen, "l1").unwrap();
    let t0 = Instant::now();
    wait_until("fallback", || e.tree()["layers"]["l1"]["on_air"] == false);
    let took = t0.elapsed();
    assert!(
        took >= crate::core::live_hub::FADE_CAP - Duration::from_millis(100)
            && took < crate::core::live_hub::FADE_CAP + Duration::from_millis(500),
        "{took:?}"
    );
    assert_eq!(hub.fading_count(), 0);
    // No counted renderer: off at once.
    drop(reg);
    hub.show(&e.screen, "l1").unwrap();
    hub.hide(&e.screen, "l1").unwrap();
    assert!(!hub.visible(&e.screen, "l1"));
}

#[test]
fn queue_of_events_is_bounded() {
    // Activity and diagnostics are capped.
    let e = env();
    for i in 0..(ACTIVITY_CAP + 20) {
        e.st.remote_activity_now("a", "operator", format!("x{i}"));
    }
    assert_eq!(e.st.remote_activity().len(), ACTIVITY_CAP);
    let mut d = e.st.remote().diag();
    for i in 0..(diag::MESSAGES_CAP + 10) {
        d.message(diag::MsgRecord {
            at: i as u64,
            dir: diag::Dir::In,
            t: "ping",
            corr: None,
            bytes: 1,
            seq: None,
            cause: None,
            ms: None,
        });
    }
    assert_eq!(d.messages.len(), diag::MESSAGES_CAP);
}

#[test]
fn render_path_cost_while_live() {
    // What remote control adds to every live operation: the change listener's wake and the
    // fade scan. Measured as the difference between show/hide pairs with it off and on.
    let e = env();
    let pairs = 300;
    let run = |e: &Env| {
        let t0 = Instant::now();
        for _ in 0..pairs {
            e.st.live().show(&e.screen, "l1").unwrap();
            e.st.live().hide(&e.screen, "l1").unwrap();
        }
        t0.elapsed() / (pairs * 2)
    };
    run(&e);
    let off = run(&e);
    e.live();
    let on = run(&e);
    let t0 = Instant::now();
    for _ in 0..10_000 {
        e.st.remote_touch();
    }
    let touch = t0.elapsed() / 10_000;
    let t0 = Instant::now();
    let input = e.st.remote_build_input();
    let tree = state::build(&e.st, &input);
    let build = t0.elapsed();
    println!(
        "per live op: off {off:?}, on {on:?}; remote_touch {touch:?}; tree build {build:?} ({} B)",
        tree.to_string().len()
    );
    assert!(touch < Duration::from_micros(50), "{touch:?}");
}

#[test]
fn media_layers_are_read_only_with_their_on_air_state() {
    use crate::core::model::{MediaEndAction, MediaFit, MediaLayerConfig};
    let e = env();
    let media = |id: &str, asset: &str| ScreenLayer {
        id: id.into(),
        name: format!("Clip {id}"),
        rect: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        kind: ScreenLayerKind::Media {
            media: MediaLayerConfig {
                asset: asset.into(),
                fit: MediaFit::Fit,
                in_ms: 0,
                out_ms: None,
                volume: 100,
                muted: false,
                end_action: MediaEndAction::Hold,
                autoplay: false,
            },
            opacity: None,
        },
        anchor: None,
    };
    let mut doc = e.st.get_screen(&e.screen).unwrap().doc;
    doc.meta.layers.push(media("m1", "/assets/ingrid.png"));
    e.st.save_screen(doc).unwrap();
    let snap = e.live();
    state::check_refs(&snap).unwrap();
    assert_eq!(
        snap["layers"]["m1"],
        json!({"kind": "media", "screen": e.screen, "name": "Clip m1", "on_air": false,
            "media": {"type": "image", "name": "ingrid.png"}})
    );
    // Videos are `video`; audio has no protocol type and is left out.
    assert_eq!(
        state::media_of("/assets/intro.mp4"),
        Some(("video", "intro.mp4".into()))
    );
    assert_eq!(state::media_of("/assets/bed.mp3"), None);
    assert_eq!(
        snap["screens"][&e.screen]["layers"],
        json!(["m1", "l2", "l1"])
    );
    assert_eq!(snap["layers"]["l1"]["kind"], "template");
    // Its on-air state follows local takes.
    e.st.live().show(&e.screen, "m1").unwrap();
    wait_until("on air", || e.tree()["layers"]["m1"]["on_air"] == true);
    // Every command on it is refused.
    for (corr, action, args) in [
        ("m-a", "take", json!({"layer": "m1"})),
        ("m-b", "clear", json!({"layer": "m1"})),
        (
            "m-c",
            "update_fields",
            json!({"layer": "m1", "values": {"x": "1"}}),
        ),
    ] {
        let r = e.command(corr, action, args, "operator", e.seq());
        assert_eq!(r["code"], "read_only", "{action}: {r}");
    }
    assert!(e.on_air("m1"));
}

#[test]
fn layer_ids_shared_by_two_screens_get_distinct_keys() {
    let e = env();
    // A second screen whose layer reuses the id "l1" (layer ids are unique per screen only).
    let mut doc =
        e.st.create_screen(&NewScreen {
            name: "Second".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap()
        .doc;
    let template = match &e.st.get_screen(&e.screen).unwrap().doc.meta.layers[0].kind {
        ScreenLayerKind::Template { template_id, .. } => template_id.clone(),
        _ => unreachable!(),
    };
    doc.meta.layers.push(layer("l1", &template, &[]));
    let second = e.st.save_screen(doc).unwrap().doc.id;
    let snap = e.live();
    state::check_refs(&snap).unwrap();
    let mut keys: Vec<String> = snap["layers"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    let qualified = format!("{second}_l1");
    assert!(
        keys.contains(&"l1".to_string()) && keys.contains(&qualified),
        "{keys:?}"
    );
    // Each key reaches its own screen's layer.
    let r = e.command(
        "k1",
        "take",
        json!({"layer": qualified}),
        "operator",
        e.seq(),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(e.st.live().layer_view(&second, "l1").unwrap().on_air);
    assert!(!e.on_air("l1"));
}
