use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::*;
use crate::core::events::EventSink;
use crate::core::live::LiveLayerView;
use crate::core::model::{
    LiveEdit, NewScreen, ProjectFile, Rect, ScreenDoc, ScreenLayer, TemplateSource, UiState,
};
use crate::core::presets::testing;
use crate::core::protocol::Fields;

#[derive(Default)]
struct Quiet(Mutex<Vec<String>>);

impl EventSink for Quiet {
    fn emit(&self, event: &str, _payload: Value) {
        self.0.lock().unwrap().push(event.to_string());
    }
}

struct Env {
    _presets: testing::Guard,
    _tmp: tempfile::TempDir,
    st: AppState,
    dir: PathBuf,
}

fn env() -> Env {
    let presets = testing::install(testing::library());
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
    Env {
        _presets: presets,
        _tmp: tmp,
        st,
        dir,
    }
}

impl Env {
    fn made_from(&self, preset: &str, name: &str) -> TemplateDoc {
        self.st
            .templates()
            .create(
                &self.dir,
                name,
                &TemplateSource::Preset {
                    preset_id: preset.into(),
                },
            )
            .unwrap()
    }

    /// A template from the `lt` preset as an older Kinetra made it: no `frame` variant.
    fn without_frame(&self, name: &str) -> TemplateDoc {
        let t = self.made_from("lt", name);
        self.st
            .templates()
            .delete_variant(&self.dir, &t.id, "frame")
            .unwrap()
    }

    fn blank(&self, name: &str) -> TemplateDoc {
        self.st
            .templates()
            .create(&self.dir, name, &TemplateSource::Blank)
            .unwrap()
    }

    fn screen(&self, name: &str, layers: Vec<ScreenLayer>) -> String {
        let mut doc = self
            .st
            .create_screen(&NewScreen {
                name: name.into(),
                width: 1920,
                height: 1080,
                background: "transparent".into(),
            })
            .unwrap()
            .doc;
        doc.meta.layers = layers;
        self.st.save_screen(doc).unwrap().doc.id
    }

    fn doc(&self, id: &str) -> ScreenDoc {
        self.st.get_screen(id).unwrap().doc
    }

    fn view(&self, screen: &str, layer: &str) -> LiveLayerView {
        self.st.live().layer_view(screen, layer).unwrap()
    }

    fn on_air_variant(&self, screen: &str, layer: &str) -> String {
        self.st
            .live()
            .with_layer(screen, layer, |l| l.variant.clone())
            .unwrap()
    }

    fn var_values(&self, screen: &str, layer: &str) -> Fields {
        self.st
            .live()
            .with_layer(screen, layer, |l| l.var_values.clone())
            .unwrap()
    }

    fn set_mode(&self, mode: LiveEdit) {
        let mut ui: UiState = self.st.ui_state().unwrap();
        ui.control.live_edit = mode;
        self.st.set_ui_state(ui).unwrap();
    }

    fn style(&self) -> Option<String> {
        self.st.current_project().unwrap().settings.style
    }
}

fn layer(id: &str, template_id: &str, fields: &[(&str, &str)]) -> ScreenLayer {
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
            fields: fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            vars: Fields::new(),
            bindings: Default::default(),
            variant: None,
            sizing: None,
            opacity: None,
        },
        anchor: None,
    }
}

fn variant_of(doc: &ScreenDoc, layer: &str) -> Option<String> {
    let l = doc.meta.layers.iter().find(|l| l.id == layer).unwrap();
    let ScreenLayerKind::Template { variant, .. } = &l.kind else {
        unreachable!()
    };
    variant.clone()
}

fn set_override(doc: &mut ScreenDoc, layer: &str, key: &str, value: &str) {
    let l = doc.meta.layers.iter_mut().find(|l| l.id == layer).unwrap();
    let ScreenLayerKind::Template { vars, .. } = &mut l.kind else {
        unreachable!()
    };
    vars.insert(key.into(), value.into());
}

