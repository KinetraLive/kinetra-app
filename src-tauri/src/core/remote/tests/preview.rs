//! Live preview (protocol 2 §6) against the mock relay and a fake capture host: `hello.preview`,
//! demands → captures, `preview_state`, `preview_caps`, the preview socket with credit, the
//! on-air rate and stopping.

use std::collections::{HashMap, HashSet};

use super::*;
use crate::core::model::ServerStatus;
use crate::core::remote::preview::{
    PreviewFrame, PreviewHost, PreviewPlan, PreviewSink, HEADER_BYTES,
};

/// Records every plan; "captures" by putting one picture per stream at once.
#[derive(Default)]
struct FakeHost {
    plans: Mutex<Vec<PreviewPlan>>,
    sink: Mutex<Option<Arc<PreviewSink>>>,
    fail: Mutex<HashSet<u16>>,
}

impl PreviewHost for FakeHost {
    fn apply(&self, plan: &PreviewPlan, sink: &Arc<PreviewSink>) -> HashMap<u16, String> {
        self.plans.lock().unwrap().push(plan.clone());
        *self.sink.lock().unwrap() = Some(sink.clone());
        let fail = self.fail.lock().unwrap().clone();
        let mut failed = HashMap::new();
        for c in &plan.captures {
            if fail.contains(&c.stream) {
                failed.insert(c.stream, "no capture".into());
            } else {
                sink.put(c.stream, picture(c.stream as u8));
            }
        }
        failed
    }
}

fn picture(n: u8) -> PreviewFrame {
    PreviewFrame {
        jpeg: vec![0xff, 0xd8, n, 0xff, 0xd9],
        width: 640,
        height: 360,
        captured_ms: 1_700_000_000_000,
    }
}

impl FakeHost {
    fn last(&self) -> Option<PreviewPlan> {
        self.plans.lock().unwrap().last().cloned()
    }
}

/// A protocol 2 mock with `preview`, a capture host and the output server "running".
fn setup(credit: u64) -> (Env, Arc<FakeHost>) {
    let e = env();
    e.cfg(|c| {
        c.protocols = &["kinetra-control.v2", "kinetra-control.v1"];
        c.features = &["permissions", "preview"];
    });
    *e.sh.preview_credit.lock().unwrap() = credit;
    let host = Arc::new(FakeHost::default());
    e.st.set_preview_host(Some(host.clone()));
    e.st.outputs_set_server(&ServerStatus::Running {
        port: 7770,
        url: "http://127.0.0.1:7770".into(),
        lan_url: None,
    });
    (e, host)
}

impl Env {
    fn preview_url(&self) -> String {
        format!(
            "{}/v1/control/app/preview",
            self.base.replacen("http://", "ws://", 1)
        )
    }

    fn demand(&self, streams: Value, ticket: &str) {
        self.push(json!({
            "t": "preview_demand", "streams": streams, "fps": 5, "max_width": 640,
            "quality": 70, "url": self.preview_url(), "ticket": ticket
        }));
    }

    fn pv(&self) -> Vec<PvEv> {
        self.sh.preview.lock().unwrap().clone()
    }

