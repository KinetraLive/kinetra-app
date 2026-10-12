//! `run_control`: every command id in each live mode (onTake, immediate, Studio), value targets
//! (opacity, range variable, number field) with pacing, and layer opacity through the live rules.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::broadcast::Receiver;

use super::*;
use crate::core::events::EventSink;
use crate::core::live_hub::HubMsg;
use crate::core::model::{
    FlowEdge, FlowNode, LiveEdit, NewFlow, NewScreen, NodeKind, NodePos, PortRef, PreviewAfterTake,
    Rect, Retrigger, ScreenDoc, ScreenLayer, SourceDoc, StudioSettings, TakeTransition,
    TakeTransitionKind, TemplateSource, WireKind,
};
use crate::core::protocol::Fields;

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Value)>>);

impl EventSink for Recorder {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    _rt: Option<tokio::runtime::Runtime>,
    st: AppState,
    dir: PathBuf,
    screen: String,
}

/// A project with one screen holding `l1` (classic bar: `radius` is a range 0..24 step 1) and
/// `l2` (scoreboard: `home_score` is a number field), both off air. `rt`: inject a runtime.
fn env_with(rt: bool) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        Arc::new(Recorder::default()),
        Duration::from_secs(60),
    )
    .unwrap();
    st.set_content_flush_delay(Duration::from_secs(60));
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    let rt = rt.then(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build()
            .unwrap();
        st.live().set_runtime(rt.handle().clone());
        rt
    });
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    let bar = template(&st, &dir, "classic-bar");
    let score = template(&st, &dir, "scoreboard");
    let mut doc: ScreenDoc = st
        .create_screen(&NewScreen {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap()
        .doc;
    doc.meta.layers.push(layer("l1", &bar));
    doc.meta.layers.push(layer("l2", &score));
    let screen = st.save_screen(doc).unwrap().doc.id;
    Env {
        _tmp: tmp,
        _rt: rt,
        st,
        dir,
        screen,
    }
}

fn env() -> Env {
    env_with(false)
}

fn template(st: &AppState, dir: &std::path::Path, preset: &str) -> String {
    st.templates()
        .create(
            dir,
            preset,
            &TemplateSource::Preset {
                preset_id: preset.into(),
            },
        )
        .unwrap()
        .id
}

fn layer(id: &str, template_id: &str) -> ScreenLayer {
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
            bindings: Default::default(),
            variant: None,
            sizing: None,
            opacity: None,
        },
        anchor: None,
    }
}

fn cmd(id: &str) -> ControlTarget {
    ControlTarget::Command {
        command_id: id.into(),
    }
}

fn value(screen: &str, layer: &str, target: ValueTarget) -> ControlTarget {
    ControlTarget::Value {
        screen_id: screen.into(),
        layer_id: layer.into(),
        target,
    }
}

fn studio(on: bool) -> StudioSettings {
    StudioSettings {
        enabled: on,
        preview_after_take: PreviewAfterTake::Keep,
        transition: TakeTransition {
            kind: TakeTransitionKind::Cut,
            ms: 300,
        },
    }
}

fn drain(rx: &mut Receiver<Arc<HubMsg>>, screen: &str) -> Vec<Value> {
    let mut out = Vec::new();
    while let Ok(m) = rx.try_recv() {
        if &*m.screen_id == screen {
            out.push(serde_json::from_str(&m.json).unwrap());
        }
    }
    out
}