const PROJECT: ApplyStyleScope = ApplyStyleScope::Project;
const FADE: VariantTransition = VariantTransition::Crossfade;

#[test]
fn apply_to_project_copies_variants_merges_fields_and_switches_layers() {
    let e = env();
    let has = e.made_from("lt", "Guest");
    let old = e.without_frame("Old guest");
    let custom = e.blank("Custom");
    // The older template lacks the field `logo`; drop it to see the merge.
    let mut trimmed = e.st.templates().get(&e.dir, &old.id).unwrap();
    trimmed.meta.fields.retain(|f| f.key != "logo");
    let trimmed = e.st.templates().save(&e.dir, trimmed).unwrap();
    e.st.template_saved(&trimmed);
    assert!(trimmed.meta.fields.iter().all(|f| f.key != "logo"));

    let a = e.screen(
        "A",
        vec![
            layer("l1", &has.id, &[("name", "Ada"), ("title", "CEO")]),
            layer("l2", &old.id, &[("name", "Grace")]),
            layer("l3", &custom.id, &[("name", "x")]),
        ],
    );
    let b = e.screen("B", vec![layer("l4", &custom.id, &[])]);

    let r = e.st.apply_style("frame", &PROJECT, FADE).unwrap();
    println!("{r:?}");
    assert_eq!(r.switched, 2);
    assert_eq!(r.unchanged, 2, "the custom template's two layers");
    assert_eq!(r.unchanged_templates, 1);
    assert_eq!(r.added, 1, "copied into the older template only");
    assert!(r.undo.is_some());
    assert_eq!(e.style().as_deref(), Some("frame"));

    let doc = e.doc(&a);
    assert_eq!(variant_of(&doc, "l1").as_deref(), Some("frame"));
    assert_eq!(variant_of(&doc, "l2").as_deref(), Some("frame"));
    assert_eq!(variant_of(&doc, "l3"), None);
    assert_eq!(variant_of(&e.doc(&b), "l4"), None);
    // Field data is kept.
    let v1 = e.view(&a, "l1");
    assert_eq!(v1.variant, "frame");
    assert_eq!(v1.fields["name"], "Ada");
    assert_eq!(v1.fields["title"], "CEO");
    assert_eq!(e.view(&a, "l2").fields["name"], "Grace");
    // The older template got the look and the missing field definition (and kept its default).
    let t = e.st.templates().get(&e.dir, &old.id).unwrap();
    let ids: Vec<&str> = t.variants.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["classic", "frame"]);
    assert_eq!(t.meta.default_variant, "classic");
    assert!(t.meta.fields.iter().any(|f| f.key == "logo"));
    assert!(e.view(&a, "l2").field_defs.iter().any(|f| f.key == "logo"));
    assert_eq!(t.variants[1].html, "<div class=\"lt-frame\">{{name}}</div>");

    // Applying again: nothing to do, no undo step.
    let again = e.st.apply_style("frame", &PROJECT, FADE).unwrap();
    assert_eq!((again.switched, again.added, again.unchanged), (0, 0, 2));
    assert_eq!(again.undo, None);

    // The screens are written by the content flusher.
    e.st.flush_all();
    let on_disk = crate::core::screens::get(&e.dir, &a).unwrap();
    assert_eq!(variant_of(&on_disk, "l1").as_deref(), Some("frame"));
    let file: ProjectFile = crate::core::persist::load_strict::<ProjectFile>(&e.dir).unwrap();
    assert_eq!(file.settings.style.as_deref(), Some("frame"));

    // Back to the default variant: stored as "no variant".
    let r = e.st.apply_style("classic", &PROJECT, FADE).unwrap();
    assert_eq!(r.switched, 2);
    assert_eq!(variant_of(&e.doc(&a), "l1"), None);
    assert_eq!(e.style().as_deref(), Some("classic"));
}

