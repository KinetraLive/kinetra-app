use super::*;
use crate::core::model::TemplateSource;

fn project() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join(SCREENS_DIR)).unwrap();
    fs::create_dir_all(t.path().join(templates::TEMPLATES_DIR)).unwrap();
    t
}

fn new_screen(name: &str) -> NewScreen {
    NewScreen {
        name: name.into(),
        width: 1920,
        height: 1080,
        background: "transparent".into(),
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

fn values(pairs: &[(&str, &str)]) -> Fields {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn preset(p: &Path) -> String {
    templates::create(
        p,
        "Bar",
        &TemplateSource::Preset {
            preset_id: "classic-bar".into(),
        },
    )
    .unwrap()
    .id
}

#[test]
fn round_trip_and_file_shape() {
    let t = project();
    let p = t.path();
    let doc = create(p, &new_screen("  Main  ")).unwrap();
    assert_eq!(doc.meta.name, "Main", "trimmed");
    assert_eq!(doc.id.len(), 10);
    assert!(!doc.meta.updated_at.is_empty());
    let text = fs::read_to_string(p.join("screens").join(format!("{}.json", doc.id))).unwrap();
    assert!(
        text.starts_with("{\n  \"schemaVersion\": 1,\n  \"name\": \"Main\","),
        "{text}"
    );
    assert_eq!(get(p, &doc.id).unwrap(), doc);

    let tid = preset(p);
    let mut edited = doc.clone();
    let mut l = layer("l1", &tid);
    {
        let ScreenLayerKind::Template { fields, vars, .. } = &mut l.kind else {
            unreachable!()
        };
        *fields = values(&[("name", "Ada")]);
        *vars = values(&[("accent", "#ff0000")]);
    }
    edited.meta.layers.push(l);
    let saved = save(p, edited.clone(), false).unwrap();
    assert_eq!(saved.meta.layers, edited.meta.layers);
    assert_eq!(get(p, &doc.id).unwrap(), saved);
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(screen_path(p, &doc.id).unwrap()).unwrap())
            .unwrap();
    assert_eq!(
        v["layers"][0],
        serde_json::json!({
            "id": "l1", "name": "Layer l1", "rect": {"x": 0, "y": 0, "w": 1920, "h": 1080},
            "kind": "template", "templateId": tid,
            "fields": {"name": "Ada"}, "vars": {"accent": "#ff0000"}
        })
    );
    // No temp files left behind.
    let names: Vec<String> = fs::read_dir(p.join("screens"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
}

#[test]
fn unchanged_save_writes_nothing_and_save_needs_an_existing_screen() {
    let t = project();
    let p = t.path();
    let doc = create(p, &new_screen("A")).unwrap();
    let path = screen_path(p, &doc.id).unwrap();
    let stamp = || fs::metadata(&path).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let before = stamp();
    let same = save(p, doc.clone(), false).unwrap();
    assert_eq!(same.meta.updated_at, doc.meta.updated_at);
    assert_eq!(stamp(), before);

    let mut ghost = doc.clone();
    ghost.id = "ghost12345".into();
    assert!(matches!(
        save(p, ghost.clone(), false),
        Err(ScreenError::NotFound(_))
    ));
    assert!(save(p, ghost, true).is_ok(), "create allowed");
}

fn assert_invalid(p: &Path, base: &ScreenDoc, edit: impl FnOnce(&mut ScreenDoc), what: &str) {
    let mut d = base.clone();
    edit(&mut d);
    let err = save(p, d, false).expect_err(what);
    assert!(
        matches!(AppError::from(err), AppError::Invalid(_)),
        "{what} should be invalid"
    );
    assert_eq!(&get(p, &base.id).unwrap(), base, "{what}: nothing written");
}

#[test]
fn every_validation_rule() {
    let t = project();
    let p = t.path();
    let mut base = create(p, &new_screen("A")).unwrap();
    base.meta.layers = vec![layer("l1", "tmpl000001"), layer("l2", "tmpl000001")];
    let base = save(p, base, false).unwrap();
    let rect = |d: &mut ScreenDoc, r: Rect| d.meta.layers[0].rect = r;
    let r = |x, y, w, h| Rect { x, y, w, h };

    assert_invalid(p, &base, |d| d.meta.name = " ".into(), "blank name");
    assert_invalid(p, &base, |d| d.meta.name = "x".repeat(101), "long name");
    assert_invalid(p, &base, |d| d.meta.width = 15, "narrow");
    assert_invalid(p, &base, |d| d.meta.height = 8193, "tall");
    for bg in [
        "",
        "red",
        "#fff",
        "#12345",
        "#1234567",
        "#gggggg",
        "transparent;",
    ] {
        assert_invalid(p, &base, |d| d.meta.background = bg.into(), bg);
    }
    assert_invalid(
        p,
        &base,
        |d| {
            d.meta.layers = (0..=SCREEN_LAYERS_MAX)
                .map(|i| layer(&format!("x{i}"), "t"))
                .collect()
        },
        "too many layers",
    );
    assert_invalid(
        p,
        &base,
        |d| d.meta.layers[1].id = "l1".into(),
        "dup layer id",
    );
    for bad in ["", "../x", "a b", "a.b", &"a".repeat(65)] {
        assert_invalid(p, &base, |d| d.meta.layers[0].id = bad.into(), bad);
        assert_invalid(
            p,
            &base,
            |d| {
                d.meta.layers[0].kind = ScreenLayerKind::Template {
                    template_id: bad.into(),
                    fields: Fields::new(),
                    vars: Fields::new(),
                    bindings: Default::default(),
                    variant: None,
                    sizing: None,
                    opacity: None,
                }
            },
            "template id",
        );
    }
    assert_invalid(
        p,
        &base,
        |d| d.meta.layers[0].name = "  ".into(),
        "layer name",
    );
    assert_invalid(
        p,
        &base,
        |d| d.meta.layers[0].name = "x".repeat(101),
        "layer name long",
    );
    assert_invalid(p, &base, |d| rect(d, r(0, 0, 0, 10)), "zero width");
    assert_invalid(p, &base, |d| rect(d, r(0, 0, 10, 0)), "zero height");
    assert_invalid(
        p,
        &base,
        |d| rect(d, r(0, 0, RECT_SIDE_MAX + 1, 10)),
        "huge",
    );
    assert_invalid(p, &base, |d| rect(d, r(1920, 0, 10, 10)), "right of screen");
    assert_invalid(p, &base, |d| rect(d, r(0, 1080, 10, 10)), "below screen");
    assert_invalid(p, &base, |d| rect(d, r(-10, 0, 10, 10)), "left of screen");
    assert_invalid(p, &base, |d| rect(d, r(0, -10, 10, 10)), "above screen");
    let kv = |f: Fields, v: Fields| ScreenLayerKind::Template {
        template_id: "tmpl000001".into(),
        fields: f,
        vars: v,
        bindings: Default::default(),
        variant: None,
        sizing: None,
        opacity: None,
    };
    assert_invalid(
        p,
        &base,
        |d| d.meta.layers[0].kind = kv(values(&[("Bad", "x")]), Fields::new()),
        "field key",
    );
    assert_invalid(
        p,
        &base,
        |d| d.meta.layers[0].kind = kv(Fields::new(), values(&[("a b", "x")])),
        "var key",
    );
    let long = "x".repeat(VALUE_MAX + 1);
    assert_invalid(
        p,
        &base,
        |d| d.meta.layers[0].kind = kv(values(&[("name", &long)]), Fields::new()),
        "long value",
    );
    // Edges that are fine: partly off screen, #rrggbbaa, max size.
    let mut ok = base.clone();
    ok.meta.layers[0].rect = r(-100, -100, 101, 101);
    ok.meta.layers[1].rect = r(1919, 1079, RECT_SIDE_MAX, RECT_SIDE_MAX);
    ok.meta.background = "#00000080".into();
    save(p, ok, false).unwrap();
}

#[test]
fn corrupt_and_newer_files() {
    let t = project();
    let p = t.path();
    let good = create(p, &new_screen("Good")).unwrap();
    let newer = create(p, &new_screen("Newer")).unwrap();
    let newer_path = screen_path(p, &newer.id).unwrap();
    let text = r#"{"schemaVersion": 9, "name": "future", "width": 1, "height": 1}"#;
    fs::write(&newer_path, text).unwrap();
    let corrupt_path = p.join("screens/corrupt123.json");
    fs::write(&corrupt_path, "{ nope").unwrap();
    let invalid_path = p.join("screens/invalid123.json");
    fs::write(
        &invalid_path,
        r#"{"schemaVersion":1,"name":"","width":1920,"height":1080,"background":"transparent"}"#,
    )
    .unwrap();

    assert!(matches!(
        AppError::from(get(p, &newer.id).unwrap_err()),
        AppError::NewerVersion { found: 9, .. }
    ));
    assert!(matches!(
        AppError::from(save(p, newer.clone(), false).unwrap_err()),
        AppError::NewerVersion { .. }
    ));
    assert!(rename(p, &newer.id, "x").is_err());
    assert_eq!(
        fs::read_to_string(&newer_path).unwrap(),
        text,
        "never overwritten"
    );

    // Listing skips the bad ones and changes nothing.
    let ids: Vec<String> = list_uncached(p)
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, std::slice::from_ref(&good.id));
    assert!(corrupt_path.exists() && invalid_path.exists());

    // Opening sets the corrupt one aside (only that one).
    let (docs, recovered) = load_all(p);
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].id, good.id);
    assert_eq!(recovered.len(), 1);
    assert!(!corrupt_path.exists());
    assert!(recovered[0]
        .quarantined_as
        .contains("corrupt123.json.corrupt-"));
    assert!(Path::new(&recovered[0].quarantined_as).exists());
    assert!(invalid_path.exists() && newer_path.exists());
}

#[test]
fn list_sorts_newest_first_and_ignores_junk() {
    let t = project();
    let p = t.path();
    let a = create(p, &new_screen("A")).unwrap();
    let mut b = create(p, &new_screen("B")).unwrap();
    b.meta.updated_at = "2000-01-01T00:00:00Z".into();
    write(p, &b).unwrap();
    fs::write(p.join("screens/notes.txt"), "x").unwrap();
    fs::write(p.join("screens/x.json.tmp-abc"), "x").unwrap();
    fs::create_dir(p.join("screens/dir.json")).unwrap();
    let ids: Vec<String> = list_uncached(p)
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, [a.id, b.id]);
    let empty = tempfile::tempdir().unwrap();
    assert!(list_uncached(empty.path()).unwrap().is_empty());
}