fn kinds(msgs: &[Value]) -> Vec<String> {
    msgs.iter()
        .map(|m| m["t"].as_str().unwrap().to_string())
        .collect()
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

impl Env {
    fn run(&self, id: &str) -> AppResult<()> {
        self.st.run_control(&cmd(id), ControlInput::Press)
    }

    fn on_air(&self, l: &str) -> bool {
        self.st.live().layer_view(&self.screen, l).unwrap().on_air
    }

    fn pending(&self, l: &str) -> bool {
        self.st.live().layer_view(&self.screen, l).unwrap().pending
    }

    fn set_mode(&self, mode: LiveEdit) {
        let mut ui = self.st.ui_state().unwrap();
        ui.control.live_edit = mode;
        self.st.set_ui_state(ui).unwrap();
    }

    fn layer_doc(&self, l: &str) -> ScreenLayerKind {
        let doc = self.st.get_screen(&self.screen).unwrap().doc;
        doc.meta
            .layers
            .into_iter()
            .find(|x| x.id == l)
            .unwrap()
            .kind
    }

    fn opacity_doc(&self, l: &str) -> Option<u8> {
        let ScreenLayerKind::Template { opacity, .. } = self.layer_doc(l) else {
            unreachable!()
        };
        opacity
    }

    fn live_opacity(&self, l: &str) -> u8 {
        self.st
            .live()
            .with_layer(&self.screen, l, |x| x.opacity)
            .unwrap()
    }

    fn preview_on(&self, l: &str) -> bool {
        let pv = self.st.preview_view(&self.screen).unwrap();
        pv.layers.iter().find(|x| x.id == l).unwrap().on_air
    }

    fn set_opacity(&self, l: &str, o: Option<u8>) -> AppResult<ScreenState> {
        let mut doc = self.st.get_screen(&self.screen).unwrap().doc;
        let layer = doc.meta.layers.iter_mut().find(|x| x.id == l).unwrap();
        let ScreenLayerKind::Template { opacity, .. } = &mut layer.kind else {
            unreachable!()
        };
        *opacity = o;
        self.st.save_screen(doc)
    }
}

use crate::core::model::ScreenState;

// ---- Parsing -----------------------------------------------------------------------------------

#[test]
fn command_ids_parse_exactly() {
    use ControlCommand as C;
    let ok = [
        (
            "live.toggle:s1/l1",
            C::Toggle {
                screen: "s1",
                layer: "l1",
            },
        ),
        (
            "live.take:s1/l1",
            C::Take {
                screen: "s1",
                layer: "l1",
            },
        ),
        (
            "live.clear:s1/l1",
            C::Clear {
                screen: "s1",
                layer: "l1",
            },
        ),
        (
            "live.update:s1/l1",
            C::Update {
                screen: "s1",
                layer: "l1",
            },
        ),
        ("cue.go:f1", C::CueGo { flow: "f1" }),
        ("source.next:src", C::SourceNext { source: "src" }),
        ("source.prev:src", C::SourcePrev { source: "src" }),
        ("live.clearScreen:s1", C::ClearScreen { screen: "s1" }),
        ("live.clearAll", C::ClearAll),
        ("live.panic", C::Panic),
    ];
    for (id, want) in ok {
        assert_eq!(parse_command(id), Some(want), "{id}");
    }
    assert_eq!(
        parse_command("live.take:s1/l1").unwrap().layer(),
        Some(("s1", "l1"))
    );
    for bad in [
        "",
        "live.toggle",
        "live.toggle:s1",
        "live.toggle:s1/",
        "live.toggle:s 1/l1",
        "live.toggle:s1/l1/x",
        "live.cue:s1/l1",
        "cue.go:",
        "cue.go:a/b",
        "live.clearAll:x",
        "nav.control",
        "live.panic ",
    ] {
        assert_eq!(parse_command(bad), None, "{bad}");
    }
}

#[test]
fn unknown_targets_are_errors() {
    let e = env();
    assert!(matches!(e.run("nav.control"), Err(AppError::Invalid(_))));
    assert!(matches!(
        e.run(&format!("live.toggle:{}/nope", e.screen)),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.run("live.toggle:nope/l1"),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.run("live.clearScreen:nope"),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.run("live.clearScreen:__preview"),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(e.run("cue.go:nope"), Err(AppError::NotFound(_))));
    assert!(e.run("source.next:nope").is_err());
    let v = value(&e.screen, "nope", ValueTarget::Opacity);
    assert!(matches!(
        e.st.run_control(&v, ControlInput::Level(0.5)),
        Err(AppError::NotFound(_))
    ));
    let v = value(&e.screen, "l1", ValueTarget::Opacity);
    assert!(matches!(
        e.st.run_control(&v, ControlInput::Level(1.5)),
        Err(AppError::Invalid(_))
    ));
    assert!(e.st.run_control(&v, ControlInput::Level(f32::NAN)).is_err());
    // A range variable of the wrong type, a field that isn't a number, unknown keys.
    let wrong = |t| {
        e.st.run_control(&value(&e.screen, "l1", t), ControlInput::Level(0.5))
    };
    assert!(matches!(
        wrong(ValueTarget::Var {
            key: "accent".into()
        }),
        Err(AppError::Invalid(_))
    ));
    assert!(matches!(
        wrong(ValueTarget::Field { key: "name".into() }),
        Err(AppError::Invalid(_))
    ));
    assert!(wrong(ValueTarget::Var { key: "nope".into() }).is_err());
    assert!(wrong(ValueTarget::Field { key: "nope".into() }).is_err());
    e.st.close_project();
    assert!(matches!(e.run("live.panic"), Err(AppError::NoProject)));
}

// ---- Commands ------------------------------------------------------------------------------------

#[test]
fn toggle_take_clear_off_studio() {
    let e = env();
    let s = e.screen.clone();
    // Only Press acts.
    e.st.run_control(&cmd(&format!("live.toggle:{s}/l1")), ControlInput::Release)
        .unwrap();
    e.st.run_control(
        &cmd(&format!("live.toggle:{s}/l1")),
        ControlInput::Level(1.0),
    )
    .unwrap();
    assert!(!e.on_air("l1"));
    e.run(&format!("live.toggle:{s}/l1")).unwrap();
    assert!(e.on_air("l1"));
    e.run(&format!("live.toggle:{s}/l1")).unwrap();
    assert!(!e.on_air("l1"));
    e.run(&format!("live.take:{s}/l1")).unwrap();
    e.run(&format!("live.take:{s}/l1")).unwrap();
    assert!(e.on_air("l1"));
    e.run(&format!("live.clear:{s}/l1")).unwrap();
    assert!(!e.on_air("l1"));
    // Update off air does nothing.
    e.run(&format!("live.update:{s}/l1")).unwrap();
    assert!(!e.on_air("l1"));
}

#[test]
fn update_applies_staged_content_without_a_replay() {
    let e = env();
    let s = e.screen.clone();
    e.run(&format!("live.take:{s}/l1")).unwrap();
    let f: Fields = [("name".to_string(), "Ada".to_string())].into();
    e.st.set_layer_fields(&s, "l1", f).unwrap();
    assert!(e.pending("l1"), "onTake stages");
    let mut rx = e.st.live().subscribe();
    e.run(&format!("live.update:{s}/l1")).unwrap();
    assert!(!e.pending("l1"));
    assert!(e.on_air("l1"));
    let msgs = drain(&mut rx, &s);
    assert!(
        msgs.iter().all(|m| m["t"] != "state"),
        "no replay: {msgs:?}"
    );
    assert_eq!(
        e.st.live().layer_view(&s, "l1").unwrap().fields["name"],
        "Ada"
    );
}

#[test]
fn clear_screen_clear_all_and_panic() {
    let e = env();
    let s = e.screen.clone();
    e.run(&format!("live.take:{s}/l1")).unwrap();
    e.run(&format!("live.take:{s}/l2")).unwrap();
    e.run(&format!("live.clearScreen:{s}")).unwrap();
    assert!(!e.on_air("l1") && !e.on_air("l2"));
    e.run(&format!("live.take:{s}/l1")).unwrap();
    e.run("live.clearAll").unwrap();
    assert!(!e.on_air("l1"));
    e.run(&format!("live.take:{s}/l2")).unwrap();
    let mut rx = e.st.live().subscribe();
    e.run("live.panic").unwrap();
    assert!(!e.on_air("l2"));
    assert!(kinds(&drain(&mut rx, &s)).contains(&"cut".to_string()));
}

#[test]
fn studio_mode_cues_like_control() {
    let e = env();
    let s = e.screen.clone();
    e.st.set_studio(studio(true), None).unwrap();
    let n0 = e.st.live().studio_op_count();
    // Toggle: cue in (Program unchanged), toggle again: back to "as Program" (no cue).
    e.run(&format!("live.toggle:{s}/l1")).unwrap();
    assert!(
        e.st.live().studio_op_count() > n0,
        "through the entry point"
    );
    assert!(!e.on_air("l1"));
    assert!(e.preview_on("l1"));
    assert_eq!(e.st.preview_view(&s).unwrap().cues[0].visible, Some(true));
    e.run(&format!("live.toggle:{s}/l1")).unwrap();
    assert!(!e.preview_on("l1"));
    assert!(e.st.preview_view(&s).unwrap().cues.is_empty());
    // Take = cue in, Clear = cue out (idempotent).
    e.run(&format!("live.take:{s}/l1")).unwrap();
    e.run(&format!("live.take:{s}/l1")).unwrap();
    assert!(e.preview_on("l1") && !e.on_air("l1"));
    // Update in Studio mode takes the Preview.
    e.run(&format!("live.update:{s}/l1")).unwrap();
    assert!(e.on_air("l1"));
    assert!(e.st.preview_view(&s).unwrap().cues.is_empty());
    // On air in Program: clear cues it out of Preview only.
    e.run(&format!("live.clear:{s}/l1")).unwrap();
    assert!(e.on_air("l1"));
    assert!(!e.preview_on("l1"));
    assert_eq!(e.st.preview_view(&s).unwrap().cues[0].visible, Some(false));
    e.run(&format!("live.take:{s}/l1")).unwrap();
    assert!(e.st.preview_view(&s).unwrap().cues.is_empty());
    // Clear screen / PANIC go to Program directly, as from Control.
    e.run(&format!("live.clearScreen:{s}")).unwrap();
    assert!(!e.on_air("l1"));
}

fn cue_flow(e: &Env) -> String {
    let n = |id: &str, kind| FlowNode {
        id: id.into(),
        position: NodePos::default(),
        kind,
    };
    let info =
        e.st.create_flow(NewFlow {
            name: "Go".into(),
            armed: true,
            retrigger: Retrigger::Restart,
            origin: None,
            nodes: vec![
                n("c", NodeKind::Cue { label: None }),
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
                    node: "c".into(),
                    port: "fired".into(),
                },
                to: PortRef {
                    node: "t".into(),
                    port: "in".into(),
                },
                kind: WireKind::Event,
            }],
        })
        .unwrap();
    info.flow.id
}

