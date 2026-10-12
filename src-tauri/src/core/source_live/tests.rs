//! Sources, bindings, Control field edits, live-changed and panic, through `AppState`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::broadcast::Receiver;

use crate::core::events::{names, EventSink};
use crate::core::live_hub::HubMsg;
use crate::core::model::{
    LiveEdit, NewScreen, Rect, ScreenDoc, ScreenLayer, ScreenLayerKind, ScreenWarningCode,
    SourceBinding, SourceColumn, SourceDoc, SourceKind, SourceStep, TemplateDoc, TemplateSource,
    UiState,
};
use crate::core::protocol::Fields;
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

    /// Takes the `live-changed` payloads recorded so far (other events stay).
    fn take_live(&self) -> Vec<Value> {
        let mut all = self.0.lock().unwrap();
        let (live, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *all)
            .into_iter()
            .partition(|(n, _)| n == names::LIVE_CHANGED);
        *all = rest;
        live.into_iter().map(|(_, v)| v).collect()
    }
}

struct Env {
    tmp: tempfile::TempDir,
    st: AppState,
    rec: Arc<Recorder>,
    dir: PathBuf,
}

/// An open project with backups off; the content flusher waits a minute unless a test shortens it,
/// so only explicit flushes write.
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
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    Env { tmp, st, rec, dir }
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

impl Env {
    /// A template from the classic-bar preset (fields `name`, `title`).
    fn template(&self) -> TemplateDoc {
        self.st
            .templates()
            .create(
                &self.dir,
                "Bar",
                &TemplateSource::Preset {
                    preset_id: "classic-bar".into(),
                },
            )
            .unwrap()
    }

    /// A text source with columns `name`, `title` and the given names as rows.
    fn source(&self, names: &[&str]) -> SourceDoc {
        let mut doc = self.st.create_source("Guests").unwrap();
        doc.meta.kind = SourceKind::Text {
            columns: vec![col("name"), col("title")],
            rows: names
                .iter()
                .map(|n| row(&[("name", n), ("title", &format!("{n} title"))]))
                .collect(),
            index: 0,
            auto: None,
        };
        self.st.save_source(doc).unwrap()
    }

    /// A screen with one layer `l1` of template `t`, field `name` bound to `source`'s column.
    fn bound_screen(&self, t: &str, source: &str, column: &str) -> ScreenDoc {
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
        let mut bindings = BTreeMap::new();
        bindings.insert(
            "name".to_string(),
            SourceBinding {
                source_id: source.into(),
                column: column.into(),
                sort: None,
                limit: None,
                row: None,
            },
        );
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
        self.st.live().layer_view(screen, "l1").unwrap().fields[key].clone()
    }

