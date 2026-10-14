//! The Control API over a real socket: the guard, REST, the WebSocket session, kicks and limits.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message as WsMsg;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::*;
use crate::core::events::EventSink;
use crate::core::model::{NewScreen, Rect, ScreenLayer, ScreenLayerKind, TemplateSource};
use crate::core::protocol::Fields;
use crate::core::secrets::MemorySecrets;
use crate::core::state::AppState;
use crate::server::{serve_with, AssetDir};

struct Rec;

impl EventSink for Rec {
    fn emit(&self, _event: &str, _payload: Value) {}
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Env {
    _tmp: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    st: AppState,
    screen: String,
    base: String,
    token: String,
    stop: Mutex<Option<oneshot::Sender<()>>>,
    http: reqwest::Client,
}

/// A project with screen "Main" holding lower third `l1`, the Control API on, and a server on a
/// free port with the control routes.
fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        Arc::new(Rec),
        Duration::from_millis(20),
    )
    .unwrap();
    st.set_content_flush_delay(Duration::from_secs(60));
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    st.set_control_api_secrets(Arc::new(MemorySecrets::default()));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    st.create_project(tmp.path(), "Nordlys Live").unwrap();
    let dir = st.project_dir().unwrap();
    let lt = st
        .templates()
        .create(
            &dir,
            "lower-third",
            &TemplateSource::Preset {
                preset_id: "lower-third".into(),
            },
        )
        .unwrap()
        .id;
    let mut doc = st
        .create_screen(&NewScreen {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap()
        .doc;
    doc.meta.layers.push(ScreenLayer {
        id: "l1".into(),
        name: "Lower third".into(),
        rect: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        kind: ScreenLayerKind::Template {
            template_id: lt,
            fields: Fields::new(),
            vars: Fields::new(),
            bindings: Default::default(),
            variant: None,
            sizing: None,
            opacity: None,
        },
        anchor: None,
    });
    let screen = st.save_screen(doc).unwrap().doc.id;
    st.control_api_set_enabled(true).unwrap();
    let token = st.control_api_token().unwrap();

    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let base = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (stop, stop_rx) = oneshot::channel::<()>();
    let hub = st.live().clone();
    let extra = routes(&st);
    rt.spawn(async move {
        let shutdown = async move {
            let _ = stop_rx.await;
        };
        let _ = serve_with(listener, hub, AssetDir::default(), Some(extra), shutdown).await;
    });
    Env {
        _tmp: tmp,
        rt,
        st,
        screen,
        base,
        token,
        stop: Mutex::new(Some(stop)),
        http: reqwest::Client::new(),
    }
}

impl Env {
    fn key(&self) -> String {
        format!("{}.l1", self.screen)
    }

    fn on_air(&self) -> bool {
        self.st
            .live()
            .layer_view(&self.screen, "l1")
            .unwrap()
            .on_air
    }

    /// A request with the right token; `(status, body)`.
    fn req(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> (u16, Value) {
        self.req_with(method, path, body, Some(&self.token), None)
    }

    fn req_with(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        token: Option<&str>,
        origin: Option<&str>,
    ) -> (u16, Value) {
        self.rt.block_on(async {
            let mut r = self
                .http
                .request(method, format!("http://{}{path}", self.base));
            if let Some(t) = token {
                r = r.header("Authorization", format!("Bearer {t}"));
            }
            if let Some(o) = origin {
                r = r.header("Origin", o);
            }
            if let Some(b) = body {
                r = r
                    .header("Content-Type", "application/json")
                    .body(b.to_string());
            }
            let res = r.send().await.unwrap();
            let status = res.status().as_u16();
            let text = res.text().await.unwrap();
            (status, serde_json::from_str(&text).unwrap_or(Value::Null))
        })
    }

    fn post(&self, path: &str) -> (u16, Value) {
        self.req(reqwest::Method::POST, path, None)
    }

    /// Opens `/api/control` with the token (and the subprotocol); `Err` = the HTTP status.
    fn connect(&self, token: &str) -> Result<Ws, u16> {
        self.rt.block_on(async {
            let mut req = format!("ws://{}/api/control", self.base)
                .into_client_request()
                .unwrap();
            let h = req.headers_mut();
            h.insert(
                "Authorization",
                HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
            );
            h.insert(
                "Sec-WebSocket-Protocol",
                HeaderValue::from_static(SUBPROTOCOL),
            );
            match tokio_tungstenite::connect_async(req).await {
                Ok((ws, res)) => {
                    assert_eq!(
                        res.headers()
                            .get("sec-websocket-protocol")
                            .and_then(|v| v.to_str().ok()),
                        Some(SUBPROTOCOL)
                    );
                    Ok(ws)
                }
                Err(tokio_tungstenite::tungstenite::Error::Http(r)) => Err(r.status().as_u16()),
                Err(e) => panic!("{e}"),
            }
        })
    }

    fn send(&self, ws: &mut Ws, v: Value) {
        self.rt
            .block_on(ws.send(WsMsg::Text(v.to_string().into())))
            .unwrap();
    }

    /// The next text message (pings skipped), or the close code.
    fn recv(&self, ws: &mut Ws) -> Result<Value, Option<u16>> {
        self.rt.block_on(async {
            loop {
                let m = tokio::time::timeout(Duration::from_secs(5), ws.next())
                    .await
                    .expect("a message in time");
                match m {
                    Some(Ok(WsMsg::Text(t))) => return Ok(serde_json::from_str(&t).unwrap()),
                    Some(Ok(WsMsg::Close(f))) => return Err(f.map(|f| u16::from(f.code))),
                    Some(Ok(_)) => continue,
                    Some(Err(_)) | None => return Err(None),
                }
            }
        })
    }

    fn hello(&self) -> (Ws, Value) {
        let mut ws = self.connect(&self.token).unwrap();
        self.send(
            &mut ws,
            json!({"t": "hello", "protocol": 1, "client": "test/1.0"}),
        );
        let w = self.recv(&mut ws).unwrap();
        assert_eq!(w["t"], "welcome", "{w}");
        let snap = self.recv(&mut ws).unwrap();
        assert_eq!(snap["t"], "snapshot", "{snap}");
        (ws, snap)
    }

    /// The next message that isn't a diff.
    fn recv_skipping_diffs(&self, ws: &mut Ws) -> Result<Value, Option<u16>> {
        loop {
            let v = self.recv(ws)?;
            if v["t"] != "diff" {
                return Ok(v);
            }
        }
    }
}

#[test]
fn the_guard_refuses_browsers_wrong_tokens_and_a_switched_off_api() {
    let e = env();
    let get = reqwest::Method::GET;
    // A browser page (any Origin), even with the right token.
    let (s, b) = e.req_with(
        get.clone(),
        "/api/state",
        None,
        Some(&e.token),
        Some("http://evil.example"),
    );
    assert_eq!((s, b["code"].clone()), (403, json!("origin")));
    // No token, a wrong one, another scheme.
    let (s, b) = e.req_with(get.clone(), "/api/state", None, None, None);
    assert_eq!((s, b["code"].clone()), (401, json!("token")));
    let (s, _) = e.req_with(get.clone(), "/api/state", None, Some("kt_wrong"), None);
    assert_eq!(s, 401);
    // The right one.
    let (s, b) = e.req(get.clone(), "/api/state", None);
    assert_eq!(s, 200);
    assert_eq!(b["state"]["layers"][e.key()]["on_air"], false);
    // Off: 503 with a code clients can explain.
    e.st.control_api_set_enabled(false).unwrap();
    let (s, b) = e.req(get.clone(), "/api/state", None);
    assert_eq!((s, b["code"].clone()), (503, json!("disabled")));
    e.st.control_api_set_enabled(true).unwrap();
    // Ten wrong tokens in a minute lock the IP out, the right token included.
    for _ in 0..8 {
        e.req_with(get.clone(), "/api/state", None, Some("kt_wrong"), None);
    }
    let (s, b) = e.req(get.clone(), "/api/state", None);
    assert_eq!((s, b["code"].clone()), (429, json!("rate_limited")));
    // The WebSocket goes through the same guard.
    assert_eq!(e.connect(&e.token).unwrap_err(), 429);
}

#[test]
fn rest_routes_run_commands() {
    let e = env();
    let s = e.screen.clone();
    assert_eq!(e.post(&format!("/api/layers/{s}/l1/take")).0, 200);
    assert!(e.on_air());
    assert_eq!(e.post(&format!("/api/layers/{s}/l1/clear")).0, 200);
    assert!(!e.on_air());
    assert_eq!(e.post(&format!("/api/layers/{s}/l1/toggle")).0, 200);
    assert!(e.on_air());
    assert_eq!(e.post(&format!("/api/screens/{s}/clear")).0, 200);
    assert!(!e.on_air());

    let tree = e.st.control_api_tree();
    let tid = tree["layers"][e.key()]["template"].as_str().unwrap();
    let key = tree["templates"][tid]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["type"] == "text")
        .unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let (st, b) = e.req(
        reqwest::Method::PATCH,
        &format!("/api/layers/{s}/l1/fields"),
        Some(json!({"values": {&key: "Ingrid Vale"}, "take": true})),
    );
    assert_eq!((st, b), (200, json!({"ok": true})));
    let view = e.st.live().layer_view(&e.screen, "l1").unwrap();
    assert!(view.on_air);
    assert_eq!(view.fields[&key], "Ingrid Vale");

    // The generic route takes any command.
    let (st, _) = e.req(
        reqwest::Method::POST,
        "/api/command",
        Some(json!({"action": "clear", "args": {"layer": e.key()}})),
    );
    assert_eq!(st, 200);
    assert!(!e.on_air());
    assert_eq!(e.post("/api/clear-all").0, 200);
    assert_eq!(e.post("/api/panic").0, 200);

    // Refusals map to HTTP statuses.
    assert_eq!(e.post(&format!("/api/layers/{s}/nope/take")).0, 404);
    assert_eq!(e.post(&format!("/api/layers/{s}/l1/dance")).0, 404);
    assert_eq!(e.post("/api/cues/next").0, 404);
    assert_eq!(e.post("/api/cues/nope/run").0, 404);
    assert_eq!(e.post("/api/sources/nope/next").0, 404);
    assert_eq!(e.post(&format!("/api/screens/{s}/nope")).0, 404);
    let (st, b) = e.req(
        reqwest::Method::PATCH,
        &format!("/api/layers/{s}/l1/fields"),
        Some(json!({"values": {}})),
    );
    assert_eq!((st, b["code"].clone()), (400, json!("invalid")));
    let (st, _) = e.rt.block_on(async {
        let r = e
            .http
            .post(format!("http://{}/api/command", e.base))
            .header("Authorization", format!("Bearer {}", e.token))
            .body("not json")
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), ())
    });
    assert_eq!(st, 400);
    let (st, b) = e.req(
        reqwest::Method::POST,
        "/api/command",
        Some(json!({"action": "studio_take", "args": {"screen": s}})),
    );
    assert_eq!((st, b["code"].clone()), (409, json!("not_studio")));
}