#[test]
fn cue_go_runs_the_flow_even_with_automation_off() {
    let e = env_with(true);
    let id = cue_flow(&e);
    let mut ui = e.st.ui_state().unwrap();
    ui.control.automation_off = true;
    e.st.set_ui_state(ui).unwrap();
    e.run(&format!("cue.go:{id}")).unwrap();
    wait_until("the cue's take", || e.on_air("l1"));
    e.st.set_flow_armed(&id, false).unwrap();
    assert!(matches!(
        e.run(&format!("cue.go:{id}")),
        Err(AppError::Invalid(_))
    ));
}

#[test]
fn source_next_and_prev_step_the_source() {
    let e = env();
    let doc = e.st.create_source("Guests").unwrap();
    let mut v = serde_json::to_value(&doc).unwrap();
    v["columns"] = json!([{"key": "name", "label": "Name"}]);
    v["rows"] = json!([{"name": "A"}, {"name": "B"}, {"name": "C"}]);
    let doc: SourceDoc = serde_json::from_value(v).unwrap();
    let id = doc.id.clone();
    e.st.save_source(doc).unwrap();
    let index = |e: &Env| {
        serde_json::to_value(e.st.sources().doc(&id).unwrap()).unwrap()["index"]
            .as_u64()
            .unwrap()
    };
    e.run(&format!("source.next:{id}")).unwrap();
    e.run(&format!("source.next:{id}")).unwrap();
    assert_eq!(index(&e), 2);
    e.run(&format!("source.prev:{id}")).unwrap();
    assert_eq!(index(&e), 1);
}