#[test]
fn unknown_style_and_scope_errors() {
    let e = env();
    assert!(matches!(
        e.st.apply_style("nope", &PROJECT, FADE),
        Err(AppError::Invalid(_))
    ));
    assert!(matches!(
        e.st.apply_style("../x", &PROJECT, FADE),
        Err(AppError::Invalid(_))
    ));
    let scope = ApplyStyleScope::Layer {
        screen_id: "missing".into(),
        layer_id: "l1".into(),
    };
    assert!(matches!(
        e.st.apply_style("frame", &scope, FADE),
        Err(AppError::NotFound(_))
    ));
    let t = e.made_from("lt", "Guest");
    let s = e.screen("A", vec![layer("l1", &t.id, &[])]);
    let scope = ApplyStyleScope::Layer {
        screen_id: s,
        layer_id: "nope".into(),
    };
    assert!(e.st.apply_style("frame", &scope, FADE).is_err());
    // A style no preset has yet (listed, but without variants) changes nothing.
    let r = e.st.apply_style("minimal", &PROJECT, FADE).unwrap();
    assert_eq!((r.switched, r.unchanged, r.added), (0, 1, 0));
}

#[test]
fn apply_to_one_layer_only() {
    let e = env();
    let t = e.without_frame("Guest");
    let s = e.screen(
        "A",
        vec![
            layer("l1", &t.id, &[("name", "Ada")]),
            layer("l2", &t.id, &[("name", "Bo")]),
        ],
    );
    let other = e.screen("B", vec![layer("l1", &t.id, &[])]);
    let scope = ApplyStyleScope::Layer {
        screen_id: s.clone(),
        layer_id: "l2".into(),
    };
    let r = e.st.apply_style("frame", &scope, FADE).unwrap();
    assert_eq!((r.switched, r.unchanged, r.added), (1, 0, 1));
    let doc = e.doc(&s);
    assert_eq!(variant_of(&doc, "l1"), None);
    assert_eq!(variant_of(&doc, "l2").as_deref(), Some("frame"));
    assert_eq!(
        variant_of(&e.doc(&other), "l1"),
        None,
        "same layer id elsewhere"
    );
    assert_eq!(e.style(), None, "the layer scope leaves the project style");
    // The other screen's layer follows the merged template (it can show the new variant now).
    assert!(e.st.set_layer_variant(&other, "l1", "frame", FADE).is_ok());
    // A custom template can't get it.
    let c = e.blank("Custom");
    let s2 = e.screen("C", vec![layer("x", &c.id, &[])]);
    let r =
        e.st.apply_style(
            "frame",
            &ApplyStyleScope::Layer {
                screen_id: s2,
                layer_id: "x".into(),
            },
            FADE,
        )
        .unwrap();
    assert_eq!((r.switched, r.unchanged, r.unchanged_templates), (0, 1, 1));
    assert_eq!(r.undo, None);
}

#[test]
fn on_air_under_on_take_stages_until_the_take() {
    let e = env();
    let t = e.made_from("lt", "Guest");
    let s = e.screen("A", vec![layer("l1", &t.id, &[("name", "Ada")])]);
    e.st.live().show(&s, "l1").unwrap();
    let mut rx = e.st.live().subscribe();
    let r = e.st.apply_style("frame", &PROJECT, FADE).unwrap();
    assert_eq!(r.switched, 1);
    let v = e.view(&s, "l1");
    assert!(v.pending && v.on_air);
    assert_eq!(v.variant, "frame", "staged");
    assert_eq!(
        e.on_air_variant(&s, "l1"),
        "classic",
        "the output keeps showing"
    );
    let mut sent = 0;
    while let Ok(m) = rx.try_recv() {
        if *m.screen_id == *s {
            sent += 1;
        }
    }
    assert_eq!(sent, 0, "nothing reaches the output before the take");
    let v = e.st.live().show(&s, "l1").unwrap();
    assert!(!v.pending);
    assert_eq!(e.on_air_variant(&s, "l1"), "frame");
    assert_eq!(v.fields["name"], "Ada");
}

