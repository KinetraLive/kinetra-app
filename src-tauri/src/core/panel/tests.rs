//! `panel.json`: validation, the store (save, events, flush), the watcher, backups and the
//! settings export; missing targets; pressing a button (steps in order, the first failure
//! stops it) and its `on_air`.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::*;
use crate::core::events::EventSink;
use crate::core::model::{
    BackupReason, ColumnKind, NewScreen, PanelColor, Rect, ScreenLayer, SettingsSection,
    SourceBinding, SourceColumn, SourceKind, TemplateSource,
};
use crate::core::protocol::Fields;
use crate::core::settings_io;

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Value)>>);

impl EventSink for Recorder {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

impl Recorder {
    fn named(&self, name: &str) -> Vec<Value> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

struct Env {
    tmp: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
    st: AppState,
    rec: Arc<Recorder>,
    dir: PathBuf,
    screen: String,
    source: String,
}

/// A project with screen "Main": `l1` (lower third, `title` bound to the source "Speakers")
/// and `l2` (a lower third with nothing bound).
fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        rec.clone(),
        Duration::from_millis(20),
    )
    .unwrap();
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
    let mut src = st.create_source("Speakers").unwrap();
    src.meta.kind = SourceKind::Text {
        columns: vec![SourceColumn {
            key: "name".into(),
            label: "Name".into(),
            kind: ColumnKind::Text,
        }],
        rows: ["Ingrid Vale", "Kari Nordmann"]
            .iter()
            .map(|n| Fields::from([("name".to_string(), n.to_string())]))
            .collect(),
        index: 0,
        auto: None,
    };
    let source = st.save_source(src).unwrap().id;
    let mut doc = st
        .create_screen(&NewScreen {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap()
        .doc;
    let layer = |id: &str, bound: bool| ScreenLayer {
        id: id.into(),
        name: format!("Layer {id}"),
        rect: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        kind: ScreenLayerKind::Template {
            template_id: lt.clone(),
            fields: Fields::new(),
            vars: Fields::new(),
            bindings: if bound {
                [(
                    "title".to_string(),
                    SourceBinding {
                        source_id: source.clone(),
                        column: "name".into(),
                        sort: None,
                        limit: None,
                        row: None,
                    },
                )]
                .into()
            } else {
                Default::default()
            },
            variant: None,
            sizing: None,
            opacity: None,
        },
        anchor: None,
    };
    doc.meta.layers.push(layer("l1", true));
    doc.meta.layers.push(layer("l2", false));
    let screen = st.save_screen(doc).unwrap().doc.id;
    Env {
        tmp,
        _rt: rt,
        st,
        rec,
        dir,
        screen,
        source,
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn button(id: &str, actions: Vec<PanelAction>) -> PanelButton {
    PanelButton {
        id: id.into(),
        label: format!("Button {id}"),
        color: PanelColor::Blue,
        confirm: false,
        actions,
    }
}

impl Env {
    fn take(&self, layer: &str) -> PanelAction {
        PanelAction::Take {
            screen: self.screen.clone(),
            layer: layer.into(),
        }
    }

    fn set_name(&self, layer: &str, name: &str) -> PanelAction {
        PanelAction::SetFields {
            screen: self.screen.clone(),
            layer: layer.into(),
            values: Fields::from([("name".to_string(), name.to_string())]),
        }
    }

    fn save(&self, buttons: Vec<PanelButton>) -> PanelView {
        self.st.save_panel(PanelDoc { buttons }).unwrap()
    }

    fn name(&self, layer: &str) -> String {
        self.st
            .live()
            .layer_view(&self.screen, layer)
            .unwrap()
            .fields
            .get("name")
            .cloned()
            .unwrap_or_default()
    }
}

#[test]
fn validation_refuses_what_the_protocol_doesnt_allow() {
    let e = env();
    let ok = button("b1", vec![e.take("l1")]);
    assert!(validate(&PanelDoc {
        buttons: vec![ok.clone()]
    })
    .is_ok());
    let bad = |b: PanelButton| validate(&PanelDoc { buttons: vec![b] }).is_err();
    assert!(bad(PanelButton {
        label: "  ".into(),
        ..ok.clone()
    }));
    assert!(bad(PanelButton {
        label: "x".repeat(41),
        ..ok.clone()
    }));
    assert!(bad(PanelButton {
        actions: vec![],
        ..ok.clone()
    }));
    assert!(bad(PanelButton {
        actions: vec![e.take("l1"); 9],
        ..ok.clone()
    }));
    assert!(bad(PanelButton {
        id: "no spaces".into(),
        ..ok.clone()
    }));
    assert!(validate(&PanelDoc {
        buttons: vec![ok.clone(), ok.clone()]
    })
    .is_err());
    assert!(validate(&PanelDoc {
        buttons: (0..49)
            .map(|i| button(&format!("b{i}"), vec![e.take("l1")]))
            .collect()
    })
    .is_err());
    // A loaded file keeps what's valid.
    let kept = sanitize(PanelDoc {
        buttons: vec![
            ok.clone(),
            PanelButton {
                label: String::new(),
                ..button("b2", vec![e.take("l1")])
            },
            ok,
        ],
    });
    assert_eq!(kept.buttons.len(), 1);
}

#[test]
fn saved_announced_flushed_and_backed_up() {
    let e = env();
    let view = e.save(vec![button("b1", vec![e.take("l1")])]);
    assert!(view.issues.is_empty(), "{:?}", view.issues);
    assert_eq!(
        e.rec.named(names::PANEL_CHANGED).last().unwrap()["doc"]["buttons"][0]["id"],
        "b1"
    );
    e.st.flush_all();
    let path = e.dir.join(PANEL_FILE);
    let v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(v["schemaVersion"], 1);
    assert_eq!(
        v["buttons"][0]["actions"][0],
        json!({"kind": "take", "screen": e.screen, "layer": "l1"})
    );

    // Backups carry it.
    let info = e.st.create_snapshot(BackupReason::Manual).unwrap();
    e.save(vec![]);
    e.st.restore_backup(&info.id).unwrap();
    assert_eq!(e.st.get_panel().unwrap().doc.buttons.len(), 1);
}

#[test]
fn the_watcher_reloads_external_edits_and_keeps_the_last_good_state() {
    let e = env();
    e.save(vec![button("b1", vec![e.take("l1")])]);
    e.st.flush_all();
    let path = e.dir.join(PANEL_FILE);
    wait_until("own write", || path.exists());
    let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    v["buttons"][0]["label"] = json!("Edited");
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    wait_until("reloaded", || {
        e.st.get_panel().unwrap().doc.buttons[0].label == "Edited"
    });
    // Invalid: the last good state stays.
    v["buttons"][0]["label"] = json!("");
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(e.st.get_panel().unwrap().doc.buttons[0].label, "Edited");
    fs::remove_file(&path).unwrap();
    wait_until("removed", || {
        e.st.get_panel().unwrap().doc.buttons.is_empty()
    });
}

#[test]
fn settings_export_and_import_carry_the_panel() {
    let e = env();
    e.save(vec![button("b1", vec![e.take("l1")])]);
    let file = e.tmp.path().join("settings.json");
    settings_io::export(&e.st, &file).unwrap();
    let v: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(v["panel"]["buttons"][0]["id"], "b1");
    e.save(vec![]);
    let parsed = settings_io::read_file(&file).unwrap();
    settings_io::apply(&e.st, &parsed, &[SettingsSection::Panel]).unwrap();
    assert_eq!(e.st.get_panel().unwrap().doc.buttons[0].id, "b1");
}

#[test]
fn missing_targets_are_issues_never_failures() {
    let e = env();
    let rows = e.st.panel_rows().unwrap();
    let row = rows[0].rows[1].id.clone();
    assert_eq!(rows[0].rows[1].label, "Kari Nordmann");
    let view = e.save(vec![button(
        "b1",
        vec![
            e.take("l1"),
            e.take("gone"),
            e.set_name("l1", "x"),
            PanelAction::SetFields {
                screen: e.screen.clone(),
                layer: "l1".into(),
                values: Fields::from([("title".to_string(), "y".to_string())]),
            },
            PanelAction::SelectRow {
                source: e.source.clone(),
                row,
            },
            PanelAction::SelectRow {
                source: e.source.clone(),
                row: "000000000000".into(),
            },
            PanelAction::RunCue {
                flow: "nocue".into(),
            },
        ],
    )]);
    let steps: Vec<u32> = view.issues.iter().map(|i| i.step).collect();
    assert_eq!(steps, [2, 4, 6, 7], "{:?}", view.issues);
}

#[test]
fn a_press_runs_the_steps_in_order_and_stops_at_the_first_failure() {
    let e = env();
    let rows = e.st.panel_rows().unwrap();
    let kari = rows[0].rows[1].id.clone();
    e.save(vec![
        button(
            "show",
            vec![
                e.set_name("l2", "Ingrid"),
                PanelAction::SelectRow {
                    source: e.source.clone(),
                    row: kari,
                },
                e.take("l2"),
            ],
        ),
        button(
            "half",
            vec![
                e.set_name("l1", "Done"),
                e.take("gone"),
                e.set_name("l2", "Never"),
            ],
        ),
    ]);
    e.st.press_panel_button("show").unwrap();
    assert_eq!(e.name("l2"), "Ingrid");
    assert!(e.st.live().layer_view(&e.screen, "l2").unwrap().on_air);
    assert_eq!(
        e.st.live().layer_view(&e.screen, "l1").unwrap().fields["title"],
        "Kari Nordmann"
    );

    let f = e.st.press_panel_button("half").unwrap_err();
    assert_eq!((f.step, f.steps, f.code), (2, 3, "not_found"));
    assert_eq!(e.name("l1"), "Done", "steps before the failure stay done");
    assert_eq!(e.name("l2"), "Ingrid");

    let f = e.st.press_panel_button("nope").unwrap_err();
    assert_eq!(f.code, "not_found");
}

#[test]
fn on_air_follows_the_buttons_actions() {
    let e = env();
    let edit = button("edit", vec![e.set_name("l2", "x")]);
    let take = button("take", vec![e.take("l2")]);
    assert!(!e.st.panel_button_on_air(&edit));
    assert!(e.st.panel_button_on_air(&take));
    e.st.live().show(&e.screen, "l2").unwrap();
    assert!(e.st.panel_button_on_air(&edit));
    // A missing target: unsure, so on air.
    assert!(e
        .st
        .panel_button_on_air(&button("x", vec![e.set_name("gone", "x")])));
}