// ---- Values --------------------------------------------------------------------------------------

#[test]
fn value_targets_set_opacity_range_variables_and_number_fields() {
    let e = env();
    let s = e.screen.clone();
    let op = value(&s, "l1", ValueTarget::Opacity);
    e.st.run_control(&op, ControlInput::Level(0.5)).unwrap();
    assert_eq!(e.opacity_doc("l1"), Some(50));
    assert_eq!(e.live_opacity("l1"), 50);
    std::thread::sleep(PACE);
    e.st.run_control(&op, ControlInput::Press).unwrap();
    assert_eq!(e.opacity_doc("l1"), None, "100 % is stored as absent");
    assert_eq!(e.live_opacity("l1"), 100);
    // Release does nothing.
    e.st.run_control(&op, ControlInput::Release).unwrap();

    // classic-bar radius: 0..24 step 1.
    let radius = value(
        &s,
        "l1",
        ValueTarget::Var {
            key: "radius".into(),
        },
    );
    e.st.run_control(&radius, ControlInput::Level(0.5)).unwrap();
    let ScreenLayerKind::Template { vars, .. } = e.layer_doc("l1") else {
        unreachable!()
    };
    assert_eq!(vars["radius"], "12");
    std::thread::sleep(PACE);
    e.st.run_control(&radius, ControlInput::Level(1.0 / 127.0))
        .unwrap();
    let ScreenLayerKind::Template { vars, .. } = e.layer_doc("l1") else {
        unreachable!()
    };
    assert_eq!(vars["radius"], "0", "snapped to the step");
    let css =
        e.st.live()
            .with_layer(&s, "l1", |l| l.vars.get("radius").cloned())
            .unwrap();
    assert_eq!(css.as_deref(), Some("0px"));

    // scoreboard home_score: the CC value.
    let score = value(
        &s,
        "l2",
        ValueTarget::Field {
            key: "home_score".into(),
        },
    );
    e.st.run_control(&score, ControlInput::Level(1.0)).unwrap();
    let ScreenLayerKind::Template { fields, .. } = e.layer_doc("l2") else {
        unreachable!()
    };
    assert_eq!(fields["home_score"], "127");
    std::thread::sleep(PACE);
    e.st.run_control(&score, ControlInput::Level(3.0 / 127.0))
        .unwrap();
    assert_eq!(
        e.st.live().layer_view(&s, "l2").unwrap().fields["home_score"],
        "3"
    );
    // Saved coalesced: the screen is dirty, written by the content flusher.
    assert!(e.st.screens().is_dirty(&s));
    e.st.flush_all();
    let file = std::fs::read_to_string(e.dir.join("screens").join(format!("{s}.json"))).unwrap();
    assert!(file.contains("\"home_score\": \"3\""), "{file}");
}

