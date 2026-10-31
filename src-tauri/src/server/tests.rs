//! Server lifecycle, HTTP routes and the WebSocket session, over real sockets with a raw
//! `std::net::TcpStream` client (no extra crates).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Instant;

use serde_json::Value;

use super::*;
use crate::core::live::{LiveState, TEST_LAYER_ID, TEST_SCREEN_ID};
use crate::core::model::ProjectSettings;
use crate::core::protocol::Fields;

#[derive(Default)]
struct Recorder(Mutex<Vec<Value>>);

impl EventSink for Recorder {
    fn emit(&self, event: &str, payload: Value) {
        assert_eq!(event, names::SERVER_STATUS);
        self.0.lock().unwrap().push(payload);
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn project(port: u16) -> Project {
    Project {
        path: "C:/show".into(),
        name: "Show".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        settings: ProjectSettings {
            port,
            ..Default::default()
        },
    }
}

/// Deadlines are generous: a loaded CI runner can take seconds to answer a debug-build server.
fn wait_for_status(m: &ServerManager, what: &str, f: impl Fn(&ServerStatus) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let s = m.status();
        if f(&s) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {s:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn running_on(port: u16) -> impl Fn(&ServerStatus) -> bool {
    move |s| matches!(s, ServerStatus::Running { port: p, .. } if *p == port)
}

fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn connect(port: u16) -> TcpStream {
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    s
}

/// `(status, lowercase headers, body)` of a GET.
fn http_get(port: u16, path: &str) -> (u16, String, String) {
    let mut s = connect(port);
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, head.to_ascii_lowercase(), body.to_string())
}

/// Opens `/ws?screen=<screen>` and returns the stream positioned after the handshake.
fn ws_connect(port: u16, screen: &str) -> TcpStream {
    let mut s = connect(port);
    write!(
        s,
        "GET /ws?screen={screen} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    )
    .unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    s
}

/// Reads one unmasked server frame: `(opcode, payload)`.
fn ws_read(s: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut h = [0u8; 2];
    s.read_exact(&mut h).unwrap();
    let opcode = h[0] & 0x0f;
    let len = match h[1] & 0x7f {
        126 => {
            let mut b = [0u8; 2];
            s.read_exact(&mut b).unwrap();
            u16::from_be_bytes(b) as usize
        }
        127 => {
            let mut b = [0u8; 8];
            s.read_exact(&mut b).unwrap();
            u64::from_be_bytes(b) as usize
        }
        n => n as usize,
    };
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).unwrap();
    (opcode, payload)
}

fn ws_json(s: &mut TcpStream) -> Value {
    let (op, payload) = ws_read(s);
    assert_eq!(op, 1, "text frame");
    serde_json::from_slice(&payload).unwrap()
}

fn manager() -> (ServerManager, Arc<Recorder>, Arc<LiveHub>) {
    let rec = Arc::new(Recorder::default());
    let hub = Arc::new(LiveHub::with_test_screen());
    (ServerManager::new(rec.clone(), hub.clone()), rec, hub)
}

#[test]
fn follows_project_open_change_and_close() {
    let (m, rec, _) = manager();
    let p1 = free_port();
    // No project: stopped, nothing listening.
    m.project_changed(None);
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(m.status(), ServerStatus::Stopped);
    assert!(
        rec.0.lock().unwrap().is_empty(),
        "no event without a change"
    );
    assert!(port_is_free(p1));

    m.project_changed(Some(&project(p1)));
    wait_for_status(&m, "running", running_on(p1));
    assert_eq!(
        m.status(),
        ServerStatus::Running {
            port: p1,
            url: format!("http://127.0.0.1:{p1}"),
            lan_url: None,
        }
    );
    assert_eq!(http_get(p1, "/health").0, 200);
    assert_eq!(
        *rec.0.lock().unwrap(),
        [
            serde_json::json!({"state": "starting", "port": p1}),
            serde_json::json!({"state": "running", "port": p1,
                "url": format!("http://127.0.0.1:{p1}"), "lanUrl": null}),
        ]
    );

    // Same settings again (e.g. a name change): no restart, no event.
    m.project_changed(Some(&project(p1)));
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(rec.0.lock().unwrap().len(), 2);

    // Port change: rebinds.
    let p2 = free_port();
    m.project_changed(Some(&project(p2)));
    wait_for_status(&m, "rebound", running_on(p2));
    assert_eq!(http_get(p2, "/health").0, 200);
    assert!(port_is_free(p1), "old port released");

    m.project_changed(None);
    wait_for_status(&m, "stopped", |s| *s == ServerStatus::Stopped);
    assert!(port_is_free(p2), "no listening socket without a project");
    assert_eq!(
        rec.0.lock().unwrap().last().unwrap(),
        &serde_json::json!({"state": "stopped"})
    );
}

#[test]
fn occupied_port_reports_port_in_use() {
    let (m, _, _) = manager();
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = blocker.local_addr().unwrap().port();
    m.project_changed(Some(&project(port)));
    wait_for_status(&m, "error", |s| matches!(s, ServerStatus::Error { .. }));
    match m.status() {
        ServerStatus::Error {
            code,
            port: p,
            message,
        } => {
            assert_eq!(code, ServerErrorCode::PortInUse);
            assert_eq!(p, port);
            assert!(message.contains(&port.to_string()));
        }
        s => panic!("{s:?}"),
    }
    // Free the port and reopen: recovers.
    drop(blocker);
    m.project_changed(None);
    wait_for_status(&m, "stopped", |s| *s == ServerStatus::Stopped);
    m.project_changed(Some(&project(port)));
    wait_for_status(&m, "running", running_on(port));
    m.shutdown();
}

#[test]
fn shutdown_with_open_socket_is_bounded() {
    let (m, _, _) = manager();
    let port = free_port();
    m.project_changed(Some(&project(port)));
    wait_for_status(&m, "running", running_on(port));
    let mut ws = ws_connect(port, TEST_SCREEN_ID);
    assert_eq!(ws_json(&mut ws)["t"], "hello");
    assert_eq!(ws_json(&mut ws)["t"], "snapshot");

    let t = Instant::now();
    m.shutdown();
    assert!(
        t.elapsed() < Duration::from_millis(900),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(m.status(), ServerStatus::Stopped);
    // The renderer got a Close frame rather than a hung socket.
    assert_eq!(ws_read(&mut ws).0, 8);
    assert!(port_is_free(port));
    // After shutdown, project changes are ignored.
    m.project_changed(Some(&project(port)));
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(m.status(), ServerStatus::Stopped);
}

#[test]
fn lan_ip_is_never_loopback() {
    if let Some(ip) = lan_ipv4() {
        assert!(!ip.is_loopback() && !ip.is_unspecified());
    }
}

/// Runs [`serve`] on a private runtime; returns the port, the stop sender and the runtime.
fn serve_on_test_runtime(
    hub: Arc<LiveHub>,
) -> (
    u16,
    oneshot::Sender<()>,
    JoinHandle<()>,
    tokio::runtime::Runtime,
) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let listener = rt.block_on(TcpListener::bind(("127.0.0.1", 0))).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = oneshot::channel::<()>();
    let task = rt.spawn(async move {
        serve(listener, hub, AssetDir::default(), async move {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    (port, tx, task, rt)
}

#[test]
fn http_routes() {
    let (port, stop, task, rt) = serve_on_test_runtime(Arc::new(LiveHub::with_test_screen()));

    let (status, head, body) = http_get(port, "/health");
    assert_eq!(status, 200);
    assert!(head.contains("content-type: application/json"), "{head}");
    assert!(head.contains("x-content-type-options: nosniff"));
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));

    // The renderer may or may not be built (rust-embed is compile-time in release).
    let (status, head, body) = http_get(port, "/screen/test");
    assert!(head.contains("cache-control: no-cache"), "{head}");
    assert!(head.contains("x-content-type-options: nosniff"));
    match status {
        200 => assert!(head.contains("content-type: text/html")),
        503 => assert!(body.contains("pnpm --dir renderer build")),
        s => panic!("unexpected status {s}"),
    }
    assert_eq!(http_get(port, "/screen/anything-else").0, status);

    // Display output test patterns: a static page of the renderer build, any query.
    let (status, head, body) = http_get(port, "/pattern?kind=bars&w=1920&h=1080&mode=key");
    assert!(head.contains("cache-control: no-cache"), "{head}");
    match status {
        200 => {
            assert!(head.contains("content-type: text/html"), "{head}");
            assert!(body.contains("<"), "{body}");
        }
        503 => assert!(body.contains("pnpm --dir renderer build")),
        s => panic!("unexpected status {s}"),
    }
    assert_eq!(http_get(port, "/pattern/extra").0, 404);

    let (status, head, _) = http_get(port, "/renderer/nope.js");
    assert_eq!(status, 404);
    assert!(head.contains("x-content-type-options: nosniff"));
    // Fonts: a missing font (e.g. renderer not built) is a 404, and so is anything not a font.
    assert_eq!(http_get(port, "/fonts/definitely-missing.css").0, 404);
    assert_eq!(http_get(port, "/fonts/../index.html").0, 404);
    assert_eq!(http_get(port, "/fonts/index.html").0, 404);
    let (status, head, _) = http_get(port, "/unknown");
    assert_eq!(status, 404);
    assert!(head.contains("x-content-type-options: nosniff"));
    assert_eq!(http_get(port, "/").0, 404);

    stop.send(()).unwrap();
    rt.block_on(async { tokio::time::timeout(STOP_TIMEOUT, task).await })
        .unwrap()
        .unwrap();
}

#[test]
fn websocket_hello_snapshot_diffs_and_shutdown() {
    let hub = Arc::new(LiveHub::with_test_screen());
    let (port, stop, task, rt) = serve_on_test_runtime(hub.clone());

    let mut ws = ws_connect(port, TEST_SCREEN_ID);
    let hello = ws_json(&mut ws);
    assert_eq!(
        (&hello["t"], &hello["protocol"]),
        (&serde_json::json!("hello"), &serde_json::json!(1))
    );
    let now = crate::core::fsx::now_epoch_ms();
    let sent = hello["now"].as_u64().unwrap();
    assert!(
        sent <= now && now - sent < 10_000,
        "hello carries server time"
    );
    let snap = ws_json(&mut ws);
    assert_eq!(snap["t"], "snapshot");
    assert_eq!(snap["screen"]["id"], "test");
    assert_eq!(snap["layers"][0]["state"], "out");

    let mut other = ws_connect(port, "nope");
    assert_eq!(ws_json(&mut other)["t"], "hello");
    assert_eq!(
        ws_json(&mut other),
        serde_json::json!({"t": "error", "code": "unknownScreen"})
    );

    hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
    assert_eq!(
        ws_json(&mut ws),
        serde_json::json!({"t": "state", "layerId": "lower-third", "state": "in"})
    );

    // A project switch resets live state: renderers get a fresh snapshot.
    hub.reset(LiveState::with_test_screen());
    let snap = ws_json(&mut ws);
    assert_eq!(snap["t"], "snapshot");
    assert_eq!(snap["layers"][0]["state"], "out");

    // Shutdown with sockets open: each gets a Close frame and serve returns promptly.
    let t = Instant::now();
    stop.send(()).unwrap();
    assert_eq!(ws_read(&mut ws).0, 8);
    assert_eq!(ws_read(&mut other).0, 8);
    rt.block_on(async { tokio::time::timeout(STOP_TIMEOUT, task).await })
        .unwrap()
        .unwrap();
    assert!(t.elapsed() < STOP_TIMEOUT);
}

#[test]
fn renderer_of_a_screen_follows_its_creation_and_deletion() {
    use crate::core::live::ScreenPlan;
    use crate::core::model::LiveEdit;
    use crate::core::protocol::ScreenInfo;

    let hub = Arc::new(LiveHub::with_test_screen());
    let (port, stop, task, rt) = serve_on_test_runtime(hub.clone());
    // Connected before the screen exists (e.g. OBS opened first): it waits.
    let mut ws = ws_connect(port, "later12345");
    assert_eq!(ws_json(&mut ws)["t"], "hello");
    assert_eq!(ws_json(&mut ws)["code"], "unknownScreen");

    let plan = ScreenPlan {
        info: ScreenInfo {
            id: "later12345".into(),
            name: "Later".into(),
            width: 1280,
            height: 720,
            background: "transparent".into(),
        },
        layers: Vec::new(),
    };
    hub.apply_screen(plan, LiveEdit::OnTake).unwrap();
    let snap = ws_json(&mut ws);
    assert_eq!(snap["t"], "snapshot");
    assert_eq!(snap["screen"]["width"], 1280);

    assert!(hub.remove_screen("later12345"));
    assert_eq!(
        ws_json(&mut ws),
        serde_json::json!({"t": "error", "code": "unknownScreen"})
    );
    // A new connection to the deleted screen is told the same.
    let mut again = ws_connect(port, "later12345");
    assert_eq!(ws_json(&mut again)["t"], "hello");
    assert_eq!(ws_json(&mut again)["code"], "unknownScreen");

    stop.send(()).unwrap();
    rt.block_on(async { tokio::time::timeout(STOP_TIMEOUT, task).await })
        .unwrap()
        .unwrap();
}

/// A GET with extra request headers: `(status, head, body bytes)`.
fn http_get_with(port: u16, path: &str, extra: &str) -> (u16, String, Vec<u8>) {
    let mut s = connect(port);
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n{extra}\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, head, raw[split + 4..].to_vec())
}

fn header_of(head: &str, name: &str) -> Option<String> {
    head.lines().find_map(|l| {
        l.split_once(':')
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_string())
    })
}

#[test]
fn project_assets_are_served_safely_with_revalidation() {
    let tmp = tempfile::tempdir().unwrap();
    let assets_dir = tmp.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let png = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
    std::fs::write(assets_dir.join("logo.png"), png).unwrap();
    std::fs::write(
        assets_dir.join("mark.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
    )
    .unwrap();
    std::fs::write(tmp.path().join("project.json"), "{}").unwrap();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let listener = rt.block_on(TcpListener::bind(("127.0.0.1", 0))).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = oneshot::channel::<()>();
    let assets = AssetDir::default();
    let served = assets.clone();
    let task = rt.spawn(async move {
        serve(listener, Arc::new(LiveHub::default()), served, async move {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });

    // No project: nothing is served.
    assert_eq!(http_get_with(port, "/assets/logo.png", "").0, 404);
    assets.set_project(Some(tmp.path().to_path_buf()));

    let (status, head, body) = http_get_with(port, "/assets/logo.png", "");
    assert_eq!(status, 200, "{head}");
    assert_eq!(body, png);
    assert_eq!(
        header_of(&head, "content-type").as_deref(),
        Some("image/png")
    );
    assert_eq!(
        header_of(&head, "cache-control").as_deref(),
        Some("no-cache")
    );
    assert_eq!(
        header_of(&head, "x-content-type-options").as_deref(),
        Some("nosniff")
    );
    assert!(header_of(&head, "content-security-policy").is_none());
    let etag = header_of(&head, "etag").unwrap();
    let modified = header_of(&head, "last-modified").unwrap();
    assert!(modified.ends_with(" GMT"), "{modified}");

    // Revalidation: 304 with no body.
    let (status, _, body) = http_get_with(
        port,
        "/assets/logo.png",
        &format!("If-None-Match: {etag}\r\n"),
    );
    assert_eq!(status, 304);
    assert!(body.is_empty());
    let (status, _, _) = http_get_with(port, "/assets/logo.png", "If-None-Match: \"other\"\r\n");
    assert_eq!(status, 200);
    let lm = modified.clone();
    let (status, _, _) = http_get_with(
        port,
        "/assets/logo.png",
        &format!("If-Modified-Since: {lm}\r\n"),
    );
    assert_eq!(status, 304);

    // SVG: no script.
    let (status, head, _) = http_get_with(port, "/assets/mark.svg", "");
    assert_eq!(status, 200);
    assert_eq!(
        header_of(&head, "content-type").as_deref(),
        Some("image/svg+xml")
    );
    assert_eq!(
        header_of(&head, "content-security-policy").as_deref(),
        Some("script-src 'none'")
    );

    // Path safety: only plain image names from assets/.
    for bad in [
        "/assets/..%2Fproject.json",
        "/assets/%2E%2E%2Fproject.json",
        "/assets/../project.json",
        "/assets/project.json",
        "/assets/.logo.png",
        "/assets/missing.png",
        "/assets/logo.PNG",
        "/assets/sub/logo.png",
        "/assets/..%5Cproject.json",
    ] {
        let (status, _, _) = http_get_with(port, bad, "");
        assert_eq!(status, 404, "{bad}");
    }

    let _ = tx.send(());
    rt.block_on(task).unwrap();
}

#[test]
fn preview_page_and_bus() {
    use crate::core::live_hub::StudioAction;
    use crate::core::model::PreviewAfterTake;
    use crate::core::protocol::{TakeTransition, TakeTransitionKind};

    let mut live = LiveState::with_test_screen();
    live.studio = true;
    let hub = Arc::new(LiveHub::new(live));
    let (port, stop, task, rt) = serve_on_test_runtime(hub.clone());

    // The preview page is the same page as Program's.
    let program_status = http_get(port, "/screen/test").0;
    assert_eq!(http_get(port, "/screen/test/preview").0, program_status);
    assert_eq!(http_get(port, "/screen/test/preview/").0, program_status);

    let mut prog = ws_connect(port, TEST_SCREEN_ID);
    let mut pv = ws_connect(port, "test&bus=preview");
    for ws in [&mut prog, &mut pv] {
        assert_eq!(ws_json(ws)["t"], "hello");
        let snap = ws_json(ws);
        assert_eq!(snap["t"], "snapshot");
        assert_eq!(snap["layers"][0]["state"], "out");
    }
    assert_eq!(hub.preview_buses(), 1);

    // A cue reaches the preview bus only.
    hub.studio_op(
        TEST_SCREEN_ID,
        StudioAction::Cue {
            layer_id: TEST_LAYER_ID,
            visible: Some(true),
        },
    )
    .unwrap();
    assert_eq!(
        ws_json(&mut pv),
        serde_json::json!({"t": "state", "layerId": "lower-third", "state": "in"})
    );
    // The take reaches Program only (Preview already shows it).
    hub.studio_op(
        TEST_SCREEN_ID,
        StudioAction::Take {
            transition: TakeTransition {
                kind: TakeTransitionKind::Crossfade,
                ms: 400,
            },
            after: PreviewAfterTake::Keep,
        },
    )
    .unwrap();
    let take = ws_json(&mut prog);
    assert_eq!(take["t"], "take");
    assert_eq!(
        take["transition"],
        serde_json::json!({"kind": "crossfade", "ms": 400})
    );
    assert_eq!(take["layers"][0]["id"], "lower-third");
    assert_eq!(take["layers"][0]["state"], "in");
    assert_eq!(take["hide"], serde_json::json!([]));
    // A Program change reaches both; on the preview bus it is the next message.
    hub.hide(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
    assert_eq!(ws_json(&mut prog)["state"], "out");
    assert_eq!(ws_json(&mut pv)["state"], "out");

    stop.send(()).unwrap();
    assert_eq!(ws_read(&mut prog).0, 8);
    assert_eq!(ws_read(&mut pv).0, 8);
    rt.block_on(async { tokio::time::timeout(STOP_TIMEOUT, task).await })
        .unwrap()
        .unwrap();
    // The session is gone: nothing is derived any more.
    let deadline = Instant::now() + Duration::from_secs(3);
    while hub.preview_buses() > 0 {
        assert!(Instant::now() < deadline, "preview bus not released");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Sends one masked text frame (client → server).
fn ws_send(s: &mut TcpStream, text: &str) {
    let p = text.as_bytes();
    assert!(p.len() < 126);
    let mask = [1u8, 2, 3, 4];
    let mut f = vec![0x81, 0x80 | p.len() as u8];
    f.extend_from_slice(&mask);
    f.extend(p.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    s.write_all(&f).unwrap();
}

fn wait_until(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn media_readiness_counts_program_renderers_only() {
    use crate::core::model::{LiveEdit, MediaEndAction};
    let hub = Arc::new(LiveHub::default());
    hub.apply_screen(
        crate::core::media::test_plan(MediaEndAction::Hold, Some(5000)),
        LiveEdit::OnTake,
    )
    .unwrap();
    let (port, stop, task, rt) = serve_on_test_runtime(hub.clone());
    let readiness = || {
        hub.layer_view("main", "clip")
            .unwrap()
            .media
            .unwrap()
            .readiness
    };
    let mut a = ws_connect(port, "main");
    let mut b = ws_connect(port, "main");
    // App previews and the Preview bus don't count, and their reports are ignored.
    let mut muted = ws_connect(port, "main&mute=1");
    let mut pv = ws_connect(port, "main&bus=preview");
    for s in [&mut a, &mut b, &mut muted, &mut pv] {
        ws_json(s); // hello
        let snap = ws_json(s);
        assert_eq!(snap["t"], "snapshot");
        assert_eq!(snap["layers"][0]["media"]["url"], "/assets/clip.mp4");
    }
    wait_until("two renderers", || readiness().total == 2);
    ws_send(&mut muted, r#"{"t":"ready","layerId":"clip","ok":true}"#);
    ws_send(&mut pv, r#"{"t":"ready","layerId":"clip","ok":true}"#);
    ws_send(&mut a, r#"{"t":"ready","layerId":"clip","ok":true}"#);
    wait_until("one ready", || readiness().ready == 1);
    ws_send(
        &mut b,
        r#"{"t":"ready","layerId":"clip","ok":false,"error":"Can't play this codec"}"#,
    );
    wait_until("an error", || readiness().error.is_some());
    let r = readiness();
    assert_eq!((r.ready, r.total), (1, 2));
    assert_eq!(r.error.as_deref(), Some("Can't play this codec"));
    // Stray text is ignored; a renderer that leaves no longer counts.
    ws_send(&mut a, "not json");
    drop(b);
    wait_until("one renderer", || readiness().total == 1);
    assert_eq!(readiness().error, None);
    let _ = stop.send(());
    rt.block_on(task).unwrap();
}

#[test]
fn an_ndi_capture_page_counts_for_readiness_but_not_as_a_program_renderer() {
    use crate::core::model::{LiveEdit, MediaEndAction};
    let hub = Arc::new(LiveHub::default());
    hub.apply_screen(
        crate::core::media::test_plan(MediaEndAction::Hold, Some(5000)),
        LiveEdit::OnTake,
    )
    .unwrap();
    let (port, stop, task, rt) = serve_on_test_runtime(hub.clone());
    let total = || {
        hub.layer_view("main", "clip")
            .unwrap()
            .media
            .unwrap()
            .readiness
            .total
    };
    let mut cap = ws_connect(port, "main&cap=1");
    ws_json(&mut cap); // hello
    assert_eq!(ws_json(&mut cap)["t"], "snapshot");
    wait_until("the capture page counted", || total() == 1);
    assert_eq!(
        hub.program_renderers(),
        0,
        "NDI is on air only with a receiver"
    );
    let mut obs = ws_connect(port, "main");
    ws_json(&mut obs);
    ws_json(&mut obs);
    wait_until("both counted", || total() == 2);
    assert_eq!(hub.program_renderers(), 1);
    let _ = stop.send(());
    rt.block_on(task).unwrap();
}

#[test]
fn assets_answer_range_requests_streamed() {
    let tmp = tempfile::tempdir().unwrap();
    let assets_dir = tmp.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    // 300 KB: several 64 KB chunks.
    let mut clip = b"\0\0\0\x18ftypisom".to_vec();
    clip.extend((0..300 * 1024).map(|i| (i % 251) as u8));
    std::fs::write(assets_dir.join("clip.mp4"), &clip).unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let listener = rt.block_on(TcpListener::bind(("127.0.0.1", 0))).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = oneshot::channel::<()>();
    let assets = AssetDir::default();
    assets.set_project(Some(tmp.path().to_path_buf()));
    let task = rt.spawn(async move {
        serve(listener, Arc::new(LiveHub::default()), assets, async move {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    let len = clip.len();
    let (status, head, body) = http_get_with(port, "/assets/clip.mp4", "");
    assert_eq!(status, 200);
    assert_eq!(body, clip);
    assert_eq!(header_of(&head, "accept-ranges").as_deref(), Some("bytes"));
    assert_eq!(
        header_of(&head, "content-type").as_deref(),
        Some("video/mp4")
    );
    assert_eq!(header_of(&head, "content-length"), Some(len.to_string()));
    let etag = header_of(&head, "etag").unwrap();

    let (status, head, body) =
        http_get_with(port, "/assets/clip.mp4", "Range: bytes=100000-200000\r\n");
    assert_eq!(status, 206);
    assert_eq!(body, &clip[100_000..=200_000]);
    assert_eq!(
        header_of(&head, "content-range"),
        Some(format!("bytes 100000-200000/{len}"))
    );
    let (status, _, body) = http_get_with(port, "/assets/clip.mp4", "Range: bytes=-10\r\n");
    assert_eq!(status, 206);
    assert_eq!(body, &clip[len - 10..]);
    let (status, _, body) = http_get_with(port, "/assets/clip.mp4", "Range: bytes=307000-\r\n");
    assert_eq!(status, 206);
    assert_eq!(body, &clip[307_000..]);
    let (status, head, body) = http_get_with(
        port,
        "/assets/clip.mp4",
        &format!("Range: bytes={len}-\r\n"),
    );
    assert_eq!(status, 416);
    assert!(body.is_empty());
    assert_eq!(
        header_of(&head, "content-range"),
        Some(format!("bytes */{len}"))
    );
    // If-Range: the range only while the file is unchanged.
    let (status, _, _) = http_get_with(
        port,
        "/assets/clip.mp4",
        &format!("Range: bytes=0-9\r\nIf-Range: {etag}\r\n"),
    );
    assert_eq!(status, 206);
    let (status, _, body) = http_get_with(
        port,
        "/assets/clip.mp4",
        "Range: bytes=0-9\r\nIf-Range: \"stale\"\r\n",
    );
    assert_eq!(status, 200);
    assert_eq!(body.len(), len);
    // Several ranges: the whole file.
    let (status, _, _) = http_get_with(port, "/assets/clip.mp4", "Range: bytes=0-1,5-6\r\n");
    assert_eq!(status, 200);
    let _ = tx.send(());
    rt.block_on(task).unwrap();
}

#[test]
fn web_images_download_only_links_in_use_and_keep_the_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = Arc::new(LiveHub::with_test_screen());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    // The "internet": one PNG, every request counted.
    let png: &'static [u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    let hits = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let web = rt.block_on(TcpListener::bind(("127.0.0.1", 0))).unwrap();
    let url = format!("http://{}/logo.png", web.local_addr().unwrap());
    let h = hits.clone();
    let site = axum::Router::new().route(
        "/logo.png",
        axum::routing::get(move || {
            h.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move { png }
        }),
    );
    rt.spawn(async move {
        let _ = axum::serve(web, site).await;
    });
    let count = || hits.load(std::sync::atomic::Ordering::SeqCst);

    let listener = rt.block_on(TcpListener::bind(("127.0.0.1", 0))).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = oneshot::channel::<()>();
    let assets = AssetDir::allowing_loopback_web_images();
    let served = assets.clone();
    let h2 = hub.clone();
    let task = rt.spawn(async move {
        serve(listener, h2, served, async move {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    let path = |u: &str| format!("/web-image?u={}", u.replace(':', "%3A").replace('/', "%2F"));

    // No project: nothing.
    assert_eq!(http_get_with(port, &path(&url), "").0, 404);
    assets.set_project(Some(tmp.path().to_path_buf()));
    // A link no layer holds is never fetched.
    assert_eq!(http_get_with(port, &path(&url), "").0, 404);
    assert_eq!(count(), 0);
    // Not web links at all.
    for bad in [
        "file:///C:/x.png",
        "javascript:alert(1)",
        "/assets/logo.png",
        "",
    ] {
        assert_eq!(http_get_with(port, &path(bad), "").0, 404, "{bad}");
    }

    // A layer shows it: downloaded once, served from the copy.
    let set = |v: &str| {
        let mut f = Fields::new();
        f.insert("name".into(), v.into());
        hub.set_fields(TEST_SCREEN_ID, TEST_LAYER_ID, f).unwrap();
    };
    set(&url);
    let (status, head, body) = http_get_with(port, &path(&url), "");
    assert_eq!(status, 200, "{head}");
    assert_eq!(body, png);
    assert_eq!(
        header_of(&head, "content-type").as_deref(),
        Some("image/png")
    );
    assert!(header_of(&head, "etag").is_some());
    assert_eq!(count(), 1);
    assert_eq!(http_get_with(port, &path(&url), "").0, 200);
    assert_eq!(count(), 1);
    // The copy stays usable when the layer no longer holds the link (the sweep removes it later).
    set("Ada");
    assert_eq!(http_get_with(port, &path(&url), "").0, 200);
    assert_eq!(count(), 1);

    let _ = tx.send(());
    rt.block_on(task).unwrap();
}