#[test]
fn on_air_under_immediate_swaps_with_the_transition() {
    let e = env();
    e.set_mode(LiveEdit::Immediate);
    let t = e.made_from("lt", "Guest");
    let s = e.screen("A", vec![layer("l1", &t.id, &[("name", "Ada")])]);
    e.st.live().show(&s, "l1").unwrap();
    let mut rx = e.st.live().subscribe();
    e.st.apply_style("frame", &PROJECT, VariantTransition::OutIn)
        .unwrap();
    assert_eq!(e.on_air_variant(&s, "l1"), "frame");
    let mut msgs = Vec::new();
    while let Ok(m) = rx.try_recv() {
        if *m.screen_id == *s {
            msgs.push(serde_json::from_str::<Value>(&m.json).unwrap());
        }
    }
    assert_eq!(msgs.len(), 1, "one message for the switch: {msgs:?}");
    assert_eq!(msgs[0]["t"], "layer");
    assert_eq!(msgs[0]["layer"]["swap"], "outIn");
    assert_eq!(msgs[0]["layer"]["state"], "in");
}

#[test]
fn undo_restores_previous_variants_and_style() {
    let e = env();
    let t = e.made_from("lt", "Guest");
    let s = e.screen(
        "A",
        vec![
            layer("l1", &t.id, &[("name", "Ada")]),
            layer("l2", &t.id, &[]),
        ],
    );
    // l2 already shows frame (picked by hand): undo leaves it there.
    e.st.set_layer_variant(&s, "l2", "frame", FADE).unwrap();
    let r = e.st.apply_style("frame", &PROJECT, FADE).unwrap();
    assert_eq!(r.switched, 1);
    let token = r.undo.unwrap();
    e.st.undo_apply_style(&token).unwrap();
    let doc = e.doc(&s);
    assert_eq!(variant_of(&doc, "l1"), None);
    assert_eq!(variant_of(&doc, "l2").as_deref(), Some("frame"));
    assert_eq!(e.view(&s, "l1").variant, "classic");
    assert_eq!(e.style(), None);
    // Consumed.
    assert!(matches!(
        e.st.undo_apply_style(&token),
        Err(AppError::Invalid(m)) if m == "Nothing to undo"
    ));
    // Only the latest token is valid.
    let first =
        e.st.apply_style("frame", &PROJECT, FADE)
            .unwrap()
            .undo
            .unwrap();
    let second =
        e.st.apply_style("classic", &PROJECT, FADE)
            .unwrap()
            .undo
            .unwrap();
    assert!(e.st.undo_apply_style(&first).is_err());
    e.st.undo_apply_style(&second).unwrap();
    assert_eq!(variant_of(&e.doc(&s), "l1").as_deref(), Some("frame"));
    assert_eq!(e.style().as_deref(), Some("frame"));
    // Copied variants stay after an undo.
    let old = e.without_frame("Old");
    e.screen("B", vec![layer("x", &old.id, &[])]);
    let tok =
        e.st.apply_style("frame", &PROJECT, FADE)
            .unwrap()
            .undo
            .unwrap();
    e.st.undo_apply_style(&tok).unwrap();
    let t = e.st.templates().get(&e.dir, &old.id).unwrap();
    assert!(t.variants.iter().any(|v| v.id == "frame"));
    assert!(e.st.undo_apply_style("garbage").is_err());
}