    fn source_file(&self, id: &str) -> Value {
        let path = self.dir.join("sources").join(format!("{id}.json"));
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn screen_file(&self, id: &str) -> Value {
        let path = self.dir.join("screens").join(format!("{id}.json"));
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }
}

fn next() -> SourceStep {
    SourceStep::By { delta: 1 }
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

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until the project watcher has evaluated a batch after `since` (a
/// [`AppState::watcher_batches`] count taken before the writes it should see): the positive
/// signal that it looked at them, instead of sleeping and hoping it did.
fn watcher_settled(e: &Env, since: u64) {
    wait_until("the watcher's batch", || e.st.watcher_batches() > since);
}

fn warning_codes(e: &Env, screen: &str) -> Vec<ScreenWarningCode> {
    e.st.get_screen(screen)
        .unwrap()
        .warnings
        .iter()
        .map(|w| w.code)
        .collect()
}

// ---- Sources ---------------------------------------------------------------------------------

#[test]
fn source_commands_need_a_project() {
    let e = env();
    e.st.close_project();
    assert!(matches!(e.st.list_sources(), Err(AppError::NoProject)));
    assert!(matches!(e.st.create_source("x"), Err(AppError::NoProject)));
    assert!(matches!(
        e.st.step_source("abc", next()),
        Err(AppError::NoProject)
    ));
    assert!(matches!(
        e.st.set_layer_fields("a", "b", Fields::new()),
        Err(AppError::NoProject)
    ));
    assert!(matches!(e.st.live_panic(), Err(AppError::NoProject)));
    assert!(matches!(e.st.live_clear_all(), Err(AppError::NoProject)));
    assert!(matches!(e.st.live_clear("x"), Err(AppError::NoProject)));
}

#[test]
fn list_get_rename_duplicate_delete() {
    let e = env();
    let t = e.template();
    let b = e.st.create_source("beta").unwrap();
    let a = e.source(&["Ada", "Grace"]);
    let a = e.st.rename_source(&a.id, " Alpha ").unwrap();
    assert_eq!(a.name, "Alpha");
    let screen = e.bound_screen(&t.id, &a.id, "name");

    let list = e.st.list_sources().unwrap();
    let names: Vec<&str> = list.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Alpha", "beta"], "by name, case-insensitive");
    assert_eq!(list[0].used_by, std::slice::from_ref(&screen.id));
    assert!(list[1].used_by.is_empty());
    assert_eq!(list[0].row_count, 2);
    assert_eq!(list[0].current["name"], "Ada");
    assert_eq!(list[1].columns[0].key, "text");

    let copy = e.st.duplicate_source(&a.id, "Alpha copy").unwrap();
    assert_ne!(copy.id, a.id);
    assert_eq!(e.st.list_sources().unwrap().len(), 3);
    assert!(matches!(e.st.get_source("../x"), Err(AppError::Invalid(_))));
    assert!(matches!(
        e.st.get_source("nosuch0001"),
        Err(AppError::NotFound(_))
    ));
    e.st.delete_source(&b.id).unwrap();
    assert!(matches!(
        e.st.delete_source(&b.id),
        Err(AppError::NotFound(_))
    ));
    assert_eq!(e.st.list_sources().unwrap().len(), 2);
}

#[test]
fn steps_are_clamped_in_memory_and_written_coalesced() {
    let e = env();
    // Long against any stall of this thread, so the burst surely falls in one window.
    e.st.set_content_flush_delay(Duration::from_millis(800));
    let watched = e.st.watcher_batches();
    let s = e.source(&["A", "B", "C"]);
    let before = e.st.content_write_count();
    let mut indexes = Vec::new();
    for _ in 0..5 {
        indexes.push(e.st.step_source(&s.id, next()).unwrap().index);
    }
    assert_eq!(indexes, [1, 2, 2, 2, 2], "clamped, no wrap");
    let summary =
        e.st.step_source(&s.id, SourceStep::By { delta: -1 })
            .unwrap();
    assert_eq!(summary.index, 1);
    assert_eq!(summary.current["name"], "B");
    // Memory has it before the disk does.
    assert_eq!(e.source_file(&s.id)["index"], 0);
    let SourceKind::Text { index, .. } = e.st.get_source(&s.id).unwrap().meta.kind else {
        panic!("not a text source")
    };
    assert_eq!(index, 1);
    assert_eq!(e.st.list_sources().unwrap()[0].index, 1);
    // One write for the whole burst: nothing is left dirty for a flush.
    wait_until("coalesced write", || e.st.content_write_count() > before);
    e.st.flush_all();
    assert_eq!(e.st.content_write_count(), before + 1);
    let file = e.source_file(&s.id);
    assert_eq!(file["index"], 1);
    assert_eq!(
        file["updatedAt"], s.meta.updated_at,
        "a step is not an edit"
    );
    // Stepping to the same index does nothing (no write, no apply): nothing dirty to flush.
    e.st.step_source(&s.id, SourceStep::Index { index: 1 })
        .unwrap();
    e.st.flush_all();
    assert_eq!(e.st.content_write_count(), before + 1);
    // Own writes are not reported by the watcher.
    watcher_settled(&e, watched);
    assert!(e.rec.named(names::PROJECT_FILES_CHANGED).is_empty());
}

#[test]
fn a_pending_step_is_flushed_on_close_and_kept() {
    let e = env();
    let s = e.source(&["A", "B", "C"]);
    e.st.step_source(&s.id, SourceStep::Index { index: 2 })
        .unwrap();
    assert_eq!(e.source_file(&s.id)["index"], 0);
    e.st.close_project();
    assert_eq!(e.source_file(&s.id)["index"], 2);
    e.st.open_project(&e.dir).unwrap();
    assert_eq!(e.st.list_sources().unwrap()[0].index, 2);
    // flush_all (exit) writes too.
    e.st.step_source(&s.id, SourceStep::Index { index: 0 })
        .unwrap();
    e.st.flush_all();
    assert_eq!(e.source_file(&s.id)["index"], 0);
}

// ---- Bindings --------------------------------------------------------------------------------

#[test]
fn bindings_resolve_and_fall_back_with_a_warning() {
    let e = env();
    let t = e.template();
    let s = e.source(&["Ada", "Grace"]);
    let screen = e.bound_screen(&t.id, &s.id, "name");
    assert_eq!(e.field(&screen.id, "name"), "Ada");
    assert_eq!(e.field(&screen.id, "title"), "Own title", "unbound");
    assert!(warning_codes(&e, &screen.id).is_empty());
    // The file keeps the binding (and the layer's own value).
    let file = e.screen_file(&screen.id);
    assert_eq!(
        file["layers"][0]["bindings"],
        json!({"name": {"sourceId": s.id, "column": "name"}})
    );

    // Missing column.
    let mut doc = screen.clone();
    let ScreenLayerKind::Template { bindings, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    bindings.get_mut("name").unwrap().column = "nope".into();
    e.st.save_screen(doc.clone()).unwrap();
    assert_eq!(e.field(&screen.id, "name"), "Own");
    assert_eq!(
        warning_codes(&e, &screen.id),
        [ScreenWarningCode::SourceMissing]
    );

    // Missing source.
    let ScreenLayerKind::Template { bindings, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    *bindings.get_mut("name").unwrap() = SourceBinding {
        source_id: "nosuch0001".into(),
        column: "name".into(),
        sort: None,
        limit: None,
        row: None,
    };
    e.st.save_screen(doc.clone()).unwrap();
    assert_eq!(e.field(&screen.id, "name"), "Own");
    assert_eq!(
        warning_codes(&e, &screen.id),
        [ScreenWarningCode::SourceMissing]
    );

    // A binding for a field the template doesn't have: ignored, soft warning.
    let ScreenLayerKind::Template { bindings, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    bindings.clear();
    bindings.insert(
        "ghost".into(),
        SourceBinding {
            source_id: s.id.clone(),
            column: "name".into(),
            sort: None,
            limit: None,
            row: None,
        },
    );
    e.st.save_screen(doc.clone()).unwrap();
    assert_eq!(
        warning_codes(&e, &screen.id),
        [ScreenWarningCode::InvalidValue]
    );

    // Invalid binding shapes are rejected outright.
    for (key, sid, column) in [
        ("Bad", s.id.as_str(), "name"),
        ("name", "../x", "name"),
        ("name", s.id.as_str(), "Bad Col"),
    ] {
        let ScreenLayerKind::Template { bindings, .. } = &mut doc.meta.layers[0].kind else {
            unreachable!()
        };
        bindings.clear();
        bindings.insert(
            key.into(),
            SourceBinding {
                source_id: sid.into(),
                column: column.into(),
                sort: None,
                limit: None,
                row: None,
            },
        );
        assert!(
            matches!(e.st.save_screen(doc.clone()), Err(AppError::Invalid(_))),
            "{key} {sid} {column}"
        );
    }
}

#[test]
fn an_empty_source_falls_back() {
    let e = env();
    let t = e.template();
    let s = e.source(&[]);
    let screen = e.bound_screen(&t.id, &s.id, "name");
    assert_eq!(e.field(&screen.id, "name"), "Own");
    assert_eq!(
        warning_codes(&e, &screen.id),
        [ScreenWarningCode::SourceMissing]
    );
    // Adding rows makes it resolve.
    let mut doc = e.st.get_source(&s.id).unwrap();
    let SourceKind::Text { rows, .. } = &mut doc.meta.kind else {
        panic!("not a text source")
    };
    rows.push(row(&[("name", "First")]));
    e.st.save_source(doc).unwrap();
    assert_eq!(e.field(&screen.id, "name"), "First");
}

#[test]
fn a_step_reapplies_bound_screens_by_the_live_rules() {
    let e = env();
    let t = e.template();
    let s = e.source(&["Ada", "Grace", "Hedy"]);
    let screen = e.bound_screen(&t.id, &s.id, "name");
    let id = screen.id.clone();
    let mut rx = e.st.live().subscribe();

    // Off air: applies now, only the bound field.
    e.st.step_source(&s.id, next()).unwrap();
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "fields", "layerId": "l1", "fields": {"name": "Grace"}})]
    );

    // On air under onTake: staged; TAKE shows it.
    e.st.live().show(&id, "l1").unwrap();
    drain(&mut rx, &id);
    e.st.step_source(&s.id, next()).unwrap();
    assert!(drain(&mut rx, &id).is_empty());
    let v = e.st.live().layer_view(&id, "l1").unwrap();
    assert!(v.pending);
    assert_eq!(v.fields["name"], "Hedy", "the view shows the staged value");
    assert_eq!(
        on_air_field(&e, &id, "name"),
        "Grace",
        "the output still shows"
    );
    let v = e.st.live().show(&id, "l1").unwrap();
    assert!(!v.pending);
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "fields", "layerId": "l1", "fields": {"name": "Hedy"}})]
    );

    // Immediate: on air goes out now.
    e.set_mode(LiveEdit::Immediate);
    e.st.step_source(&s.id, SourceStep::Index { index: 0 })
        .unwrap();
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "fields", "layerId": "l1", "fields": {"name": "Ada"}})]
    );

    // Saving the source re-applies too.
    let mut doc = e.st.get_source(&s.id).unwrap();
    let SourceKind::Text { rows, .. } = &mut doc.meta.kind else {
        panic!("not a text source")
    };
    rows[0].insert("name".into(), "Ada L.".into());
    e.st.save_source(doc).unwrap();
    assert_eq!(e.field(&id, "name"), "Ada L.");

    // Deleting it: the layer's own value, with a warning.
    e.st.delete_source(&s.id).unwrap();
    assert_eq!(e.field(&id, "name"), "Own");
    assert_eq!(warning_codes(&e, &id), [ScreenWarningCode::SourceMissing]);
}