#[test]
fn values_follow_the_live_rules_as_operator_edits() {
    let e = env();
    let s = e.screen.clone();
    let op = value(&s, "l1", ValueTarget::Opacity);
    e.run(&format!("live.take:{s}/l1")).unwrap();
    let mut rx = e.st.live().subscribe();
    // onTake: staged.
    e.st.run_control(&op, ControlInput::Level(0.2)).unwrap();
    assert!(drain(&mut rx, &s).is_empty());
    assert!(e.pending("l1"));
    assert_eq!(e.live_opacity("l1"), 100);
    e.run(&format!("live.update:{s}/l1")).unwrap();
    assert_eq!(e.live_opacity("l1"), 20);
    let rev =
        e.st.live()
            .with_layer(&s, "l1", |l| l.template_rev)
            .unwrap();
    drain(&mut rx, &s);
    // Immediate: one same-rev `layer` at once, no state change.
    e.set_mode(LiveEdit::Immediate);
    std::thread::sleep(PACE);
    e.st.run_control(&op, ControlInput::Level(0.7)).unwrap();
    let msgs = drain(&mut rx, &s);
    assert_eq!(kinds(&msgs), ["layer"], "{msgs:?}");
    assert_eq!(msgs[0]["layer"]["opacity"], 70);
    assert_eq!(msgs[0]["layer"]["templateRev"], rev);
    assert_eq!(msgs[0]["layer"]["state"], "in");
    // Studio: the Preview diff, Program untouched.
    e.st.set_studio(studio(true), None).unwrap();
    drain(&mut rx, &s);
    std::thread::sleep(PACE);
    e.st.run_control(&op, ControlInput::Level(0.3)).unwrap();
    assert!(drain(&mut rx, &s).is_empty());
    assert_eq!(e.live_opacity("l1"), 70);
    assert!(e.st.preview_view(&s).unwrap().cues[0].staged);
}