#[test]
fn websocket_snapshot_diffs_commands_and_resync() {
    let e = env();
    let (mut ws, snap) = e.hello();
    assert_eq!(snap["state"]["layers"][e.key()]["on_air"], false);
    assert_eq!(e.st.control_api_view().clients[0].name, "test/1.0");
    assert_eq!(e.st.control_api_view().clients[0].addr, "127.0.0.1");

    // A change from elsewhere arrives as a diff.
    e.st.live().show(&e.screen, "l1").unwrap();
    let d = e.recv(&mut ws).unwrap();
    assert_eq!(d["t"], "diff");
    let path = format!("/layers/{}/on_air", e.key());
    assert!(d["ops"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["path"] == path && o["value"] == true));

    // A command: its result, and the diff it caused names it.
    e.send(
        &mut ws,
        json!({"t": "command", "corr": "c1", "action": "clear", "args": {"layer": e.key()}}),
    );
    let mut got_result = false;
    let mut got_cause = false;
    while !(got_result && got_cause) {
        let m = e.recv(&mut ws).unwrap();
        match m["t"].as_str() {
            Some("result") => {
                assert_eq!(m, json!({"t": "result", "corr": "c1", "ok": true}));
                got_result = true;
            }
            Some("diff") if m["cause"] == "c1" => got_cause = true,
            _ => {}
        }
    }
    assert!(!e.on_air());

    // A refused command.
    e.send(
        &mut ws,
        json!({"t": "command", "corr": "c2", "action": "take", "args": {"layer": "x.y"}}),
    );
    let r = e.recv_skipping_diffs(&mut ws).unwrap();
    assert_eq!(r["corr"], "c2");
    assert_eq!(r["ok"], false);
    assert_eq!(r["code"], "not_found");

    // Resync: a snapshot of the current tree.
    e.send(&mut ws, json!({"t": "resync"}));
    let s = e.recv_skipping_diffs(&mut ws).unwrap();
    assert_eq!(s["t"], "snapshot");
    assert_eq!(s["state"], e.st.control_api_tree());

    // Garbage closes with 4400.
    e.send(&mut ws, json!({"t": "dance"}));
    assert_eq!(
        e.recv_skipping_diffs(&mut ws),
        Err(Some(close::BAD_MESSAGE))
    );
}

