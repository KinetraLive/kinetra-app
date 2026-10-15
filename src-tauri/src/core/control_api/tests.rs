use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::core::control_actions::ControlInput;
use crate::core::events::EventSink;
use crate::core::model::{
    ControlTarget, FlowEdge, FlowNode, MediaEndAction, MediaFit, MediaLayerConfig, NewFlow,
    NewScreen, NodeKind, NodePos, PortRef, PreviewAfterTake, Rect, Retrigger, ScreenLayer,
    ScreenLayerKind, SourceBinding, SourceStep, StudioSettings, TakeTransition, TakeTransitionKind,
    TemplateSource, WireKind,
};
use crate::core::protocol::Fields;
use crate::core::remote::state;
use crate::core::state::AppState;

/// docs/control-api.fixture.json.
mod fixture;
/// The service: switch, token, feed, clients, commands (this file: the tree).
mod service;

#[derive(Default)]
struct Rec(Mutex<Vec<(String, Value)>>);

impl EventSink for Rec {
    fn emit(&self, event: &str, payload: Value) {
        if let Ok(mut g) = self.0.lock() {
            g.push((event.to_string(), payload));
        }
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    st: AppState,
    screen: String,
    rec: Arc<Rec>,
}

impl Env {
    fn key(&self, layer: &str) -> String {
        state::local_key(&self.screen, layer)
    }

