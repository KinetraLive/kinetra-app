use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::broadcast::Receiver;

use super::*;
use crate::core::live::{PREVIEW_SCREEN_ID, TEST_SCREEN_ID};
use crate::core::live_hub::{HubMsg, REMOVE_DELAY};
use crate::core::model::{
    ScreenLayer, ScreenLayerKind, ScreenWarningCode, TemplateSource, UiState,
};
use crate::core::protocol::Fields;

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
    _tmp: tempfile::TempDir,
    st: AppState,
    rec: Arc<Recorder>,
    dir: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        rec.clone(),
        Duration::from_secs(60),
    )
    .unwrap();
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    Env {
        _tmp: tmp,
        st,
        rec,
        dir,
    }
}

impl Env {
    /// A template from the classic-bar preset (fields name/title, colour variables).
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

    fn screen(&self, name: &str) -> ScreenDoc {
        self.st
            .create_screen(&NewScreen {
                name: name.into(),
                width: 1920,
                height: 1080,
                background: "transparent".into(),
            })
            .unwrap()
            .doc
    }

    fn set_mode(&self, mode: LiveEdit) {
        let mut ui: UiState = self.st.ui_state().unwrap();
        ui.control.live_edit = mode;
        self.st.set_ui_state(ui).unwrap();
    }

    fn pending(&self, screen: &str, layer: &str) -> bool {
        self.st.live().layer_view(screen, layer).unwrap().pending
    }
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

fn set_field(doc: &mut ScreenDoc, layer: usize, key: &str, value: &str) {
    let ScreenLayerKind::Template { fields, .. } = &mut doc.meta.layers[layer].kind else {
        unreachable!()
    };
    fields.insert(key.into(), value.into());
}

/// Every message for `screen` received so far, as JSON.
fn drain(rx: &mut Receiver<Arc<HubMsg>>, screen: &str) -> Vec<Value> {
    let mut out = Vec::new();
    while let Ok(m) = rx.try_recv() {
        if &*m.screen_id == screen {
            out.push(serde_json::from_str(&m.json).unwrap());
        }
    }
    out
}

/// Every message received so far, split by two screens.
fn drain_two(rx: &mut Receiver<Arc<HubMsg>>, a: &str, b: &str) -> (Vec<Value>, Vec<Value>) {
    let (mut ma, mut mb) = (Vec::new(), Vec::new());
    while let Ok(m) = rx.try_recv() {
        let v: Value = serde_json::from_str(&m.json).unwrap();
        if &*m.screen_id == a {
            ma.push(v);
        } else if &*m.screen_id == b {
            mb.push(v);
        }
    }
    (ma, mb)
}

fn kinds(msgs: &[Value]) -> Vec<String> {
    msgs.iter()
        .map(|m| m["t"].as_str().unwrap().to_string())
        .collect()
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn screens_go_live_and_follow_the_rules() {
    let e = env();
    let t = e.template();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    // Listed, empty, ordered after nothing.
    let views = e.st.live().screen_views();
    assert!(views.iter().any(|v| v.screen.id == id));
    let mut rx = e.st.live().subscribe();

    // Layer added: off air, one `layer` with its rect.
    doc.meta.layers.push(layer("l1", &t.id));
    set_field(&mut doc, 0, "name", "Ada");
    let s = e.st.save_screen(doc.clone()).unwrap();
    assert!(s.warnings.is_empty());
    assert_eq!(s.live.layers.len(), 1);
    assert!(!s.live.layers[0].on_air);
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer"]);
    let wl = &msgs[0]["layer"];
    assert_eq!(wl["state"], "out");
    assert_eq!(wl["templateRev"], 1);
    assert_eq!(wl["rect"], json!({"x": 0, "y": 0, "w": 1920, "h": 1080}));
    assert_eq!(wl["fields"], json!({"name": "Ada", "title": ""}));
    assert_eq!(wl["vars"]["accent"], "#ffb81c");

    // Off air: a field change applies now.
    set_field(&mut doc, 0, "name", "Bob");
    e.st.save_screen(doc.clone()).unwrap();
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "fields", "layerId": "l1", "fields": {"name": "Bob"}})]
    );

    // On air under onTake (the default): staged, nothing sent.
    e.st.live().show(&id, "l1").unwrap();
    assert_eq!(kinds(&drain(&mut rx, &id)), ["state"]);
    set_field(&mut doc, 0, "name", "Cy");
    let ScreenLayerKind::Template { vars, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    vars.insert("accent".into(), "#00ff00".into());
    doc.meta.layers[0].rect.x = 100;
    let s = e.st.save_screen(doc.clone()).unwrap();
    assert!(s.live.layers[0].pending);
    assert_eq!(
        s.live.layers[0].fields["name"], "Cy",
        "the view shows the staged value"
    );
    let showing =
        e.st.live()
            .with_layer(&id, "l1", |l| l.fields["name"].clone());
    assert_eq!(showing.unwrap(), "Bob", "the output still shows");
    assert!(drain(&mut rx, &id).is_empty());
    // Saving the same again keeps it staged; hide keeps it.
    e.st.save_screen(doc.clone()).unwrap();
    assert!(drain(&mut rx, &id).is_empty());
    e.st.live().hide(&id, "l1").unwrap();
    assert!(e.pending(&id, "l1"));
    assert_eq!(kinds(&drain(&mut rx, &id)), ["state"]);
    // The next show applies it: content first, then `state in`.
    let v = e.st.live().show(&id, "l1").unwrap();
    assert!(!v.pending && v.on_air);
    assert_eq!(v.fields["name"], "Cy");
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["vars", "fields", "rect", "state"]);
    assert_eq!(msgs[0]["vars"], json!({"accent": "#00ff00"}));
    assert_eq!(msgs[2]["rect"]["x"], 100);
    assert_eq!(msgs[3]["state"], "in");

    // Reverting a staged change drops it.
    set_field(&mut doc, 0, "name", "Dee");
    e.st.save_screen(doc.clone()).unwrap();
    assert!(e.pending(&id, "l1"));
    set_field(&mut doc, 0, "name", "Cy");
    e.st.save_screen(doc.clone()).unwrap();
    assert!(!e.pending(&id, "l1"));

    // Immediate: on-air changes go out now.
    e.set_mode(LiveEdit::Immediate);
    set_field(&mut doc, 0, "name", "Eve");
    let s = e.st.save_screen(doc.clone()).unwrap();
    assert!(!s.live.layers[0].pending);
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "fields", "layerId": "l1", "fields": {"name": "Eve"}})]
    );
}

