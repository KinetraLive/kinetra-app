use super::*;
use crate::core::control_api::{close, token, AttachError, Attached, Out, Refused, CLIENTS_MAX};
use crate::core::events::names;
use crate::core::secrets::{MemorySecrets, SecretStore};

impl Env {
    pub(super) fn enable(&self) -> String {
        self.st.control_api_set_enabled(true).unwrap();
        self.st.control_api_token().unwrap()
    }

    fn attach(&self, name: &str) -> Attached {
        let _g = self.rt.enter();
        self.st.control_api_attach(name, "127.0.0.1").unwrap()
    }

    /// The next message within 3 s.
    fn next(&self, a: &mut Attached) -> Out {
        self.rt
            .block_on(async { tokio::time::timeout(Duration::from_secs(3), a.next()).await })
            .expect("a message in time")
            .expect("the feed is running")
    }

    pub(super) fn cmd(&self, action: &str, args: Value) -> Result<(), Refused> {
        self.st.control_api_command(None, None, action, &args)
    }

    fn code(&self, action: &str, args: Value) -> &'static str {
        self.cmd(action, args).unwrap_err().code
    }

    fn events(&self, name: &str) -> usize {
        self.rec
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .count()
    }
}

fn parse(out: &Out) -> Value {
    serde_json::from_str(out.as_str()).unwrap()
}

/// Applies a diff's ops to `tree` (put / del by JSON Pointer).
fn apply(tree: &mut Value, diff: &Value) {
    for op in diff["ops"].as_array().unwrap() {
        let path = op["path"].as_str().unwrap();
        let (parent, key) = path.rsplit_once('/').unwrap();
        let key = key.replace("~1", "/").replace("~0", "~");
        let obj = tree.pointer_mut(parent).unwrap().as_object_mut().unwrap();
        if op["op"] == "put" {
            obj.insert(key, op["value"].clone());
        } else {
            obj.remove(&key);
        }
    }
}

fn studio_on(st: &AppState) {
    st.set_studio(
        StudioSettings {
            enabled: true,
            preview_after_take: PreviewAfterTake::Keep,
            transition: TakeTransition {
                kind: TakeTransitionKind::Crossfade,
                ms: 300,
            },
        },
        None,
    )
    .unwrap();
}

#[test]
fn off_by_default_and_nothing_runs() {
    let e = env();
    assert!(!e.st.control_api_enabled());
    {
        let _g = e.rt.enter();
        assert!(matches!(
            e.st.control_api_attach("x", "127.0.0.1"),
            Err(AttachError::Disabled)
        ));
    }
    assert_eq!(e.code("clear_all", json!({})), "disabled");
    assert_eq!(e.st.control_api_tasks(), 0);
    // No token until the API is switched on, and nothing matches without one.
    assert!(!e.st.control_api_check_token("kt_anything"));
    // Waking costs one atomic load without a client.
    let t0 = Instant::now();
    for _ in 0..10_000 {
        e.st.control_api_touch();
    }
    assert!(t0.elapsed() / 10_000 < Duration::from_micros(5));
}

#[test]
fn the_token_lives_in_the_keychain_only() {
    let e = env();
    let keychain = Arc::new(MemorySecrets::default());
    e.st.set_control_api_secrets(keychain.clone());
    let t = e.enable();
    assert!(token::is_well_formed(&t));
    assert_eq!(
        keychain.get(token::SECRET_ID).unwrap().as_deref(),
        Some(t.as_str())
    );
    // Stable across reads; never in prefs.
    assert_eq!(e.st.control_api_token().unwrap(), t);
    let prefs = serde_json::to_string(&e.st.prefs()).unwrap();
    assert!(
        prefs.contains("\"controlApi\":{\"enabled\":true}"),
        "{prefs}"
    );
    assert!(!prefs.contains(&t));
    assert!(e.st.control_api_check_token(&t));
    assert!(!e.st.control_api_check_token(&t[..t.len() - 1]));
    assert!(!e.st.control_api_check_token(""));
    // Regenerating replaces it.
    let t2 = e.st.control_api_regenerate_token().unwrap();
    assert_ne!(t, t2);
    assert!(!e.st.control_api_check_token(&t));
    assert!(e.st.control_api_check_token(&t2));
    // A keychain that already has one keeps it (one token per machine, every project).
    let other = Arc::new(MemorySecrets::default());
    other.set(token::SECRET_ID, &t2).unwrap();
    e.st.set_control_api_secrets(other);
    assert_eq!(e.st.control_api_token().unwrap(), t2);
}