// ---- Control field edits ---------------------------------------------------------------------

#[test]
fn set_layer_fields_validates_and_follows_the_live_rules() {
    let e = env();
    let t = e.template();
    let s = e.source(&["Ada"]);
    let screen = e.bound_screen(&t.id, &s.id, "name");
    let id = screen.id.clone();
    let f = |k: &str, v: &str| row(&[(k, v)]);

    for (fields, what) in [
        (f("name", "x"), "bound field"),
        (f("ghost", "x"), "unknown field"),
        (f("title", &"x".repeat(64 * 1024 + 1)), "too long"),
    ] {
        assert!(
            matches!(
                e.st.set_layer_fields(&id, "l1", fields),
                Err(AppError::Invalid(_))
            ),
            "{what}"
        );
    }
    assert!(matches!(
        e.st.set_layer_fields(&id, "nope", f("title", "x")),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.st.set_layer_fields("nosuch0001", "l1", f("title", "x")),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.st.set_layer_fields("test", "lower-third", f("name", "x")),
        Err(AppError::NotFound(_))
    ));
    assert!(!e.st.screens().is_dirty(&id), "nothing changed");

    let mut rx = e.st.live().subscribe();
    let v =
        e.st.set_layer_fields(&id, "l1", f("title", "Analyst"))
            .unwrap();
    assert_eq!(v.fields["title"], "Analyst");
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "fields", "layerId": "l1", "fields": {"title": "Analyst"}})]
    );

    // On air under onTake: staged until TAKE. The view reports what the next take shows (so
    // Control's inputs keep the edit); the output keeps the on-air value.
    e.st.live().show(&id, "l1").unwrap();
    drain(&mut rx, &id);
    e.rec.take_live();
    let v =
        e.st.set_layer_fields(&id, "l1", f("title", "Engineer"))
            .unwrap();
    assert!(v.pending);
    assert_eq!(v.fields["title"], "Engineer", "staged value");
    assert_eq!(
        on_air_field(&e, &id, "title"),
        "Analyst",
        "output unchanged"
    );
    assert!(drain(&mut rx, &id).is_empty());
    let changed = e.rec.take_live();
    assert_eq!(changed.len(), 1, "staged values are part of the view");
    assert_eq!(changed[0]["layers"][0]["fields"]["title"], "Engineer");
    assert_eq!(changed[0]["layers"][0]["pending"], true);
    // Another staged edit changes the view again.
    e.st.set_layer_fields(&id, "l1", f("title", "Engineer II"))
        .unwrap();
    assert_eq!(e.rec.take_live().len(), 1);
    // TAKE on the on-air layer applies the staged change: content only, no state change (the
    // renderer does not replay the in-animation).
    let v = e.st.live().show(&id, "l1").unwrap();
    assert_eq!(v.fields["title"], "Engineer II");
    assert!(!v.pending && v.on_air);
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "fields", "layerId": "l1", "fields": {"title": "Engineer II"}})]
    );
    assert_eq!(on_air_field(&e, &id, "title"), "Engineer II");
}

