//! Template data and automation through `AppState`: list fields and bindings, timers, countdown end, auto-advance and
//! cue flows (former macros). Commands run on the test thread (like the app's sync commands, off the runtime); the
//! automation tasks run on a runtime injected into the hub, as in the app.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::broadcast::Receiver;

use crate::core::events::{names, EventSink};
use crate::core::fieldvals;
use crate::core::live::FieldDef;
use crate::core::live_hub::HubMsg;
use crate::core::model::{
    AutoAdvance, AutoTransition, ListSort, LiveEdit, NewFlow, NewScreen, NodeKind, PauseMode, Rect,
    Retrigger, ScreenDoc, ScreenLayer, ScreenLayerKind, ScreenWarningCode, SourceBinding,
    SourceColumn, SourceDoc, SourceKind, SourceStep, TemplateDoc, TemplateSource, TimerAction,
    UiState,
};
use crate::core::protocol::{FieldType, Fields, TimerDir, TimerEnd};
use crate::core::state::AppState;
use crate::error::AppError;

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

    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    st: AppState,
    rec: Arc<Recorder>,
    dir: PathBuf,
}

/// An open project (backups off, content flusher slow) whose hub runs timers and automation on
/// its own runtime.
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
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    Env {
        _tmp: tmp,
        rt,
        st,
        rec,
        dir,
    }
}