#[test]
fn show_on_air_takes_staged_changes_without_a_state_change() {
    let e = env();
    let t = e.template();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers.push(layer("l1", &t.id));
    e.st.save_screen(doc.clone()).unwrap();
    e.st.live().show(&id, "l1").unwrap();
    set_field(&mut doc, 0, "title", "Engineer");
    e.st.save_screen(doc.clone()).unwrap();
    let mut rx = e.st.live().subscribe();
    let v = e.st.live().show(&id, "l1").unwrap();
    assert!(v.on_air && !v.pending);
    assert_eq!(kinds(&drain(&mut rx, &id)), ["fields"]);
}

#[test]
fn reorder_resends_layers_with_the_same_rev() {
    let e = env();
    let t = e.template();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers = vec![layer("a", &t.id), layer("b", &t.id)];
    e.st.save_screen(doc.clone()).unwrap();
    e.st.live().show(&id, "a").unwrap();
    let mut rx = e.st.live().subscribe();
    doc.meta.layers.swap(0, 1);
    let s = e.st.save_screen(doc.clone()).unwrap();
    let ids: Vec<&str> = s.live.layers.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["b", "a"], "bottom first");
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer", "layer"]);
    for m in &msgs {
        let l = &m["layer"];
        assert_eq!(l["templateRev"], 1, "no recompile");
        let want = if l["id"] == "b" { 0 } else { 1 };
        assert_eq!(l["z"], want);
    }
    // The on-air layer is reordered immediately even under onTake, and stays in.
    let a = msgs.iter().find(|m| m["layer"]["id"] == "a").unwrap();
    assert_eq!(a["layer"]["state"], "in");
    assert!(!e.pending(&id, "a"));
}

#[test]
fn removing_layers_off_air_now_and_on_air_after_the_out_animation() {
    let e = env();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .unwrap();
    e.st.live().set_runtime(rt.handle().clone());
    let t = e.template();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers = vec![layer("a", &t.id), layer("b", &t.id)];
    e.st.save_screen(doc.clone()).unwrap();
    e.st.live().show(&id, "b").unwrap();
    let mut rx = e.st.live().subscribe();

    doc.meta.layers.retain(|l| l.id != "a");
    e.st.save_screen(doc.clone()).unwrap();
    let msgs = drain(&mut rx, &id);
    assert_eq!(msgs[0], json!({"t": "remove", "layerId": "a"}));
    // `b` moved down to z 0: resent with the same rev, still in.
    assert_eq!(kinds(&msgs), ["remove", "layer"]);
    assert_eq!(msgs[1]["layer"]["z"], 0);
    assert_eq!(msgs[1]["layer"]["state"], "in");

    let started = Instant::now();
    doc.meta.layers.clear();
    let s = e.st.save_screen(doc.clone()).unwrap();
    assert!(s.live.layers.is_empty(), "gone from the live state at once");
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "state", "layerId": "b", "state": "out"})]
    );
    let mut got = Vec::new();
    wait_until("delayed remove", || {
        got.extend(drain(&mut rx, &id));
        !got.is_empty()
    });
    assert!(started.elapsed() >= REMOVE_DELAY - Duration::from_millis(50));
    assert_eq!(got, [json!({"t": "remove", "layerId": "b"})]);
}

#[test]
fn template_changes_propagate_to_layers() {
    let e = env();
    let t = e.template();
    let mut one = e.screen("One");
    let mut two = e.screen("Two");
    one.meta.layers.push(layer("l1", &t.id));
    two.meta.layers.push(layer("l2", &t.id));
    e.st.save_screen(one.clone()).unwrap();
    e.st.save_screen(two.clone()).unwrap();
    e.st.live().show(&two.id, "l2").unwrap();
    let mut rx = e.st.live().subscribe();

    // Saved through the command path: off-air layer gets one `layer`, on-air one is staged.
    let mut edited = t.clone();
    edited.variants[0].css.push_str("\n.extra { color: red; }");
    let saved = e.st.templates().save(&e.dir, edited).unwrap();
    e.st.template_saved(&saved);
    let (m1, m2) = drain_two(&mut rx, &one.id, &two.id);
    assert_eq!(kinds(&m1), ["layer"]);
    assert_eq!(m1[0]["layer"]["templateRev"], 2);
    assert!(m1[0]["layer"]["css"].as_str().unwrap().contains(".extra"));
    assert!(m2.is_empty(), "{m2:?}");
    assert!(e.pending(&two.id, "l2"));

    // A variable default change reaches layers without an override, as `vars`.
    let mut edited = saved.clone();
    edited.meta.variables[0].default = "#123456".into();
    let saved = e.st.templates().save(&e.dir, edited).unwrap();
    e.st.template_saved(&saved);
    let (m1, m2) = drain_two(&mut rx, &one.id, &two.id);
    assert_eq!(
        m1,
        [json!({"t": "vars", "layerId": "l1", "vars": {"accent": "#123456"}})]
    );
    assert!(m2.is_empty(), "{m2:?}");

    // Deleted: off-air layer removed now, on-air one animates out; both are "missing".
    e.st.templates().delete(&e.dir, &t.id).unwrap();
    e.st.template_deleted(&t.id);
    let (m1, m2) = drain_two(&mut rx, &one.id, &two.id);
    assert_eq!(kinds(&m1), ["remove"]);
    assert_eq!(m2, [json!({"t": "state", "layerId": "l2", "state": "out"})]);
    let s = e.st.get_screen(&two.id).unwrap();
    assert!(s.live.layers.is_empty());
    assert_eq!(s.doc.meta.layers.len(), 1, "the layer stays in the file");
    assert_eq!(s.warnings.len(), 1);
    assert_eq!(s.warnings[0].code, ScreenWarningCode::TemplateMissing);
    assert_eq!(s.warnings[0].layer_id, "l2");
}

#[test]
fn set_layer_rect_is_memory_only_and_follows_the_rules() {
    let e = env();
    let t = e.template();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers.push(layer("l1", &t.id));
    e.st.save_screen(doc.clone()).unwrap();
    let path = e.dir.join("screens").join(format!("{id}.json"));
    let before = (
        fs::read(&path).unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap(),
    );
    let mut rx = e.st.live().subscribe();
    let r = |x| Rect {
        x,
        y: 10,
        w: 640,
        h: 360,
    };

    e.st.set_layer_rect(&id, "l1", r(5)).unwrap();
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "rect", "layerId": "l1", "rect": {"x": 5, "y": 10, "w": 640, "h": 360}})]
    );
    e.st.live().show(&id, "l1").unwrap();
    drain(&mut rx, &id);
    let v = e.st.set_layer_rect(&id, "l1", r(6)).unwrap();
    assert!(v.pending, "on air under onTake: staged");
    assert!(drain(&mut rx, &id).is_empty());
    e.set_mode(LiveEdit::Immediate);
    e.st.set_layer_rect(&id, "l1", r(7)).unwrap();
    assert_eq!(kinds(&drain(&mut rx, &id)), ["rect"]);

    // Invalid boxes and unknown targets are refused.
    let off = Rect {
        x: 5000,
        y: 0,
        w: 10,
        h: 10,
    };
    assert!(matches!(
        e.st.set_layer_rect(&id, "l1", off),
        Err(AppError::Invalid(_))
    ));
    assert!(matches!(
        e.st.set_layer_rect(&id, "nope", r(1)),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.st.set_layer_rect("nope", "l1", r(1)),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.st.set_layer_rect(TEST_SCREEN_ID, "lower-third", r(1)),
        Err(AppError::NotFound(_))
    ));

    // Never touched the disk.
    let after = (
        fs::read(&path).unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap(),
    );
    assert_eq!(before, after);
    // The drop's save writes the box.
    let mut moved = doc.clone();
    moved.meta.layers[0].rect = r(7);
    e.st.save_screen(moved).unwrap();
    assert_eq!(e.st.get_screen(&id).unwrap().doc.meta.layers[0].rect, r(7));
}