#[test]
fn duplicate_rename_delete() {
    let t = project();
    let p = t.path();
    let mut src = create(p, &new_screen("Src")).unwrap();
    src.meta.layers = vec![layer("aaaaaaaaaa", "t1"), layer("bbbbbbbbbb", "t2")];
    let src = save(p, src, false).unwrap();
    let copy = duplicate(p, &src.id, "Copy").unwrap();
    assert_ne!(copy.id, src.id);
    assert_eq!(copy.meta.name, "Copy");
    assert_eq!(copy.meta.layers.len(), 2);
    for (a, b) in copy.meta.layers.iter().zip(&src.meta.layers) {
        assert_ne!(a.id, b.id, "new layer ids");
        assert_eq!((&a.name, a.rect, &a.kind), (&b.name, b.rect, &b.kind));
    }
    assert_ne!(copy.meta.layers[0].id, copy.meta.layers[1].id);
    assert!(duplicate(p, &src.id, " ").is_err());

    let renamed = rename(p, &copy.id, " Renamed ").unwrap();
    assert_eq!(renamed.meta.name, "Renamed");
    assert_eq!(get(p, &copy.id).unwrap().meta.name, "Renamed");

    delete(p, &copy.id).unwrap();
    assert!(matches!(get(p, &copy.id), Err(ScreenError::NotFound(_))));
    assert!(matches!(delete(p, &copy.id), Err(ScreenError::NotFound(_))));
    for bad in ["..", "../project", "a/b", ""] {
        assert!(matches!(
            AppError::from(get(p, bad).unwrap_err()),
            AppError::Invalid(_)
        ));
        assert!(delete(p, bad).is_err());
    }
}