#[test]
fn clients_share_one_feed_that_ends_with_the_last_client() {
    let e = env();
    e.enable();
    let mut a = e.attach("Companion");
    let mut b = e.attach("Script");
    assert_eq!(e.st.control_api_tasks(), 1);
    let view = e.st.control_api_view();
    assert_eq!(view.clients.len(), 2);
    assert_eq!(view.clients[0].name, "Companion");
    assert!(e.events(names::CONTROL_API) >= 2);

    let snap_a = parse(&e.next(&mut a));
    let snap_b = parse(&e.next(&mut b));
    assert_eq!(snap_a["t"], "snapshot");
    assert_eq!(snap_a["state"]["layers"][e.key("l1")]["on_air"], false);
    assert_eq!(snap_a["seq"], snap_b["seq"]);

    e.st.live().show(&e.screen, "l1").unwrap();
    let (da, db) = (e.next(&mut a), e.next(&mut b));
    // Serialized once, shared by both.
    match (&da, &db) {
        (Out::Shared(x), Out::Shared(y)) => assert!(Arc::ptr_eq(x, y)),
        other => panic!("{other:?}"),
    }
    let diff = parse(&da);
    assert_eq!(diff["t"], "diff");
    assert_eq!(diff["seq"], snap_a["seq"].as_u64().unwrap() + 1);
    let mut tree = snap_a["state"].clone();
    apply(&mut tree, &diff);
    assert_eq!(tree, e.st.control_api_tree());

    // A client attaching later starts from the current tree.
    let mut c = e.attach("Late");
    let snap_c = parse(&e.next(&mut c));
    assert_eq!(snap_c["state"]["layers"][e.key("l1")]["on_air"], true);
    assert_eq!(snap_c["seq"], diff["seq"]);

    drop((a, b, c));
    wait_until("the feed ends", || e.st.control_api_tasks() == 0);
    assert!(e.st.control_api_view().clients.is_empty());
    // And starts again for the next client.
    let mut d = e.attach("Again");
    assert_eq!(parse(&e.next(&mut d))["t"], "snapshot");
    assert_eq!(e.st.control_api_tasks(), 1);
}

#[test]
fn a_lagging_client_gets_a_snapshot_not_the_backlog() {
    let e = env();
    e.enable();
    let mut slow = e.attach("Slow");
    let mut fast = e.attach("Fast");
    e.next(&mut slow);
    e.next(&mut fast);
    // More diffs than the channel holds while `slow` reads nothing.
    for i in 0..(crate::core::control_api::feed::FRAMES_CAP + 8) {
        if i % 2 == 0 {
            e.st.live().show(&e.screen, "l2").unwrap();
        } else {
            e.st.live().hide(&e.screen, "l2").unwrap();
        }
        assert_eq!(parse(&e.next(&mut fast))["t"], "diff");
    }
    let got = parse(&e.next(&mut slow));
    assert_eq!(got["t"], "snapshot", "{got}");
    assert_eq!(got["state"], e.st.control_api_tree());
}

#[test]
fn clients_are_kicked_with_the_right_code() {
    let e = env();
    e.enable();
    let a = e.attach("A");
    let b = e.attach("B");
    e.st.control_api_disconnect(a.id);
    assert_eq!(*a.kick.borrow(), Some(close::DISCONNECTED));
    assert_eq!(*b.kick.borrow(), None);
    e.st.control_api_regenerate_token().unwrap();
    assert_eq!(*b.kick.borrow(), Some(close::TOKEN_CHANGED));
    let c = e.attach("C");
    e.st.control_api_set_enabled(false).unwrap();
    assert_eq!(*c.kick.borrow(), Some(close::TURNED_OFF));
    e.st.control_api_set_enabled(true).unwrap();
    let d = e.attach("D");
    e.st.close_project();
    assert_eq!(*d.kick.borrow(), Some(close::GOING_AWAY));
}