#[test]
fn screen_info_changes_send_a_snapshot_and_order_by_name() {
    let e = env();
    let b = e.screen("bravo");
    let a = e.screen("Alpha");
    let names = |e: &Env| -> Vec<String> {
        e.st.live()
            .screen_views()
            .into_iter()
            .map(|v| v.screen.name)
            .collect()
    };
    assert_eq!(
        names(&e),
        ["Output test", "Template preview", "Alpha", "bravo"],
        "reserved first, then by name"
    );
    let mut rx = e.st.live().subscribe();
    let s = e.st.rename_screen(&a.id, "zulu").unwrap();
    assert_eq!(s.name, "zulu");
    assert_eq!(kinds(&drain(&mut rx, &a.id)), ["snapshot"]);
    assert_eq!(names(&e)[2..], ["bravo", "zulu"]);
    let mut bigger = e.st.get_screen(&b.id).unwrap().doc;
    bigger.meta.width = 1280;
    bigger.meta.height = 720;
    e.st.save_screen(bigger).unwrap();
    let msgs = drain(&mut rx, &b.id);
    assert_eq!(kinds(&msgs), ["snapshot"]);
    assert_eq!(msgs[0]["screen"]["width"], 1280);

    // Summaries.
    let list = e.st.list_screen_docs().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, b.id, "newest first");
}

#[test]
fn create_duplicate_delete() {
    let e = env();
    let t = e.template();
    let mut rx = e.st.live().subscribe();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    assert_eq!(
        kinds(&drain(&mut rx, &id)),
        ["snapshot"],
        "existing renderers learn of it"
    );
    doc.meta.layers.push(layer("l1", &t.id));
    e.st.save_screen(doc.clone()).unwrap();
    e.st.live().show(&id, "l1").unwrap();

    let copy = e.st.duplicate_screen(&id, "Copy").unwrap();
    assert_ne!(copy.doc.id, id);
    assert_eq!(copy.live.layers.len(), 1);
    assert!(!copy.live.layers[0].on_air, "copies start off air");
    assert_ne!(copy.doc.meta.layers[0].id, "l1");

    drain(&mut rx, &id);
    e.st.delete_screen(&id).unwrap();
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "error", "code": "unknownScreen"})]
    );
    assert!(!e.st.live().has_screen(&id));
    assert!(!e.dir.join("screens").join(format!("{id}.json")).exists());
    assert!(matches!(
        e.st.delete_screen(&id),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(e.st.get_screen(&id), Err(AppError::NotFound(_))));
    let ids: Vec<String> =
        e.st.list_screen_docs()
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
    assert_eq!(ids, std::slice::from_ref(&copy.doc.id));

    // Nothing without a project.
    e.st.close_project();
    assert!(matches!(e.st.list_screen_docs(), Err(AppError::NoProject)));
    assert!(matches!(
        e.st.get_screen(&copy.doc.id),
        Err(AppError::NoProject)
    ));
    assert!(matches!(
        e.st.save_screen(copy.doc.clone()),
        Err(AppError::NoProject)
    ));
    assert!(matches!(
        e.st.set_layer_rect(
            &copy.doc.id,
            "x",
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1
            }
        ),
        Err(AppError::NoProject)
    ));
}

#[test]
fn opening_a_project_loads_its_screens_off_air() {
    let e = env();
    let t = e.template();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers = vec![layer("l1", &t.id), layer("gone", "missing01")];
    set_field(&mut doc, 0, "name", "Ada");
    e.st.save_screen(doc).unwrap();
    e.st.live().show(&id, "l1").unwrap();
    // A corrupt screen file is set aside on open.
    let corrupt = e.dir.join("screens/broken0001.json");
    fs::write(&corrupt, "{ nope").unwrap();
    e.st.close_project();
    assert!(!e.st.live().has_screen(&id));

    e.st.open_project(&e.dir).unwrap();
    let v = e.st.live().screen_view(&id).unwrap();
    assert_eq!(v.layers.len(), 1, "missing template left out");
    assert!(!v.layers[0].on_air);
    assert_eq!(v.layers[0].fields["name"], "Ada");
    assert!(e.st.live().has_screen(TEST_SCREEN_ID));
    assert!(e.st.live().has_screen(PREVIEW_SCREEN_ID));
    assert!(!corrupt.exists());
    let recovered = e.rec.named(names::STORE_RECOVERED);
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0]["quarantinedAs"]
        .as_str()
        .unwrap()
        .contains("broken0001.json.corrupt-"));
    let s = e.st.get_screen(&id).unwrap();
    assert_eq!(s.warnings.len(), 1);
    assert_eq!(s.warnings[0].code, ScreenWarningCode::TemplateMissing);
}

#[test]
fn restore_resyncs_screens() {
    let e = env();
    let a = e.screen("A");
    let info =
        e.st.create_snapshot(crate::core::model::BackupReason::Manual)
            .unwrap();
    let b = e.screen("B");
    assert!(e.st.live().has_screen(&b.id));
    e.st.restore_backup(&info.id).unwrap();
    assert!(e.st.live().has_screen(&a.id));
    assert!(!e.st.live().has_screen(&b.id), "not in the snapshot");
    assert!(e.st.watcher_running());
}

// ---- Watcher ---------------------------------------------------------------------------------

fn files_changed(e: &Env) -> Vec<Value> {
    e.rec.named(names::PROJECT_FILES_CHANGED)
}