#[test]
fn plan_overlays_values_and_reports_soft_problems() {
    let t = project();
    let p = t.path();
    let tid = preset(p);
    let mut doc = create(p, &new_screen("A")).unwrap();
    let mut good = layer("good000001", &tid);
    good.rect = Rect {
        x: 10,
        y: 20,
        w: 300,
        h: 200,
    };
    {
        let ScreenLayerKind::Template { fields, vars, .. } = &mut good.kind else {
            unreachable!()
        };
        *fields = values(&[("name", "Ada"), ("nope", "x")]);
        *vars = values(&[("accent", "#00ff00"), ("panel", "red"), ("ghost", "1")]);
    }
    doc.meta.layers = vec![layer("miss000001", "nosuchtmpl"), good];
    let templates = load_templates_for(p, &[doc.clone()]);
    assert_eq!(templates.len(), 1);
    let (plan, warnings) = plan(&doc, &templates, &SourceMap::new(), &BrandKit::default());
    assert_eq!(plan.info.id, doc.id);
    assert_eq!(plan.layers.len(), 1, "missing template left out");
    let l = &plan.layers[0];
    assert_eq!(l.id, "good000001");
    let c = &l.content;
    assert_eq!(c.fields, values(&[("name", "Ada"), ("title", "")]));
    assert_eq!(c.var_values["accent"], "#00ff00");
    assert_eq!(c.vars["accent"], "#00ff00");
    assert_eq!(
        c.var_values["panel"], "#0d1b2ae6",
        "invalid value → default"
    );
    assert_eq!(c.vars["font"], "\"Barlow\", system-ui, sans-serif");
    assert!(!c.vars.contains_key("ghost"));
    assert_eq!(
        c.rect,
        Some(Rect {
            x: 10,
            y: 20,
            w: 300,
            h: 200
        })
    );
    let codes: Vec<(&str, ScreenWarningCode)> = warnings
        .iter()
        .map(|w| (w.layer_id.as_str(), w.code))
        .collect();
    assert_eq!(
        codes,
        [
            ("miss000001", ScreenWarningCode::TemplateMissing),
            ("good000001", ScreenWarningCode::InvalidValue),
            ("good000001", ScreenWarningCode::InvalidValue),
            ("good000001", ScreenWarningCode::InvalidValue),
        ]
    );
    assert!(warnings.iter().all(|w| !w.message.is_empty()));
}