/// The field value the output shows (staged changes aside).
fn on_air_field(e: &Env, screen: &str, key: &str) -> String {
    e.st.live()
        .with_layer(screen, "l1", |l| l.fields[key].clone())
        .unwrap()
}

#[test]
fn set_layer_fields_is_saved_coalesced_and_flushed_on_close() {
    let e = env();
    let t = e.template();
    let s = e.source(&["Ada"]);
    let screen = e.bound_screen(&t.id, &s.id, "name");
    let id = screen.id.clone();
    // The watcher looked at the own writes above (template, source, screen) and reported none.
    watcher_settled(&e, 0);
    assert!(
        e.rec.named(names::PROJECT_FILES_CHANGED).is_empty(),
        "own writes"
    );
    let before = e.st.content_write_count();
    for i in 0..10 {
        e.st.set_layer_fields(&id, "l1", row(&[("title", &format!("T{i}"))]))
            .unwrap();
    }
    // Not on disk yet, but get_screen sees the edit.
    assert_eq!(
        e.screen_file(&id)["layers"][0]["fields"]["title"],
        "Own title"
    );
    let got = e.st.get_screen(&id).unwrap();
    let ScreenLayerKind::Template { fields, .. } = &got.doc.meta.layers[0].kind else {
        unreachable!()
    };
    assert_eq!(fields["title"], "T9");
    assert_eq!(e.st.content_write_count(), before);

    // Close writes it once.
    e.st.close_project();
    assert_eq!(e.st.content_write_count(), before + 1);
    let file = e.screen_file(&id);
    assert_eq!(file["layers"][0]["fields"]["title"], "T9");
    assert_eq!(
        file["layers"][0]["fields"]["name"], "Own",
        "a bound field keeps its own value"
    );
    e.st.open_project(&e.dir).unwrap();
    assert_eq!(e.field(&id, "title"), "T9");
    // The close's write happened before this watcher started: an own write it can't see, so
    // nothing ever reports it (no batch to wait for; a short look for a stray event).
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        e.rec.named(names::PROJECT_FILES_CHANGED).is_empty(),
        "own write"
    );
}