#[test]
fn a_session_needs_hello_first() {
    let e = env();
    let mut ws = e.connect(&e.token).unwrap();
    e.send(
        &mut ws,
        json!({"t": "command", "corr": "x", "action": "panic"}),
    );
    assert_eq!(e.recv(&mut ws), Err(Some(close::BAD_MESSAGE)));
    assert!(e.st.control_api_view().clients.is_empty());
}

#[test]
fn kicks_close_with_their_codes() {
    let e = env();
    let (mut a, _) = e.hello();
    let (mut b, _) = e.hello();
    let id = e.st.control_api_view().clients[0].id;
    e.st.control_api_disconnect(id);
    assert_eq!(
        e.recv_skipping_diffs(&mut a),
        Err(Some(close::DISCONNECTED))
    );
    e.st.control_api_regenerate_token().unwrap();
    assert_eq!(
        e.recv_skipping_diffs(&mut b),
        Err(Some(close::TOKEN_CHANGED))
    );
    // The old token no longer opens a connection.
    assert_eq!(e.connect(&e.token).unwrap_err(), 401);

    let token = e.st.control_api_token().unwrap();
    let mut c = e.connect(&token).unwrap();
    e.send(&mut c, json!({"t": "hello", "protocol": 1, "client": "c"}));
    assert_eq!(e.recv(&mut c).unwrap()["t"], "welcome");
    e.st.control_api_set_enabled(false).unwrap();
    assert_eq!(e.recv_skipping_diffs(&mut c), Err(Some(close::TURNED_OFF)));
    wait_until("clients gone", || {
        e.st.control_api_view().clients.is_empty()
    });
    wait_until("feed gone", || e.st.control_api_tasks() == 0);
}

