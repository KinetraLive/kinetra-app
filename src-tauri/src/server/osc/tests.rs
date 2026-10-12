//! The OSC listener end to end over real loopback UDP: lifecycle (no task / socket when off,
//! rebinds, port in use), commands and levels into the live state, bundles in order, the
//! `trigger.osc` flow trigger, and on-air feedback. Latencies are printed (`--nocapture`).

use std::net::UdpSocket as StdUdp;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rosc::{OscBundle, OscPacket, OscTime, OscType};

use super::*;
use crate::core::events::NullSink;
use crate::core::model::{
    FlowEdge, FlowNode, LiveEdit, NewFlow, NewScreen, NodeKind, NodePos, OscSettings, PortRef,
    Rect, Retrigger, ScreenDoc, ScreenLayer, ScreenLayerKind, TemplateSource, WireKind,
};
use crate::core::protocol::Fields;

struct Env {
    _tmp: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    st: AppState,
    osc: OscManager,
    screen: String,
    _dir: PathBuf,
}

/// A project with one screen holding `l1` and `l2` (classic bar), both off air, and an OSC
/// manager on its own runtime (OSC still disabled).
fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        Arc::new(NullSink),
        Duration::from_secs(60),
    )
    .unwrap();
    st.set_content_flush_delay(Duration::from_secs(60));
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    let bar = st
        .templates()
        .create(
            &dir,
            "classic-bar",
            &TemplateSource::Preset {
                preset_id: "classic-bar".into(),
            },
        )
        .unwrap()
        .id;
    let mut doc: ScreenDoc = st
        .create_screen(&NewScreen {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap()
        .doc;
    for id in ["l1", "l2"] {
        doc.meta.layers.push(ScreenLayer {
            id: id.into(),
            name: id.into(),
            rect: Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            kind: ScreenLayerKind::Template {
                template_id: bar.clone(),
                fields: Fields::new(),
                vars: Fields::new(),
                bindings: Default::default(),
                variant: None,
                sizing: None,
                opacity: None,
            },
            anchor: None,
        });
    }
    let screen = st.save_screen(doc).unwrap().doc.id;
    let osc = OscManager::new(&st, rt.handle().clone());
    st.add_project_observer(Arc::new(osc.clone()));
    Env {
        _tmp: tmp,
        rt,
        st,
        osc,
        screen,
        _dir: dir,
    }
}

/// A UDP port free right now on loopback.
fn free_port() -> u16 {
    StdUdp::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn msg(addr: &str, args: Vec<OscType>) -> OscPacket {
    OscPacket::Message(rosc::OscMessage {
        addr: addr.into(),
        args,
    })
}

impl Env {
    fn set_osc(&self, osc: Option<OscSettings>) {
        let mut s = self.st.current_project().unwrap().settings;
        s.osc = osc;
        self.st.set_project_settings(s).unwrap();
    }

    /// Enables OSC on a free port and waits until it listens.
    fn enable(&self, feedback: Option<OscFeedback>) -> u16 {
        let port = free_port();
        self.set_osc(Some(OscSettings {
            enabled: true,
            port,
            allow: vec![],
            feedback,
        }));
        wait_until(
            "listening",
            || matches!(self.osc.status(), OscStatus::Listening { port: p, .. } if p == port),
        );
        port
    }

    fn on_air(&self, l: &str) -> bool {
        self.st.live().is_on_air(&self.screen, l)
    }

    fn opacity(&self, l: &str) -> u8 {
        self.st
            .live()
            .with_layer(&self.screen, l, |x| x.opacity)
            .unwrap()
    }
}

fn send(port: u16, packet: &OscPacket) {
    let s = StdUdp::bind("127.0.0.1:0").unwrap();
    s.send_to(&rosc::encoder::encode(packet).unwrap(), ("127.0.0.1", port))
        .unwrap();
}

#[test]
fn no_task_or_socket_until_enabled_and_rebinds_on_change() {
    let e = env();
    assert_eq!(e.osc.status(), OscStatus::Off);
    assert!(!e.osc.is_running());
    // Present but disabled: still nothing.
    e.set_osc(Some(OscSettings::default()));
    assert!(!e.osc.is_running());
    assert_eq!(e.osc.status(), OscStatus::Off);

    let port = e.enable(None);
    assert!(e.osc.is_running());
    assert!(StdUdp::bind(("127.0.0.1", port)).is_err(), "port is held");

    // A new port: the old one is released, the new one bound.
    let port2 = free_port();
    e.set_osc(Some(OscSettings {
        enabled: true,
        port: port2,
        allow: vec![],
        feedback: None,
    }));
    wait_until(
        "rebound",
        || matches!(e.osc.status(), OscStatus::Listening { port: p, .. } if p == port2),
    );
    wait_until("old port released", || {
        StdUdp::bind(("127.0.0.1", port)).is_ok()
    });

    // Allowlist changes don't rebind.
    let mut s = e.st.current_project().unwrap().settings;
    s.osc.as_mut().unwrap().allow = vec!["192.168.1.20".into()];
    e.st.set_project_settings(s).unwrap();
    assert!(matches!(e.osc.status(), OscStatus::Listening { port: p, .. } if p == port2));

    // Disabled: the task ends and the port is free.
    e.set_osc(Some(OscSettings {
        enabled: false,
        port: port2,
        allow: vec![],
        feedback: None,
    }));
    assert_eq!(e.osc.status(), OscStatus::Off);
    wait_until("task ended", || !e.osc.is_running());
    wait_until("port released", || {
        StdUdp::bind(("127.0.0.1", port2)).is_ok()
    });

    // Enabled again, then the project closes: off.
    e.enable(None);
    e.st.close_project();
    assert_eq!(e.osc.status(), OscStatus::Off);
    wait_until("task ended on close", || !e.osc.is_running());
}

#[test]
fn a_port_in_use_is_reported() {
    let e = env();
    let blocker = StdUdp::bind("127.0.0.1:0").unwrap();
    let port = blocker.local_addr().unwrap().port();
    e.set_osc(Some(OscSettings {
        enabled: true,
        port,
        allow: vec![],
        feedback: None,
    }));
    wait_until("error status", || {
        matches!(
            e.osc.status(),
            OscStatus::Error {
                code: ServerErrorCode::PortInUse,
                ..
            }
        )
    });
    wait_until("task ended", || !e.osc.is_running());
    // Freed and touched again (any settings change retries).
    drop(blocker);
    let s = e.st.current_project().unwrap().settings;
    e.st.set_project_settings(s).unwrap();
    wait_until("listening after retry", || {
        matches!(e.osc.status(), OscStatus::Listening { .. })
    });
}

#[test]
fn take_opacity_clear_all_and_bundles_over_udp() {
    let e = env();
    let port = e.enable(None);
    let mut rx = e.st.live().subscribe();
    let base = format!("/kinetra/layer/{}/l1", e.screen);

    // Take: the renderer's `state in` arrives (latency measured to the hub message).
    let t0 = Instant::now();
    send(port, &msg(&format!("{base}/take"), vec![OscType::Int(1)]));
    let latency = e.rt.block_on(async {
        loop {
            let m = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("a hub message")
                .unwrap();
            if m.json.contains("\"state\"") && m.json.contains("\"in\"") {
                return t0.elapsed();
            }
        }
    });
    println!("OSC take → renderer message: {latency:?}");
    assert!(e.on_air("l1"));
    assert!(latency < Duration::from_millis(50), "{latency:?}");

    // A release (0) does nothing.
    send(port, &msg(&format!("{base}/clear"), vec![OscType::Int(0)]));
    std::thread::sleep(Duration::from_millis(50));
    assert!(e.on_air("l1"));

    // Opacity 0.5 → 50 % (Live edit: immediate, so it goes on air at once).
    let mut ui = e.st.ui_state().unwrap();
    ui.control.live_edit = LiveEdit::Immediate;
    e.st.set_ui_state(ui).unwrap();
    send(
        port,
        &msg(&format!("{base}/opacity"), vec![OscType::Float(0.5)]),
    );
    wait_until("opacity", || e.opacity("l1") == 50);

    // A bundle runs in order: take l2, then clear it.
    let b2 = format!("/kinetra/layer/{}/l2", e.screen);
    send(
        port,
        &OscPacket::Bundle(OscBundle {
            timetag: OscTime::from((0, 1)),
            content: vec![
                msg(&format!("{b2}/take"), vec![]),
                msg("/kinetra/clear-all", vec![]),
            ],
        }),
    );
    wait_until("clear all", || !e.on_air("l1"));
    assert!(!e.on_air("l2"));
    // Unknown addresses and garbage are ignored; the listener keeps running.
    send(port, &msg("/kinetra/layer/nope/l1/take", vec![]));
    StdUdp::bind("127.0.0.1:0")
        .unwrap()
        .send_to(b"not osc", ("127.0.0.1", port))
        .unwrap();
    send(port, &msg(&format!("{base}/toggle"), vec![]));
    wait_until("toggle on", || e.on_air("l1"));
    assert!(e.osc.is_running());
    let (accepted, dropped) = e.osc.packet_counts();
    assert!(accepted >= 6 && dropped == 0, "{accepted} {dropped}");
}

#[test]
fn the_accept_filter_follows_the_settings() {
    let e = env();
    e.enable(None);
    let lan_ip: IpAddr = "192.168.1.20".parse().unwrap();
    assert!(e.osc.inner.accepts("127.0.0.1".parse().unwrap()));
    assert!(!e.osc.inner.accepts(lan_ip));
    let mut s = e.st.current_project().unwrap().settings;
    s.osc.as_mut().unwrap().allow = vec!["192.168.1.20".into()];
    e.st.set_project_settings(s.clone()).unwrap();
    assert!(
        !e.osc.inner.accepts(lan_ip),
        "allowlisted but LAN access off"
    );
    s.lan_access = true;
    e.st.set_project_settings(s).unwrap();
    assert!(e.osc.inner.accepts(lan_ip));
    assert!(e.osc.inner.accepts("::ffff:192.168.1.20".parse().unwrap()));
    assert!(!e.osc.inner.accepts("192.168.1.21".parse().unwrap()));
}

#[test]
fn an_osc_trigger_fires_its_flow_with_the_first_argument() {
    let e = env();
    let port = e.enable(None);
    let n = |id: &str, kind| FlowNode {
        id: id.into(),
        position: NodePos::default(),
        kind,
    };
    let edge = |id: &str, from: (&str, &str), to: (&str, &str), kind| FlowEdge {
        id: id.into(),
        from: PortRef {
            node: from.0.into(),
            port: from.1.into(),
        },
        to: PortRef {
            node: to.0.into(),
            port: to.1.into(),
        },
        kind,
    };
    e.st.create_flow(NewFlow {
        name: "Name".into(),
        armed: true,
        retrigger: Retrigger::Restart,
        origin: None,
        nodes: vec![
            n(
                "o",
                NodeKind::Osc {
                    address: "/stage/*/name".into(),
                },
            ),
            n(
                "f",
                NodeKind::SetFields {
                    screen_id: e.screen.clone(),
                    layer_id: "l2".into(),
                    fields: [("name".to_string(), String::new())].into(),
                },
            ),
        ],
        edges: vec![
            edge("e1", ("o", "fired"), ("f", "in"), WireKind::Event),
            edge("e2", ("o", "value"), ("f", "name"), WireKind::Data),
        ],
    })
    .unwrap();
    let name = || {
        e.st.live()
            .layer_view(&e.screen, "l2")
            .unwrap()
            .fields
            .get("name")
            .cloned()
            .unwrap_or_default()
    };
    // Not matching: one segment too many.
    send(
        port,
        &msg("/stage/a/b/name", vec![OscType::String("No".into())]),
    );
    send(
        port,
        &msg("/stage/one/name", vec![OscType::String("Ada".into())]),
    );
    wait_until("the flow's field", || name() == "Ada");
    send(port, &msg("/stage/two/name", vec![OscType::Float(0.5)]));
    wait_until("a number as text", || name() == "0.5");
}

#[test]
fn feedback_sends_the_full_state_then_changes() {
    let e = env();
    let rx = StdUdp::bind("127.0.0.1:0").unwrap();
    rx.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let fb_port = rx.local_addr().unwrap().port();
    let port = e.enable(Some(OscFeedback {
        host: "127.0.0.1".into(),
        port: fb_port,
    }));
    let mut buf = [0u8; 1024];
    let mut recv = || -> (String, i32) {
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        match rosc::decoder::decode_udp(&buf[..n]).unwrap().1 {
            OscPacket::Message(m) => {
                let v = match m.args.as_slice() {
                    [OscType::Int(v)] => *v,
                    a => panic!("args {a:?}"),
                };
                (m.addr, v)
            }
            p => panic!("{p:?}"),
        }
    };
    let s = e.screen.clone();
    // The full state once.
    let mut full = vec![recv(), recv()];
    full.sort();
    assert_eq!(
        full,
        vec![
            (format!("/kinetra/layer/{s}/l1/onair"), 0),
            (format!("/kinetra/layer/{s}/l2/onair"), 0)
        ]
    );
    // A take from Control: one message, within 10 ms of the change.
    let t0 = Instant::now();
    e.st.live().show(&s, "l1").unwrap();
    // `show` on the hub runs the hook synchronously.
    assert_eq!(recv(), (format!("/kinetra/layer/{s}/l1/onair"), 1));
    let latency = t0.elapsed();
    println!("on-air change → OSC feedback received: {latency:?}");
    assert!(latency < Duration::from_millis(10), "{latency:?}");
    // From OSC itself.
    send(port, &msg(&format!("/kinetra/layer/{s}/l1/clear"), vec![]));
    assert_eq!(recv(), (format!("/kinetra/layer/{s}/l1/onair"), 0));
    // Nothing more (no repeats).
    rx.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    assert!(rx.recv_from(&mut buf).is_err());
    // Feedback off: nothing is sent.
    let mut st = e.st.current_project().unwrap().settings;
    st.osc.as_mut().unwrap().feedback = None;
    e.st.set_project_settings(st).unwrap();
    e.st.live().show(&s, "l2").unwrap();
    assert!(rx.recv_from(&mut buf).is_err());
}

#[test]
fn decode_flattens_bundles_in_order() {
    let p = OscPacket::Bundle(OscBundle {
        timetag: OscTime::from((0, 1)),
        content: vec![
            msg("/a", vec![OscType::Int(1), OscType::Long(2)]),
            OscPacket::Bundle(OscBundle {
                timetag: OscTime::from((0, 1)),
                content: vec![msg(
                    "/b",
                    vec![
                        OscType::Float(0.5),
                        OscType::Double(0.25),
                        OscType::String("x".into()),
                        OscType::Bool(true),
                        OscType::Nil,
                    ],
                )],
            }),
            msg("/c", vec![]),
        ],
    });
    let m = decode(&rosc::encoder::encode(&p).unwrap());
    let addrs: Vec<&str> = m.iter().map(|m| m.address.as_str()).collect();
    assert_eq!(addrs, ["/a", "/b", "/c"]);
    assert_eq!(m[0].args, [OscArg::Int(1), OscArg::Int(2)]);
    assert_eq!(
        m[1].args,
        [
            OscArg::Float(0.5),
            OscArg::Double(0.25),
            OscArg::Text("x".into()),
            OscArg::Bool(true),
            OscArg::Other
        ]
    );
    assert!(decode(b"garbage").is_empty());
}