#[test]
fn store_caches_the_list_and_tracks_templates_in_use() {
    let t = project();
    let p = t.path();
    let store = ScreenStore::default();
    assert!(store.list(p).unwrap().is_empty());
    assert!(store.is_cached(p));
    create(p, &new_screen("Outside")).unwrap();
    assert!(store.list(p).unwrap().is_empty(), "served from memory");
    store.invalidate();
    assert_eq!(store.list(p).unwrap().len(), 1);

    let tid = preset(p);
    let mut doc = create(p, &new_screen("A")).unwrap();
    doc.meta.layers.push(layer("l1", &tid));
    store.install(
        p,
        vec![doc.clone()],
        TemplateMap::new(),
        BrandKit::default(),
    );
    assert!(store.is_project(p));
    let map = store.templates_for(p, &doc);
    assert!(map.contains_key(&tid));
    assert!(store.templates().contains_key(&tid), "remembered");
    let users = store.set_template(&tid, None);
    assert_eq!(users.len(), 1);
    assert!(store.templates().is_empty());
    // An unused template is not kept.
    let other = load_template(p, &tid).unwrap().unwrap();
    assert!(store.set_template("unused0001", Some(other)).is_empty());
    assert!(!store.templates().contains_key("unused0001"));
    store.set_layer_rect(
        &doc.id,
        "l1",
        Rect {
            x: 5,
            y: 5,
            w: 5,
            h: 5,
        },
        None,
    );
    assert_eq!(store.doc(&doc.id).unwrap().meta.layers[0].rect.x, 5);
    store.forget();
    assert!(!store.is_project(p));
    assert!(store.doc(&doc.id).is_none());
}