#[test]
fn at_most_eight_clients() {
    let e = env();
    e.enable();
    let clients: Vec<_> = (0..CLIENTS_MAX)
        .map(|i| e.attach(&format!("c{i}")))
        .collect();
    let _g = e.rt.enter();
    assert!(matches!(
        e.st.control_api_attach("one more", "10.0.0.2"),
        Err(AttachError::Full)
    ));
    drop(clients);
    assert!(e.st.control_api_attach("now", "10.0.0.2").is_ok());
}

#[test]
fn a_commands_diff_names_it_as_cause() {
    let e = env();
    e.enable();
    let mut a = e.attach("A");
    e.next(&mut a);
    let tree = a.tree();
    e.st.control_api_command(Some("c-1"), tree, "take", &json!({"layer": e.key("l1")}))
        .unwrap();
    let d = parse(&e.next(&mut a));
    assert_eq!(d["cause"], "c-1");
    let path = format!("/layers/{}/on_air", e.key("l1"));
    assert!(d["ops"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["path"] == path));
    // A change from elsewhere has none.
    e.st.live().hide(&e.screen, "l1").unwrap();
    assert!(parse(&e.next(&mut a)).get("cause").is_none());
}

#[test]
fn layer_commands_follow_the_live_rules() {
    let e = env();
    e.enable();
    let l1 = json!({"layer": e.key("l1")});
    let on_air = || e.st.live().layer_view(&e.screen, "l1").unwrap().on_air;
    e.cmd("take", l1.clone()).unwrap();
    assert!(on_air());
    e.cmd("clear", l1.clone()).unwrap();
    assert!(!on_air());
    e.cmd("toggle", l1.clone()).unwrap();
    assert!(on_air());
    e.cmd("toggle", l1.clone()).unwrap();
    assert!(!on_air());

    // Studio mode: take cues into Preview; Studio take puts it on air.
    assert_eq!(
        e.code("studio_take", json!({"screen": e.screen})),
        "not_studio"
    );
    studio_on(&e.st);
    e.cmd("take", l1.clone()).unwrap();
    assert!(!on_air());
    assert_eq!(e.tree()["preview"][&e.screen], json!([e.key("l1")]));
    e.cmd(
        "studio_take",
        json!({"screen": e.screen, "transition": "cut"}),
    )
    .unwrap();
    assert!(on_air());

    // Refusals.
    assert_eq!(e.code("take", json!({})), "invalid");
    assert_eq!(e.code("take", json!({"layer": "l1"})), "invalid");
    assert_eq!(
        e.code("take", json!({"layer": format!("{}.nope", e.screen)})),
        "not_found"
    );
    assert_eq!(e.code("dance", json!({})), "invalid");
    assert_eq!(
        e.code("clear_screen", json!({"screen": "nope"})),
        "not_found"
    );
    assert_eq!(
        e.code(
            "studio_take",
            json!({"screen": e.screen, "transition": "wipe"})
        ),
        "invalid"
    );
    e.cmd("clear_screen", json!({"screen": e.screen})).unwrap();
    e.cmd("clear_all", json!({})).unwrap();
    e.cmd("panic", json!({})).unwrap();
}

#[test]
fn set_fields_checks_like_the_inspector_and_can_take() {
    let e = env();
    e.enable();
    let t = e.tree();
    let tid = t["layers"][e.key("l1")]["template"].as_str().unwrap();
    let fields = t["templates"][tid]["fields"].as_array().unwrap().clone();
    let text_key = fields
        .iter()
        .find(|f| f["type"] == "text" && f["key"] != "title")
        .unwrap()["key"]
        .as_str()
        .unwrap()
        .to_string();
    let l1 = e.key("l1");
    e.cmd(
        "set_fields",
        json!({"layer": l1, "values": {&text_key: "Ingrid Vale"}, "take": true}),
    )
    .unwrap();
    let view = e.st.live().layer_view(&e.screen, "l1").unwrap();
    assert!(view.on_air);
    assert_eq!(view.fields[&text_key], "Ingrid Vale");
    assert_eq!(e.tree()["layers"][&l1]["values"][&text_key], "Ingrid Vale");

    let code = |values: Value| e.code("set_fields", json!({"layer": l1, "values": values}));
    assert_eq!(code(json!({"title": "x"})), "bound");
    assert_eq!(code(json!({"nope": "x"})), "invalid");
    assert_eq!(code(json!({})), "invalid");
    assert_eq!(code(json!({&text_key: ["no"]})), "invalid");
    if let Some(img) = fields.iter().find(|f| f["type"] == "image") {
        let k = img["key"].as_str().unwrap();
        assert_eq!(code(json!({k: "https://example.com/a.png"})), "invalid");
        e.cmd("set_fields", json!({"layer": l1, "values": {k: ""}}))
            .unwrap();
    }
    // Numbers are written as text.
    e.cmd(
        "set_fields",
        json!({"layer": l1, "values": {&text_key: 42}}),
    )
    .unwrap();
    assert_eq!(
        e.st.live().layer_view(&e.screen, "l1").unwrap().fields[&text_key],
        "42"
    );
}