    fn tree(&self) -> Value {
        let t = self.st.control_api_tree();
        state::check_refs(&t).unwrap();
        t
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

/// A project with screen "Main": `l1` (lower third, `title` bound to a source) and `l2`
/// (scoreboard), both off air.
fn env() -> Env {
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
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
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
    let mut doc = st
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
    Env {
        _tmp: tmp,
        rt,
        st,
        screen,
        rec,
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn press(st: &AppState, id: &str) {
    st.run_control(
        &ControlTarget::Command {
            command_id: id.into(),
        },
        ControlInput::Press,
    )
    .unwrap();
}

#[test]
fn local_tree_uses_screen_qualified_keys_and_its_own_fields() {
    let e = env();
    let t = e.tree();
    let l1 = e.key("l1");
    assert_eq!(
        t["screens"][&e.screen]["layers"],
        json!([e.key("l2"), l1.clone()])
    );
    let layer = &t["layers"][&l1];
    assert_eq!(layer["kind"], "template");
    assert_eq!(layer["screen"], json!(e.screen));
    assert_eq!(layer["on_air"], false);
    assert_eq!(layer["bound"], json!(["title"]));
    assert_eq!(layer["pending"], false);
    assert_eq!(layer["opacity"], 100);
    assert!(layer["variant"].is_string());
    assert_eq!(layer["ready"], Value::Null);
    assert!(layer.get("remote_editable").is_none());
    // Templates list their looks; top level has the local additions, not the take lock.
    let tid = layer["template"].as_str().unwrap();
    assert!(!t["templates"][tid]["variants"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(t["templates"][tid]["ranges"].is_array());
    assert!(t.get("remote").is_none());
    assert_eq!(t["automation_off"], false);
    assert_eq!(t["sources"], json!({}));
    assert_eq!(t["source_order"], json!([]));
    assert_eq!(t["cue_lists"]["cues"]["order"], json!([]));
}

#[test]
fn the_remote_tree_is_unchanged() {
    let e = env();
    let mut input = e.st.remote_build_input();
    input.take_locked = true;
    let t = state::build(&e.st, &input);
    state::check_refs(&t).unwrap();
    assert_eq!(t["remote"], json!({"take_locked": true}));
    assert_eq!(t["screens"][&e.screen]["layers"], json!(["l2", "l1"]));
    let l1 = t["layers"]["l1"].as_object().unwrap();
    let mut keys: Vec<&str> = l1.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "bound",
            "kind",
            "name",
            "on_air",
            "remote_editable",
            "screen",
            "template",
            "values"
        ]
    );
    let tid = l1["template"].as_str().unwrap();
    let mut tkeys: Vec<&String> = t["templates"][tid].as_object().unwrap().keys().collect();
    tkeys.sort_unstable();
    assert_eq!(tkeys, ["fields", "name"]);
    for k in ["sources", "source_order", "automation_off"] {
        assert!(t.get(k).is_none(), "{k}");
    }
}

#[test]
fn on_air_is_the_logical_state_from_take_to_clear() {
    let e = env();
    e.st.live().show(&e.screen, "l1").unwrap();
    assert_eq!(e.tree()["layers"][e.key("l1")]["on_air"], true);
    e.st.live().hide(&e.screen, "l1").unwrap();
    // Off at once, while the out-animation still plays.
    assert_eq!(e.tree()["layers"][e.key("l1")]["on_air"], false);
}

#[test]
fn layer_ids_shared_by_two_screens_never_collide() {
    let e = env();
    let template = e.tree()["layers"][e.key("l1")]["template"]
        .as_str()
        .unwrap()
        .to_string();
    let mut doc =
        e.st.create_screen(&NewScreen {
            name: "Second".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap()
        .doc;
    doc.meta.layers.push(layer("l1", &template, &[]));
    let second = e.st.save_screen(doc).unwrap().doc.id;
    let t = e.tree();
    let other = state::local_key(&second, "l1");
    assert_eq!(t["layers"][&other]["screen"], json!(second));
    assert_eq!(t["layers"][e.key("l1")]["screen"], json!(e.screen));
    assert_eq!(
        state::split_local_key(&other),
        Some((second.as_str(), "l1"))
    );
    assert_eq!(state::split_local_key("l1"), None);
    assert_eq!(state::split_local_key("a.b.c"), None);
}

#[test]
fn sources_show_position_and_label() {
    let e = env();
    let mut doc = e.st.create_source("Guests").unwrap();
    {
        let t = doc.meta.kind.table_mut().unwrap();
        *t.rows = ["Ingrid Vale", "Kari Nordmann"]
            .iter()
            .map(|n| Fields::from([("text".to_string(), n.to_string())]))
            .collect();
    }
    let id = e.st.save_source(doc).unwrap().id;
    let t = e.tree();
    assert_eq!(t["source_order"], json!([id]));
    assert_eq!(
        t["sources"][&id],
        json!({"name": "Guests", "kind": "text", "index": 0, "count": 2, "label": "Ingrid Vale"})
    );
    e.st.step_source(&id, SourceStep::By { delta: 1 }).unwrap();
    let s = &e.tree()["sources"][&id];
    assert_eq!(
        (s["index"].clone(), s["label"].clone()),
        (json!(1), json!("Kari Nordmann"))
    );
}

#[test]
fn running_cues_and_automation_off_are_shown() {
    let e = env();
    let n = |id: &str, kind| FlowNode {
        id: id.into(),
        position: NodePos::default(),
        kind,
    };
    let edge = |id: &str, from: &str, to: &str, port: &str| FlowEdge {
        id: id.into(),
        from: PortRef {
            node: from.into(),
            port: port.into(),
        },
        to: PortRef {
            node: to.into(),
            port: "in".into(),
        },
        kind: WireKind::Event,
    };
    let flow =
        e.st.create_flow(NewFlow {
            name: "Open".into(),
            armed: true,
            retrigger: Retrigger::Restart,
            origin: None,
            nodes: vec![
                n("c", NodeKind::Cue { label: None }),
                n("d", NodeKind::Delay { ms: 400 }),
            ],
            edges: vec![edge("e1", "c", "d", "fired")],
        })
        .unwrap()
        .flow
        .id;
    let t = e.tree();
    assert_eq!(t["cue_lists"]["cues"]["cues"][&flow]["running"], false);
    assert_eq!(t["cue_lists"]["cues"]["next"], json!(flow));
    press(&e.st, &format!("cue.go:{flow}"));
    wait_until("running", || {
        e.tree()["cue_lists"]["cues"]["cues"][&flow]["running"] == true
    });
    assert_eq!(e.tree()["cue_lists"]["cues"]["current"], json!(flow));
    wait_until("finished", || {
        e.tree()["cue_lists"]["cues"]["cues"][&flow]["running"] == false
    });

    e.st.engine_set_automation_off(true);
    assert_eq!(e.tree()["automation_off"], true);
}

#[test]
fn studio_preview_uses_local_keys() {
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
    assert_eq!(e.tree()["studio_mode"], true);
    press(&e.st, &format!("live.take:{}/l1", e.screen));
    let t = e.tree();
    assert_eq!(t["preview"][&e.screen], json!([e.key("l1")]));
    assert_eq!(t["layers"][e.key("l1")]["on_air"], false);
}

#[test]
fn media_layers_include_audio_with_playback() {
    let e = env();
    add_audio_layer(&e);
    let t = e.tree();
    let m = &t["layers"][e.key("m1")];
    assert_eq!(m["kind"], "media");
    assert_eq!(m["media"]["type"], "audio");
    assert_eq!(m["media"]["name"], "bed.mp3");
    assert_eq!(m["opacity"], 80);
    assert_eq!(m["media"]["playback"]["state"], "stopped");
    assert_eq!(m["media"]["playback"]["in_ms"], 1500);
    for k in ["template", "values", "bound"] {
        assert!(m.get(k).is_none(), "{k}");
    }
}

/// Media layer `m1` on the main screen playing `bed.mp3` (in point 1.5 s, opacity 80).
fn add_audio_layer(e: &Env) {
    let mut doc = e.st.get_screen(&e.screen).unwrap().doc;
    doc.meta.layers.push(ScreenLayer {
        id: "m1".into(),
        name: "Bed".into(),
        rect: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        kind: ScreenLayerKind::Media {
            media: MediaLayerConfig {
                asset: "/assets/bed.mp3".into(),
                fit: MediaFit::Fit,
                in_ms: 1500,
                out_ms: None,
                volume: 100,
                muted: false,
                end_action: MediaEndAction::Hold,
                autoplay: false,
            },
            opacity: Some(80),
        },
        anchor: None,
    });
    // One MPEG-1 Layer III frame (128 kbit/s, 44.1 kHz): enough for the probe.
    let mut mp3 = vec![0xff, 0xfb, 0x90, 0x00];
    mp3.resize(417, 0);
    let root = crate::core::assets::assets_root(&e.st.project_dir().unwrap());
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("bed.mp3"), mp3).unwrap();
    e.st.save_screen(doc).unwrap();
}
