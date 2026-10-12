use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::core::events::EventSink;
use crate::core::model::{
    ContentSizing, NewScreen, Rect, ScreenDoc, ScreenLayer, ScreenLayerKind, TemplateSource,
    VariableKind,
};
use crate::core::presets::testing::{self, Fixture};
use crate::core::protocol::Fields;
use crate::core::templates::TemplateStore;

/// `testing::library()` as version 2 of its presets:
/// - `lt`: version 2, a design box, new looks (`lt2-*`), `title`'s default changed from "" (listed
///   in history.json), `label` dropped (history `removed`), `subtitle` added, `accent`'s default
///   changed (old one in history), `radius` became a select, `frame`'s vars changed;
/// - `legacy-bar`: version 2, still one root look (schema 1);
/// - `poll`: unchanged (version 1).
fn v2() -> Fixture {
    let mut f = testing::library();
    let path = "lt/template.json".to_string();
    let mut v: Value = serde_json::from_slice(&f.files[&path]).unwrap();
    v["preset"] = json!({ "id": "lt", "version": 2 });
    v["design"] = json!({ "w": 800, "h": 200, "x": 100, "y": 800, "sizing": "fit" });
    v["fields"] = json!([
        testing::field("name", "text"),
        { "key": "title", "label": "Job title", "type": "text", "default": "Title" },
        testing::field("subtitle", "text"),
        testing::field("logo", "image"),
    ]);
    let vars = v["variables"].as_array_mut().unwrap();
    for var in vars.iter_mut() {
        if var["key"] == "accent" {
            var["default"] = json!("#0000ff");
        }
        if var["key"] == "radius" {
            *var = json!({
                "key": "radius", "label": "Corners", "type": "select",
                "options": [{ "value": "sharp", "label": "Sharp" }, { "value": "round", "label": "Round" }],
                "default": "round"
            });
        }
    }
    v["variants"] = json!([
        { "id": "classic", "name": "Classic", "vars": {} },
        { "id": "frame", "name": "Frame", "vars": { "accent": "#123456" } }
    ]);
    f.files.insert(path, v.to_string().into_bytes());
    for vid in ["classic", "frame"] {
        f.files.insert(
            format!("lt/variants/{vid}/template.html"),
            format!("<div class=\"lt2-{vid}\">{{{{name}}}} {{{{subtitle}}}}</div>").into_bytes(),
        );
        f.files.insert(
            format!("lt/variants/{vid}/template.css"),
            format!(".lt2-{vid} {{ container-type: size; }}").into_bytes(),
        );
    }
    f.files.insert(
        "lt/history.json".into(),
        br##"{ "previousDefaults": { "title": [""], "accent": ["#ff0000"] }, "removed": ["label"] }"##
            .to_vec(),
    );
    let path = "legacy-bar/template.json".to_string();
    let mut v: Value = serde_json::from_slice(&f.files[&path]).unwrap();
    v["preset"] = json!({ "id": "legacy-bar", "version": 2 });
    v["design"] = json!({ "w": 600, "h": 100, "sizing": "fit" });
    f.files.insert(path, v.to_string().into_bytes());
    f.files.insert(
        "legacy-bar/template.html".into(),
        b"<div class=\"bar2\">{{name}}</div>".to_vec(),
    );
    f.files.insert(
        "legacy-bar/template.css".into(),
        b".bar2 { color: blue; }".to_vec(),
    );
    f
}

/// `legacy-bar` as version 2 that now ships variants (`classic` default, `frame`) and no
/// `default` look, like the rebuilt sponsor wall.
fn legacy_with_variants() -> Fixture {
    let mut f = testing::library();
    f.files.remove("legacy-bar/template.html");
    f.files.remove("legacy-bar/template.css");
    f.files.insert(
        "legacy-bar/template.json".into(),
        json!({
            "schemaVersion": 2,
            "name": "Legacy bar",
            "preset": { "id": "legacy-bar", "version": 2 },
            "fields": [testing::field("name", "text")],
            "variables": [],
            "defaultVariant": "classic",
            "variants": [
                { "id": "classic", "name": "Classic", "vars": {} },
                { "id": "frame", "name": "Frame", "vars": {} }
            ]
        })
        .to_string()
        .into_bytes(),
    );
    for vid in ["classic", "frame"] {
        f.files.insert(
            format!("legacy-bar/variants/{vid}/template.html"),
            format!("<div class=\"{vid}\">{{{{name}}}}</div>").into_bytes(),
        );
        f.files.insert(
            format!("legacy-bar/variants/{vid}/template.css"),
            format!(".{vid} {{ color: red; }}").into_bytes(),
        );
    }
    f
}