#[test]
fn watcher_reloads_external_edits_and_ignores_own_writes() {
    let e = env();
    assert!(e.st.watcher_running());
    let t = e.template();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers.push(layer("l1", &t.id));
    e.st.save_screen(doc.clone()).unwrap();
    // Own writes (create, save, template create) produce no event: the watcher evaluates them
    // and reports nothing.
    wait_until("the watcher's batch", || e.st.watcher_batches() > 0);
    assert!(files_changed(&e).is_empty(), "{:?}", files_changed(&e));
    let mut rx = e.st.live().subscribe();

    // External screen edit: reloaded and applied.
    let path = e.dir.join("screens").join(format!("{id}.json"));
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("\"Main\"", "\"Edited\"");
    fs::write(&path, text).unwrap();
    wait_until("screen change", || !files_changed(&e).is_empty());
    assert_eq!(
        files_changed(&e)[0],
        json!({"kind": "screen", "id": id, "change": "changed", "message": null})
    );
    assert_eq!(e.st.live().screen_view(&id).unwrap().screen.name, "Edited");
    assert_eq!(kinds(&drain(&mut rx, &id)), ["snapshot"]);
    assert_eq!(e.st.list_screen_docs().unwrap()[0].name, "Edited");

    // External template edit: its layers follow.
    let css = e
        .dir
        .join("templates")
        .join(&t.id)
        .join("variants/default/template.css");
    let mut text = fs::read_to_string(&css).unwrap();
    text.push_str("\n.outside { color: blue; }\n");
    fs::write(&css, text).unwrap();
    wait_until("template change", || files_changed(&e).len() == 2);
    assert_eq!(files_changed(&e)[1]["kind"], "template");
    assert_eq!(files_changed(&e)[1]["change"], "changed");
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer"]);
    assert!(msgs[0]["layer"]["css"]
        .as_str()
        .unwrap()
        .contains(".outside"));

    // Invalid external screen file: announced, last good state kept, file left alone.
    fs::write(&path, "{ half written").unwrap();
    wait_until("invalid", || files_changed(&e).len() == 3);
    let ev = &files_changed(&e)[2];
    assert_eq!(ev["change"], "invalid");
    assert!(!ev["message"].as_str().unwrap().is_empty());
    assert_eq!(e.st.live().screen_view(&id).unwrap().layers.len(), 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), "{ half written");

    // Removed outside: dropped from the live state.
    fs::remove_file(&path).unwrap();
    wait_until("removed", || files_changed(&e).len() == 4);
    assert_eq!(files_changed(&e)[3]["change"], "removed");
    assert!(!e.st.live().has_screen(&id));

    // A new screen file appears: it goes live.
    let new_path = e.dir.join("screens/fresh00001.json");
    fs::write(
        &new_path,
        r##"{"schemaVersion":1,"name":"Fresh","width":1280,"height":720,"background":"#000000"}"##,
    )
    .unwrap();
    wait_until("added", || e.st.live().has_screen("fresh00001"));
}

#[test]
fn watcher_stops_with_the_project() {
    let e = env();
    let doc = e.screen("Main");
    assert!(e.st.watcher_running());
    e.st.close_project();
    assert!(!e.st.watcher_running(), "stopped and joined");
    let path = e.dir.join("screens").join(format!("{}.json", doc.id));
    fs::write(&path, "{ broken").unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert!(files_changed(&e).is_empty());
    // Reopening watches again (the broken file was set aside on open).
    e.st.open_project(&e.dir).unwrap();
    assert!(e.st.watcher_running());
    // Switching to another project moves the watcher there.
    let other = e.st.create_project(e._tmp.path(), "Other").unwrap();
    assert!(e.st.watcher_running());
    fs::write(e.dir.join("screens/late000001.json"), "{}").unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        files_changed(&e).is_empty(),
        "old project no longer watched"
    );
    let _ = other;
}

#[test]
fn reloading_an_unchanged_file_announces_nothing() {
    // Our own write seen late (after the own-write memory expired) must not look external.
    let e = env();
    let doc = e.screen("Main");
    assert!(e.st.reload_screen(&e.dir, &doc.id).is_none());

    let path = e.dir.join("screens").join(format!("{}.json", doc.id));
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("\"Main\"", "\"Edited\"")).unwrap();
    let c = e.st.reload_screen(&e.dir, &doc.id).unwrap();
    assert_eq!(c.change, FileChange::Changed);
    assert_eq!(e.st.get_screen(&doc.id).unwrap().doc.meta.name, "Edited");
}

// ---- Template variants ------------------------------------------------------------------------

impl Env {
    /// The classic-bar template with a second variant `compact` (its own look and accent).
    fn template_with_compact(&self) -> TemplateDoc {
        let t = self.template();
        let doc = self
            .st
            .templates()
            .create_variant(&self.dir, &t.id, "default", "Compact")
            .unwrap();
        self.st.template_saved(&doc);
        let doc = self
            .st
            .templates()
            .save_variant(
                &self.dir,
                &t.id,
                "compact",
                "<div class=\"c\">{{name}} / {{title}}</div>",
                ".c { color: var(--accent); }",
                Fields::from([("accent".to_string(), "#00ff00".to_string())]),
            )
            .unwrap();
        self.st.template_saved(&doc);
        doc
    }

    fn view(&self, screen: &str, layer: &str) -> LiveLayerView {
        self.st.live().layer_view(screen, layer).unwrap()
    }
}

fn layer_variant(doc: &ScreenDoc, layer: usize) -> Option<String> {
    let ScreenLayerKind::Template { variant, .. } = &doc.meta.layers[layer].kind else {
        unreachable!()
    };
    variant.clone()
}

#[test]
fn set_layer_variant_off_air_applies_at_once() {
    let e = env();
    let t = e.template_with_compact();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers.push(layer("l1", &t.id));
    set_field(&mut doc, 0, "name", "Ada");
    e.st.save_screen(doc).unwrap();
    assert_eq!(e.view(&id, "l1").variant, "default");
    let mut rx = e.st.live().subscribe();

    let s =
        e.st.set_layer_variant(&id, "l1", "compact", VariantTransition::OutIn)
            .unwrap();
    assert_eq!(layer_variant(&s.doc, 0).as_deref(), Some("compact"));
    assert!(s.warnings.is_empty());
    assert_eq!(s.live.layers[0].variant, "compact");
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer"]);
    let wl = &msgs[0]["layer"];
    assert_eq!(wl["templateRev"], 2);
    assert!(wl.get("swap").is_none(), "off air: no swap");
    assert_eq!(wl["state"], "out");
    assert!(wl["html"].as_str().unwrap().contains("class=\"c\""));
    assert_eq!(wl["vars"]["accent"], "#00ff00", "the variant's default");
    assert_eq!(wl["fields"]["name"], "Ada", "field values untouched");

    // Saved (coalesced): in memory now, on disk after the flush. The default is stored as absent.
    assert_eq!(
        layer_variant(&e.st.get_screen(&id).unwrap().doc, 0).as_deref(),
        Some("compact")
    );
    e.st.flush_all();
    assert_eq!(
        layer_variant(&screens::get(&e.dir, &id).unwrap(), 0).as_deref(),
        Some("compact")
    );
    let s =
        e.st.set_layer_variant(&id, "l1", "default", VariantTransition::Crossfade)
            .unwrap();
    assert_eq!(layer_variant(&s.doc, 0), None);
    assert_eq!(drain(&mut rx, &id)[0]["layer"]["templateRev"], 3);
    // Same variant again: nothing to send.
    e.st.set_layer_variant(&id, "l1", "default", VariantTransition::Crossfade)
        .unwrap();
    assert!(drain(&mut rx, &id).is_empty());

    // Errors.
    let x = VariantTransition::Crossfade;
    assert!(matches!(
        e.st.set_layer_variant(&id, "l1", "ghost", x),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.st.set_layer_variant(&id, "nope", "compact", x),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.st.set_layer_variant("nope", "l1", "compact", x),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.st.set_layer_variant(TEST_SCREEN_ID, "lower-third", "compact", x),
        Err(AppError::NotFound(_))
    ));
    e.st.templates().delete(&e.dir, &t.id).unwrap();
    e.st.template_deleted(&t.id);
    assert!(matches!(
        e.st.set_layer_variant(&id, "l1", "compact", x),
        Err(AppError::Invalid(_))
    ));
}