#[test]
fn timers_cues_sources_levels_and_automation() {
    let e = env();
    e.enable();
    let t = e.tree();
    // Timer (the scoreboard has one).
    let l2 = e.key("l2");
    let tid = t["layers"][&l2]["template"].as_str().unwrap();
    let timer = t["templates"][tid]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["type"] == "timer")
        .expect("scoreboard has a timer")["key"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        e.code("set_fields", json!({"layer": l2, "values": {&timer: "{}"}})),
        "read_only"
    );
    let running = || {
        let v = &e.tree()["layers"][&l2]["values"][&timer];
        let s: Value = serde_json::from_str(v.as_str().unwrap()).unwrap();
        s["run"] == true
    };
    e.cmd("timer", json!({"layer": l2, "field": timer, "op": "start"}))
        .unwrap();
    assert!(running());
    e.cmd("timer", json!({"layer": l2, "field": timer, "op": "pause"}))
        .unwrap();
    assert!(!running());
    e.cmd("timer", json!({"layer": l2, "field": timer, "op": "reset"}))
        .unwrap();
    assert_eq!(
        e.code(
            "timer",
            json!({"layer": l2, "field": timer, "op": "rewind"})
        ),
        "invalid"
    );

    // Cues: run one, then GO next runs the one after.
    let cue = |name: &str| {
        e.st.create_flow(NewFlow {
            name: name.into(),
            armed: true,
            retrigger: Retrigger::Restart,
            origin: None,
            nodes: vec![FlowNode {
                id: "c".into(),
                position: NodePos::default(),
                kind: NodeKind::Cue { label: None },
            }],
            edges: vec![],
        })
        .unwrap()
        .flow
        .id
    };
    let (c1, c2) = (cue("One"), cue("Two"));
    e.cmd("run_cue", json!({"cue": c1})).unwrap();
    wait_until("current", || {
        e.tree()["cue_lists"]["cues"]["current"] == json!(c1)
    });
    e.cmd("go_next", json!({})).unwrap();
    wait_until("next ran", || {
        e.tree()["cue_lists"]["cues"]["current"] == json!(c2)
    });
    assert_eq!(e.code("go_next", json!({})), "not_found");
    assert_eq!(e.code("run_cue", json!({"cue": "nope"})), "not_found");

    // Sources.
    let mut doc = e.st.create_source("Guests").unwrap();
    *doc.meta.kind.table_mut().unwrap().rows = ["A", "B"]
        .iter()
        .map(|n| Fields::from([("text".to_string(), n.to_string())]))
        .collect();
    let sid = e.st.save_source(doc).unwrap().id;
    e.cmd("source_step", json!({"source": sid, "dir": "next"}))
        .unwrap();
    assert_eq!(e.tree()["sources"][&sid]["label"], "B");
    e.cmd("source_step", json!({"source": sid, "dir": "prev"}))
        .unwrap();
    assert_eq!(e.tree()["sources"][&sid]["index"], 0);
    assert_eq!(
        e.code("source_step", json!({"source": sid, "dir": "up"})),
        "invalid"
    );

    // Levels.
    let l1 = e.key("l1");
    e.cmd(
        "set_level",
        json!({"layer": l1, "target": "opacity", "value": 0.5}),
    )
    .unwrap();
    wait_until("opacity", || e.tree()["layers"][&l1]["opacity"] == 50);
    assert_eq!(
        e.code(
            "set_level",
            json!({"layer": l1, "target": "opacity", "value": 2})
        ),
        "invalid"
    );

    // Automation off: set, toggle; saved in the UI state like the switch next to PANIC.
    e.cmd("automation", json!({"off": true})).unwrap();
    assert_eq!(e.tree()["automation_off"], true);
    assert!(e.st.ui_state().unwrap().control.automation_off);
    e.cmd("automation", json!({"off": "toggle"})).unwrap();
    assert_eq!(e.tree()["automation_off"], false);
    assert_eq!(e.code("automation", json!({"off": "maybe"})), "invalid");

    // Media commands need a media layer.
    assert_eq!(
        e.code("media", json!({"layer": l1, "op": "play"})),
        "invalid"
    );
}