/// Creates a template from `preset` under the version-1 fixture (as an older Kinetra copied it).
fn made_from(project: &Path, preset: &str, name: &str) -> TemplateDoc {
    let _g = testing::install(testing::library());
    templates::create(
        project,
        name,
        &TemplateSource::Preset {
            preset_id: preset.into(),
        },
    )
    .unwrap()
}

fn field<'a>(doc: &'a TemplateDoc, key: &str) -> Option<&'a FieldDef> {
    doc.meta.fields.iter().find(|f| f.key == key)
}

fn variable<'a>(doc: &'a TemplateDoc, key: &str) -> Option<&'a VariableDef> {
    doc.meta.variables.iter().find(|v| v.key == key)
}

fn look<'a>(doc: &'a TemplateDoc, id: &str) -> &'a crate::core::model::VariantDoc {
    doc.variants.iter().find(|v| v.id == id).unwrap()
}

#[test]
fn preset_update_compares_with_the_shipped_version() {
    let _g = testing::install(v2());
    let r = |id: &str, version| {
        preset_update(Some(&PresetRef {
            id: id.into(),
            version,
        }))
    };
    assert_eq!(r("lt", 1), Some(PresetUpdate { from: 1, to: 2 }));
    assert_eq!(r("lt", 2), None);
    assert_eq!(r("lt", 3), None, "a newer copy than the app's preset");
    assert_eq!(r("poll", 1), None, "unchanged preset");
    assert_eq!(r("gone", 1), None, "unknown preset");
    assert_eq!(r("test-lower-third", 1), None, "dev only");
    assert_eq!(preset_update(None), None, "own template");
}

#[test]
fn summaries_carry_the_update() {
    let tmp = tempfile::tempdir().unwrap();
    let lt = made_from(tmp.path(), "lt", "Strap");
    let poll = made_from(tmp.path(), "poll", "Poll");
    let own = {
        let _g = testing::install(testing::library());
        templates::create(tmp.path(), "Mine", &TemplateSource::Blank).unwrap()
    };
    let _g = testing::install(v2());
    let store = TemplateStore::default();
    let list = store.list(tmp.path()).unwrap();
    let get = |id: &str| list.iter().find(|t| t.id == id).unwrap().preset_update;
    assert_eq!(get(&lt.id), Some(PresetUpdate { from: 1, to: 2 }));
    assert_eq!(get(&poll.id), None);
    assert_eq!(get(&own.id), None);
    store.update_from_preset(tmp.path(), &lt.id).unwrap();
    let list = store.list(tmp.path()).unwrap();
    assert_eq!(
        list.iter().find(|t| t.id == lt.id).unwrap().preset_update,
        None,
        "the write invalidated the cached summaries"
    );
    let json = serde_json::to_value(&list[0]).unwrap();
    assert!(
        json.get("presetUpdate").is_some(),
        "camelCase, null when none"
    );
}