#[test]
fn set_layer_variant_on_air_on_take_stages_until_the_next_take() {
    let e = env();
    let t = e.template_with_compact();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers.push(layer("l1", &t.id));
    e.st.save_screen(doc).unwrap();
    e.st.live().show(&id, "l1").unwrap();
    let mut rx = e.st.live().subscribe();

    let s =
        e.st.set_layer_variant(&id, "l1", "compact", VariantTransition::OutIn)
            .unwrap();
    assert!(
        drain(&mut rx, &id).is_empty(),
        "staged: the output keeps showing"
    );
    assert!(s.live.layers[0].pending);
    assert_eq!(s.live.layers[0].variant, "compact", "the staged variant");
    assert_eq!(
        e.st.live()
            .with_layer(&id, "l1", |l| l.variant.clone())
            .unwrap(),
        "default"
    );

    // Back to what is on air: nothing pending any more.
    e.st.set_layer_variant(&id, "l1", "default", VariantTransition::Crossfade)
        .unwrap();
    assert!(!e.pending(&id, "l1"));
    e.st.set_layer_variant(&id, "l1", "compact", VariantTransition::OutIn)
        .unwrap();

    // The take applies it: a new rev, no swap, no state change (no out/in replay).
    let v = e.st.live().show(&id, "l1").unwrap();
    assert!(!v.pending && v.on_air);
    assert_eq!(v.variant, "compact");
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer"]);
    assert_eq!(msgs[0]["layer"]["state"], "in");
    assert_eq!(msgs[0]["layer"]["templateRev"], 2);
    assert!(msgs[0]["layer"].get("swap").is_none());
}