fn row(pairs: &[(&str, &str)]) -> Fields {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn col(key: &str) -> SourceColumn {
    SourceColumn {
        key: key.into(),
        label: key.to_uppercase(),
        kind: Default::default(),
    }
}

fn field(key: &str, kind: FieldType) -> FieldDef {
    FieldDef {
        key: key.into(),
        label: key.to_uppercase(),
        kind,
        ..Default::default()
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
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

fn bind(source: &str, column: &str) -> SourceBinding {
    SourceBinding {
        source_id: source.into(),
        column: column.into(),
        sort: None,
        limit: None,
        row: None,
    }
}

impl Env {
    /// A template with fields `name` (text), `title` (text), `rows` (list: name, score),
    /// `clock` (clock, `HH:mm`) and `timer` (timer).
    fn template(&self) -> TemplateDoc {
        let mut doc = self
            .st
            .templates()
            .create(&self.dir, "Board", &TemplateSource::Blank)
            .unwrap();
        let mut rows = field("rows", FieldType::List);
        rows.columns = vec![col("name"), col("score")];
        let mut clock = field("clock", FieldType::Clock);
        clock.format = Some("HH:mm".into());
        doc.meta.fields = vec![
            field("name", FieldType::Text),
            field("title", FieldType::Text),
            rows,
            clock,
            field("timer", FieldType::Timer),
        ];
        doc.variants[0].html = "<div><b>{{name}}</b><i>{{title}}</i><ol><li data-each=\"rows\">{{.name}}</li></ol></div>".into();
        self.st.templates().save(&self.dir, doc).unwrap()
    }

    /// A text source with columns `name`, `score`, `team`.
    fn source(&self, rows: &[(&str, &str)]) -> SourceDoc {
        let mut doc = self.st.create_source("Players").unwrap();
        doc.meta.kind = SourceKind::Text {
            columns: vec![col("name"), col("score"), col("team")],
            rows: rows
                .iter()
                .map(|(n, s)| row(&[("name", n), ("score", s)]))
                .collect(),
            index: 0,
            auto: None,
        };
        self.st.save_source(doc).unwrap()
    }

    /// A screen with one layer `l1` of template `t`, with the given bindings.
    fn screen(&self, t: &str, bindings: BTreeMap<String, SourceBinding>) -> ScreenDoc {
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
        doc.meta.layers.push(ScreenLayer {
            id: "l1".into(),
            name: "Board".into(),
            rect: Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            kind: ScreenLayerKind::Template {
                template_id: t.into(),
                fields: row(&[("name", "Own"), ("title", "Own title")]),
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

    fn set_mode(&self, mode: LiveEdit) {
        let mut ui: UiState = self.st.ui_state().unwrap();
        ui.control.live_edit = mode;
        self.st.set_ui_state(ui).unwrap();
    }

    fn field(&self, screen: &str, key: &str) -> String {
        self.st
            .live()
            .with_layer(screen, "l1", |l| l.fields[key].clone())
            .unwrap()
    }

    fn on_air(&self, screen: &str) -> bool {
        self.st.live().is_on_air(screen, "l1")
    }

    fn index(&self, source: &str) -> u32 {
        self.st.get_source(source).unwrap().meta.kind.index()
    }

    fn auto(&self, source: &str, looped: bool, transition: AutoTransition) {
        self.st
            .set_auto_advance(
                source,
                Some(AutoAdvance {
                    every_ms: 1000,
                    looped,
                    transition,
                    shuffle: false,
                    paused: false,
                    item_durations: false,
                }),
            )
            .unwrap();
    }
}

trait KindExt {
    fn index(&self) -> u32;
}

impl KindExt for SourceKind {
    fn index(&self) -> u32 {
        let SourceKind::Text { index, .. } = self else {
            panic!("not a text source")
        };
        *index
    }
}

fn names_of(list_json: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(list_json).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect()
}

// ---- List, clock and timer values on screens ---------------------------------------------------

#[test]
fn template_field_definitions_are_validated() {
    let e = env();
    let mut t = e.template();
    let saved = e.st.templates().get(&e.dir, &t.id).unwrap();
    assert_eq!(saved.meta.fields[2].columns.len(), 2);
    assert_eq!(saved.meta.fields[3].format.as_deref(), Some("HH:mm"));
    t.meta.fields[2].columns.clear();
    assert!(
        e.st.templates().save(&e.dir, t.clone()).is_err(),
        "a list needs columns"
    );
    t.meta.fields[2].columns = vec![col("name")];
    t.meta.fields[2].default = r#"[{"other":"x"}]"#.into();
    assert!(
        e.st.templates().save(&e.dir, t.clone()).is_err(),
        "default must be valid"
    );
    t.meta.fields[2].default = r#"[{"name":"x"}]"#.into();
    t.meta.fields[3].format = Some("YYYY".into());
    assert!(
        e.st.templates().save(&e.dir, t.clone()).is_err(),
        "clock format"
    );
    t.meta.fields[3].format = None;
    t.meta.fields[4].default = "{".into();
    assert!(
        e.st.templates().save(&e.dir, t.clone()).is_err(),
        "timer default"
    );
    t.meta.fields[4].default = r#"{"run":false,"base":60000,"dir":"down"}"#.into();
    e.st.templates().save(&e.dir, t).unwrap();
}

#[test]
fn plan_sends_canonical_values_per_type() {
    let e = env();
    let t = e.template();
    let s = e.screen(&t.id, BTreeMap::new());
    // Defaults: an empty list, the clock's format, the default timer state.
    assert_eq!(e.field(&s.id, "rows"), "[]");
    assert_eq!(e.field(&s.id, "clock"), "HH:mm");
    assert_eq!(
        e.field(&s.id, "timer"),
        r#"{"run":false,"at":0,"base":0,"dir":"up","end":"stop"}"#
    );
    // Own values: canonicalized; unknown cells dropped with a soft warning; invalid → default.
    let mut doc = s.clone();
    let ScreenLayerKind::Template { fields, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    fields.insert("rows".into(), r#"[{"score":1,"name":"A","x":"y"}]"#.into());
    fields.insert("clock".into(), "ignored".into());
    fields.insert("timer".into(), "garbage".into());
    let state = e.st.save_screen(doc).unwrap();
    assert_eq!(e.field(&s.id, "rows"), r#"[{"name":"A","score":"1"}]"#);
    assert_eq!(e.field(&s.id, "clock"), "HH:mm");
    assert!(e.field(&s.id, "timer").starts_with(r#"{"run":false"#));
    let codes: Vec<_> = state.warnings.iter().map(|w| w.code).collect();
    assert_eq!(
        codes,
        [
            ScreenWarningCode::InvalidValue,
            ScreenWarningCode::InvalidValue
        ],
        "dropped cells + invalid timer"
    );
}

#[test]
fn set_layer_fields_validates_and_canonicalizes_lists_and_timers() {
    let e = env();
    let t = e.template();
    let s = e.screen(&t.id, BTreeMap::new());
    let set = |k: &str, v: &str| e.st.set_layer_fields(&s.id, "l1", row(&[(k, v)]));
    set("rows", r#"[ {"score":"9", "name":"Ada"} ]"#).unwrap();
    assert_eq!(e.field(&s.id, "rows"), r#"[{"name":"Ada","score":"9"}]"#);
    let doc = e.st.get_screen(&s.id).unwrap().doc;
    let ScreenLayerKind::Template { fields, .. } = &doc.meta.layers[0].kind else {
        unreachable!()
    };
    assert_eq!(
        fields["rows"], r#"[{"name":"Ada","score":"9"}]"#,
        "stored canonically"
    );
    for bad in [r#"[{"nope":"x"}]"#, "{}", "[1]", "x"] {
        assert!(
            matches!(set("rows", bad), Err(AppError::Invalid(_))),
            "{bad}"
        );
    }
    let many = format!("[{}]", vec!["{}"; 101].join(","));
    assert!(matches!(set("rows", &many), Err(AppError::Invalid(_))));
    assert!(matches!(set("clock", "12:00"), Err(AppError::Invalid(_))));
    assert!(matches!(set("timer", "[]"), Err(AppError::Invalid(_))));
    set("timer", r#"{"run":false,"base":5000}"#).unwrap();
    assert_eq!(
        e.field(&s.id, "timer"),
        r#"{"run":false,"at":0,"base":5000,"dir":"up","end":"stop"}"#
    );
}

#[test]
fn list_bindings_take_whole_sources_sorted_and_limited() {
    let e = env();
    let t = e.template();
    let src = e.source(&[("ada", "10"), ("bob", "x"), ("cy", "30"), ("dee", "20")]);
    let mut b = BTreeMap::new();
    b.insert(
        "rows".to_string(),
        SourceBinding {
            sort: Some(ListSort {
                column: "score".into(),
                numeric: true,
                desc: true,
            }),
            limit: Some(3),
            ..bind(&src.id, "")
        },
    );
    let s = e.screen(&t.id, b);
    assert_eq!(names_of(&e.field(&s.id, "rows")), ["cy", "dee", "ada"]);
    assert!(e.st.get_screen(&s.id).unwrap().warnings.is_empty());

    // A source edit re-applies the list (off air: at once).
    let mut doc = e.st.get_source(&src.id).unwrap();
    let SourceKind::Text { rows, .. } = &mut doc.meta.kind else {
        panic!("not a text source")
    };
    rows.push(row(&[("name", "eve"), ("score", "99")]));
    e.st.save_source(doc).unwrap();
    assert_eq!(names_of(&e.field(&s.id, "rows")), ["eve", "cy", "dee"]);
    // On air under onTake: staged, applied on the next take.
    e.set_mode(LiveEdit::OnTake);
    e.st.live().show(&s.id, "l1").unwrap();
    let mut doc = e.st.get_source(&src.id).unwrap();
    let SourceKind::Text { rows, .. } = &mut doc.meta.kind else {
        panic!("not a text source")
    };
    rows.retain(|r| r["name"] != "eve");
    e.st.save_source(doc).unwrap();
    assert_eq!(names_of(&e.field(&s.id, "rows")), ["eve", "cy", "dee"]);
    assert!(e.st.live().layer_view(&s.id, "l1").unwrap().pending);
    e.st.live().show(&s.id, "l1").unwrap();
    assert_eq!(names_of(&e.field(&s.id, "rows")), ["cy", "dee", "ada"]);

    // Hard validation of the binding shape.
    let mut doc = e.st.get_screen(&s.id).unwrap().doc;
    let ScreenLayerKind::Template { bindings, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    bindings.get_mut("rows").unwrap().limit = Some(0);
    assert!(matches!(
        e.st.save_screen(doc.clone()),
        Err(AppError::Invalid(_))
    ));
    let ScreenLayerKind::Template { bindings, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    bindings.get_mut("rows").unwrap().limit = Some(101);
    assert!(matches!(
        e.st.save_screen(doc.clone()),
        Err(AppError::Invalid(_))
    ));
    let ScreenLayerKind::Template { bindings, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    let r = bindings.get_mut("rows").unwrap();
    r.limit = None;
    r.sort.as_mut().unwrap().column = "Bad Key".into();
    assert!(matches!(e.st.save_screen(doc), Err(AppError::Invalid(_))));
}

#[test]
fn binding_misuse_is_a_soft_warning() {
    let e = env();
    let t = e.template();
    let src = e.source(&[("ada", "10")]);
    let mut b = BTreeMap::new();
    b.insert("rows".to_string(), bind(&src.id, "name")); // a column on a list field
    b.insert("name".to_string(), bind(&src.id, "")); // no column on a text field
    b.insert("clock".to_string(), bind(&src.id, "name")); // clocks cannot be bound
    let s = e.screen(&t.id, b);
    let warnings = e.st.get_screen(&s.id).unwrap().warnings;
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(warnings
        .iter()
        .all(|w| w.code == ScreenWarningCode::InvalidValue));
    // The bindings are ignored: own values / defaults apply.
    assert_eq!(e.field(&s.id, "rows"), "[]");
    assert_eq!(e.field(&s.id, "name"), "Own");
    assert_eq!(e.field(&s.id, "clock"), "HH:mm");
}

#[test]
fn preview_samples_per_type() {
    let e = env();
    let t = e.template();
    let v = crate::core::preview::label_values(&t.meta.fields);
    assert_eq!(v["name"], "NAME");
    assert_eq!(names_of(&v["rows"]), ["NAME 1", "NAME 2", "NAME 3"]);
    assert_eq!(v["clock"], "HH:mm");
    assert_eq!(v["timer"], fieldvals::wire_default(&t.meta.fields[4]));
}

// ---- Timers -------------------------------------------------------------------------------------

fn timer_of(e: &Env, screen: &str) -> Value {
    serde_json::from_str(&e.field(screen, "timer")).unwrap()
}

#[test]
fn set_timer_math_saves_and_sends_one_fields_message() {
    let e = env();
    let t = e.template();
    let s = e.screen(&t.id, BTreeMap::new());
    let mut rx = e.st.live().subscribe();
    let set = |a: TimerAction, now: u64| e.st.set_timer_at(&s.id, "l1", "timer", a, now);

    set(TimerAction::Set { ms: 90_000 }, 1_000_000).unwrap();
    let start = set(TimerAction::Start, 1_000_000).unwrap();
    assert_eq!(start.fields["timer"], e.field(&s.id, "timer"));
    let v = timer_of(&e, &s.id);
    assert_eq!(
        (v["run"].as_bool(), v["base"].as_u64()),
        (Some(true), Some(90_000))
    );
    assert_eq!(v["at"], 1_000_000);
    let msgs = drain(&mut rx, &s.id);
    assert_eq!(msgs.len(), 2, "one fields message per action: {msgs:?}");
    assert!(msgs.iter().all(|m| m["t"] == "fields"));
    assert_eq!(msgs[1]["fields"].as_object().unwrap().len(), 1);

    // Start while running: nothing.
    set(TimerAction::Start, 1_005_000).unwrap();
    assert!(drain(&mut rx, &s.id).is_empty());
    // Pause 2.5 s later: up adds the elapsed time.
    set(TimerAction::Pause, 1_002_500).unwrap();
    let v = timer_of(&e, &s.id);
    assert_eq!(
        (v["run"].as_bool(), v["base"].as_u64()),
        (Some(false), Some(92_500))
    );
    // Configure, reset.
    set(
        TimerAction::Configure {
            dir: TimerDir::Down,
            end: TimerEnd::Hide,
        },
        1_003_000,
    )
    .unwrap();
    assert_eq!(timer_of(&e, &s.id)["dir"], "down");
    set(TimerAction::Reset, 1_004_000).unwrap();
    assert_eq!(
        e.field(&s.id, "timer"),
        r#"{"run":false,"at":0,"base":0,"dir":"up","end":"stop"}"#
    );

    // Saved to the screen (in memory now, written coalesced).
    assert!(e.st.screens().is_dirty(&s.id));
    e.st.flush_all();
    let file: Value = serde_json::from_str(
        &fs::read_to_string(e.dir.join("screens").join(format!("{}.json", s.id))).unwrap(),
    )
    .unwrap();
    assert_eq!(
        file["layers"][0]["fields"]["timer"],
        e.field(&s.id, "timer")
    );

    // Not a timer field / bound / unknown targets.
    assert!(matches!(
        e.st.set_timer(&s.id, "l1", "name", TimerAction::Start),
        Err(AppError::Invalid(_))
    ));
    assert!(matches!(
        e.st.set_timer(&s.id, "nope", "timer", TimerAction::Start),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.st.set_timer("nope", "l1", "timer", TimerAction::Start),
        Err(AppError::NotFound(_))
    ));
}

#[test]
fn set_timer_goes_live_under_on_take_while_other_changes_stay_staged() {
    let e = env();
    let t = e.template();
    let s = e.screen(&t.id, BTreeMap::new());
    e.set_mode(LiveEdit::OnTake);
    e.st.live().show(&s.id, "l1").unwrap();
    e.st.set_layer_fields(&s.id, "l1", row(&[("title", "Next")]))
        .unwrap();
    assert!(e.st.live().layer_view(&s.id, "l1").unwrap().pending);
    let mut rx = e.st.live().subscribe();
    e.st.set_timer(&s.id, "l1", "timer", TimerAction::Start)
        .unwrap();
    let msgs = drain(&mut rx, &s.id);
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert_eq!(msgs[0]["t"], "fields");
    assert_eq!(
        msgs[0]["fields"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["timer"]
    );
    assert_eq!(timer_of(&e, &s.id)["run"], true, "on air now");
    assert_eq!(
        e.field(&s.id, "title"),
        "Own title",
        "the other edit is still staged"
    );
    let view = e.st.live().layer_view(&s.id, "l1").unwrap();
    assert!(view.pending);
    assert_eq!(view.fields["title"], "Next");
    assert_eq!(
        view.fields["timer"],
        e.field(&s.id, "timer"),
        "the staged copy has the timer too"
    );
    // A timer change alone does not leave anything staged.
    e.st.live().show(&s.id, "l1").unwrap();
    e.st.set_timer(&s.id, "l1", "timer", TimerAction::Pause)
        .unwrap();
    assert!(!e.st.live().layer_view(&s.id, "l1").unwrap().pending);
}

/// A countdown with end: hide, `ms` long, started now (the hide time is snapped to a whole
/// second, so it falls 0.5 to 1.5 s after `ms` from now for `ms` = 1000).
fn start_countdown(e: &Env, s: &str, ms: u64) {
    let set = |a| e.st.set_timer(s, "l1", "timer", a).unwrap();
    set(TimerAction::Configure {
        dir: TimerDir::Down,
        end: TimerEnd::Hide,
    });
    set(TimerAction::Set { ms });
    set(TimerAction::Start);
}

#[test]
fn countdown_end_hides_the_layer() {
    let e = env();
    let t = e.template();
    let s = e.screen(&t.id, BTreeMap::new());
    e.st.live().show(&s.id, "l1").unwrap();
    let mut rx = e.st.live().subscribe();
    start_countdown(&e, &s.id, 1000);
    assert_eq!(e.st.live().pending_countdowns(), 1);
    let at = timer_of(&e, &s.id)["at"].as_u64().unwrap() + 1000;
    assert_eq!(at % 1000, 0, "zero falls on a whole second");
    wait_until("countdown end", || !e.on_air(&s.id));
    assert!(
        crate::core::fsx::now_epoch_ms() + 50 >= at,
        "not before zero"
    );
    let msgs = drain(&mut rx, &s.id);
    assert!(msgs
        .iter()
        .any(|m| m["t"] == "state" && m["state"] == "out"));
    assert_eq!(e.st.live().pending_countdowns(), 0);
    // Shown again with the countdown at zero: it holds (nothing scheduled).
    e.st.live().show(&s.id, "l1").unwrap();
    assert_eq!(e.st.live().pending_countdowns(), 0);
    assert!(e.on_air(&s.id));
}

#[test]
fn countdown_end_is_cancelled_by_pause_reset_and_hide() {
    let e = env();
    let t = e.template();
    let s = e.screen(&t.id, BTreeMap::new());
    e.st.live().show(&s.id, "l1").unwrap();
    // Pause.
    start_countdown(&e, &s.id, 1000);
    assert_eq!(e.st.live().pending_countdowns(), 1);
    e.st.set_timer(&s.id, "l1", "timer", TimerAction::Pause)
        .unwrap();
    assert_eq!(e.st.live().pending_countdowns(), 0);
    // Reset.
    start_countdown(&e, &s.id, 1000);
    e.st.set_timer(&s.id, "l1", "timer", TimerAction::Reset)
        .unwrap();
    assert_eq!(e.st.live().pending_countdowns(), 0);
    // Hide; and off air nothing is scheduled.
    start_countdown(&e, &s.id, 1000);
    assert_eq!(e.st.live().pending_countdowns(), 1);
    e.st.live().hide(&s.id, "l1").unwrap();
    assert_eq!(e.st.live().pending_countdowns(), 0);
    e.st.set_timer(&s.id, "l1", "timer", TimerAction::Set { ms: 5000 })
        .unwrap();
    e.st.set_timer(&s.id, "l1", "timer", TimerAction::Start)
        .unwrap();
    assert_eq!(e.st.live().pending_countdowns(), 0, "off air");
    // A take schedules it; PANIC cancels it.
    e.st.live().show(&s.id, "l1").unwrap();
    assert_eq!(e.st.live().pending_countdowns(), 1);
    e.st.live_panic().unwrap();
    assert_eq!(e.st.live().pending_countdowns(), 0);
    // The paused/hidden layers stayed as they were.
    e.st.live().show(&s.id, "l1").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(e.on_air(&s.id));
}

// ---- Auto-advance -----------------------------------------------------------------------------

/// A source with rows a, b, c bound to `name` of a screen layer; fast test timing.
fn rotation(e: &Env) -> (SourceDoc, ScreenDoc) {
    e.st.automation()
        .set_test_timing(Some(Duration::from_millis(80)), Duration::from_millis(60));
    let t = e.template();
    let src = e.source(&[("a", "1"), ("b", "2"), ("c", "3")]);
    let mut b = BTreeMap::new();
    b.insert("name".to_string(), bind(&src.id, "name"));
    let s = e.screen(&t.id, b);
    (src, s)
}

fn source_events(e: &Env, id: &str) -> Vec<Value> {
    e.rec
        .named(names::SOURCE_CHANGED)
        .into_iter()
        .filter(|v| v["id"] == id)
        .collect()
}

#[test]
fn auto_advance_arms_only_while_a_bound_layer_is_on_air() {
    let e = env();
    let (src, s) = rotation(&e);
    e.rec.clear();
    let summary =
        e.st.set_auto_advance(
            &src.id,
            Some(AutoAdvance {
                every_ms: 1000,
                looped: true,
                transition: AutoTransition::Update,
                shuffle: false,
                paused: false,
                item_durations: false,
            }),
        )
        .unwrap();
    assert!(summary.auto.is_some());
    assert_eq!(summary.next_at, None, "nothing on air: not armed");
    assert_eq!(e.st.automation().task_count(), 0);
    let file: Value = serde_json::from_str(
        &fs::read_to_string(e.dir.join("sources").join(format!("{}.json", src.id))).unwrap(),
    )
    .unwrap();
    assert_eq!(
        file["auto"],
        json!({"everyMs": 1000, "loop": true, "transition": "update", "shuffle": false, "paused": false, "itemDurations": false})
    );
    assert!(source_events(&e, &src.id).is_empty());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(e.index(&src.id), 0, "no step while off air");

    // Validation.
    assert!(matches!(
        e.st.set_auto_advance(
            &src.id,
            Some(AutoAdvance {
                every_ms: 999,
                looped: false,
                transition: AutoTransition::Update,
                shuffle: false,
                paused: false,
                item_durations: false,
            })
        ),
        Err(AppError::Invalid(_))
    ));

    // On air: armed, steps, `source-changed` on arm and on each step.
    e.st.live().show(&s.id, "l1").unwrap();
    assert_eq!(e.st.automation().armed(), std::slice::from_ref(&src.id));
    let armed = source_events(&e, &src.id);
    assert_eq!(armed.len(), 1);
    assert!(armed[0]["nextAt"].as_u64().is_some());
    assert!(e.st.list_sources().unwrap()[0].next_at.is_some());
    wait_until("a step", || e.index(&src.id) == 1);
    wait_until("the bound field", || e.field(&s.id, "name") == "b");
    wait_until("source-changed for the step", || {
        source_events(&e, &src.id).iter().any(|v| v["index"] == 1)
    });

    // Off air: disarmed (one event with nextAt null), no task left, no more steps.
    e.st.live().hide(&s.id, "l1").unwrap();
    assert!(e.st.automation().armed().is_empty());
    assert_eq!(e.st.automation().task_count(), 0);
    let last = source_events(&e, &src.id).pop().unwrap();
    assert_eq!(last["nextAt"], Value::Null);
    let at = e.index(&src.id);
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(e.index(&src.id), at);

    // Turning it off while on air disarms too.
    e.st.live().show(&s.id, "l1").unwrap();
    assert_eq!(e.st.automation().task_count(), 1);
    let off = e.st.set_auto_advance(&src.id, None).unwrap();
    assert_eq!((off.auto, off.next_at), (None, None));
    assert_eq!(e.st.automation().task_count(), 0);
}

#[test]
fn update_steps_in_place_on_air_even_under_on_take() {
    let e = env();
    let (src, s) = rotation(&e);
    e.set_mode(LiveEdit::OnTake);
    e.st.live().show(&s.id, "l1").unwrap();
    // A staged Control edit stays staged through rotation.
    e.st.set_layer_fields(&s.id, "l1", row(&[("title", "Next")]))
        .unwrap();
    let mut rx = e.st.live().subscribe();
    e.auto(&src.id, false, AutoTransition::Update);
    wait_until("two steps", || e.index(&src.id) == 2);
    wait_until("disarmed at the last row", || {
        e.st.automation().armed().is_empty()
    });
    assert_eq!(e.field(&s.id, "name"), "c");
    let msgs = drain(&mut rx, &s.id);
    assert_eq!(
        msgs,
        [
            json!({"t": "fields", "layerId": "l1", "fields": {"name": "b"}}),
            json!({"t": "fields", "layerId": "l1", "fields": {"name": "c"}}),
        ],
        "one fields message per step, no animation"
    );
    assert_eq!(e.field(&s.id, "title"), "Own title");
    let view = e.st.live().layer_view(&s.id, "l1").unwrap();
    assert!(view.pending);
    assert_eq!(view.fields["title"], "Next");
    assert_eq!(view.fields["name"], "c");
    assert_eq!(e.st.automation().task_count(), 0, "stopped at the end");
    // Index writes coalesce like manual steps.
    assert!(e.st.sources().is_dirty(&src.id));
}

#[test]
fn loop_wraps_to_the_first_row() {
    let e = env();
    let (src, s) = rotation(&e);
    e.st.step_source(&src.id, SourceStep::Index { index: 2 })
        .unwrap();
    e.st.live().show(&s.id, "l1").unwrap();
    e.auto(&src.id, true, AutoTransition::Update);
    wait_until("wrap", || e.index(&src.id) == 0);
    assert_eq!(
        e.st.automation().armed(),
        std::slice::from_ref(&src.id),
        "still armed"
    );
    // Without loop at the last row: not armed at all.
    e.st.step_source(&src.id, SourceStep::Index { index: 2 })
        .unwrap();
    e.auto(&src.id, false, AutoTransition::Update);
    assert!(e.st.automation().armed().is_empty());
}

#[test]
fn retake_plays_out_steps_and_plays_in() {
    let e = env();
    let (src, s) = rotation(&e);
    e.st.live().show(&s.id, "l1").unwrap();
    let mut rx = e.st.live().subscribe();
    e.auto(&src.id, true, AutoTransition::Retake);
    wait_until("a retake", || e.index(&src.id) == 1 && e.on_air(&s.id));
    // Let the step's messages arrive (the layer may read as on air just before its `in` is
    // broadcast).
    let mut msgs: Vec<Value> = Vec::new();
    wait_until("the retake's messages", || {
        msgs.extend(drain(&mut rx, &s.id));
        msgs.len() >= 3
    });
    let first: Vec<&Value> = msgs.iter().take(3).collect();
    assert_eq!(
        first,
        [
            &json!({"t": "state", "layerId": "l1", "state": "out"}),
            &json!({"t": "fields", "layerId": "l1", "fields": {"name": "b"}}),
            &json!({"t": "state", "layerId": "l1", "state": "in"}),
        ],
        "out, then the new row, then in"
    );
    assert_eq!(
        e.st.automation().armed(),
        std::slice::from_ref(&src.id),
        "kept armed across the gap"
    );
    // An operator hide during the gap is respected: the layer is not brought back.
    e.st.set_auto_advance(&src.id, None).unwrap();
    e.st.automation()
        .set_test_timing(Some(Duration::from_millis(50)), Duration::from_millis(300));
    e.auto(&src.id, true, AutoTransition::Retake);
    wait_until("the out of the next retake", || !e.on_air(&s.id));
    // Taken in and out by hand during the gap.
    e.st.live().show(&s.id, "l1").unwrap();
    e.st.live().hide(&s.id, "l1").unwrap();
    std::thread::sleep(Duration::from_millis(450));
    assert!(!e.on_air(&s.id), "not re-taken after a manual change");
}

#[test]
fn panic_and_close_disarm_everything() {
    let e = env();
    let (src, s) = rotation(&e);
    e.st.live().show(&s.id, "l1").unwrap();
    e.auto(&src.id, true, AutoTransition::Update);
    assert_eq!(e.st.automation().task_count(), 1);
    e.st.live_panic().unwrap();
    assert_eq!(e.st.automation().task_count(), 0);
    let last = source_events(&e, &src.id).pop().unwrap();
    assert_eq!(last["nextAt"], Value::Null, "disarming is announced");
    let at = e.index(&src.id);
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(e.index(&src.id), at);
    // A new take re-arms (the setting stays on).
    e.st.live().show(&s.id, "l1").unwrap();
    assert_eq!(e.st.automation().task_count(), 1);
    e.st.close_project();
    assert_eq!(e.st.automation().task_count(), 0);
    assert_eq!(e.st.live().pending_countdowns(), 0);
    assert_eq!(e.st.live().pending_auto_outs(), 0);
    // Nothing runs on the runtime any more (the rotation task ended; the runtime reaps aborted
    // tasks shortly after).
    wait_until("no task left after close", || {
        e.rt.metrics().num_alive_tasks() == 0
    });
}

// ---- Cue flows (macros are flows) --------------------------------------------------------------

/// A Cue flow running `steps` in a line (what a migrated macro is). Returns (flow id, node ids
/// in order: the Cue first).
fn cue(e: &Env, name: &str, steps: Vec<NodeKind>) -> (String, Vec<String>) {
    let (nodes, edges) = crate::core::flows::linear(NodeKind::Cue { label: None }, steps);
    let ids = nodes.iter().map(|n| n.id.clone()).collect();
    let info =
        e.st.create_flow(NewFlow {
            name: name.into(),
            armed: true,
            retrigger: Retrigger::Restart,
            origin: None,
            nodes,
            edges,
        })
        .unwrap();
    (info.flow.id, ids)
}

fn flow_events(e: &Env, id: &str) -> Vec<Value> {
    e.rec
        .named(names::FLOW_STATE)
        .into_iter()
        .filter(|v| v["id"] == id)
        .collect()
}

fn ended(e: &Env, id: &str) -> bool {
    flow_events(e, id)
        .iter()
        .any(|v| v["state"] == "finished" || v["state"] == "failed")
}

fn delay(ms: u64) -> NodeKind {
    NodeKind::Delay { ms }
}

#[test]
fn a_cue_runs_its_steps_in_order_and_skips_missing_targets() {
    let e = env();
    let t = e.template();
    let src = e.source(&[("a", "1"), ("b", "2")]);
    let mut b = BTreeMap::new();
    b.insert("name".to_string(), bind(&src.id, "name"));
    let s = e.screen(&t.id, b);
    let (id, nodes) = cue(
        &e,
        "Open",
        vec![
            NodeKind::SetFields {
                screen_id: s.id.clone(),
                layer_id: "l1".into(),
                fields: row(&[("title", "From cue")]),
            },
            NodeKind::Take {
                screen_id: s.id.clone(),
                layer_id: "l1".into(),
                for_ms: None,
            },
            delay(60),
            NodeKind::StepSource {
                source_id: src.id.clone(),
                step: SourceStep::By { delta: 1 },
            },
            NodeKind::Take {
                screen_id: "gone".into(),
                layer_id: "l1".into(),
                for_ms: None,
            },
            NodeKind::StepSource {
                source_id: "gone".into(),
                step: SourceStep::By { delta: 1 },
            },
            NodeKind::Clear {
                screen_id: s.id.clone(),
                layer_id: "l1".into(),
            },
        ],
    );
    let mut rx = e.st.live().subscribe();
    e.rec.clear();
    let started = Instant::now();
    e.st.run_flow(&id, None).unwrap();
    wait_until("the cue", || ended(&e, &id));
    assert!(
        started.elapsed() >= Duration::from_millis(60),
        "the delay waited"
    );
    let ev = flow_events(&e, &id);
    let running: Vec<Value> = ev
        .iter()
        .filter(|v| v["state"] == "running")
        .map(|v| v["node"].clone())
        .collect();
    // The cue, then every action node (the delay is logic: no event of its own).
    let expected: Vec<Value> = [0, 1, 2, 4, 5, 6, 7]
        .iter()
        .map(|i| json!(nodes[*i]))
        .collect();
    assert_eq!(running, expected);
    let at = &ev[0]["at"];
    assert!(
        ev.iter()
            .filter(|v| v["state"] == "running")
            .all(|v| &v["at"] == at),
        "running carries the firing's lastFiredAt"
    );
    let last = ev.last().unwrap();
    assert_eq!(last["state"], "finished");
    let msg = last["message"].as_str().unwrap();
    assert_eq!(msg.matches("skipped").count(), 2, "{msg}");
    assert!(msg.contains(&nodes[5]) && msg.contains(&nodes[6]), "{msg}");
    assert!(!e.st.flow_engine().is_running(&id));
    assert_eq!(e.st.flow_engine().task_count(), 0);

    // The steps took effect through the command paths, in order.
    assert_eq!(e.index(&src.id), 1);
    assert_eq!(e.field(&s.id, "title"), "From cue");
    assert!(!e.on_air(&s.id));
    let states: Vec<Value> = drain(&mut rx, &s.id)
        .into_iter()
        .filter(|m| m["t"] == "state")
        .map(|m| m["state"].clone())
        .collect();
    assert_eq!(states, [json!("in"), json!("out")]);

    // A failing action (an unknown field is not a missing target) fails the run, but later
    // actions still run.
    let (bad, nodes) = cue(
        &e,
        "Bad",
        vec![
            NodeKind::SetFields {
                screen_id: s.id.clone(),
                layer_id: "l1".into(),
                fields: row(&[("nope", "x")]),
            },
            NodeKind::Take {
                screen_id: s.id.clone(),
                layer_id: "l1".into(),
                for_ms: None,
            },
        ],
    );
    e.st.run_flow(&bad, None).unwrap();
    wait_until("bad", || ended(&e, &bad));
    let last = flow_events(&e, &bad).pop().unwrap();
    assert_eq!(
        (&last["state"], &last["node"]),
        (&json!("failed"), &json!(nodes[1]))
    );
    assert!(e.on_air(&s.id), "carried on through done");
    assert!(matches!(
        e.st.run_flow("nope000001", None),
        Err(AppError::NotFound(_))
    ));
}

#[test]
fn cues_restart_stop_and_end_with_the_project() {
    let e = env();
    let (id, _) = cue(&e, "Long", vec![delay(400), delay(1)]);
    e.rec.clear();
    e.st.run_flow(&id, None).unwrap();
    wait_until("running", || !flow_events(&e, &id).is_empty());
    // Restart: the old run never reports again.
    e.st.run_flow(&id, None).unwrap();
    assert!(e.st.flow_engine().is_running(&id));
    assert_eq!(e.st.flow_engine().task_count(), 1);
    wait_until("finished", || ended(&e, &id));
    let ev = flow_events(&e, &id);
    let states: Vec<&str> = ev.iter().map(|v| v["state"].as_str().unwrap()).collect();
    assert_eq!(
        states,
        ["running", "stopped", "running", "finished"],
        "{ev:?}"
    );
    assert_eq!(ev[1]["message"], "Restarted");

    // Stop.
    e.rec.clear();
    e.st.run_flow(&id, None).unwrap();
    e.st.stop_flow(&id).unwrap();
    e.st.stop_flow(&id).unwrap();
    assert!(!e.st.flow_engine().is_running(&id));
    std::thread::sleep(Duration::from_millis(500));
    let states: Vec<String> = flow_events(&e, &id)
        .iter()
        .map(|v| v["state"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(states, ["running", "stopped"]);

    // PANIC stops cues too; closing the project stops them and leaves no task.
    e.st.run_flow(&id, None).unwrap();
    e.st.live_panic().unwrap();
    assert!(!e.st.flow_engine().is_running(&id));
    assert_eq!(flow_events(&e, &id).last().unwrap()["message"], "PANIC");
    e.st.run_flow(&id, None).unwrap();
    e.st.close_project();
    assert_eq!(e.st.flow_engine().task_count(), 0);
    wait_until("no task on the runtime", || {
        e.rt.metrics().num_alive_tasks() == 0
    });
    assert!(matches!(e.st.run_flow(&id, None), Err(AppError::NoProject)));
}

// ---- Slideshow settings ------------------------------------------------------------------

fn slideshow(e: &Env, rows: usize, auto: AutoAdvance) -> (SourceDoc, ScreenDoc) {
    let t = e.template();
    let names: Vec<String> = (0..rows).map(|i| format!("r{i}")).collect();
    let pairs: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "1")).collect();
    let src = e.source(&pairs);
    let mut b = BTreeMap::new();
    b.insert("name".to_string(), bind(&src.id, "name"));
    b.insert("rows".to_string(), bind(&src.id, ""));
    let s = e.screen(&t.id, b);
    e.st.set_auto_advance(&src.id, Some(auto)).unwrap();
    (src, s)
}

fn auto_settings(every_ms: u32) -> AutoAdvance {
    AutoAdvance {
        every_ms,
        looped: true,
        transition: AutoTransition::Update,
        shuffle: false,
        paused: false,
        item_durations: false,
    }
}

#[test]
fn lists_mark_the_current_row_and_its_duration() {
    let e = env();
    let (src, s) = slideshow(&e, 3, auto_settings(6400));
    let rows: Value = serde_json::from_str(&e.field(&s.id, "rows")).unwrap();
    assert_eq!(rows[0]["_current"], "true");
    assert!(rows[1].get("_current").is_none());
    assert_eq!(rows[0]["_duration"], "6", "whole seconds");
    e.st.step_source(&src.id, SourceStep::By { delta: 1 })
        .unwrap();
    let rows: Value = serde_json::from_str(&e.field(&s.id, "rows")).unwrap();
    assert!(rows[0].get("_current").is_none());
    assert_eq!(rows[1]["_current"], "true");
    // Item durations: a row's `duration` cell (seconds) wins, clamped.
    let mut a = auto_settings(6000);
    a.item_durations = true;
    let row_of = |d: &str| row(&[("duration", d)]);
    assert_eq!(
        crate::core::sources::row_duration_ms(&a, Some(&row_of("2.5"))),
        2500
    );
    assert_eq!(
        crate::core::sources::row_duration_ms(&a, Some(&row_of("x"))),
        6000
    );
    assert_eq!(
        crate::core::sources::row_duration_ms(&a, Some(&row_of("0.1"))),
        1000
    );
    assert_eq!(crate::core::sources::row_duration_ms(&a, None), 6000);
    a.item_durations = false;
    assert_eq!(
        crate::core::sources::row_duration_ms(&a, Some(&row_of("2.5"))),
        6000
    );
}

#[test]
fn paused_slideshows_are_not_armed() {
    let e = env();
    e.st.automation()
        .set_test_timing(Some(Duration::from_millis(60)), Duration::from_millis(40));
    let mut a = auto_settings(1000);
    a.paused = true;
    let (src, s) = slideshow(&e, 3, a);
    e.st.live().show(&s.id, "l1").unwrap();
    assert!(e.st.automation().armed().is_empty(), "paused");
    e.st.set_auto_paused(&src.id, false).unwrap();
    assert_eq!(e.st.automation().armed(), std::slice::from_ref(&src.id));
    wait_until("a step", || e.index(&src.id) != 0);
    e.st.set_auto_paused(&src.id, true).unwrap();
    assert!(e.st.automation().armed().is_empty());
    assert!(e.st.get_source(&src.id).unwrap().meta.kind.auto_paused());
}

trait PausedExt {
    fn auto_paused(&self) -> bool;
}

impl PausedExt for SourceKind {
    fn auto_paused(&self) -> bool {
        matches!(self, SourceKind::Text { auto: Some(a), .. } if a.paused)
    }
}

#[test]
fn shuffle_visits_every_row_once_per_cycle_without_a_seam_repeat() {
    let e = env();
    e.st.automation()
        .set_test_timing(Some(Duration::from_millis(25)), Duration::from_millis(20));
    let mut a = auto_settings(1000);
    a.shuffle = true;
    let (src, s) = slideshow(&e, 5, a);
    let mut rx = e.st.live().subscribe();
    e.st.live().show(&s.id, "l1").unwrap();
    // Collect the indexes the rotation visits (from the `name` field updates).
    let mut seen: Vec<u32> = vec![0];
    let deadline = Instant::now() + Duration::from_secs(8);
    while seen.len() < 16 {
        assert!(Instant::now() < deadline, "rotation too slow: {seen:?}");
        for m in drain(&mut rx, &s.id) {
            if let Some(n) = m["fields"]["name"].as_str() {
                seen.push(n.trim_start_matches('r').parse().unwrap());
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    e.st.live().hide(&s.id, "l1").unwrap();
    for w in seen.windows(2) {
        assert_ne!(w[0], w[1], "never the same row twice in a row: {seen:?}");
    }
    // Every cycle of 5 is a permutation.
    for cycle in seen[..15].chunks(5) {
        let mut c = cycle.to_vec();
        c.sort();
        assert_eq!(c, [0, 1, 2, 3, 4], "{seen:?}");
    }
    let _ = src;
}

#[test]
fn wraps_fire_slideshow_wrap_flows() {
    let e = env();
    e.st.automation()
        .set_test_timing(Some(Duration::from_millis(40)), Duration::from_millis(20));
    let (src, s) = slideshow(&e, 2, auto_settings(1000));
    let (nodes, edges) = crate::core::flows::linear(
        NodeKind::SlideshowWrap {
            source_id: src.id.clone(),
        },
        vec![NodeKind::PauseSource {
            source_id: src.id.clone(),
            mode: PauseMode::Pause,
        }],
    );
    let id =
        e.st.create_flow(NewFlow {
            name: "Pause after one cycle".into(),
            armed: true,
            retrigger: Retrigger::Restart,
            origin: None,
            nodes,
            edges,
        })
        .unwrap()
        .flow
        .id;
    e.st.live().show(&s.id, "l1").unwrap();
    wait_until("paused by the wrap flow", || {
        e.st.get_source(&src.id).unwrap().meta.kind.auto_paused()
    });
    assert_eq!(e.index(&src.id), 0, "wrapped to the first row");
    wait_until("disarmed", || e.st.automation().armed().is_empty());
    wait_until("finished", || ended(&e, &id));
    let last = flow_events(&e, &id).pop().unwrap();
    assert_eq!(last["state"], "finished");
    assert!(last["message"].is_null());
}