    fn frames(&self) -> Vec<Vec<u8>> {
        self.pv()
            .into_iter()
            .filter_map(|e| match e {
                PvEv::Frame(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    fn credit(&self, n: u64) {
        let tx = self
            .sh
            .to_preview
            .lock()
            .unwrap()
            .clone()
            .expect("preview socket");
        tx.send(ToApp::Text(json!({"t": "credit", "n": n}).to_string()))
            .unwrap();
    }

    fn states(&self) -> Vec<Value> {
        self.of("preview_state")
    }
}

#[test]
fn hello_offers_the_cap_only_with_a_capture_host_and_protocol_2() {
    let (e, _) = setup(0);
    e.live();
    assert_eq!(e.of("hello")[0]["preview"], json!({"max_streams": 2}));

    // A protocol 1 server: no preview at all.
    let (e, _) = setup(0);
    e.cfg(|c| c.protocols = &["kinetra-control.v1"]);
    e.live();
    assert!(e.of("hello")[0].get("preview").is_none());
}

#[test]
fn a_demand_is_captured_and_pictures_go_out_only_with_credit() {
    let (e, host) = setup(1);
    e.live();
    e.demand(
        json!([{"stream": 1, "screen": e.screen, "kind": "program"}]),
        "tk-1",
    );
    wait_until("capture", || {
        host.last().is_some_and(|p| p.captures.len() == 1)
    });
    let plan = host.last().unwrap();
    let c = &plan.captures[0];
    assert_eq!(
        c.url,
        format!(
            "http://127.0.0.1:7770/screen/{}?output=preview&bg=checker&mute=1&audio=0",
            e.screen
        )
    );
    assert_eq!((c.width, c.height, plan.quality), (640, 360, 70));
    assert_eq!(plan.max_bytes, 256 * 1024);

    // The socket: Bearer session, the preview subprotocol, the ticket in the hello.
    wait_until("preview socket", || !e.frames().is_empty());
    let pv = e.pv();
    match &pv[0] {
        PvEv::Connected { auth, proto } => {
            assert_eq!(auth, &format!("Bearer {SESSION}"));
            assert_eq!(proto, "kinetra-control-preview.v1");
        }
        other => panic!("{other:?}"),
    }
    let hello = pv
        .iter()
        .find_map(|e| match e {
            PvEv::Text(v) if v["t"] == "preview_hello" => Some(v.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(hello["ticket"], "tk-1");
    assert_eq!(hello["run_id"], "run-1");
    let f = &e.frames()[0];
    assert_eq!(&f[..4], &[1, 1, 0, 1]);
    assert_eq!(u32::from_be_bytes(f[4..8].try_into().unwrap()), 1);
    assert_eq!(&f[16..20], &[0x02, 0x80, 0x01, 0x68]);
    assert_eq!(&f[HEADER_BYTES..], &picture(1).jpeg[..]);

    // No credit left: newer pictures wait (and replace each other), nothing is sent.
    let sink = host.sink.lock().unwrap().clone().unwrap();
    sink.put(1, picture(7));
    sink.put(1, picture(8));
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(e.frames().len(), 1);
    e.credit(5);
    wait_until("second picture", || e.frames().len() == 2);
    let f = &e.frames()[1];
    assert_eq!(u32::from_be_bytes(f[4..8].try_into().unwrap()), 2);
    assert_eq!(f[HEADER_BYTES + 2], 8);
    // All active from the start: no preview_state needed.
    assert!(e.states().is_empty());
}

#[test]
fn inactive_streams_are_reported_with_their_reason() {
    let (e, host) = setup(1);
    e.live();
    e.demand(
        json!([
            {"stream": 1, "screen": e.screen, "kind": "preview"},
            {"stream": 2, "screen": "nope", "kind": "program"},
            {"stream": 3, "screen": e.screen, "kind": "program"}
        ]),
        "tk-1",
    );
    wait_until("preview_state", || !e.states().is_empty());
    assert_eq!(
        e.states()[0]["streams"],
        json!([
            {"stream": 1, "active": false, "reason": "unsupported",
             "message": "Studio mode is off on the show machine."},
            {"stream": 2, "active": false, "reason": "unsupported",
             "message": "This screen isn't on the show machine."},
            {"stream": 3, "active": true}
        ])
    );
    wait_until("capture", || {
        host.last().is_some_and(|p| p.captures.len() == 1)
    });

    // "Allow remote preview" off: every stream disabled, nothing captured, the socket closes.
    let mut s = e.st.current_project().unwrap().settings;
    s.remote_preview = false;
    e.st.set_project_settings(s.clone()).unwrap();
    wait_until("disabled", || {
        e.states()
            .last()
            .is_some_and(|m| m["streams"][2]["reason"] == "disabled")
    });
    wait_until("captures stopped", || {
        host.last().is_some_and(|p| p.captures.is_empty())
    });
    wait_until("socket closed", || {
        matches!(e.pv().last(), Some(PvEv::Gone))
    });
    s.remote_preview = true;
    e.st.set_project_settings(s).unwrap();
    wait_until("captured again", || {
        host.last().is_some_and(|p| p.captures.len() == 1)
    });
}

#[test]
fn the_cap_limits_streams_and_changes_are_announced() {
    let (e, host) = setup(4);
    e.live();
    let s = |n: u16| json!({"stream": n, "screen": e.screen, "kind": "program"});
    e.demand(json!([s(1), s(2), s(3)]), "tk-1");
    wait_until("limit", || !e.states().is_empty());
    assert_eq!(e.states()[0]["streams"][2]["reason"], "limit");
    wait_until("two captures", || {
        host.last().is_some_and(|p| p.captures.len() == 2)
    });

    let mut p = e.st.prefs();
    p.remote.preview_streams = 3;
    e.st.set_prefs(p);
    wait_until("preview_caps", || !e.of("preview_caps").is_empty());
    assert_eq!(e.of("preview_caps")[0]["max_streams"], 3);
    wait_until("three captures", || {
        host.last().is_some_and(|p| p.captures.len() == 3)
    });
    // Out of range values are clamped (1–4).
    let mut p = e.st.prefs();
    p.remote.preview_streams = 9;
    assert_eq!(e.st.set_prefs(p).remote.preview_streams, 4);
}

#[test]
fn a_failed_capture_is_unsupported_until_the_next_demand() {
    let (e, host) = setup(1);
    host.fail.lock().unwrap().insert(1);
    e.live();
    e.demand(
        json!([{"stream": 1, "screen": e.screen, "kind": "program"}]),
        "tk-1",
    );
    wait_until("unsupported", || !e.states().is_empty());
    assert_eq!(
        e.states()[0]["streams"][0]["message"],
        "This output can't be captured."
    );
    wait_until("not retried", || {
        host.last().is_some_and(|p| p.captures.is_empty())
    });
    let applies = host.plans.lock().unwrap().len();
    std::thread::sleep(Duration::from_millis(1300));
    assert_eq!(
        host.plans.lock().unwrap().len(),
        applies,
        "no retry every tick"
    );
}

#[test]
fn on_air_lowers_the_rate_to_one_picture_per_second() {
    let (e, host) = setup(1);
    e.live();
    e.demand(
        json!([{"stream": 1, "screen": e.screen, "kind": "program"}]),
        "tk-1",
    );
    wait_until("sink", || host.sink.lock().unwrap().is_some());
    let sink = host.sink.lock().unwrap().clone().unwrap();
    assert_eq!(sink.interval(), Duration::from_millis(200));
    let r = e.command_p(
        "t1",
        "take",
        json!({"layer": "l1"}),
        &["take_clear"],
        e.seq(),
    );
    assert_eq!(r["ok"], true, "{r}");
    wait_until("1 per second", || sink.interval() == Duration::from_secs(1));
}

#[test]
fn an_empty_demand_and_switching_off_stop_everything() {
    let (e, host) = setup(1);
    e.live();
    e.demand(
        json!([{"stream": 1, "screen": e.screen, "kind": "program"}]),
        "tk-1",
    );
    wait_until("socket", || !e.frames().is_empty());
    e.demand(json!([]), "");
    wait_until("stopped", || {
        host.last().is_some_and(|p| p.captures.is_empty())
    });
    wait_until("socket closed", || {
        matches!(e.pv().last(), Some(PvEv::Gone))
    });

    // Remote control off while capturing: the host stops too.
    e.demand(
        json!([{"stream": 1, "screen": e.screen, "kind": "program"}]),
        "tk-2",
    );
    wait_until("capturing", || {
        host.last().is_some_and(|p| p.captures.len() == 1)
    });
    e.st.remote_set(false).unwrap();
    wait_until("stopped after off", || {
        host.last().is_some_and(|p| p.captures.is_empty())
    });
}

#[test]
fn a_lost_preview_socket_reopens_only_with_the_next_ticket() {
    let (e, host) = setup(4);
    e.live();
    let streams = json!([{"stream": 1, "screen": e.screen, "kind": "program"}]);
    e.demand(streams.clone(), "tk-1");
    wait_until("first picture", || e.frames().len() == 1);
    let connects = |e: &Env| {
        e.pv()
            .iter()
            .filter(|p| matches!(p, PvEv::Connected { .. }))
            .count()
    };

    // The server drops the socket (§6.2): tickets are single use, so the app waits for the
    // server's next demand instead of reconnecting by itself; capturing goes on.
    let tx = e.sh.to_preview.lock().unwrap().clone().unwrap();
    tx.send(ToApp::Close(4000, "no_credit".into())).unwrap();
    wait_until("socket gone", || matches!(e.pv().last(), Some(PvEv::Gone)));
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(connects(&e), 1);
    assert!(host.last().is_some_and(|p| p.captures.len() == 1));

    // The same streams with a fresh ticket: one new socket, that ticket, pictures again.
    e.demand(streams, "tk-2");
    wait_until("reopened", || connects(&e) == 2);
    wait_until("second hello", || {
        e.pv()
            .iter()
            .any(|p| matches!(p, PvEv::Text(v) if v["ticket"] == "tk-2"))
    });
    let sink = host.sink.lock().unwrap().clone().unwrap();
    sink.put(1, picture(9));
    wait_until("picture on the new socket", || e.frames().len() == 2);
    assert_eq!(e.frames()[1][HEADER_BYTES + 2], 9);
}

#[test]
fn the_preview_socket_never_goes_to_another_host() {
    let (e, host) = setup(1);
    e.live();
    e.push(json!({
        "t": "preview_demand", "fps": 5, "max_width": 640, "quality": 70,
        "streams": [{"stream": 1, "screen": e.screen, "kind": "program"}],
        "url": "ws://evil.example:9/v1/control/app/preview", "ticket": "tk-1"
    }));
    wait_until("capture", || {
        host.last().is_some_and(|p| p.captures.len() == 1)
    });
    std::thread::sleep(Duration::from_millis(200));
    assert!(e.pv().is_empty());
}

#[test]
fn without_a_capture_host_every_stream_is_unsupported() {
    let e = env();
    e.cfg(|c| {
        c.protocols = &["kinetra-control.v2", "kinetra-control.v1"];
        c.features = &["permissions", "preview"];
    });
    e.live();
    assert!(e.of("hello")[0].get("preview").is_none());
    e.push(json!({
        "t": "preview_demand", "fps": 5, "max_width": 640, "quality": 70, "url": "",
        "streams": [{"stream": 1, "screen": e.screen, "kind": "program"}], "ticket": "x"
    }));
    wait_until("unsupported", || !e.of("preview_state").is_empty());
    assert_eq!(
        e.of("preview_state")[0]["streams"][0]["message"],
        "The live picture isn't available on this Kinetra computer."
    );
}