#[test]
fn the_server_stopping_closes_sessions() {
    let e = env();
    let (mut ws, _) = e.hello();
    if let Some(stop) = e.stop.lock().unwrap().take() {
        let _ = stop.send(());
    }
    assert_eq!(e.recv_skipping_diffs(&mut ws), Err(Some(close::GOING_AWAY)));
}

#[test]
fn at_most_eight_clients_and_commands_are_limited() {
    let e = env();
    let mut all: Vec<Ws> = (0..CLIENTS_MAX).map(|_| e.hello().0).collect();
    assert_eq!(e.connect(&e.token).unwrap_err(), 503);
    // A burst over the per-second limit: some are refused with `rate_limited`, none lost.
    let ws = &mut all[0];
    let n = COMMANDS_PER_SEC as usize + 20;
    for i in 0..n {
        e.send(
            ws,
            json!({"t": "command", "corr": format!("r{i}"), "action": "take", "args": {"layer": "x.y"}}),
        );
    }
    let mut results = Vec::new();
    while results.len() < n {
        let m = e.recv_skipping_diffs(ws).unwrap();
        if m["t"] == "result" {
            results.push(m["code"].as_str().unwrap_or("ok").to_string());
        }
    }
    let limited = results.iter().filter(|c| *c == "rate_limited").count();
    let busy = results.iter().filter(|c| *c == "busy").count();
    let ran = results.iter().filter(|c| *c == "not_found").count();
    assert!(limited >= 20, "{limited} rate limited");
    assert_eq!(limited + busy + ran, n);
    assert!(ran >= 1);
}