#[test]
fn set_layer_variant_on_air_immediate_swaps_with_the_transition() {
    let e = env();
    let t = e.template_with_compact();
    e.set_mode(LiveEdit::Immediate);
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers.push(layer("l1", &t.id));
    e.st.save_screen(doc).unwrap();
    e.st.live().show(&id, "l1").unwrap();
    let mut rx = e.st.live().subscribe();

    let s =
        e.st.set_layer_variant(&id, "l1", "compact", VariantTransition::OutIn)
            .unwrap();
    assert!(!s.live.layers[0].pending);
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer"], "one message per switch");
    let wl = &msgs[0]["layer"];
    assert_eq!(wl["swap"], "outIn");
    assert_eq!(wl["state"], "in");
    assert_eq!(wl["templateRev"], 2);

    e.st.set_layer_variant(&id, "l1", "default", VariantTransition::Crossfade)
        .unwrap();
    let msgs = drain(&mut rx, &id);
    assert_eq!(msgs[0]["layer"]["swap"], "crossfade");
    assert_eq!(msgs[0]["layer"]["templateRev"], 3);

    // Snapshots (reconnects) never swap.
    let snap: Value = serde_json::from_str(&e.st.live().snapshot_json(&id).unwrap()).unwrap();
    assert!(snap["layers"][0].get("swap").is_none());
    // A variant change saved with the screen (Screens inspector) swaps too, by crossfade.
    let mut doc = e.st.get_screen(&id).unwrap().doc;
    let ScreenLayerKind::Template { variant, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    *variant = Some("compact".into());
    e.st.save_screen(doc).unwrap();
    assert_eq!(drain(&mut rx, &id)[0]["layer"]["swap"], "crossfade");
}

#[test]
fn field_values_survive_switching_variants() {
    let e = env();
    let t = e.template_with_compact();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers.push(layer("l1", &t.id));
    e.st.save_screen(doc).unwrap();
    let vals = Fields::from([
        ("name".to_string(), "Ada".to_string()),
        ("title".to_string(), "Host".to_string()),
    ]);
    e.st.set_layer_fields(&id, "l1", vals.clone()).unwrap();
    let mut rx = e.st.live().subscribe();
    let tr = VariantTransition::Crossfade;

    // Off air: away and back.
    e.st.set_layer_variant(&id, "l1", "compact", tr).unwrap();
    assert_eq!(e.view(&id, "l1").fields, vals);
    e.st.set_layer_variant(&id, "l1", "default", tr).unwrap();
    assert_eq!(e.view(&id, "l1").fields, vals);
    for m in drain(&mut rx, &id) {
        assert_eq!(m["layer"]["fields"]["name"], "Ada");
        assert_eq!(m["layer"]["fields"]["title"], "Host");
    }

    // On air (immediate): the swapped layer carries the same values, both ways.
    e.set_mode(LiveEdit::Immediate);
    e.st.live().show(&id, "l1").unwrap();
    drain(&mut rx, &id);
    e.st.set_layer_variant(&id, "l1", "compact", tr).unwrap();
    e.st.set_layer_variant(&id, "l1", "default", tr).unwrap();
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer", "layer"]);
    for m in &msgs {
        assert_eq!(m["layer"]["fields"]["name"], "Ada");
        assert_eq!(m["layer"]["fields"]["title"], "Host");
        assert_eq!(m["layer"]["state"], "in");
    }
    assert_eq!(e.view(&id, "l1").fields, vals);

    // Layer overrides beat the variant's defaults, which beat the definition's.
    let mut doc = e.st.get_screen(&id).unwrap().doc;
    let ScreenLayerKind::Template { vars, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    vars.insert("text".into(), "#111111".into());
    e.st.save_screen(doc).unwrap();
    drain(&mut rx, &id);
    e.st.set_layer_variant(&id, "l1", "compact", tr).unwrap();
    let wl = &drain(&mut rx, &id).pop().unwrap()["layer"];
    assert_eq!(wl["vars"]["accent"], "#00ff00");
    assert_eq!(wl["vars"]["text"], "#111111");
    assert_eq!(wl["vars"]["panel"], "#0d1b2ae6");
    let mut doc = e.st.get_screen(&id).unwrap().doc;
    let ScreenLayerKind::Template { vars, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    vars.insert("accent".into(), "#222222".into());
    e.st.save_screen(doc).unwrap();
    assert_eq!(
        e.st.live()
            .with_layer(&id, "l1", |l| l.vars["accent"].clone())
            .unwrap(),
        "#222222"
    );
}

#[test]
fn a_missing_variant_falls_back_to_the_default() {
    let e = env();
    let t = e.template_with_compact();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    let mut l = layer("l1", &t.id);
    let ScreenLayerKind::Template { variant, .. } = &mut l.kind else {
        unreachable!()
    };
    *variant = Some("ghost".into());
    doc.meta.layers.push(l);
    let s = e.st.save_screen(doc).unwrap();
    assert_eq!(s.warnings.len(), 1);
    assert_eq!(s.warnings[0].code, ScreenWarningCode::VariantMissing);
    assert!(
        s.warnings[0].message.contains("ghost"),
        "{}",
        s.warnings[0].message
    );
    assert_eq!(s.live.layers[0].variant, "default");
    // Kept in the file (the variant may come back).
    assert_eq!(layer_variant(&s.doc, 0).as_deref(), Some("ghost"));

    // Deleting the variant a layer shows: it falls back to the default (no screen rewrite).
    e.st.set_layer_variant(&id, "l1", "compact", VariantTransition::Crossfade)
        .unwrap();
    e.st.flush_all();
    let mut rx = e.st.live().subscribe();
    let doc =
        e.st.templates()
            .delete_variant(&e.dir, &t.id, "compact")
            .unwrap();
    e.st.template_saved(&doc);
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer"]);
    assert!(!msgs[0]["layer"]["html"]
        .as_str()
        .unwrap()
        .contains("class=\"c\""));
    let s = e.st.get_screen(&id).unwrap();
    assert_eq!(s.live.layers[0].variant, "default");
    assert_eq!(s.warnings[0].code, ScreenWarningCode::VariantMissing);
    assert_eq!(layer_variant(&s.doc, 0).as_deref(), Some("compact"));

    // A new default: layers without a variant follow it.
    let doc =
        e.st.templates()
            .create_variant(&e.dir, &t.id, "default", "Big")
            .unwrap();
    e.st.template_saved(&doc);
    let mut doc2 = e.screen("Second");
    doc2.meta.layers.push(layer("l2", &t.id));
    let s2 = e.st.save_screen(doc2).unwrap();
    assert_eq!(s2.live.layers[0].variant, "default");
    let doc =
        e.st.templates()
            .set_default_variant(&e.dir, &t.id, "big")
            .unwrap();
    e.st.template_saved(&doc);
    assert_eq!(e.view(&s2.doc.id, "l2").variant, "big");
}

fn pre_migrate_backups(dir: &std::path::Path) -> Vec<crate::core::model::BackupInfo> {
    crate::core::backup::list(dir)
        .unwrap()
        .into_iter()
        .filter(|b| b.reason == crate::core::model::BackupReason::PreMigrate)
        .collect()
}

#[test]
fn opening_a_project_migrates_v1_templates_once_with_a_backup() {
    let e = env();
    let dir = e.dir.clone();
    e.st.close_project();
    crate::core::templates::tests::write_v1(&dir, "old0000001", "<div>{{name}}</div>", ".a{}");
    let mut doc = ScreenDoc {
        id: "scr0000001".into(),
        meta: Default::default(),
    };
    doc.meta.name = "Main".into();
    doc.meta.layers.push(layer("l1", "old0000001"));
    screens::save(&dir, doc, true).unwrap();

    e.st.open_project(&dir).unwrap();
    let b = pre_migrate_backups(&dir);
    assert_eq!(b.len(), 1);
    let snap = dir.join(".backups").join(&b[0].id);
    assert!(
        snap.join("templates/old0000001/template.html").is_file(),
        "the old files are kept"
    );
    let t = dir.join("templates/old0000001");
    assert!(t.join("variants/default/template.html").is_file());
    assert!(!t.join("template.html").exists());
    let s = e.st.get_screen("scr0000001").unwrap();
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    assert_eq!(s.live.layers[0].variant, "default");
    assert_eq!(e.st.templates().list(&dir).unwrap()[0].variant_count, 1);

    // The next open does nothing.
    e.st.close_project();
    e.st.open_project(&dir).unwrap();
    assert_eq!(pre_migrate_backups(&dir).len(), 1);
    assert!(e.rec.named(names::PROJECT_FILES_CHANGED).is_empty());
}

#[test]
fn the_watcher_ignores_own_variant_writes_and_reloads_external_ones() {
    let e = env();
    assert!(e.st.watcher_running());
    let t = e.template_with_compact();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers.push(layer("l1", &t.id));
    e.st.save_screen(doc).unwrap();
    e.st.set_layer_variant(&id, "l1", "compact", VariantTransition::Crossfade)
        .unwrap();
    let store = e.st.templates();
    let d = store
        .create_variant(&e.dir, &t.id, "compact", "Wide")
        .unwrap();
    e.st.template_saved(&d);
    store
        .rename_variant(&e.dir, &t.id, "wide", "Wider")
        .unwrap();
    store.set_default_variant(&e.dir, &t.id, "wide").unwrap();
    store.delete_variant(&e.dir, &t.id, "wide").unwrap();
    // Own writes (create, save, rename, default, delete) produce no event.
    let n = e.st.watcher_batches();
    wait_until("the watcher's batch", || e.st.watcher_batches() > n);
    std::thread::sleep(crate::core::watch::DEBOUNCE * 2);
    assert!(files_changed(&e).is_empty(), "{:?}", files_changed(&e));

    // An external edit of a variant's CSS: the layers showing it follow.
    let mut rx = e.st.live().subscribe();
    let css = e
        .dir
        .join("templates")
        .join(&t.id)
        .join("variants/compact/template.css");
    fs::write(&css, ".c { color: red; }\n.outside {}\n").unwrap();
    wait_until("template change", || !files_changed(&e).is_empty());
    assert_eq!(files_changed(&e)[0]["kind"], "template");
    assert_eq!(files_changed(&e)[0]["change"], "changed");
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer"]);
    assert!(msgs[0]["layer"]["css"]
        .as_str()
        .unwrap()
        .contains(".outside"));
}

fn set_sizing(doc: &mut ScreenDoc, layer: usize, s: Option<crate::core::model::ContentSizing>) {
    let ScreenLayerKind::Template { sizing, .. } = &mut doc.meta.layers[layer].kind else {
        unreachable!()
    };
    *sizing = s;
}

#[test]
fn sizing_round_trips_and_follows_the_live_rules() {
    use crate::core::model::{ContentSizing, TemplateDesign};
    let e = env();
    let mut t = e.template();
    t.meta.design = Some(TemplateDesign {
        w: 960,
        h: 160,
        x: Some(96.0),
        y: None,
        sizing: ContentSizing::Fit,
    });
    let t = e.st.templates().save(&e.dir, t).unwrap();
    e.st.template_saved(&t);
    let json =
        fs::read_to_string(e.dir.join("templates").join(&t.id).join("template.json")).unwrap();
    assert!(json.contains("\"design\""), "{json}");
    let back = e.st.templates().get(&e.dir, &t.id).unwrap();
    assert_eq!(back.meta.design, t.meta.design);

    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    doc.meta.layers.push(layer("l1", &t.id));
    doc.meta.layers.push(layer("l2", &t.id));
    set_sizing(&mut doc, 0, Some(ContentSizing::Fit));
    let mut rx = e.st.live().subscribe();
    e.st.save_screen(doc.clone()).unwrap();
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer", "layer"]);
    assert_eq!(msgs[0]["layer"]["sizing"], "fit");
    assert_eq!(msgs[0]["layer"]["design"], json!({"w": 960, "h": 160}));
    assert!(msgs[1]["layer"].get("sizing").is_none(), "absent = reflow");
    let rev = msgs[0]["layer"]["templateRev"].clone();
    // The file holds `sizing` only where it is set.
    let file = fs::read_to_string(e.dir.join("screens").join(format!("{id}.json"))).unwrap();
    assert_eq!(file.matches("\"sizing\"").count(), 1, "{file}");
    let ScreenLayerKind::Template { sizing, .. } =
        &e.st.get_screen(&id).unwrap().doc.meta.layers[0].kind
    else {
        unreachable!()
    };
    assert_eq!(*sizing, Some(ContentSizing::Fit));

    // On air under onTake: staged; the next show applies it without a recompile.
    e.st.live().show(&id, "l2").unwrap();
    drain(&mut rx, &id);
    set_sizing(&mut doc, 1, Some(ContentSizing::Fit));
    e.st.save_screen(doc.clone()).unwrap();
    assert!(drain(&mut rx, &id).is_empty());
    assert!(e.pending(&id, "l2"));
    e.st.live().show(&id, "l2").unwrap();
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer"]);
    assert_eq!(msgs[0]["layer"]["sizing"], "fit");
    assert_eq!(msgs[0]["layer"]["templateRev"], rev);
    // Immediate: a same-rev `layer` at once.
    e.set_mode(LiveEdit::Immediate);
    set_sizing(&mut doc, 1, Some(ContentSizing::Reflow));
    e.st.save_screen(doc.clone()).unwrap();
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer"]);
    assert!(msgs[0]["layer"].get("sizing").is_none());
    assert_eq!(msgs[0]["layer"]["templateRev"], rev);

    // A template save that changes the design re-sends both layers (same rev).
    let mut edited = e.st.templates().get(&e.dir, &t.id).unwrap();
    edited.meta.design.as_mut().unwrap().w = 1200;
    let saved = e.st.templates().save(&e.dir, edited).unwrap();
    e.st.template_saved(&saved);
    let msgs = drain(&mut rx, &id);
    assert_eq!(kinds(&msgs), ["layer", "layer"], "{msgs:?}");
    for m in &msgs {
        assert_eq!(m["layer"]["design"]["w"], 1200);
        assert_eq!(m["layer"]["templateRev"], rev);
    }
}

// ---- Anchors ------------------------------------------------------------------------------------

fn anchor(
    h: Option<crate::core::model::AnchorH>,
    v: Option<crate::core::model::AnchorV>,
    safe: bool,
) -> Option<crate::core::model::LayerAnchor> {
    Some(crate::core::model::LayerAnchor { h, v, safe })
}

fn rect(x: i32, y: i32, w: u32, h: u32) -> Rect {
    Rect { x, y, w, h }
}

/// Screen `Main` with layer `l1` (400 × 100) anchored centre / middle and saved.
fn anchored_env() -> (Env, ScreenDoc) {
    use crate::core::model::{AnchorH, AnchorV};
    let e = env();
    let t = e.template();
    let mut doc = e.screen("Main");
    let mut l = layer("l1", &t.id);
    l.rect = rect(0, 0, 400, 100);
    l.anchor = anchor(Some(AnchorH::Center), Some(AnchorV::Middle), false);
    doc.meta.layers.push(l);
    let doc = e.st.save_screen(doc).unwrap().doc;
    (e, doc)
}

#[test]
fn anchors_place_layers_on_save_and_follow_size_changes() {
    use crate::core::model::{AnchorH, AnchorV};
    let (e, mut doc) = anchored_env();
    let id = doc.id.clone();
    let c = anchor(Some(AnchorH::Center), Some(AnchorV::Middle), false);
    // The new anchor placed the layer.
    assert_eq!(doc.meta.layers[0].rect, rect(760, 490, 400, 100));
    let file = fs::read_to_string(e.dir.join("screens").join(format!("{id}.json"))).unwrap();
    assert!(file.contains("\"anchor\""), "{file}");
    // A resize keeps the anchor: still centred.
    doc.meta.layers[0].rect.w = 600;
    let doc2 = e.st.save_screen(doc.clone()).unwrap().doc;
    assert_eq!(doc2.meta.layers[0].rect, rect(660, 490, 600, 100));
    assert_eq!(doc2.meta.layers[0].anchor, c);
    // A screen resize re-places it.
    let mut doc3 = doc2.clone();
    doc3.meta.width = 1280;
    doc3.meta.height = 720;
    let doc3 = e.st.save_screen(doc3).unwrap().doc;
    assert_eq!(doc3.meta.layers[0].rect, rect(340, 310, 600, 100));
    assert_eq!(e.st.live().screen_size(&id), Some((1280, 720)));
    assert_eq!(
        e.st.live().with_layer(&id, "l1", |l| l.rect).unwrap(),
        Some(rect(340, 310, 600, 100))
    );
    // A hand move (same size) releases the moved axis only.
    let mut moved = doc3.clone();
    moved.meta.layers[0].rect.x = 20;
    let saved = e.st.save_screen(moved).unwrap().doc;
    assert_eq!(saved.meta.layers[0].rect, rect(20, 310, 600, 100));
    assert_eq!(
        saved.meta.layers[0].anchor,
        anchor(None, Some(AnchorV::Middle), false)
    );
    // Moving the other axis too drops the anchor.
    let mut moved = saved.clone();
    moved.meta.layers[0].rect.y = 5;
    let saved = e.st.save_screen(moved).unwrap().doc;
    assert_eq!(saved.meta.layers[0].anchor, None);
    assert_eq!(saved.meta.layers[0].rect, rect(20, 5, 600, 100));
    // A changed anchor wins over the rect's position (the grid click); safe uses title safe.
    doc = saved;
    doc.meta.layers[0].anchor = anchor(Some(AnchorH::Right), Some(AnchorV::Bottom), true);
    doc.meta.layers[0].rect.x = 3;
    let saved = e.st.save_screen(doc).unwrap().doc;
    // 1280 - 64 - 600, 720 - 36 - 100.
    assert_eq!(saved.meta.layers[0].rect, rect(616, 584, 600, 100));
    assert_eq!(e.st.get_screen(&id).unwrap().doc, saved);
}

#[test]
fn screens_without_anchors_load_and_save_as_before() {
    let e = env();
    let t = e.template();
    let mut doc = e.screen("Main");
    let id = doc.id.clone();
    let mut l = layer("l1", &t.id);
    l.rect = rect(13, 17, 400, 100);
    doc.meta.layers.push(l);
    let saved = e.st.save_screen(doc.clone()).unwrap().doc;
    assert_eq!(saved.meta.layers[0].rect, rect(13, 17, 400, 100));
    let path = e.dir.join("screens").join(format!("{id}.json"));
    let file = fs::read_to_string(&path).unwrap();
    assert!(!file.contains("anchor"), "{file}");
    // An empty anchor is normalized away.
    let mut d = saved.clone();
    d.meta.layers[0].anchor = anchor(None, None, false);
    let s = e.st.save_screen(d).unwrap().doc;
    assert_eq!(s, saved, "nothing changed");
    assert!(!fs::read_to_string(&path).unwrap().contains("anchor"));
    e.st.close_project();
    e.st.open_project(&e.dir).unwrap();
    assert_eq!(
        e.st.get_screen(&id).unwrap().doc.meta.layers[0].rect,
        rect(13, 17, 400, 100)
    );
}

#[test]
fn set_layer_rect_releases_the_moved_axis_and_resizes_stay_anchored() {
    use crate::core::model::AnchorV;
    let (e, doc) = anchored_env();
    let id = doc.id.clone();
    let mut rx = e.st.live().subscribe();
    // A resize while dragging: the box stays centred.
    e.st.set_layer_rect(&id, "l1", rect(760, 490, 500, 100))
        .unwrap();
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "rect", "layerId": "l1", "rect": {"x": 710, "y": 490, "w": 500, "h": 100}})]
    );
    // Its drop's save.
    let mut doc = doc;
    doc.meta.layers[0].rect = rect(760, 490, 500, 100);
    let doc = e.st.save_screen(doc).unwrap().doc;
    assert_eq!(doc.meta.layers[0].rect, rect(710, 490, 500, 100));
    drain(&mut rx, &id);
    // A horizontal drag: h released (the drag isn't fought), v kept.
    e.st.set_layer_rect(&id, "l1", rect(100, 490, 500, 100))
        .unwrap();
    assert_eq!(drain(&mut rx, &id)[0]["rect"]["x"], 100);
    let mem = e.st.screens().doc(&id).unwrap();
    assert_eq!(
        mem.meta.layers[0].anchor,
        anchor(None, Some(AnchorV::Middle), false)
    );
    // Further frames keep moving freely.
    e.st.set_layer_rect(&id, "l1", rect(140, 490, 500, 100))
        .unwrap();
    assert_eq!(drain(&mut rx, &id)[0]["rect"]["x"], 140);
    // The drop's save from an editor that still carries the old anchor: released the same way.
    let mut dropped = doc.clone();
    dropped.meta.layers[0].rect = rect(140, 490, 500, 100);
    let saved = e.st.save_screen(dropped).unwrap().doc;
    assert_eq!(saved.meta.layers[0].rect, rect(140, 490, 500, 100));
    assert_eq!(
        saved.meta.layers[0].anchor,
        anchor(None, Some(AnchorV::Middle), false)
    );
    // The same save from an editor that applied the rule itself.
    let saved2 = e.st.save_screen(saved.clone()).unwrap().doc;
    assert_eq!(saved2, saved);
}