fn area(id: &str, name: &str, x: i32, y: i32, w: u32, h: u32) -> SafeArea {
    SafeArea {
        id: id.into(),
        name: name.into(),
        rect: Rect { x, y, w, h },
    }
}

#[test]
fn guides_and_safe_areas_validate_round_trip_and_duplicate() {
    let t = project();
    let p = t.path();
    let base = create(p, &new_screen("Wall")).unwrap();
    // Omitted from the file while empty.
    let text = fs::read_to_string(screen_path(p, &base.id).unwrap()).unwrap();
    assert!(
        !text.contains("guides") && !text.contains("safeAreas"),
        "{text}"
    );

    let mut d = base.clone();
    d.meta.guides = ScreenGuides {
        x: vec![960, -1920, 3840, 960, 100],
        y: vec![540],
    };
    d.meta.safe_areas = vec![
        area("bezel00001", "  Left bezel ", 0, 0, 480, 1080),
        area("cutoff0001", "Bottom cut-off", -10, 1000, 1940, 200),
    ];
    let saved = save(p, d, false).unwrap();
    // Sorted and deduplicated; names trimmed.
    assert_eq!(saved.meta.guides.x, [-1920, 100, 960, 3840]);
    assert_eq!(saved.meta.safe_areas[0].name, "Left bezel");
    assert_eq!(get(p, &base.id).unwrap(), saved, "round trip");
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(screen_path(p, &base.id).unwrap()).unwrap())
            .unwrap();
    assert_eq!(
        v["guides"],
        serde_json::json!({"x": [-1920, 100, 960, 3840], "y": [540]})
    );
    assert_eq!(
        v["safeAreas"][0],
        serde_json::json!({"id": "bezel00001", "name": "Left bezel", "rect": {"x": 0, "y": 0, "w": 480, "h": 1080}})
    );

    // Duplicate keeps them.
    let copy = duplicate(p, &base.id, "Wall copy").unwrap();
    assert_eq!(copy.meta.guides, saved.meta.guides);
    assert_eq!(copy.meta.safe_areas, saved.meta.safe_areas);

    // Hard errors on bad shapes; nothing is written.
    let many: Vec<i32> = (0..=SCREEN_GUIDES_MAX as i32).collect();
    type Edit = Box<dyn FnOnce(&mut ScreenDoc)>;
    let cases: Vec<(&str, Edit)> = vec![
        (
            "too many x guides",
            Box::new(move |d| d.meta.guides.x = many),
        ),
        (
            "x guide below -width",
            Box::new(|d| d.meta.guides.x = vec![-1921]),
        ),
        (
            "y guide above 2 × height",
            Box::new(|d| d.meta.guides.y = vec![2161]),
        ),
        (
            "too many safe areas",
            Box::new(|d| {
                d.meta.safe_areas = (0..=SCREEN_SAFE_AREAS_MAX)
                    .map(|i| area(&format!("area{i:06}"), "A", 0, 0, 10, 10))
                    .collect()
            }),
        ),
        (
            "bad safe area id",
            Box::new(|d| d.meta.safe_areas[0].id = "../x".into()),
        ),
        (
            "duplicate safe area id",
            Box::new(|d| d.meta.safe_areas[1].id = "bezel00001".into()),
        ),
        (
            "empty safe area name",
            Box::new(|d| d.meta.safe_areas[0].name = "  ".into()),
        ),
        (
            "long safe area name",
            Box::new(|d| d.meta.safe_areas[0].name = "x".repeat(SAFE_AREA_NAME_MAX + 1)),
        ),
        (
            "zero-width safe area",
            Box::new(|d| d.meta.safe_areas[0].rect.w = 0),
        ),
        (
            "safe area off screen",
            Box::new(|d| {
                d.meta.safe_areas[0].rect = Rect {
                    x: 1920,
                    y: 0,
                    w: 10,
                    h: 10,
                }
            }),
        ),
    ];
    for (what, edit) in cases {
        assert_invalid(p, &saved, edit, what);
    }
    // Edges that are fine: the range ends and a 40-character name.
    let mut ok = saved.clone();
    ok.meta.guides = ScreenGuides {
        x: vec![-1920, 3840],
        y: vec![-1080, 2160],
    };
    ok.meta.safe_areas[0].name = "x".repeat(SAFE_AREA_NAME_MAX);
    save(p, ok, false).unwrap();
}