#[test]
fn undo_on_air_follows_the_live_rules() {
    let e = env();
    let t = e.made_from("lt", "Guest");
    let s = e.screen("A", vec![layer("l1", &t.id, &[])]);
    let tok =
        e.st.apply_style("frame", &PROJECT, FADE)
            .unwrap()
            .undo
            .unwrap();
    e.st.live().show(&s, "l1").unwrap();
    assert_eq!(e.on_air_variant(&s, "l1"), "frame");
    e.st.undo_apply_style(&tok).unwrap();
    assert!(e.view(&s, "l1").pending, "onTake: staged");
    assert_eq!(e.on_air_variant(&s, "l1"), "frame");
    e.st.live().show(&s, "l1").unwrap();
    assert_eq!(e.on_air_variant(&s, "l1"), "classic");
}

#[test]
fn new_layers_start_on_the_project_style() {
    let e = env();
    let t = e.made_from("lt", "Guest");
    let c = e.blank("Custom");
    let s = e.screen("A", vec![layer("l1", &t.id, &[])]);
    e.st.apply_style("frame", &PROJECT, FADE).unwrap();
    let mut doc = e.doc(&s);
    // l1 back to classic by hand: an existing layer is never changed by a save.
    let l = doc.meta.layers.iter_mut().find(|l| l.id == "l1").unwrap();
    let ScreenLayerKind::Template { variant, .. } = &mut l.kind else {
        unreachable!()
    };
    *variant = None;
    doc.meta.layers.push(layer("l2", &t.id, &[]));
    doc.meta.layers.push(layer("l3", &c.id, &[]));
    let saved = e.st.save_screen(doc).unwrap().doc;
    assert_eq!(variant_of(&saved, "l1"), None);
    assert_eq!(variant_of(&saved, "l2").as_deref(), Some("frame"));
    assert_eq!(variant_of(&saved, "l3"), None);
}

#[test]
fn brand_kit_precedence_for_every_key() {
    let e = env();
    let t = e.made_from("lt", "Guest");
    let c = e.blank("Custom");
    let s = e.screen(
        "A",
        vec![
            layer("l1", &t.id, &[]),
            layer("l2", &t.id, &[]),
            layer("l3", &c.id, &[]),
        ],
    );
    e.st.set_layer_variant(&s, "l2", "frame", FADE).unwrap();
    // Before: definition defaults (classic) and the variant's values (frame).
    assert_eq!(e.var_values(&s, "l1")["accent"], "#ff0000");
    assert_eq!(e.var_values(&s, "l2")["accent"], "#22b8cf");
    assert_eq!(e.var_values(&s, "l2")["font-heading"], "Space Grotesk");
    let mut doc = e.doc(&s);
    set_override(&mut doc, "l1", "secondary", "#123456");
    e.st.save_screen(doc).unwrap();

    let brand = BrandKit {
        accent: Some("#ABCDEF".into()),
        secondary: Some("#111111".into()),
        text: Some("#222222".into()),
        plate: Some("#333333cc".into()),
        font_heading: Some("Oswald".into()),
        font_body: Some("Barlow".into()),
        logo: None,
    };
    let saved = e.st.save_brand_kit(brand).unwrap();
    assert_eq!(saved.accent.as_deref(), Some("#abcdef"), "normalized");
    for l in ["l1", "l2"] {
        let v = e.var_values(&s, l);
        assert_eq!(v["accent"], "#abcdef", "brand over the variant ({l})");
        assert_eq!(v["text"], "#222222");
        assert_eq!(v["plate"], "#333333cc");
        assert_eq!(v["font-heading"], "Oswald");
        assert_eq!(v["font-body"], "Barlow");
        // Keys the kit doesn't have keep the variant/definition value.
        assert_eq!(v["radius"], "4");
    }
    let css =
        e.st.live()
            .with_layer(&s, "l2", |l| l.vars.clone())
            .unwrap();
    assert_eq!(
        css["font-heading"],
        "\"Oswald\", \"Arial Narrow\", sans-serif"
    );
    assert_eq!(
        e.var_values(&s, "l1")["secondary"],
        "#123456",
        "a layer override wins"
    );
    assert_eq!(e.var_values(&s, "l2")["secondary"], "#111111");
    // A template without these variables gets none of them.
    assert!(e.var_values(&s, "l3").is_empty());
    assert_eq!(
        e.st.current_project().unwrap().settings.brand,
        saved,
        "stored in project settings"
    );
    // Clearing the kit: back to the variant's values.
    e.st.save_brand_kit(BrandKit::default()).unwrap();
    assert_eq!(e.var_values(&s, "l2")["accent"], "#22b8cf");
    assert_eq!(e.var_values(&s, "l1")["accent"], "#ff0000");
}