#[test]
fn values_are_paced_latest_wins_and_no_task_at_rest() {
    let e = env_with(true);
    let s = e.screen.clone();
    let op = value(&s, "l1", ValueTarget::Opacity);
    let pacer = &e.st.control_runtime().pacer;
    let before = pacer.applies();
    let start = Instant::now();
    // 200 values over ~0.4 s (500/s): ≤ 30 applies/s.
    for i in 0..200u32 {
        e.st.run_control(&op, ControlInput::Level(i as f32 / 199.0 * 0.99))
            .unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
    wait_until("the pacer to settle", || pacer.sleeping() == 0);
    let elapsed = start.elapsed().as_secs_f64();
    let applies = pacer.applies() - before;
    assert!(
        (applies as f64) <= elapsed * 30.0 + 2.0,
        "{applies} applies in {elapsed:.2} s"
    );
    assert!(applies >= 3, "{applies}");
    // The latest value won.
    assert_eq!(e.live_opacity("l1"), 99);
    assert_eq!(pacer.sleeping(), 0, "no task at rest");
}

// ---- Opacity through screens ---------------------------------------------------------------------

#[test]
fn opacity_is_validated_and_stored_only_below_100() {
    let e = env();
    assert!(matches!(
        e.set_opacity("l1", Some(101)),
        Err(AppError::Invalid(_))
    ));
    e.set_opacity("l1", Some(100)).unwrap();
    assert_eq!(e.opacity_doc("l1"), None);
    e.set_opacity("l1", Some(0)).unwrap();
    assert_eq!(e.opacity_doc("l1"), Some(0));
    let file =
        std::fs::read_to_string(e.dir.join("screens").join(format!("{}.json", e.screen))).unwrap();
    assert_eq!(file.matches("\"opacity\"").count(), 1, "{file}");
}

#[test]
fn opacity_follows_the_live_rules_like_rect() {
    let e = env();
    let s = e.screen.clone();
    let mut rx = e.st.live().subscribe();
    // Off air: a same-rev layer message at once.
    e.set_opacity("l1", Some(50)).unwrap();
    let msgs = drain(&mut rx, &s);
    assert_eq!(kinds(&msgs), ["layer"]);
    assert_eq!(msgs[0]["layer"]["opacity"], 50);
    let rev = msgs[0]["layer"]["templateRev"].clone();
    // On air under onTake: staged, then the take applies it without a recompile or replay.
    e.st.live().show(&s, "l1").unwrap();
    drain(&mut rx, &s);
    e.set_opacity("l1", None).unwrap();
    assert!(drain(&mut rx, &s).is_empty());
    assert!(e.pending("l1"));
    e.st.live().show(&s, "l1").unwrap();
    let msgs = drain(&mut rx, &s);
    assert_eq!(kinds(&msgs), ["layer"]);
    assert!(msgs[0]["layer"].get("opacity").is_none(), "100 is absent");
    assert_eq!(msgs[0]["layer"]["templateRev"], rev);
    // The snapshot carries it too.
    e.set_mode(LiveEdit::Immediate);
    e.set_opacity("l1", Some(10)).unwrap();
    let msgs = drain(&mut rx, &s);
    assert_eq!(kinds(&msgs), ["layer"]);
    assert_eq!(msgs[0]["layer"]["opacity"], 10);
    assert_eq!(msgs[0]["layer"]["templateRev"], rev);
}

#[test]
fn control_to_renderer_message_latency() {
    let e = env();
    e.set_mode(LiveEdit::Immediate);
    let s = e.screen.clone();
    let mut rx = e.st.live().subscribe();
    let mut worst = Duration::ZERO;
    let op = value(&s, "l1", ValueTarget::Opacity);
    for i in 0..20 {
        let t0 = Instant::now();
        e.run(&format!("live.toggle:{s}/l1")).unwrap();
        recv(&mut rx);
        worst = worst.max(t0.elapsed());
        std::thread::sleep(PACE);
        let t0 = Instant::now();
        e.st.run_control(&op, ControlInput::Level((i % 10) as f32 / 10.0))
            .unwrap();
        recv(&mut rx);
        worst = worst.max(t0.elapsed());
        while rx.try_recv().is_ok() {}
    }
    println!("run_control → hub message, worst of 40: {worst:?}");
    assert!(worst < Duration::from_millis(50), "{worst:?}");
}

/// Waits (spinning, ≤ 2 s) for the next hub message.
fn recv(rx: &mut Receiver<Arc<HubMsg>>) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while rx.try_recv().is_err() {
        assert!(Instant::now() < deadline, "no hub message");
        std::hint::spin_loop();
    }
}