#[test]
fn guides_and_safe_areas_never_reach_the_live_plan() {
    let t = project();
    let p = t.path();
    let tid = preset(p);
    let mut doc = create(p, &new_screen("A")).unwrap();
    doc.meta.layers = vec![layer("good000001", &tid)];
    let templates = load_templates_for(p, &[doc.clone()]);
    let plain = plan(&doc, &templates, &SourceMap::new(), &BrandKit::default());
    let mut edited = doc.clone();
    edited.meta.guides.x = vec![100, 200];
    edited.meta.safe_areas = vec![area("bezel00001", "Bezel", 0, 0, 100, 100)];
    assert_eq!(
        plan(&edited, &templates, &SourceMap::new(), &BrandKit::default()),
        plain
    );
}

#[test]
fn new_layers_are_placed_from_the_design_box() {
    use crate::core::model::{ContentSizing, TemplateDesign};
    let d = |w, h, x, y| TemplateDesign {
        w,
        h,
        x,
        y,
        sizing: ContentSizing::Fit,
    };
    let r = |x, y, w, h| Rect { x, y, w, h };
    // No design: the full screen, no sizing (reflow).
    assert_eq!(place_layer(None, 1280, 720), (r(0, 0, 1280, 720), None));
    // The layout editor's designRect cases (src/lib/components/screens/editor/model.ts).
    let at = d(640, 180, Some(96.0), Some(840.0));
    for (design, (sw, sh), want) in [
        (at, (1920, 1080), r(96, 840, 640, 180)),
        (at, (3840, 2160), r(192, 1680, 640, 180)),
        (at, (1280, 720), r(64, 540, 640, 180)),
        (d(640, 180, None, None), (1920, 1080), r(640, 450, 640, 180)),
        (d(641, 181, None, None), (1920, 1080), r(639, 449, 641, 181)),
        (
            d(1920, 1080, Some(0.0), Some(0.0)),
            (1280, 720),
            r(0, 0, 1280, 720),
        ),
        (d(2000, 500, None, None), (1080, 1920), r(0, 825, 1080, 270)),
        (
            d(640, 180, Some(1800.0), Some(1000.0)),
            (1920, 1080),
            r(1280, 900, 640, 180),
        ),
    ] {
        assert_eq!(
            place_layer(Some(&design), sw, sh),
            (want, Some(ContentSizing::Fit)),
            "{design:?} on {sw}x{sh}"
        );
    }
    // A reflow design leaves the sizing absent (reflow until the user resizes the layer).
    let mut rf = at;
    rf.sizing = ContentSizing::Reflow;
    assert_eq!(place_layer(Some(&rf), 1920, 1080).1, None);
}