#[test]
fn closing_the_last_socket_ends_the_feed() {
    let e = env();
    let (mut ws, _) = e.hello();
    assert_eq!(e.st.control_api_tasks(), 1);
    e.rt.block_on(ws.close(None)).unwrap();
    wait_until("feed gone", || e.st.control_api_tasks() == 0);
    assert!(e.st.control_api_view().clients.is_empty());
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn close_codes_are_application_codes() {
    // The codes the app sends are in the private range a client can tell apart.
    for c in [
        close::BAD_MESSAGE,
        close::TOKEN_CHANGED,
        close::DISCONNECTED,
        close::CLIENT_LIMIT,
        close::TURNED_OFF,
    ] {
        assert!((4000..5000).contains(&c));
        assert!(matches!(CloseCode::from(c), CloseCode::Library(_)));
    }
}

#[test]
fn the_flow_route_fires_the_api_call_trigger() {
    use crate::core::model::{FlowEdge, FlowNode, NewFlow, NodeKind, PortRef, Retrigger, WireKind};
    let e = env();
    let n = |id: &str, kind| FlowNode {
        id: id.into(),
        position: Default::default(),
        kind,
    };
    let flow =
        e.st.create_flow(NewFlow {
            name: "Goal".into(),
            armed: true,
            retrigger: Retrigger::Restart,
            origin: None,
            nodes: vec![
                n("r", NodeKind::Rest {}),
                n(
                    "t",
                    NodeKind::Take {
                        screen_id: e.screen.clone(),
                        layer_id: "l1".into(),
                        for_ms: None,
                    },
                ),
            ],
            edges: vec![FlowEdge {
                id: "e1".into(),
                from: PortRef {
                    node: "r".into(),
                    port: "fired".into(),
                },
                to: PortRef {
                    node: "t".into(),
                    port: "in".into(),
                },
                kind: WireKind::Event,
            }],
        })
        .unwrap()
        .flow
        .id;
    let (s, b) = e.req(
        reqwest::Method::POST,
        &format!("/api/flows/{flow}/run"),
        Some(json!({"value": "Home 2–1"})),
    );
    assert_eq!((s, b), (200, json!({"ok": true})));
    wait_until("on air", || e.on_air());
    // No body is fine too; unknown and disarmed flows have their own statuses.
    assert_eq!(e.post(&format!("/api/flows/{flow}/run")).0, 200);
    assert_eq!(e.post("/api/flows/nope/run").0, 404);
    e.st.set_flow_armed(&flow, false).unwrap();
    let (s, b) = e.post(&format!("/api/flows/{flow}/run"));
    assert_eq!((s, b["code"].clone()), (409, json!("disarmed")));
}