#[test]
fn shipped_variants_take_the_new_look_and_added_ones_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    let t = made_from(p, "lt", "Strap");
    // The user edits Classic, renames Frame, adds a look of their own and makes it the default.
    templates::save_variant(
        p,
        &t.id,
        "classic",
        "<div class=\"mine\">{{name}}</div>",
        ".mine { color: red; }",
        Fields::new(),
    )
    .unwrap();
    templates::rename_variant(p, &t.id, "frame", "Brackets").unwrap();
    let own = templates::create_variant(p, &t.id, "classic", "Studio").unwrap();
    let own_id = own.variants.last().unwrap().id.clone();
    templates::save_variant(
        p,
        &t.id,
        &own_id,
        "<div class=\"studio\">{{name}} {{label}}</div>",
        ".studio { border-radius: var(--radius); }",
        [("radius".to_string(), "10".to_string())].into(),
    )
    .unwrap();
    templates::set_default_variant(p, &t.id, &own_id).unwrap();

    let _g = testing::install(v2());
    let doc = update(p, &t.id).unwrap();
    assert_eq!(
        doc.meta.preset,
        Some(PresetRef {
            id: "lt".into(),
            version: 2
        })
    );
    let design = doc.meta.design.expect("design copied");
    assert_eq!(
        (design.w, design.h, design.sizing),
        (800, 200, ContentSizing::Fit)
    );
    let ids: Vec<&str> = doc.variants.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["classic", "frame", own_id.as_str()], "order kept");
    assert_eq!(
        look(&doc, "classic").html,
        "<div class=\"lt2-classic\">{{name}} {{subtitle}}</div>"
    );
    assert_eq!(
        look(&doc, "frame").css,
        ".lt2-frame { container-type: size; }"
    );
    assert_eq!(look(&doc, "frame").name, "Brackets", "names kept");
    assert_eq!(
        look(&doc, "frame").vars,
        Fields::from([("accent".to_string(), "#123456".to_string())]),
        "the preset's variant defaults"
    );
    let mine = look(&doc, &own_id);
    assert_eq!(
        mine.html, "<div class=\"studio\">{{name}} {{label}}</div>",
        "kept"
    );
    assert!(
        mine.vars.is_empty(),
        "a value the new definition rejects is dropped: {:?}",
        mine.vars
    );
    assert_eq!(doc.meta.default_variant, own_id, "default kept");
    assert_eq!(doc.meta.name, "Strap");
    // On disk as returned.
    assert_eq!(templates::get(p, &t.id).unwrap(), doc);
    // Up to date now: nothing more to do.
    assert!(matches!(update(p, &t.id), Err(TemplateError::UpToDate(_))));
}

#[test]
fn fields_and_variables_merge_by_key() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    let t = made_from(p, "lt", "Strap");
    let mut doc = templates::get(p, &t.id).unwrap();
    for f in &mut doc.meta.fields {
        if f.key == "name" {
            f.default = "Ada".into(); // the user's own default
            f.label = "Guest".into();
        }
    }
    doc.meta.fields.push(FieldDef {
        key: "extra".into(),
        label: "Extra".into(),
        default: "x".into(),
        ..Default::default()
    });
    for v in &mut doc.meta.variables {
        if v.key == "speed" {
            v.default = "1.5".into(); // the user's own default
        }
        if v.key == "radius" {
            v.default = "12".into();
        }
    }
    templates::save(p, doc).unwrap();

    let _g = testing::install(v2());
    let doc = update(p, &t.id).unwrap();
    let keys: Vec<&str> = doc.meta.fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(
        keys,
        ["name", "title", "subtitle", "logo", "extra"],
        "the preset's order, then the user's own; the dropped `label` goes"
    );
    let name = field(&doc, "name").unwrap();
    assert_eq!(name.default, "Ada", "the user's default stays");
    assert_eq!(
        name.label, "name",
        "the definition (label) follows the preset"
    );
    let title = field(&doc, "title").unwrap();
    assert_eq!(
        (title.label.as_str(), title.default.as_str()),
        ("Job title", "Title"),
        "an old preset default takes the new one"
    );
    assert_eq!(field(&doc, "extra").unwrap().default, "x");
    assert_eq!(
        variable(&doc, "accent").unwrap().default,
        "#0000ff",
        "old preset default (history) → new default"
    );
    assert_eq!(
        variable(&doc, "speed").unwrap().default,
        "1.5",
        "own default kept"
    );
    let radius = variable(&doc, "radius").unwrap();
    assert!(
        matches!(radius.kind, VariableKind::Select { .. }),
        "type follows the preset"
    );
    assert_eq!(
        radius.default, "round",
        "a type change takes the preset's default"
    );
    assert_eq!(variable(&doc, "text").unwrap().default, "#ffffff");
}