#[test]
fn media_transport_acts_on_program() {
    let e = env();
    e.enable();
    add_audio_layer(&e);
    let m1 = e.key("m1");
    let state = || e.tree()["layers"][&m1]["media"]["playback"]["state"].clone();
    e.cmd("take", json!({"layer": m1})).unwrap();
    e.cmd("media", json!({"layer": m1, "op": "play"})).unwrap();
    assert_eq!(state(), "playing");
    e.cmd("media", json!({"layer": m1, "op": "pause"})).unwrap();
    assert_eq!(state(), "paused");
    e.cmd("media", json!({"layer": m1, "op": "seek", "sec": 2.5}))
        .unwrap();
    // The test clip is one frame long: the seek is clamped to its end.
    let pb = e.tree()["layers"][&m1]["media"]["playback"].clone();
    assert_eq!(pb["pos_ms"], pb["duration_ms"], "{pb}");
    assert_eq!(
        e.code("media", json!({"layer": m1, "op": "seek"})),
        "invalid"
    );
    e.cmd("media", json!({"layer": m1, "op": "stop"})).unwrap();
    assert_eq!(state(), "stopped");
    // A media layer has no fields.
    assert_eq!(
        e.code("set_fields", json!({"layer": m1, "values": {"x": "1"}})),
        "read_only"
    );
}

#[test]
fn welcome_names_the_app_and_features() {
    let e = env();
    let w = e.st.control_api_welcome();
    assert_eq!(w["t"], "welcome");
    assert_eq!(w["protocol"], 1);
    assert_eq!(w["app"], "Kinetra");
    assert_eq!(w["project"]["name"], "Nordlys Live");
    assert_eq!(
        w["features"],
        json!(["studio_take", "automation", "timer", "run_flow"])
    );
    assert!(w["now"].as_u64().unwrap() > 0);
}

#[test]
fn flows_with_an_api_call_trigger_are_listed_and_run() {
    let e = env();
    e.enable();
    let n = |id: &str, kind| FlowNode {
        id: id.into(),
        position: NodePos::default(),
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
    assert_eq!(
        e.tree()["api_flows"],
        json!({&flow: {"name": "Goal", "armed": true}})
    );
    e.cmd("run_flow", json!({"flow": flow, "value": 3}))
        .unwrap();
    wait_until("on air", || {
        e.st.live().layer_view(&e.screen, "l1").unwrap().on_air
    });
    assert_eq!(e.code("run_flow", json!({"flow": "nope"})), "not_found");
    assert_eq!(
        e.code("run_flow", json!({"flow": flow, "value": {"a": 1}})),
        "invalid"
    );
    e.st.set_flow_armed(&flow, false).unwrap();
    assert_eq!(e.tree()["api_flows"][&flow]["armed"], false);
    assert_eq!(e.code("run_flow", json!({"flow": flow})), "disarmed");
}

#[test]
fn a_preview_cue_from_elsewhere_reaches_clients() {
    let e = env();
    e.enable();
    studio_on(&e.st);
    let mut a = e.attach("A");
    let snap = parse(&e.next(&mut a));
    assert_eq!(snap["state"]["preview"][&e.screen], json!([]));
    // Control's Take in Studio mode (not through the API): Preview only, Program unchanged.
    press(&e.st, &format!("live.take:{}/l1", e.screen));
    let d = parse(&e.next(&mut a));
    let path = format!("/preview/{}", e.screen);
    assert!(
        d["ops"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["path"] == path && o["value"] == json!([e.key("l1")])),
        "{d}"
    );
}