#[test]
fn anchored_rect_changes_follow_the_live_rules_on_air() {
    let (e, doc) = anchored_env();
    let id = doc.id.clone();
    let mut rx = e.st.live().subscribe();
    e.st.live().show(&id, "l1").unwrap();
    drain(&mut rx, &id);
    // onTake: a resize re-places the box, staged.
    let mut d = doc.clone();
    d.meta.layers[0].rect.w = 600;
    let s = e.st.save_screen(d.clone()).unwrap();
    assert!(s.live.layers[0].pending);
    assert!(drain(&mut rx, &id).is_empty());
    assert_eq!(
        e.st.live().with_layer(&id, "l1", |l| l.rect).unwrap(),
        Some(rect(760, 490, 400, 100)),
        "the output still shows the old box"
    );
    let v = e.st.live().show(&id, "l1").unwrap();
    assert!(!v.pending);
    let msgs = drain(&mut rx, &id);
    assert!(
        msgs.iter()
            .any(|m| m["t"] == "rect"
                && m["rect"] == json!({"x": 660, "y": 490, "w": 600, "h": 100})),
        "{msgs:?}"
    );
    // Immediate: at once.
    e.set_mode(LiveEdit::Immediate);
    d.meta.layers[0].rect.w = 800;
    e.st.save_screen(d).unwrap();
    assert_eq!(
        drain(&mut rx, &id),
        [json!({"t": "rect", "layerId": "l1", "rect": {"x": 560, "y": 490, "w": 800, "h": 100}})]
    );
}