#[test]
fn a_kept_default_that_no_longer_fits_takes_the_new_one() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    let t = made_from(p, "lt", "Strap");
    let mut f = v2();
    // `speed` keeps its kind (range) but its range moves above the template's default.
    let path = "lt/template.json".to_string();
    let mut v: Value = serde_json::from_slice(&f.files[&path]).unwrap();
    for var in v["variables"].as_array_mut().unwrap() {
        if var["key"] == "speed" {
            var["min"] = json!(2);
            var["max"] = json!(4);
            var["default"] = json!("3");
        }
    }
    f.files.insert(path, v.to_string().into_bytes());
    let _g = testing::install(f);
    let doc = update(p, &t.id).unwrap();
    assert_eq!(variable(&doc, "speed").unwrap().default, "3");
}

#[test]
fn a_legacy_root_look_updates_the_default_variant() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    let t = made_from(p, "legacy-bar", "Bar");
    assert_eq!(t.variants[0].id, DEFAULT_VARIANT);
    let _g = testing::install(v2());
    let doc = update(p, &t.id).unwrap();
    assert_eq!(doc.variants.len(), 1);
    assert_eq!(doc.variants[0].html, "<div class=\"bar2\">{{name}}</div>");
    assert_eq!(doc.variants[0].css, ".bar2 { color: blue; }");
    assert_eq!(doc.meta.design.map(|d| (d.w, d.h)), Some((600, 100)));
}

#[test]
fn a_default_variant_takes_the_presets_default_look_when_it_has_no_default() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    let t = made_from(p, "legacy-bar", "Bar");
    let _g = testing::install(legacy_with_variants());
    let doc = update(p, &t.id).unwrap();
    let ids: Vec<&str> = doc.variants.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, [DEFAULT_VARIANT], "no variants added");
    assert_eq!(
        doc.variants[0].html,
        "<div class=\"classic\">{{name}}</div>"
    );
}

#[test]
fn templates_without_an_update_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    let poll = made_from(p, "poll", "Poll");
    let own = {
        let _g = testing::install(testing::library());
        templates::create(p, "Mine", &TemplateSource::Blank).unwrap()
    };
    let _g = testing::install(v2());
    let before = templates::get(p, &poll.id).unwrap();
    assert!(matches!(
        update(p, &poll.id),
        Err(TemplateError::UpToDate(_))
    ));
    assert!(matches!(
        update(p, &own.id),
        Err(TemplateError::NotFromPreset)
    ));
    assert_eq!(
        templates::get(p, &poll.id).unwrap(),
        before,
        "nothing written"
    );
}

#[test]
fn an_invalid_result_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    let t = made_from(p, "lt", "Strap");
    // 64 fields (the limit); the new version adds two and drops one.
    let mut doc = templates::get(p, &t.id).unwrap();
    for i in 0..60 {
        doc.meta.fields.push(FieldDef {
            key: format!("f{i}"),
            label: format!("F{i}"),
            ..Default::default()
        });
    }
    templates::save(p, doc).unwrap();
    let before = templates::get(p, &t.id).unwrap();
    let mut f = v2();
    let path = "lt/template.json".to_string();
    let mut v: Value = serde_json::from_slice(&f.files[&path]).unwrap();
    v["fields"]
        .as_array_mut()
        .unwrap()
        .push(testing::field("tagline", "text"));
    f.files.insert(path, v.to_string().into_bytes());
    let _g = testing::install(f);
    assert!(matches!(
        update(p, &t.id),
        Err(TemplateError::TooMany { .. })
    ));
    assert_eq!(templates::get(p, &t.id).unwrap(), before);
}

// ---- AppState: backup, screens, undo ------------------------------------------------------

#[derive(Default)]
struct Quiet(Mutex<Vec<String>>);

impl EventSink for Quiet {
    fn emit(&self, event: &str, _payload: Value) {
        self.0.lock().unwrap().push(event.to_string());
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    st: AppState,
    dir: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        Arc::new(Quiet::default()),
        Duration::from_secs(60),
    )
    .unwrap();
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    Env { _tmp: tmp, st, dir }
}

