//! Studio mode on `AppState`: the entry point, edits that stay off Program, settings, exits and
//! persistence across a reopen.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use super::*;
use crate::core::events::EventSink;
use crate::core::live::LiveLayerView;
use crate::core::model::{
    NewScreen, PreviewAfterTake, Rect, ScreenDoc, ScreenLayer, ScreenLayerKind, TakeTransition,
    TakeTransitionKind, TemplateSource, UiState, VariantTransition,
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
    tmp: tempfile::TempDir,
    st: AppState,
    dir: PathBuf,
    screen: String,
}

/// A project with a screen `Main` holding layer `l1` (classic-bar with a second variant
/// `compact`), Studio mode off.
fn env() -> Env {
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
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    let t = st
        .templates()
        .create(
            &dir,
            "Bar",
            &TemplateSource::Preset {
                preset_id: "classic-bar".into(),
            },
        )
        .unwrap();
    let d = st
        .templates()
        .create_variant(&dir, &t.id, "default", "Compact")
        .unwrap();
    st.template_saved(&d);
    let d = st
        .templates()
        .save_variant(&dir, &t.id, "compact", "<p>{{name}}</p>", "", Fields::new())
        .unwrap();
    st.template_saved(&d);
    let mut doc: ScreenDoc = st
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
            template_id: t.id.clone(),
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
    Env {
        tmp,
        st,
        dir,
        screen,
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

impl Env {
    fn program(&self) -> LiveLayerView {
        self.st.live().layer_view(&self.screen, "l1").unwrap()
    }

    fn preview(&self) -> PreviewView {
        self.st.preview_view(&self.screen).unwrap()
    }

    fn cue(&self, visible: Option<bool>) -> AppResult<PreviewView> {
        self.st.apply_studio_op(
            &self.screen,
            StudioOp::Cue {
                layer_id: "l1".into(),
                visible,
            },
        )
    }

    fn take(&self) -> PreviewView {
        self.st
            .apply_studio_op(&self.screen, StudioOp::Take { transition: None })
            .unwrap()
    }
}

#[test]
fn studio_ops_need_studio_mode() {
    let e = env();
    assert!(matches!(e.cue(Some(true)), Err(AppError::Invalid(_))));
    e.st.set_studio(studio(true), None).unwrap();
    assert!(e.cue(Some(true)).is_ok());
    assert!(matches!(
        e.st.apply_studio_op("__preview", StudioOp::Clear {}),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.st.apply_studio_op("nope", StudioOp::Clear {}),
        Err(AppError::NotFound(_))
    ));
    e.st.close_project();
    assert!(matches!(e.cue(Some(true)), Err(AppError::NoProject)));
}

#[test]
fn control_edits_stay_off_program_until_the_take() {
    let e = env();
    e.st.set_studio(studio(true), None).unwrap();
    e.st.live().show(&e.screen, "l1").unwrap();
    let n0 = e.st.live().studio_op_count();
    // A field edit (saved to the screen, staged in Preview).
    let f: Fields = [("name".to_string(), "Ada".to_string())].into();
    e.st.set_layer_fields(&e.screen, "l1", f).unwrap();
    assert!(
        e.st.live().studio_op_count() > n0,
        "through the entry point"
    );
    assert_eq!(e.program().fields["name"], "", "Program unchanged");
    assert!(!e.program().pending);
    assert_eq!(e.preview().layers[0].fields["name"], "Ada");
    let ScreenLayerKind::Template { fields, .. } =
        &e.st.get_screen(&e.screen).unwrap().doc.meta.layers[0].kind
    else {
        unreachable!()
    };
    assert_eq!(fields["name"], "Ada", "saved to the screen");
    // A variant switch and a drag stay off Program too.
    e.st.set_layer_variant(&e.screen, "l1", "compact", VariantTransition::Crossfade)
        .unwrap();
    let rect = Rect {
        x: 10,
        y: 20,
        w: 640,
        h: 360,
    };
    e.st.set_layer_rect(&e.screen, "l1", rect).unwrap();
    assert_eq!(e.program().variant, "default");
    let pv = e.preview();
    assert_eq!(pv.layers[0].variant, "compact");
    assert!(pv.cues[0].staged);
    let prect = e.st.live().with_layer(&e.screen, "l1", |l| l.rect).unwrap();
    assert_ne!(prect, Some(rect));
    // TAKE: everything at once.
    let pv = e.take();
    assert!(pv.cues.is_empty());
    let p = e.program();
    assert_eq!(
        (p.fields["name"].as_str(), p.variant.as_str()),
        ("Ada", "compact")
    );
    assert_eq!(
        e.st.live().with_layer(&e.screen, "l1", |l| l.rect).unwrap(),
        Some(rect)
    );
}

#[test]
fn cue_in_and_out_then_take() {
    let e = env();
    e.st.set_studio(studio(true), None).unwrap();
    let pv = e.cue(Some(true)).unwrap();
    assert!(pv.layers[0].on_air && !e.program().on_air);
    e.take();
    assert!(e.program().on_air);
    e.cue(Some(false)).unwrap();
    assert!(e.program().on_air, "cued out: still on Program");
    e.take();
    assert!(!e.program().on_air);
    // Clear empties the diff.
    e.cue(Some(true)).unwrap();
    let pv = e.st.apply_studio_op(&e.screen, StudioOp::Clear {}).unwrap();
    assert!(pv.cues.is_empty() && !pv.layers[0].on_air);
}

#[test]
fn set_studio_exit_paths_and_settings_are_kept_by_set_ui_state() {
    let e = env();
    let mut s = studio(true);
    s.transition.ms = 50_000;
    e.st.set_studio(s, None).unwrap();
    let stored = e.st.ui_state().unwrap().control.studio.unwrap();
    assert_eq!(stored.transition.ms, 5000, "clamped");
    // The UI's own ui-state writes never change the Studio settings.
    let mut ui: UiState = e.st.ui_state().unwrap();
    ui.control.studio = None;
    ui.control.grid_columns = 3;
    let back = e.st.set_ui_state(ui).unwrap();
    assert!(back.control.studio.is_some_and(|s| s.enabled));
    assert_eq!(back.control.grid_columns, 3);

    e.cue(Some(true)).unwrap();
    let err = e.st.set_studio(studio(false), None).unwrap_err();
    assert!(
        matches!(&err, AppError::Invalid(m) if m == "Preview has changes"),
        "{err:?}"
    );
    assert!(e.st.studio_settings().enabled, "unchanged");
    e.st.set_studio(studio(false), Some(StudioExit::Take))
        .unwrap();
    assert!(!e.st.studio_settings().enabled && !e.st.live().is_studio());
    assert!(e.program().on_air, "taken on the way out");

    e.st.set_studio(studio(true), None).unwrap();
    e.cue(Some(false)).unwrap();
    e.st.set_studio(studio(false), Some(StudioExit::Discard))
        .unwrap();
    assert!(e.program().on_air, "discarded");

    // Keep staged: an on-air layer's staged edit becomes "Changes on next take".
    e.st.set_studio(studio(true), None).unwrap();
    let f: Fields = [("name".to_string(), "Next".to_string())].into();
    e.st.set_layer_fields(&e.screen, "l1", f).unwrap();
    e.st.set_studio(studio(false), Some(StudioExit::KeepStaged))
        .unwrap();
    let p = e.program();
    assert!(p.pending && p.fields["name"] == "Next");
}

#[test]
fn the_preview_survives_a_reopen() {
    let e = env();
    e.st.set_studio(studio(true), None).unwrap();
    e.cue(Some(true)).unwrap();
    let cues = e.st.ui_state().unwrap().control.studio_preview;
    assert!(cues[&e.screen]["l1"], "kept in ui-state");
    e.st.close_project();
    let raw = std::fs::read_to_string(e.dir.join("ui-state.json")).unwrap();
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["control"]["studio"]["enabled"], true);
    assert_eq!(v["control"]["studioPreview"][&e.screen]["l1"], true);
    e.st.open_project(&e.dir).unwrap();
    assert!(e.st.live().is_studio());
    let pv = e.preview();
    assert_eq!(pv.cues.len(), 1);
    assert!(pv.layers[0].on_air && !e.program().on_air);
    // A take empties it in the file too (coalesced write on the next flush).
    e.take();
    e.st.flush_all();
    let raw = std::fs::read_to_string(e.dir.join("ui-state.json")).unwrap();
    assert!(!raw.contains("studioPreview"), "{raw}");
    let _ = &e.tmp;
}

#[test]
fn old_ui_state_files_load_with_studio_off() {
    let ui: UiState = serde_json::from_str(r#"{"control":{"liveEdit":"onTake"}}"#).unwrap();
    assert!(ui.control.studio.is_none());
    assert!(ui.control.studio_preview.is_empty());
    let back = serde_json::to_value(&ui).unwrap();
    assert!(back["control"].get("studio").is_none());
    assert!(back["control"].get("studioPreview").is_none());
}

#[test]
fn studio_ops_and_settings_parse_as_the_ui_sends_them() {
    let ops = [
        r#"{"op":"cue","layerId":"l1","visible":true}"#,
        r#"{"op":"cue","layerId":"l1","visible":null}"#,
        r#"{"op":"take"}"#,
        r#"{"op":"take","transition":null}"#,
        r#"{"op":"take","transition":{"kind":"outIn","ms":800}}"#,
        r#"{"op":"tbarBegin","transition":{"kind":"crossfade","ms":300}}"#,
        r#"{"op":"tbarSet","p":0.5}"#,
        r#"{"op":"tbarEnd","commit":false}"#,
        r#"{"op":"clear"}"#,
    ];
    for json in ops {
        let op: StudioOp = serde_json::from_str(json).unwrap_or_else(|e| panic!("{json}: {e}"));
        let back = serde_json::to_value(&op).unwrap();
        assert_eq!(
            back["op"],
            serde_json::from_str::<Value>(json).unwrap()["op"]
        );
    }
    let s: StudioSettings = serde_json::from_str(
        r#"{"enabled":true,"previewAfterTake":"clear","transition":{"kind":"cut","ms":100}}"#,
    )
    .unwrap();
    assert_eq!(s.preview_after_take, PreviewAfterTake::Clear);
    assert_eq!(s.transition.kind, TakeTransitionKind::Cut);
    let exit: Option<StudioExit> = serde_json::from_str(r#""keepStaged""#).unwrap();
    assert_eq!(exit, Some(StudioExit::KeepStaged));
}

#[test]
fn anchored_layers_stay_off_program_until_the_take() {
    use crate::core::model::{AnchorH, AnchorV, LayerAnchor};
    let e = env();
    e.st.set_studio(studio(true), None).unwrap();
    let mut doc = e.st.get_screen(&e.screen).unwrap().doc;
    doc.meta.layers[0].rect = Rect {
        x: 0,
        y: 0,
        w: 400,
        h: 100,
    };
    doc.meta.layers[0].anchor = Some(LayerAnchor {
        h: Some(AnchorH::Right),
        v: Some(AnchorV::Bottom),
        safe: true,
    });
    e.st.live().show(&e.screen, "l1").unwrap();
    let before = e.st.live().with_layer(&e.screen, "l1", |l| l.rect).unwrap();
    let saved = e.st.save_screen(doc).unwrap().doc;
    let placed = Rect {
        x: 1920 - 96 - 400,
        y: 1080 - 54 - 100,
        w: 400,
        h: 100,
    };
    assert_eq!(saved.meta.layers[0].rect, placed);
    assert_eq!(
        e.st.live().with_layer(&e.screen, "l1", |l| l.rect).unwrap(),
        before,
        "Program unchanged"
    );
    assert!(e.preview().cues[0].staged);
    e.take();
    assert_eq!(
        e.st.live().with_layer(&e.screen, "l1", |l| l.rect).unwrap(),
        Some(placed)
    );
}