#[test]
fn a_brand_value_the_definition_rejects_is_ignored() {
    let e = env();
    // A template whose `accent` is a range: the brand colour doesn't fit, the default applies.
    let mut t = e.blank("Odd");
    t.meta.variables = vec![crate::core::model::VariableDef {
        key: "accent".into(),
        label: "Accent".into(),
        kind: crate::core::model::VariableKind::Range {
            min: 0.0,
            max: 10.0,
            step: 1.0,
            unit: "px".into(),
        },
        default: "2".into(),
    }];
    let t = e.st.templates().save(&e.dir, t).unwrap();
    let s = e.screen("A", vec![layer("l1", &t.id, &[])]);
    e.st.save_brand_kit(BrandKit {
        accent: Some("#ffffff".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(e.var_values(&s, "l1")["accent"], "2");
}

#[test]
fn brand_logo_fills_empty_logo_fields() {
    let e = env();
    let assets = crate::core::assets::assets_root(&e.dir);
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(assets.join("logo.png"), b"\x89PNG").unwrap();
    let t = e.made_from("lt", "Guest");
    let s = e.screen(
        "A",
        vec![
            layer("l1", &t.id, &[]),
            layer("l2", &t.id, &[("logo", "/assets/own.png")]),
        ],
    );
    assert_eq!(e.view(&s, "l1").fields["logo"], "");
    e.st.save_brand_kit(BrandKit {
        logo: Some("/assets/logo.png".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(e.view(&s, "l1").fields["logo"], "/assets/logo.png");
    assert_eq!(e.view(&s, "l2").fields["logo"], "/assets/own.png");
    // The layer's own value is untouched in the screen file.
    let doc = e.doc(&s);
    let ScreenLayerKind::Template { fields, .. } = &doc.meta.layers[0].kind else {
        unreachable!()
    };
    assert!(!fields.contains_key("logo"));
    // Renaming the image keeps the kit pointing at it.
    e.st.rename_asset("logo.png", "brand").unwrap();
    assert_eq!(
        e.st.current_project()
            .unwrap()
            .settings
            .brand
            .logo
            .as_deref(),
        Some("/assets/brand.png")
    );
    assert_eq!(e.view(&s, "l1").fields["logo"], "/assets/brand.png");
    e.st.save_brand_kit(BrandKit::default()).unwrap();
    assert_eq!(e.view(&s, "l1").fields["logo"], "");
}

#[test]
fn brand_kit_validation() {
    let e = env();
    let bad = [
        BrandKit {
            accent: Some("red".into()),
            ..Default::default()
        },
        BrandKit {
            plate: Some("#12345".into()),
            ..Default::default()
        },
        BrandKit {
            font_body: Some("Comic Sans".into()),
            ..Default::default()
        },
        BrandKit {
            logo: Some("https://example.com/x.png".into()),
            ..Default::default()
        },
        BrandKit {
            logo: Some("/assets/../project.json".into()),
            ..Default::default()
        },
        // Well-formed, but not in assets/.
        BrandKit {
            logo: Some("/assets/missing.png".into()),
            ..Default::default()
        },
    ];
    for b in bad {
        assert!(
            matches!(e.st.save_brand_kit(b.clone()), Err(AppError::Invalid(_))),
            "{b:?}"
        );
    }
    assert!(e.st.current_project().unwrap().settings.brand.is_empty());
    // Blank values are dropped; fonts accept the new curated one.
    let saved =
        e.st.save_brand_kit(BrandKit {
            accent: Some("  ".into()),
            font_heading: Some(" Space Grotesk ".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        saved,
        BrandKit {
            font_heading: Some("Space Grotesk".into()),
            ..Default::default()
        }
    );
    // Project settings saved from the UI go through the same validation.
    let mut settings = e.st.current_project().unwrap().settings;
    settings.brand.text = Some("nope".into());
    assert!(e.st.set_project_settings(settings.clone()).is_err());
    settings.brand.text = Some("#FFF".into());
    let p = e.st.set_project_settings(settings).unwrap();
    assert_eq!(p.settings.brand.text.as_deref(), Some("#fff"));
    let mut settings = p.settings;
    settings.brand.plate = Some("Transparent".into());
    let p = e.st.set_project_settings(settings).unwrap();
    assert_eq!(p.settings.brand.plate.as_deref(), Some("transparent"));
}

#[test]
fn settings_without_style_or_brand_load_and_stay_out_of_the_file() {
    let old = r#"{"name":"Show","createdAt":"2026-01-01T00:00:00Z","settings":{"port":7770,"lanAccess":false,"defaultResolution":{"width":1920,"height":1080}}}"#;
    let f: ProjectFile = serde_json::from_str(old).unwrap();
    assert_eq!(f.settings.style, None);
    assert!(f.settings.brand.is_empty());
    let out = serde_json::to_string(&f).unwrap();
    assert!(!out.contains("style") && !out.contains("brand"), "{out}");
    let mut f = f;
    f.settings.brand.accent = Some("#ffffff".into());
    f.settings.style = Some("frame".into());
    let out = serde_json::to_value(&f).unwrap();
    assert_eq!(out["settings"]["style"], "frame");
    assert_eq!(
        out["settings"]["brand"],
        serde_json::json!({ "accent": "#ffffff" })
    );
}

#[test]
fn poll_winner_mark_reaches_the_live_fields() {
    let e = env();
    let t = e.made_from("poll", "Poll");
    let rows = r#"[{"label":"A","pct":"30"},{"label":"B","pct":"70"}]"#;
    let s = e.screen(
        "A",
        vec![layer("p", &t.id, &[("status", "open"), ("options", rows)])],
    );
    assert!(!e.view(&s, "p").fields["options"].contains("_winner"));
    e.st.set_layer_fields(
        &s,
        "p",
        Fields::from([("status".to_string(), "closed".to_string())]),
    )
    .unwrap();
    let v = e.view(&s, "p");
    assert_eq!(
        v.fields["options"],
        r#"[{"label":"A","pct":"30","votes":""},{"label":"B","pct":"70","votes":"","_winner":"true"}]"#
    );
}

#[test]
fn list_styles_reads_the_library() {
    let e = env();
    let ids: Vec<String> = e.st.list_styles().into_iter().map(|s| s.id).collect();
    assert_eq!(ids, ["frame", "minimal"]);
}

#[test]
fn fifty_layers_apply_within_budget() {
    let e = env();
    // Five older templates (each gets the variant copied) and five screens of ten layers.
    let templates: Vec<TemplateDoc> = (0..5).map(|i| e.without_frame(&format!("T{i}"))).collect();
    let mut screens = Vec::new();
    for s in 0..5 {
        let layers = (0..10)
            .map(|i| {
                layer(
                    &format!("l{i}"),
                    &templates[(s + i) % 5].id,
                    &[("name", "Someone")],
                )
            })
            .collect();
        screens.push(e.screen(&format!("S{s}"), layers));
    }
    // Some of them on air.
    for s in &screens {
        e.st.live().show(s, "l0").unwrap();
    }
    let started = Instant::now();
    let r = e.st.apply_style("frame", &PROJECT, FADE).unwrap();
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    println!("apply_style: 50 layers, 5 variants copied: {ms:.1} ms");
    assert_eq!(r.switched, 50);
    assert_eq!(r.added, 5);
    // Switching back (no copies): the in-memory part alone.
    let started = Instant::now();
    let r = e.st.apply_style("classic", &PROJECT, FADE).unwrap();
    let back = started.elapsed().as_secs_f64() * 1000.0;
    println!("apply_style: 50 layers, no copies: {back:.1} ms");
    assert_eq!(r.switched, 50);
    // Debug builds are far slower than release; the budget is 100 ms in release.
    assert!(ms < 2000.0, "{ms} ms");
    assert!(back < 1000.0, "{back} ms");
}

#[test]
fn retired_font_names_are_read_as_the_replacement_and_rewritten_on_open() {
    let (old, new) = vars::FONT_ALIASES[0];
    let e = env();
    let t = e.made_from("lt", "Guest");
    let s = e.screen("A", vec![layer("l1", &t.id, &[]), layer("l2", &t.id, &[])]);
    e.st.set_layer_variant(&s, "l2", "frame", FADE).unwrap();
    e.st.close_project();

    // An older project: the retired name in a template default, a variant value, a layer
    // override and the brand kit.
    let edit = |path: PathBuf, f: &dyn Fn(&mut Value)| {
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        f(&mut v);
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    };
    let tpath = e.dir.join("templates").join(&t.id).join("template.json");
    let spath = e.dir.join("screens").join(format!("{s}.json"));
    let ppath = e.dir.join("project.json");
    edit(tpath.clone(), &|v| {
        for d in v["variables"].as_array_mut().unwrap() {
            if d["key"] == "font-heading" {
                d["default"] = old.into();
            }
        }
        for x in v["variants"].as_array_mut().unwrap() {
            if x["id"] == "frame" {
                x["vars"]["font-body"] = old.into();
            }
        }
    });
    edit(spath.clone(), &|v| {
        v["layers"][0]["vars"]["font-body"] = old.into();
    });
    edit(ppath.clone(), &|v| {
        v["settings"]["brand"] = serde_json::json!({ "fontHeading": old });
    });
    let has_old = |p: &PathBuf| {
        std::fs::read_to_string(p)
            .unwrap()
            .contains(&format!("\"{old}\""))
    };
    assert!(has_old(&tpath) && has_old(&spath) && has_old(&ppath));

    e.st.open_project(&e.dir).unwrap();
    // Rewritten on open, everywhere.
    assert!(!has_old(&tpath), "template.json");
    assert!(!has_old(&spath), "screen");
    assert!(!has_old(&ppath), "project.json");
    let brand = e.st.current_project().unwrap().settings.brand;
    assert_eq!(brand.font_heading.as_deref(), Some(new));
    // And rendered in the replacement (the brand's heading font wins over the template).
    let plex = vars::font_stack(new).unwrap();
    for l in ["l1", "l2"] {
        let css = e.st.live().with_layer(&s, l, |x| x.vars.clone()).unwrap();
        assert_eq!(css["font-heading"], plex, "{l}");
    }
    assert_eq!(e.var_values(&s, "l1")["font-body"], new, "layer override");
    let doc = e.st.templates().get(&e.dir, &t.id).unwrap();
    let frame = doc.variants.iter().find(|v| v.id == "frame").unwrap();
    assert_eq!(frame.vars["font-body"], new, "variant value");

    // Saving the retired name stores the replacement.
    let saved =
        e.st.save_brand_kit(BrandKit {
            font_body: Some(old.into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(saved.font_body.as_deref(), Some(new));
    let mut b = BrandKit {
        font_heading: Some(old.into()),
        font_body: Some("Barlow".into()),
        ..Default::default()
    };
    assert!(normalize_brand_fonts(&mut b));
    assert_eq!(b.font_heading.as_deref(), Some(new));
    assert_eq!(b.font_body.as_deref(), Some("Barlow"));
    assert!(!normalize_brand_fonts(&mut b));
}