fn layer(template_id: &str) -> ScreenLayer {
    ScreenLayer {
        id: "l1".into(),
        name: "Strap".into(),
        rect: Rect {
            x: 100,
            y: 700,
            w: 600,
            h: 150,
        },
        kind: ScreenLayerKind::Template {
            template_id: template_id.into(),
            fields: [("name".to_string(), "Grace".to_string())].into(),
            vars: [("accent".to_string(), "#abcdef".to_string())].into(),
            bindings: Default::default(),
            variant: Some("frame".into()),
            sizing: None,
            opacity: None,
        },
        anchor: None,
    }
}

impl Env {
    fn screen(&self, template_id: &str) -> String {
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
        doc.meta.layers = vec![layer(template_id)];
        self.st.save_screen(doc).unwrap().doc.id
    }

    fn doc(&self, id: &str) -> ScreenDoc {
        self.st.get_screen(id).unwrap().doc
    }

    fn live_html(&self, screen: &str) -> String {
        self.st
            .live()
            .with_layer(screen, "l1", |l| l.template.html.clone())
            .unwrap()
    }

    fn live_design(&self, screen: &str) -> Option<(u32, u32)> {
        self.st
            .live()
            .with_layer(screen, "l1", |l| {
                l.template.design.as_ref().map(|d| (d.w, d.h))
            })
            .unwrap()
    }
}

#[test]
fn app_update_takes_a_backup_and_reapplies_screens_then_undo_restores() {
    let e = env();
    let t = made_from(&e.dir, "lt", "Strap");
    let screen = {
        let _g = testing::install(testing::library());
        e.screen(&t.id)
    };
    let screen_before = e.doc(&screen);
    let old_html = e.live_html(&screen);
    assert_eq!(e.live_design(&screen), None);

    let _g = testing::install(v2());
    let r = e.st.update_template_from_preset(&t.id).unwrap();
    assert_eq!(r.backup.reason, BackupReason::PreUpdate);
    assert!(r.backup.id.ends_with("-pre-update"), "{}", r.backup.id);
    let backups = e.st.list_backups().unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].id, r.backup.id);
    // The layer follows (off air: at once): the new look and the design box.
    assert_eq!(
        e.live_html(&screen),
        "<div class=\"lt2-frame\">{{name}} {{subtitle}}</div>"
    );
    assert_eq!(e.live_design(&screen), Some((800, 200)));
    // The screen's own values are untouched.
    let after = e.doc(&screen);
    assert_eq!(after.meta.layers, screen_before.meta.layers);
    // No update left; a second call takes no backup.
    assert!(e.st.update_template_from_preset(&t.id).is_err());
    assert_eq!(e.st.list_backups().unwrap().len(), 1);

    // The user adds a look after the update; Undo puts the template back as it was.
    e.st.templates()
        .create_variant(&e.dir, &t.id, "classic", "Later")
        .unwrap();
    let back = e.st.undo_template_update(&t.id, &r.backup.id).unwrap();
    assert_eq!(back.meta.preset.as_ref().map(|p| p.version), Some(1));
    assert_eq!(back.meta.design, None);
    let ids: Vec<&str> = back.variants.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["classic", "frame"]);
    assert!(!e
        .dir
        .join("templates")
        .join(&t.id)
        .join("variants")
        .join("later")
        .exists());
    assert_eq!(e.live_html(&screen), old_html);
    assert_eq!(e.live_design(&screen), None);
    let list = e.st.templates().list(&e.dir).unwrap();
    assert_eq!(
        list.iter().find(|s| s.id == t.id).unwrap().preset_update,
        Some(PresetUpdate { from: 1, to: 2 }),
        "offered again"
    );
}

#[test]
fn undo_needs_a_backup_holding_the_template() {
    let e = env();
    let _g = testing::install(v2());
    let t = templates::create(
        &e.dir,
        "Strap",
        &TemplateSource::Preset {
            preset_id: "lt".into(),
        },
    )
    .unwrap();
    assert!(e.st.undo_template_update(&t.id, "nope").is_err());
    // A snapshot from before the template existed has no copy of it.
    let tmp = tempfile::tempdir().unwrap();
    assert!(matches!(
        restore(&e.dir, &t.id, tmp.path()),
        Err(TemplateError::NotInBackup)
    ));
}