#[test]
fn a_hand_edited_file_releases_the_moved_axis_on_reload() {
    use crate::core::model::AnchorV;
    let (e, doc) = anchored_env();
    let id = doc.id.clone();
    let path = e.dir.join("screens").join(format!("{id}.json"));
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"x\": 760"), "{text}");
    fs::write(&path, text.replace("\"x\": 760", "\"x\": 33")).unwrap();
    let c = e.st.reload_screen(&e.dir, &id).unwrap();
    assert_eq!(c.change, FileChange::Changed);
    let got = e.st.get_screen(&id).unwrap().doc;
    assert_eq!(got.meta.layers[0].rect, rect(33, 490, 400, 100));
    assert_eq!(
        got.meta.layers[0].anchor,
        anchor(None, Some(AnchorV::Middle), false)
    );
    assert_eq!(
        e.st.live().with_layer(&id, "l1", |l| l.rect).unwrap(),
        Some(rect(33, 490, 400, 100))
    );
    // Written back by the content flusher.
    e.st.flush_all();
    let file = fs::read_to_string(&path).unwrap();
    assert!(file.contains("\"h\": null"), "{file}");
    // A hand edit of the size keeps the anchor (re-placed).
    fs::write(&path, file.replace("\"w\": 400", "\"w\": 200")).unwrap();
    e.st.reload_screen(&e.dir, &id).unwrap();
    let got = e.st.get_screen(&id).unwrap().doc;
    assert_eq!(got.meta.layers[0].rect, rect(33, 490, 200, 100));
    assert_eq!(
        got.meta.layers[0].anchor,
        anchor(None, Some(AnchorV::Middle), false)
    );
}