#[test]
fn layer_sizing_parses_and_rejects_unknown_values() {
    let ok: ScreenLayer = serde_json::from_value(serde_json::json!({
        "id": "l1", "name": "A", "rect": {"x": 0, "y": 0, "w": 10, "h": 10},
        "kind": "template", "templateId": "t1", "sizing": "fit"
    }))
    .unwrap();
    let ScreenLayerKind::Template { sizing, .. } = &ok.kind else {
        unreachable!()
    };
    assert_eq!(*sizing, Some(crate::core::model::ContentSizing::Fit));
    let old: ScreenLayer = serde_json::from_value(serde_json::json!({
        "id": "l1", "name": "A", "rect": {"x": 0, "y": 0, "w": 10, "h": 10},
        "kind": "template", "templateId": "t1"
    }))
    .unwrap();
    let ScreenLayerKind::Template { sizing, .. } = &old.kind else {
        unreachable!()
    };
    assert_eq!(*sizing, None, "absent = reflow");
    assert!(!serde_json::to_string(&old).unwrap().contains("sizing"));
    let bad = serde_json::from_value::<ScreenLayer>(serde_json::json!({
        "id": "l1", "name": "A", "rect": {"x": 0, "y": 0, "w": 10, "h": 10},
        "kind": "template", "templateId": "t1", "sizing": "stretch"
    }));
    assert!(bad.is_err());
}

fn media_layer(asset: &str, in_ms: u64, out_ms: Option<u64>) -> ScreenLayer {
    ScreenLayer {
        kind: ScreenLayerKind::Media {
            media: MediaLayerConfig {
                asset: asset.into(),
                fit: Default::default(),
                in_ms,
                out_ms,
                volume: 100,
                muted: false,
                end_action: Default::default(),
                autoplay: true,
            },
            opacity: Some(100),
        },
        ..layer("m1", "unused")
    }
}

#[test]
fn media_layers_validate_and_plan() {
    let t = project();
    let mut doc = create(t.path(), &new_screen("Main")).unwrap();
    // Old files without media layers keep loading; media layers are additive.
    doc.meta.layers = vec![media_layer("clip.mp4", 1000, Some(9000))];
    let v = validate(doc.clone()).unwrap();
    let ScreenLayerKind::Media { media, opacity } = &v.meta.layers[0].kind else {
        unreachable!()
    };
    assert_eq!(media.asset, "/assets/clip.mp4");
    assert_eq!(*opacity, None, "100 % is stored as absent");
    let json = serde_json::to_value(&v.meta.layers[0]).unwrap();
    assert_eq!(json["kind"], "media");
    assert_eq!(json["media"]["endAction"], "hold");
    for bad in [
        media_layer("../x.mp4", 0, None),
        media_layer("notes.txt", 0, None),
        media_layer("clip.mp4", 5000, Some(5000)),
        {
            let mut l = media_layer("clip.mp4", 0, None);
            if let ScreenLayerKind::Media { media, .. } = &mut l.kind {
                media.volume = 101;
            }
            l
        },
    ] {
        let mut d = doc.clone();
        d.meta.layers = vec![bad];
        assert!(validate(d).is_err());
    }
    // Plan: a missing file is a soft warning; a known duration clamps the range.
    let v = validate(doc).unwrap();
    let none = SourceMap::new();
    let brand = BrandKit::default();
    let mut media = MediaMap::new();
    media.insert("clip.mp4".into(), None);
    let (p, w) = plan_with(&v, &TemplateMap::new(), &none, &brand, &media);
    assert!(p.layers.is_empty());
    assert_eq!(w[0].code, ScreenWarningCode::AssetMissing);
    let mut info = crate::core::media_probe::probe_bytes(b"", "clip.mp4");
    info.issues.clear();
    info.duration_ms = Some(4000);
    media.insert("clip.mp4".into(), Some(info));
    let (p, w) = plan_with(&v, &TemplateMap::new(), &none, &brand, &media);
    assert!(w.is_empty(), "{w:?}");
    let m = p.layers[0].content.media.as_ref().unwrap();
    assert_eq!((m.config.in_ms, m.config.out_ms), (1000, Some(4000)));
    assert_eq!(m.duration_ms, Some(4000));
    // Unknown to the probe (plain `plan`): kind from the extension, no warning.
    let (p, w) = plan(&v, &TemplateMap::new(), &none, &brand);
    assert!(w.is_empty());
    assert_eq!(
        p.layers[0].content.media.as_ref().unwrap().duration_ms,
        None
    );
}
