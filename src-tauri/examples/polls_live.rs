//! Live polls against the real Kinetra API (run by hand by a maintainer):
//! device-flow sign-in → create a poll → open it → a project screen with the Poll graphic (Frame)
//! bound to it, on air on a real output server → live results → close → delete → sign out.
//!
//! `cargo run --example polls_live -- <control file>`
//!
//! Prints the code to approve in the browser, then `LINK <voting url>` and
//! `SCREEN <renderer url>` (open it in a browser to watch the graphic), and every `poll-live`
//! event the app's Polls page would get. Steps after opening are driven by writing a word into the
//! control file: `close`, then `delete` (which also signs out and quits). Secrets stay in memory
//! (never the keychain the app uses); the project and config are temporary folders.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kinetra_lib::core::cloud::service::SigninView;
use kinetra_lib::core::cloud::{config, machine};
use kinetra_lib::core::events::EventSink;
use kinetra_lib::core::model::{NewScreen, PollInput};
use kinetra_lib::core::secrets::MemorySecrets;
use kinetra_lib::core::state::AppState;
use kinetra_lib::server::{serve_with, AssetDir};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Prints the events a person would see change in the app: live results and connection status.
struct Print;

impl EventSink for Print {
    fn emit(&self, event: &str, payload: Value) {
        if event == "poll-live" || event == "poll-status" {
            let mut text = payload.to_string();
            text.truncate(600);
            println!("EVENT {event} {text}");
        }
    }
}

fn step(word: &str, file: &std::path::Path) {
    println!("WAITING for `{word}` in {}", file.display());
    loop {
        if std::fs::read_to_string(file).is_ok_and(|s| s.trim() == word) {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn main() {
    let control = std::path::PathBuf::from(std::env::args().nth(1).expect("control file path"));
    let _ = std::fs::remove_file(&control);
    let tmp = std::env::temp_dir().join(format!("kinetra-polls-live-{}", std::process::id()));
    std::fs::create_dir_all(tmp.join("project")).expect("tmp");
    let st = AppState::load(tmp.join("config"), Arc::new(Print)).expect("state");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    st.live().set_runtime(rt.handle().clone());
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    let machine = machine::fingerprint().expect("MachineGuid");
    st.cloud_service().set_test_env(
        config::API_BASE,
        Arc::new(MemorySecrets::default()),
        &machine,
    );
    st.cloud_init();

    // 1. Sign in (device flow): the owner approves in the browser.
    let v = rt.block_on(st.sign_in_start()).expect("device flow start");
    if let SigninView::Waiting { user_code, url, .. } = &v {
        println!("APPROVE: open {url} and check the code {user_code}");
    }
    let t = Instant::now();
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let v = st.license_view();
        if v.signed_in && v.status != "none" {
            println!(
                "signed in: plan={:?} features.polls={}",
                v.plan, v.features.polls
            );
            break;
        }
        match v.signin {
            SigninView::Waiting { .. } | SigninView::Idle => {}
            other => {
                println!("sign-in ended: {other:?}");
                return;
            }
        }
        if t.elapsed() > Duration::from_secs(660) {
            println!("timed out waiting for approval");
            return;
        }
    }
    match rt.block_on(st.polls_quota()) {
        Ok(q) => println!("quota {}", serde_json::to_string(&q).unwrap_or_default()),
        Err(e) => println!("quota error {e}"),
    }

    // 2. Create and open a poll.
    let poll = rt
        .block_on(st.create_poll(PollInput {
            question: Some("Kinetra live test: which color?".into()),
            options: Some(vec!["Red".into(), "Green".into(), "Blue".into()]),
            duration_seconds: None,
            clear_duration: false,
            show_results: Some(true),
        }))
        .expect("create poll");
    println!("created {} code {}", poll.id, poll.code);
    let poll = rt.block_on(st.open_poll(&poll.id)).expect("open poll");
    println!("opened: status {:?}", poll.status);
    println!("LINK {}", poll.url);

    // 3. A project with the Poll graphic bound to it, on air, served like the app serves it.
    st.create_project(&tmp.join("project"), "Polls live")
        .expect("project");
    let screen = st
        .create_screen(&NewScreen {
            name: "Polls".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .expect("screen")
        .doc
        .id;
    let source = st
        .create_poll_source("Live test poll", Some(poll.clone()), None)
        .expect("source")
        .id;
    let state = st
        .add_poll_to_screen(&source, &screen)
        .expect("add to screen");
    let layer = state.doc.meta.layers.last().expect("layer").id.clone();
    let hub = st.live().clone();
    hub.show(&screen, &layer).expect("on air");
    // The Polls page watches the poll while it is open, like the app does.
    st.watch_poll(&poll.id, true).expect("watch");
    let assets = AssetDir::default();
    assets.set_project(st.project_dir().ok());
    let listener = rt
        .block_on(TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))))
        .expect("bind");
    let port = listener.local_addr().expect("port").port();
    let (stop, rx) = oneshot::channel::<()>();
    let server = rt.spawn(serve_with(listener, hub, assets, None, async move {
        let _ = rx.await;
    }));
    println!("SCREEN http://127.0.0.1:{port}/screen/{screen}");

    // 4. Live results until the owner says close.
    step("close", &control);
    let closed = rt.block_on(st.close_poll(&poll.id)).expect("close poll");
    println!(
        "closed: status {:?}, total {}, votes {:?}",
        closed.status,
        closed.total_votes,
        closed
            .options
            .iter()
            .map(|o| (&o.label, o.votes))
            .collect::<Vec<_>>()
    );

    // 5. Delete, sign out, quit.
    step("delete", &control);
    st.watch_poll(&poll.id, false).ok();
    rt.block_on(st.delete_poll(&poll.id)).expect("delete poll");
    println!("deleted {}", poll.id);
    match rt.block_on(st.get_poll(&poll.id)) {
        Ok(p) => println!("after delete the server still returns it: {:?}", p.status),
        Err(e) => println!("after delete: {e}"),
    }
    let _ = stop.send(());
    let _ = rt.block_on(server);
    rt.block_on(st.sign_out()).expect("sign out");
    println!("signed out; done");
    let _ = std::fs::remove_dir_all(&tmp);
}