#[test]
fn set_layer_fields_burst_is_one_timed_write() {
    let e = env();
    // Long against any stall of this thread, so the burst surely falls in one window.
    e.st.set_content_flush_delay(Duration::from_millis(800));
    let t = e.template();
    let s = e.source(&["Ada"]);
    let screen = e.bound_screen(&t.id, &s.id, "name");
    let id = screen.id.clone();
    let before = e.st.content_write_count();
    for i in 0..10 {
        e.st.set_layer_fields(&id, "l1", row(&[("title", &format!("T{i}"))]))
            .unwrap();
    }
    wait_until("coalesced write", || e.st.content_write_count() > before);
    // The burst was written whole: nothing left dirty for a flush.
    e.st.flush_all();
    assert_eq!(e.st.content_write_count(), before + 1);
    assert_eq!(e.screen_file(&id)["layers"][0]["fields"]["title"], "T9");
    assert!(!e.st.screens().is_dirty(&id));
    // A save replaces pending edits and clears dirty.
    e.st.set_layer_fields(&id, "l1", row(&[("title", "Pending")]))
        .unwrap();
    let mut doc = e.st.get_screen(&id).unwrap().doc;
    doc.meta.name = "Renamed".into();
    e.st.save_screen(doc).unwrap();
    assert!(!e.st.screens().is_dirty(&id));
    assert_eq!(
        e.screen_file(&id)["layers"][0]["fields"]["title"],
        "Pending"
    );
    // Rename keeps unsaved edits.
    e.st.set_layer_fields(&id, "l1", row(&[("title", "Kept")]))
        .unwrap();
    e.st.rename_screen(&id, "Again").unwrap();
    assert_eq!(e.screen_file(&id)["layers"][0]["fields"]["title"], "Kept");
}

