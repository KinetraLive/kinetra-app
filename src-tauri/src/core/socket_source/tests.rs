//! Live data sources through `AppState` against a local axum WebSocket mock (no external
//! network): settings, in-use tracking, mapping and the filter, pacing, reconnects that keep the
//! rows, holds, the reading budget, `trigger.apiChanged`, the test connection and that rows are
//! never written to the project.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant as StdInstant};

use axum::extract::ws::{Message as AxMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use serde_json::json;
use tokio::sync::broadcast;

use super::*;
use crate::core::events::{names, EventSink};
use crate::core::model::{
    ApiMappingColumn, ColumnKind, FlowEdge, FlowNode, NewFlow, NewScreen, NodeKind, PortRef, Rect,
    Retrigger, ScreenDoc, ScreenLayer, ScreenLayerKind, SourceBinding, TemplateSource, WireKind,
};

// ---- Pure helpers --------------------------------------------------------------------------

#[test]
fn local_addresses_only() {
    for ok in [
        "127.0.0.1",
        "::1",
        "10.1.2.3",
        "172.16.0.9",
        "172.31.255.1",
        "192.168.1.20",
        "169.254.10.10",
        "fd12:3456::1",
        "fe80::1",
        "::ffff:192.168.0.2",
        "100.64.0.1",
        "100.127.255.254",
    ] {
        assert!(is_local_ip(ok.parse().unwrap()), "{ok}");
    }
    for no in [
        "8.8.8.8",
        "172.32.0.1",
        "100.63.255.255",
        "100.128.0.1",
        "0.0.0.0",
        "2001:4860::8888",
        "::ffff:8.8.8.8",
        "::",
    ] {
        assert!(!is_local_ip(no.parse().unwrap()), "{no}");
    }
    assert_eq!(check_url(""), Ok(()));
    for ok in [
        "ws://localhost:49123",
        "wss://studio.local/feed",
        "ws://192.168.1.20:8080/x",
        "ws://[::1]:9000",
    ] {
        assert_eq!(check_url(ok), Ok(()), "{ok}");
    }
    assert_eq!(
        check_url("http://localhost:1"),
        Err("The URL must start with ws:// or wss://".into())
    );
    assert_eq!(check_url("ws://8.8.8.8/"), Err(NOT_LOCAL.into()));
    assert_eq!(check_url("ws://[2001:4860::8888]/"), Err(NOT_LOCAL.into()));
    assert!(check_url("ws://").is_err());
}

#[test]
fn settings_are_normalized_and_checked() {
    let mut url = "  ws://localhost:1/x ".to_string();
    let mut mapping = ApiMapping::default();
    let mut filter = Some(SocketFilter {
        pointer: " /type ".into(),
        equals: "score".into(),
    });
    let mut send = Some(String::new());
    validate_settings(&mut url, &mut mapping, &mut filter, &mut send, 10).unwrap();
    assert_eq!(url, "ws://localhost:1/x");
    assert_eq!(filter.as_ref().unwrap().pointer, "/type");
    assert_eq!(send, None, "an empty send text is none");
    // An empty filter is none; a filter needs a pointer.
    let mut empty = Some(SocketFilter::default());
    validate_settings(&mut url, &mut mapping, &mut empty, &mut None, 10).unwrap();
    assert_eq!(empty, None);
    let mut no_ptr = Some(SocketFilter {
        pointer: String::new(),
        equals: "x".into(),
    });
    assert!(validate_settings(&mut url, &mut mapping, &mut no_ptr, &mut None, 10).is_err());
    for rate in [0, 31] {
        assert!(validate_settings(&mut url, &mut mapping, &mut None, &mut None, rate).is_err());
    }
    let mut big = Some("x".repeat(SOCKET_SEND_MAX + 1));
    assert!(validate_settings(&mut url, &mut mapping, &mut None, &mut big, 10).is_err());
}

#[test]
fn filters_and_frames() {
    let f = SocketFilter {
        pointer: "/Event".into(),
        equals: "UpdateState".into(),
    };
    assert!(passes(Some(&f), &json!({"Event": "UpdateState"})));
    assert!(!passes(Some(&f), &json!({"Event": "GoalScored"})));
    assert!(!passes(Some(&f), &json!({"Data": 1})));
    assert!(passes(None, &json!(1)));
    // Numbers and booleans compare as written in JSON.
    let n = SocketFilter {
        pointer: "/kind".into(),
        equals: "2".into(),
    };
    assert!(passes(Some(&n), &json!({"kind": 2})));
    // Several objects in one frame; not JSON at all.
    assert_eq!(frame_values(r#"{"a":1}{"a":2} {"a":3}"#).unwrap().len(), 3);
    assert_eq!(frame_values("hello"), None);
    assert_eq!(frame_values(r#"{"a":1}{oops"#).unwrap().len(), 1);
}

// ---- The mock ------------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Out {
    Text(String),
    Close,
}

struct Mock {
    tx: broadcast::Sender<Out>,
    conns: AtomicU32,
    received: Mutex<Vec<String>>,
}

async fn ws_route(State(m): State<Arc<Mock>>, up: WebSocketUpgrade) -> impl IntoResponse {
    up.max_message_size(8 * 1024 * 1024)
        .on_upgrade(move |socket| serve(m, socket))
}

async fn serve(m: Arc<Mock>, mut socket: WebSocket) {
    let mut rx = m.tx.subscribe();
    m.conns.fetch_add(1, Ordering::SeqCst);
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Ok(Out::Text(t)) => {
                    if socket.send(AxMessage::Text(t.into())).await.is_err() {
                        return;
                    }
                }
                Ok(Out::Close) => {
                    let _ = socket.send(AxMessage::Close(None)).await;
                    return;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            },
            got = socket.recv() => match got {
                Some(Ok(AxMessage::Text(t))) => m.received.lock().unwrap().push(t.to_string()),
                Some(Ok(_)) => {}
                _ => return,
            },
        }
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Value)>>);

impl EventSink for Recorder {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    st: AppState,
    rec: Arc<Recorder>,
    dir: PathBuf,
    mock: Arc<Mock>,
    /// `ws://127.0.0.1:<port>`
    base: String,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        rec.clone(),
        Duration::from_secs(60),
    )
    .unwrap();
    st.set_content_flush_delay(Duration::from_secs(60));
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    st.socket_sources()
        .set_test_backoff(Some(Duration::from_millis(50)));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    let mock = Arc::new(Mock {
        tx: broadcast::channel(4096).0,
        conns: AtomicU32::new(0),
        received: Mutex::new(Vec::new()),
    });
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let base = format!("ws://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route("/ws", axum::routing::get(ws_route))
        .route("/plain", axum::routing::get(|| async { "not a websocket" }))
        .with_state(mock.clone());
    rt.spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    Env {
        _tmp: tmp,
        rt,
        st,
        rec,
        dir,
        mock,
        base,
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = StdInstant::now() + Duration::from_secs(8);
    while !f() {
        assert!(StdInstant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn mcol(key: &str, path: &str) -> ApiMappingColumn {
    ApiMappingColumn {
        key: key.into(),
        label: key.to_uppercase(),
        kind: ColumnKind::Text,
        path: path.into(),
    }
}

fn items(rows: &[(&str, &str)]) -> String {
    json!({ "type": "rows", "items": rows.iter().map(|(n, t)| json!({ "name": n, "title": t })).collect::<Vec<_>>() })
        .to_string()
}

impl Env {
    fn send(&self, text: impl Into<String>) {
        let _ = self.mock.tx.send(Out::Text(text.into()));
    }

    fn conns(&self) -> u32 {
        self.mock.conns.load(Ordering::SeqCst)
    }

    /// A live data source on the mock: rows `/items`, columns `name` and `title`.
    fn source(&self, path: &str, edit: impl FnOnce(&mut SourceKind)) -> SourceDoc {
        let mut doc = self.st.create_socket_source("Feed").unwrap();
        doc.meta.kind = SourceKind::Socket {
            url: format!("{}{path}", self.base),
            mapping: ApiMapping {
                rows: "/items".into(),
                columns: vec![mcol("name", "/name"), mcol("title", "/title")],
                csv: Default::default(),
            },
            filter: None,
            send_on_connect: None,
            rate: 30,
            paused: false,
            columns: Vec::new(),
            rows: Vec::new(),
            index: 0,
            auto: None,
        };
        edit(&mut doc.meta.kind);
        self.st.save_source(doc).unwrap()
    }

    fn rows(&self, id: &str) -> Vec<Fields> {
        match self.st.get_source(id).unwrap().meta.kind {
            SourceKind::Socket { rows, .. } => rows,
            _ => panic!("not a live data source"),
        }
    }

    fn names(&self, id: &str) -> Vec<String> {
        self.rows(id).iter().map(|r| r["name"].clone()).collect()
    }

    fn status(&self, id: &str) -> SocketStatus {
        let doc = self.st.get_source(id).unwrap();
        self.st.socket_status(&doc).unwrap()
    }

    fn state(&self, id: &str) -> SocketState {
        self.status(id).state
    }

    fn template(&self) -> String {
        self.st
            .templates()
            .create(
                &self.dir,
                "Bar",
                &TemplateSource::Preset {
                    preset_id: "classic-bar".into(),
                },
            )
            .unwrap()
            .id
    }

    /// A screen with layer `l1` of template `t`; field `name` bound to `source` when given.
    fn screen(&self, t: &str, source: Option<&str>) -> ScreenDoc {
        let mut doc = self
            .st
            .create_screen(&NewScreen {
                name: "Main".into(),
                width: 1920,
                height: 1080,
                background: "transparent".into(),
            })
            .unwrap()
            .doc;
        let mut bindings = BTreeMap::new();
        if let Some(src) = source {
            bindings.insert(
                "name".to_string(),
                SourceBinding {
                    source_id: src.into(),
                    column: "name".into(),
                    sort: None,
                    limit: None,
                    row: None,
                },
            );
        }
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
                template_id: t.into(),
                fields: [("name", "Own"), ("title", "Own title")]
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                vars: Fields::new(),
                bindings,
                variant: None,
                sizing: None,
                opacity: None,
            },
            anchor: None,
        });
        self.st.save_screen(doc).unwrap().doc
    }

    fn field(&self, screen: &str, key: &str) -> String {
        self.st
            .live()
            .with_layer(screen, "l1", |l| {
                l.fields.get(key).cloned().unwrap_or_default()
            })
            .unwrap()
    }

    /// Watched (the editor open): in use, so it connects.
    fn watch(&self, id: &str) {
        self.st.watch_socket_source(id, true).unwrap();
    }

    fn connected(&self, id: &str, conns: u32) {
        wait_until("connected", || {
            self.conns() >= conns
                && matches!(self.state(id), SocketState::Connected | SocketState::Live)
        });
    }
}

/// Every file under `dir` (recursively) that contains `needle`.
fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            fs::read(e.path())
                .map(|b| String::from_utf8_lossy(&b).contains(needle))
                .unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

// ---- Connections ---------------------------------------------------------------------------

#[test]
fn a_bound_layer_connects_maps_and_shows_the_rows() {
    let e = env();
    let src = e.source("/ws", |k| {
        if let SourceKind::Socket {
            send_on_connect, ..
        } = k
        {
            *send_on_connect = Some(r#"{"subscribe":"all"}"#.into());
        }
    });
    assert_eq!(e.st.socket_sources().task_count(), 0, "not in use yet");
    assert_eq!(e.state(&src.id), SocketState::Idle);
    let t = e.template();
    let s = e.screen(&t, Some(&src.id));
    assert_eq!(e.st.socket_sources().task_count(), 1);
    e.connected(&src.id, 1);
    wait_until("send on connect", || {
        e.mock.received.lock().unwrap().as_slice() == [r#"{"subscribe":"all"}"#]
    });
    e.send(items(&[
        ("Ada Lovelace", "Mathematician"),
        ("Alan Turing", ""),
    ]));
    wait_until("rows", || {
        e.names(&src.id) == ["Ada Lovelace", "Alan Turing"]
    });
    wait_until("on screen", || e.field(&s.id, "name") == "Ada Lovelace");
    assert_eq!(e.state(&src.id), SocketState::Live);
    // Live rows are never written to the project (only the settings are).
    e.st.flush_all();
    assert!(files_containing(&e.dir, "Ada Lovelace").is_empty());
    let file = fs::read_to_string(e.dir.join("sources").join(format!("{}.json", src.id))).unwrap();
    assert!(file.contains("\"kind\": \"socket\"") && file.contains("/items"));
    // A save from the UI with stale rows keeps the live ones.
    let mut doc = e.st.get_source(&src.id).unwrap();
    if let SourceKind::Socket { rows, rate, .. } = &mut doc.meta.kind {
        rows.clear();
        *rate = 20;
    }
    e.st.save_source(doc).unwrap();
    assert_eq!(e.names(&src.id), ["Ada Lovelace", "Alan Turing"]);
    // Unbound: the connection stops.
    let mut screen = e.st.screens().doc(&s.id).unwrap();
    if let ScreenLayerKind::Template { bindings, .. } = &mut screen.meta.layers[0].kind {
        bindings.clear();
    }
    e.st.save_screen(screen).unwrap();
    wait_until("stopped", || e.st.socket_sources().task_count() == 0);
    assert_eq!(e.state(&src.id), SocketState::Idle);
}

#[test]
fn no_tasks_unless_in_use_and_not_paused() {
    let e = env();
    let src = e.source("/ws", |_| {});
    let empty = e.st.create_socket_source("No URL").unwrap();
    e.watch(&empty.id);
    assert_eq!(e.st.socket_sources().task_count(), 0, "no URL, no task");
    e.watch(&src.id);
    assert_eq!(e.st.socket_sources().task_count(), 1);
    e.st.set_socket_paused(&src.id, true).unwrap();
    assert_eq!(e.st.socket_sources().task_count(), 0);
    assert_eq!(e.state(&src.id), SocketState::Paused);
    e.st.set_socket_paused(&src.id, false).unwrap();
    assert_eq!(e.st.socket_sources().task_count(), 1);
    e.st.watch_socket_source(&src.id, false).unwrap();
    assert_eq!(e.st.socket_sources().task_count(), 0);
    // Deleting a watched source stops it; closing the project stops everything.
    e.watch(&src.id);
    e.st.delete_source(&src.id).unwrap();
    assert_eq!(e.st.socket_sources().task_count(), 0);
    e.watch(&empty.id);
    e.st.close_project();
    assert_eq!(e.st.socket_sources().task_count(), 0);
}

#[test]
fn the_filter_picks_the_messages_that_feed_the_rows() {
    let e = env();
    let src = e.source("/ws", |k| {
        if let SourceKind::Socket { filter, .. } = k {
            *filter = Some(SocketFilter {
                pointer: "/type".into(),
                equals: "rows".into(),
            });
        }
    });
    e.watch(&src.id);
    e.connected(&src.id, 1);
    e.send(items(&[("First", "")]));
    wait_until("first", || e.names(&src.id) == ["First"]);
    // Other message types are ignored, also inside a frame with several objects.
    e.send(json!({"type": "ping", "items": [{"name": "Nope"}]}).to_string());
    e.send(format!(
        "{}{}",
        items(&[("Second", "")]),
        json!({"type": "ping", "items": [{"name": "Nope"}]})
    ));
    wait_until("second", || e.names(&src.id) == ["Second"]);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(e.names(&src.id), ["Second"]);
}

#[test]
fn updates_are_paced_and_the_newest_wins() {
    let e = env();
    let src = e.source("/ws", |k| {
        if let SourceKind::Socket { rate, .. } = k {
            *rate = 5;
        }
    });
    e.watch(&src.id);
    e.connected(&src.id, 1);
    let before = e.st.socket_sources().applied();
    let started = StdInstant::now();
    // 120 messages a second for one second.
    for i in 0..120 {
        e.send(items(&[(&format!("n{i}"), "")]));
        std::thread::sleep(Duration::from_millis(8));
    }
    wait_until("the newest", || e.names(&src.id) == ["n119"]);
    let secs = started.elapsed().as_secs_f64();
    let applied = e.st.socket_sources().applied() - before;
    assert!(
        applied as f64 <= 5.0 * secs + 2.0,
        "{applied} updates in {secs:.2} s at 5 a second"
    );
    assert!(applied >= 2, "{applied}");
    // The same rows again: nothing applied.
    let now = e.st.socket_sources().applied();
    e.send(items(&[("n119", "")]));
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(e.st.socket_sources().applied(), now);
}

#[test]
fn a_reconnect_keeps_the_last_values() {
    let e = env();
    let src = e.source("/ws", |_| {});
    e.watch(&src.id);
    e.connected(&src.id, 1);
    e.send(items(&[("Kept", "")]));
    wait_until("rows", || e.names(&src.id) == ["Kept"]);
    let _ = e.mock.tx.send(Out::Close);
    wait_until("reconnected", || e.conns() >= 2);
    e.connected(&src.id, 2);
    assert_eq!(e.names(&src.id), ["Kept"]);
    let waited = e
        .rec
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|(n, v)| n == names::SOURCE_CHANGED && v["socket"]["state"] == "waiting");
    assert!(waited, "the drop showed as waiting");
}

#[test]
fn a_refused_connection_waits_and_retries() {
    let e = env();
    // A port nothing listens on.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let mut doc = e.st.create_socket_source("Nobody").unwrap();
    if let SourceKind::Socket { url, .. } = &mut doc.meta.kind {
        *url = format!("ws://127.0.0.1:{port}/");
    }
    let doc = e.st.save_source(doc).unwrap();
    e.watch(&doc.id);
    wait_until("waiting", || e.state(&doc.id) == SocketState::Waiting);
    let s = e.status(&doc.id);
    assert!(s.message.unwrap().starts_with("Nothing answers at"));
    assert!(s.next_at.is_some());
    assert_eq!(e.st.socket_sources().task_count(), 1, "still trying");
}

#[test]
fn a_server_that_isnt_a_websocket_holds_until_retry() {
    let e = env();
    let src = e.source("/plain", |_| {});
    e.watch(&src.id);
    wait_until("held", || e.state(&src.id) == SocketState::Error);
    assert_eq!(
        e.status(&src.id).message.as_deref(),
        Some("The server answered HTTP 200 instead of a WebSocket")
    );
    assert_eq!(e.st.socket_sources().task_count(), 0);
    e.st.retry_socket_source(&src.id).unwrap();
    assert_eq!(e.st.socket_sources().task_count(), 1, "Retry tries again");
}

#[test]
fn a_message_over_1_mb_holds_and_a_flood_is_counted() {
    let e = env();
    let src = e.source("/ws", |_| {});
    e.watch(&src.id);
    e.connected(&src.id, 1);
    // More than 200 in a second: the rest are dropped and counted.
    for i in 0..400 {
        e.send(items(&[(&format!("f{i}"), "")]));
    }
    wait_until("dropped", || e.status(&src.id).dropped > 0);
    e.send("x".repeat(SOCKET_MESSAGE_MAX + 1));
    wait_until("held", || e.state(&src.id) == SocketState::Error);
    assert!(e
        .status(&src.id)
        .message
        .unwrap()
        .contains("larger than 1 MB"));
}

#[test]
fn addresses_outside_the_network_are_refused() {
    let e = env();
    let mut doc = e.st.create_socket_source("Far").unwrap();
    if let SourceKind::Socket { url, .. } = &mut doc.meta.kind {
        *url = "ws://8.8.8.8/feed".into();
    }
    let err = e.st.save_source(doc).unwrap_err();
    assert!(err.to_string().contains(NOT_LOCAL), "{err}");
}

#[test]
fn api_changed_fires_for_live_data_sources() {
    let e = env();
    let src = e.source("/ws", |_| {});
    e.watch(&src.id);
    let node = |id: &str, kind| FlowNode {
        id: id.into(),
        position: Default::default(),
        kind,
    };
    let nodes = vec![
        node(
            "t",
            NodeKind::ApiChanged {
                source_id: src.id.clone(),
                column: Some("title".into()),
            },
        ),
        node(
            "l",
            NodeKind::Log {
                message: "changed".into(),
                toast: false,
            },
        ),
    ];
    let edges = vec![FlowEdge {
        id: "e1".into(),
        from: PortRef {
            node: "t".into(),
            port: "fired".into(),
        },
        to: PortRef {
            node: "l".into(),
            port: "in".into(),
        },
        kind: WireKind::Event,
    }];
    let check = e.st.check_flow(nodes.clone(), edges.clone()).unwrap();
    assert!(
        check.warnings.is_empty() && check.errors.is_empty(),
        "{check:?}"
    );
    let flow =
        e.st.create_flow(NewFlow {
            name: "On change".into(),
            armed: true,
            retrigger: Retrigger::Restart,
            origin: None,
            nodes,
            edges,
        })
        .unwrap()
        .flow
        .id;
    e.connected(&src.id, 1);
    let runs = || {
        e.rec
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, v)| n == names::FLOW_STATE && v["id"] == flow && v["state"] == "finished")
            .count()
    };
    e.send(items(&[("A", "one")]));
    wait_until("fired", || runs() == 1);
    // Another column changing doesn't fire it.
    e.send(items(&[("B", "one")]));
    wait_until("applied", || e.names(&src.id) == ["B"]);
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(runs(), 1);
    e.send(items(&[("B", "two")]));
    wait_until("fired again", || runs() == 2);
}

#[test]
fn the_test_connection_shows_messages_rows_and_hits() {
    let e = env();
    let src = e.source("/ws", |k| {
        if let SourceKind::Socket { filter, .. } = k {
            *filter = Some(SocketFilter {
                pointer: "/type".into(),
                equals: "rows".into(),
            });
        }
    });
    let doc = e.st.get_source(&src.id).unwrap();
    let st = e.st.clone();
    let test = e.rt.spawn(async move { st.test_socket_source(doc).await });
    wait_until("test connected", || e.conns() >= 1);
    std::thread::sleep(Duration::from_millis(50));
    e.send(json!({"type": "hello"}).to_string());
    e.send(items(&[("Ada", "Host"), ("Alan", "")]));
    let r = e.rt.block_on(test).unwrap().unwrap();
    assert_eq!(r.error, None);
    assert_eq!((r.seen, r.matched), (2, 1));
    assert_eq!(r.messages.len(), 2);
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.hits["name"], 2);
    assert_eq!(r.hits["title"], 1);
    assert!(r.json.unwrap()["items"].is_array());
    assert_eq!(e.st.socket_sources().task_count(), 0, "a test runs no task");
}