// ---- live-changed and panic ------------------------------------------------------------------

#[test]
fn live_changed_follows_every_path_once() {
    let e = env();
    let t = e.template();
    let s = e.source(&["Ada", "Grace"]);
    let screen = e.bound_screen(&t.id, &s.id, "name");
    let id = screen.id.clone();
    e.rec.take_live();

    e.st.live().show(&id, "l1").unwrap();
    let got = e.rec.take_live();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["screen"]["id"], id.as_str());
    assert_eq!(got[0]["layers"][0]["onAir"], true);
    assert_eq!(
        got[0],
        serde_json::to_value(&e.st.live().user_screen_views()[0]).unwrap(),
        "same shape as list_screens"
    );

    // No-ops: show again, an unchanged step, an unchanged field.
    e.st.live().show(&id, "l1").unwrap();
    e.st.step_source(&s.id, SourceStep::Index { index: 0 })
        .unwrap();
    e.st.set_layer_fields(&id, "l1", row(&[("title", "Own title")]))
        .unwrap();
    // Rect drags (off air: applied, not in the view).
    e.st.live().hide(&id, "l1").unwrap();
    e.rec.take_live();
    for x in 0..5 {
        let rect = Rect {
            x,
            y: 0,
            w: 800,
            h: 200,
        };
        e.st.set_layer_rect(&id, "l1", rect).unwrap();
    }
    assert!(e.rec.take_live().is_empty());

    // A step changes the view once.
    e.st.step_source(&s.id, next()).unwrap();
    let got = e.rec.take_live();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["layers"][0]["fields"]["name"], "Grace");

    // Template preview screen ops never emit.
    e.st.live()
        .update_layer(
            crate::core::live::PREVIEW_SCREEN_ID,
            crate::core::live::PREVIEW_LAYER_ID,
            |l| Ok::<_, crate::core::live::LiveError>(vec![l.state_msg()]),
        )
        .unwrap();
    assert!(e.rec.take_live().is_empty());

    // Clear, clear all.
    e.st.live().show(&id, "l1").unwrap();
    e.rec.take_live();
    let v = e.st.live_clear(&id).unwrap();
    assert!(!v.layers[0].on_air);
    assert_eq!(e.rec.take_live().len(), 1);
    e.st.live().show(&id, "l1").unwrap();
    e.rec.take_live();
    let all = e.st.live_clear_all().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(e.rec.take_live().len(), 1);
    assert!(e.st.live_clear_all().unwrap()[0]
        .layers
        .iter()
        .all(|l| !l.on_air));
    assert!(e.rec.take_live().is_empty(), "nothing changed");
}

#[test]
fn panic_cuts_keeps_staged_changes_and_the_next_take_applies_them() {
    let e = env();
    let t = e.template();
    let s = e.source(&["Ada", "Grace"]);
    let screen = e.bound_screen(&t.id, &s.id, "name");
    let id = screen.id.clone();
    e.st.live().show(&id, "l1").unwrap();
    e.st.live().show("test", "lower-third").unwrap();
    e.st.step_source(&s.id, next()).unwrap();
    assert!(e.st.live().layer_view(&id, "l1").unwrap().pending);
    let mut rx = e.st.live().subscribe();
    e.rec.take_live();

    let views = e.st.live_panic().unwrap();
    assert_eq!(views.len(), 1);
    assert!(!views[0].layers[0].on_air);
    assert!(views[0].layers[0].pending, "staged change kept");
    assert_eq!(drain(&mut rx, &id), [json!({"t": "cut"})]);
    assert!(
        !e.st
            .live()
            .layer_view("test", "lower-third")
            .unwrap()
            .on_air
    );
    let ids: Vec<Value> = e
        .rec
        .take_live()
        .iter()
        .map(|v| v["screen"]["id"].clone())
        .collect();
    assert_eq!(ids, [json!("test"), json!(id)]);

    let v = e.st.live().show(&id, "l1").unwrap();
    assert!(v.on_air && !v.pending);
    assert_eq!(v.fields["name"], "Grace");
}

// ---- Watcher ---------------------------------------------------------------------------------

fn files_changed(e: &Env) -> Vec<Value> {
    e.rec.named(names::PROJECT_FILES_CHANGED)
}

#[test]
fn watcher_reloads_sources_and_reapplies_bound_screens() {
    let e = env();
    let t = e.template();
    let s = e.source(&["Ada", "Grace"]);
    let screen = e.bound_screen(&t.id, &s.id, "name");
    let id = screen.id.clone();
    let path = e.dir.join("sources").join(format!("{}.json", s.id));
    watcher_settled(&e, 0);
    assert!(files_changed(&e).is_empty(), "own writes");

    // Edited outside: reloaded, the bound layer follows.
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("Ada", "Ada Outside");
    fs::write(&path, text).unwrap();
    wait_until("source change", || !files_changed(&e).is_empty());
    assert_eq!(
        files_changed(&e)[0],
        json!({"kind": "source", "id": s.id, "change": "changed", "message": null})
    );
    assert_eq!(e.field(&id, "name"), "Ada Outside");

    // Invalid: announced, last good state kept, file left alone.
    fs::write(&path, "{ half").unwrap();
    wait_until("invalid", || files_changed(&e).len() >= 2);
    assert_eq!(files_changed(&e)[1]["change"], "invalid");
    assert_eq!(e.field(&id, "name"), "Ada Outside");
    assert_eq!(e.st.list_sources().unwrap().len(), 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), "{ half");

    // Removed: the layer falls back.
    fs::remove_file(&path).unwrap();
    wait_until("removed", || {
        files_changed(&e).iter().any(|c| c["change"] == "removed")
    });
    assert_eq!(e.field(&id, "name"), "Own");
    assert!(e.st.list_sources().unwrap().is_empty());

    // A new file appears: the binding resolves again.
    fs::write(
        &path,
        r#"{"schemaVersion":1,"name":"Back","kind":"text","columns":[{"key":"name","label":"Name"}],"rows":[{"name":"Back again"}]}"#,
    )
    .unwrap();
    wait_until("added", || e.field(&id, "name") == "Back again");
    let _ = &e.tmp;
}
