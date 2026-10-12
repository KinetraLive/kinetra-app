use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::core::events::NullSink;
use crate::core::model::{ScreenState, SourceKindName};

struct Env {
    _tmp: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
    st: AppState,
    dir: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        Arc::new(NullSink),
        Duration::from_secs(60),
    )
    .unwrap();
    st.set_content_flush_delay(Duration::from_secs(60));
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    Env {
        _tmp: tmp,
        _rt: rt,
        st,
        dir,
    }
}

fn new_screen(name: &str) -> RecipeScreen {
    RecipeScreen::New {
        name: name.into(),
        width: 1920,
        height: 1080,
    }
}

fn install(e: &Env, recipe: &str, screen: RecipeScreen) -> AppResult<RecipeInstance> {
    e.st.install_recipe(&InstallRecipe {
        recipe_id: recipe.into(),
        name: "Day 1".into(),
        screen,
    })
}

#[test]
fn embedded_recipes_are_valid_and_ordered() {
    let ids = ids();
    for id in &ids {
        if let Err(e) = load(id) {
            panic!("recipe {id} is invalid: {e}");
        }
    }
    let listed: Vec<String> = list().into_iter().map(|r| r.id).collect();
    assert_eq!(listed, ids, "every embedded recipe is listed");
    let known: Vec<&String> = ids.iter().filter(|i| ORDER.contains(&i.as_str())).collect();
    let mut sorted = known.clone();
    sorted.sort_by_key(|i| ORDER.iter().position(|o| o == i));
    assert_eq!(known, sorted, "gallery order");
    assert!(matches!(load("nope"), Err(AppError::NotFound(_))));
    assert!(matches!(load("../x"), Err(AppError::NotFound(_))));
}

#[test]
fn placeholders_resolve_or_fail() {
    let ctx = Placeholders {
        source: Some("src".into()),
        screen: "scr".into(),
        layers: vec!["l0".into(), "l1".into()],
        templates: vec!["t0".into(), "t1".into()],
    };
    let r = |s: &str| resolve_str(s, &ctx);
    assert_eq!(r("$source").unwrap().as_deref(), Some("src"));
    assert_eq!(r("$screen").unwrap().as_deref(), Some("scr"));
    assert_eq!(r("$layer:1").unwrap().as_deref(), Some("l1"));
    assert_eq!(r("$template:0").unwrap().as_deref(), Some("t0"));
    assert_eq!(r("plain").unwrap(), None);
    for bad in ["$layer:2", "$layer:x", "$template:9", "$other"] {
        assert!(r(bad).is_err(), "{bad}");
    }
    let none = Placeholders {
        source: None,
        ..ctx
    };
    assert!(resolve_str("$source", &none).is_err());
}

fn screen_state(e: &Env, id: &str) -> ScreenState {
    e.st.get_screen(id).unwrap()
}

#[test]
fn installing_every_recipe_creates_its_parts() {
    for id in ids() {
        let e = env();
        let m = load(&id).unwrap();
        let inst = install(&e, &id, new_screen("Event")).unwrap();
        assert_eq!(inst.recipe_id, id);
        assert_eq!(inst.panel, m.panel);
        assert_eq!(inst.layer_ids.len(), m.layers.len());
        // The screen holds the layers, bound to the created source, without warnings.
        let st = screen_state(&e, &inst.screen_id);
        assert_eq!(st.doc.meta.layers.len(), m.layers.len());
        assert!(st.warnings.is_empty(), "{id}: {:?}", st.warnings);
        for l in &st.doc.meta.layers {
            let ScreenLayerKind::Template {
                template_id,
                bindings,
                ..
            } = &l.kind
            else {
                unreachable!()
            };
            assert!(inst.template_ids.contains(template_id));
            for b in bindings.values() {
                assert_eq!(Some(&b.source_id), inst.source_id.as_ref(), "{id}");
            }
        }
        // Templates: one per distinct preset, named after the instance.
        let templates = e.st.templates().list(&e.dir).unwrap();
        assert_eq!(templates.len(), inst.template_ids.len());
        assert!(templates.iter().all(|t| t.name.starts_with("Day 1 · ")));
        // The source.
        if let Some(src) = &inst.source_id {
            let sum = e.st.list_sources().unwrap();
            let s = sum.iter().find(|s| &s.id == src).unwrap();
            assert!(s.name.starts_with("Day 1 · "));
            if s.kind == SourceKindName::Schedule {
                assert!(s.row_count > 0);
            }
        }
        // Flows (one per manifest rule): placeholders resolved, deployed, armed as the
        // manifest says (disarmed), owned by the instance, valid and without warnings.
        assert!(inst.rules.is_empty());
        assert_eq!(inst.flow_ids.len(), m.rules.len());
        for (fid, mr) in inst.flow_ids.iter().zip(&m.rules) {
            let f = e.st.get_flow(fid).unwrap();
            assert_eq!(f.flow.armed, mr.enabled);
            assert_eq!(f.flow.name, mr.name.trim());
            assert_eq!(f.flow.origin.as_deref(), Some(inst.id.as_str()));
            assert!(f.flow.draft.is_none());
            assert!(f.warnings.is_empty(), "{id}: {:?}", f.warnings);
            let json = serde_json::to_string(&f.flow).unwrap();
            assert!(!json.contains('$'), "{json}");
            assert!(crate::core::flows::flows_root(&e.dir)
                .join(format!("{fid}.json"))
                .is_file());
        }
        assert_eq!(e.st.list_flows().unwrap().len(), m.rules.len());
        // Recorded in recipes/<id>.json and listed.
        assert!(recipes_root(&e.dir)
            .join(format!("{}.json", inst.id))
            .is_file());
        assert_eq!(e.st.list_recipe_instances().unwrap(), vec![inst.clone()]);
    }
}

#[test]
fn event_schedule_samples_around_now_and_flows_point_at_its_layers() {
    let e = env();
    let inst = install(&e, "event-schedule", new_screen("Event")).unwrap();
    let src = inst.source_id.clone().unwrap();
    let v = e.st.schedule_view(&src).unwrap();
    assert_eq!(v.items.len(), 6);
    let statuses: Vec<ItemStatus> = v.items.iter().map(|i| i.status).collect();
    use crate::core::model::ItemStatus;
    assert_eq!(
        statuses,
        [
            ItemStatus::Previous,
            ItemStatus::Previous,
            ItemStatus::Now,
            ItemStatus::Next,
            ItemStatus::Upcoming,
            ItemStatus::Upcoming
        ]
    );
    // "take Now & Next" targets the second layer.
    let first = e.st.get_flow(&inst.flow_ids[0]).unwrap().flow;
    let take = first
        .nodes
        .iter()
        .find_map(|n| match &n.kind {
            crate::core::model::NodeKind::Take {
                layer_id,
                screen_id,
                ..
            } => Some((screen_id.clone(), layer_id.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(take, (inst.screen_id.clone(), inst.layer_ids[1].clone()));
}

#[test]
fn installing_on_an_existing_screen_scales_the_layers() {
    let e = env();
    let s =
        e.st.create_screen(&NewScreen {
            name: "Small".into(),
            width: 960,
            height: 540,
            background: "transparent".into(),
        })
        .unwrap();
    let inst = install(
        &e,
        "sponsor-wall",
        RecipeScreen::Existing {
            screen_id: s.doc.id.clone(),
        },
    )
    .unwrap();
    assert_eq!(inst.screen_id, s.doc.id);
    let doc = screen_state(&e, &s.doc.id).doc;
    assert_eq!(doc.meta.layers[0].rect.w, 960);
    assert_eq!(doc.meta.layers[0].rect.h, 540);
}

#[test]
fn a_bad_request_creates_nothing() {
    let e = env();
    for (recipe, screen, name) in [
        (
            "event-schedule",
            RecipeScreen::Existing {
                screen_id: "missing001".into(),
            },
            "Day",
        ),
        ("nope", new_screen("X"), "Day"),
        ("event-schedule", new_screen(""), "Day"),
        ("event-schedule", new_screen("X"), " "),
    ] {
        let r = e.st.install_recipe(&InstallRecipe {
            recipe_id: recipe.into(),
            name: name.into(),
            screen,
        });
        assert!(r.is_err());
    }
    assert!(e.st.list_sources().unwrap().is_empty());
    assert!(e.st.templates().list(&e.dir).unwrap().is_empty());
    assert!(e.st.list_screen_docs().unwrap().is_empty());
    assert!(e.st.list_recipe_instances().unwrap().is_empty());
    assert!(e.st.list_flows().unwrap().is_empty());
    assert!(
        !crate::core::flows::flows_root(&e.dir).exists()
            || std::fs::read_dir(crate::core::flows::flows_root(&e.dir))
                .unwrap()
                .next()
                .is_none()
    );
}

#[test]
fn removing_keeps_or_deletes_what_it_created() {
    let e = env();
    let keep = install(&e, "sponsor-slideshow", new_screen("A")).unwrap();
    e.st.remove_recipe(&keep.id, false).unwrap();
    assert!(e.st.list_recipe_instances().unwrap().is_empty());
    assert!(!recipes_root(&e.dir)
        .join(format!("{}.json", keep.id))
        .exists());
    assert_eq!(e.st.list_sources().unwrap().len(), 1, "kept");
    assert_eq!(screen_state(&e, &keep.screen_id).doc.meta.layers.len(), 1);

    let gone = install(&e, "event-schedule", new_screen("B")).unwrap();
    e.st.live()
        .show(&gone.screen_id, &gone.layer_ids[0])
        .unwrap();
    e.st.remove_recipe(&gone.id, true).unwrap();
    assert!(screen_state(&e, &gone.screen_id).doc.meta.layers.is_empty());
    for t in &gone.template_ids {
        assert!(e.st.templates().get(&e.dir, t).is_err());
    }
    assert!(e
        .st
        .list_sources()
        .unwrap()
        .iter()
        .all(|s| Some(&s.id) != gone.source_id.as_ref()));
    // The first recipe's things are untouched.
    assert_eq!(e.st.list_sources().unwrap().len(), 1);
    assert!(matches!(
        e.st.remove_recipe(&gone.id, true),
        Err(AppError::NotFound(_))
    ));
}

#[test]
fn instances_reload_with_the_project_and_are_backed_up() {
    let e = env();
    let inst = install(&e, "countdown-to-doors", new_screen("Doors")).unwrap();
    let path = e.dir.clone();
    e.st.close_project();
    assert!(e.st.rules().instances().is_empty());
    e.st.open_project(&path).unwrap();
    assert_eq!(e.st.list_recipe_instances().unwrap(), vec![inst.clone()]);
    let v: serde_json::Value = serde_json::from_slice(
        &std::fs::read(recipes_root(&e.dir).join(format!("{}.json", inst.id))).unwrap(),
    )
    .unwrap();
    assert_eq!(v["schemaVersion"], 2);
    assert!(v.get("rules").is_none());
    let b =
        e.st.create_snapshot(crate::core::model::BackupReason::Manual)
            .unwrap();
    let files = crate::core::backup::list(&e.dir).unwrap();
    assert!(files.iter().any(|f| f.id == b.id));
    let snap = e.dir.join(".backups").join(&b.id).join("recipes");
    assert!(snap.is_dir(), "recipes/ is part of the config set");
    // A corrupt instance file is set aside, a newer one is skipped.
    std::fs::write(recipes_root(&e.dir).join("bad0000001.json"), "{{").unwrap();
    std::fs::write(
        recipes_root(&e.dir).join("new0000001.json"),
        r#"{"schemaVersion": 99}"#,
    )
    .unwrap();
    let (list, recovered) = load_all(&e.dir);
    assert_eq!(list.len(), 1);
    assert_eq!(recovered.len(), 1);
}

// ---- Relink ----------------------------------------------------------------------------------

use crate::core::model::{
    FlowEdge, FlowNode, NewFlow, NodeKind, NodePos, PauseMode, PortRef, Retrigger, RowPick,
    WireKind,
};

fn node(id: &str, kind: NodeKind) -> FlowNode {
    FlowNode {
        id: id.into(),
        position: NodePos::default(),
        kind,
    }
}

fn wire(id: &str, from: (&str, &str), to: (&str, &str)) -> FlowEdge {
    FlowEdge {
        id: id.into(),
        from: PortRef {
            node: from.0.into(),
            port: from.1.into(),
        },
        to: PortRef {
            node: to.0.into(),
            port: to.1.into(),
        },
        kind: WireKind::Data,
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(std::time::Instant::now() < until, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A text source with three rows, at row 0.
fn text_source(e: &Env, name: &str) -> String {
    let mut doc = e.st.create_source(name).unwrap();
    doc.meta.kind = SourceKind::Text {
        columns: vec![SourceColumn {
            key: "name".into(),
            label: "Name".into(),
            kind: Default::default(),
        }],
        rows: ["A", "B", "C"]
            .iter()
            .map(|v| {
                let mut f = Fields::new();
                f.insert("name".into(), (*v).into());
                f
            })
            .collect(),
        index: 0,
        auto: None,
    };
    e.st.save_source(doc).unwrap().id
}

fn row_index(e: &Env, id: &str) -> u32 {
    match e.st.get_source(id).unwrap().meta.kind {
        SourceKind::Text { index, .. } => index,
        _ => panic!("not a text source"),
    }
}

/// Every `sourceId` in the node configs.
fn source_refs(nodes: &[FlowNode]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|n| {
            let v = serde_json::to_value(&n.kind).unwrap();
            v["config"]["sourceId"].as_str().map(String::from)
        })
        .collect()
}

#[test]
fn relink_rewrites_flows_bindings_and_the_instance() {
    let e = env();
    let inst = install(&e, "sponsor-slideshow", new_screen("Main")).unwrap();
    let old = inst.source_id.clone().unwrap();
    let new = text_source(&e, "Partners");
    // A rule of the recipe with a trigger, a data node and actions on the old source.
    let (mut nodes, edges) = crate::core::flows::linear(
        NodeKind::SlideshowWrap {
            source_id: old.clone(),
        },
        vec![NodeKind::StepSource {
            source_id: old.clone(),
            step: SourceStep::By { delta: 1 },
        }],
    );
    nodes.push(node(
        "data",
        NodeKind::SourceRow {
            source_id: old.clone(),
            row: RowPick::Current,
        },
    ));
    nodes.push(node(
        "unused",
        NodeKind::PauseSource {
            source_id: old.clone(),
            mode: PauseMode::Pause,
        },
    ));
    let own =
        e.st.create_flow(NewFlow {
            name: "Step on wrap".into(),
            armed: true,
            retrigger: Retrigger::Restart,
            origin: Some(inst.id.clone()),
            nodes,
            edges,
        })
        .unwrap()
        .flow;
    // A recipe flow with an undeployed draft that also names the old source.
    let first = inst.flow_ids[0].clone();
    let f = e.st.get_flow(&first).unwrap().flow;
    let mut draft = f.nodes.clone();
    draft.push(node(
        "extra",
        NodeKind::SlideshowWrap {
            source_id: old.clone(),
        },
    ));
    e.st.save_flow_draft(&first, draft, f.edges.clone())
        .unwrap();
    let ids = e.st.rules().instance(&inst.id).unwrap().flow_ids;
    assert!(ids.contains(&own.id));
    let armed: Vec<bool> = ids
        .iter()
        .map(|id| e.st.get_flow(id).unwrap().flow.armed)
        .collect();

    let got = e.st.relink_recipe_source(&inst.id, &new).unwrap();
    assert_eq!(got.source_id.as_deref(), Some(new.as_str()));
    assert_eq!(e.st.rules().instance(&inst.id).unwrap(), got);
    let (on_disk, _) = load_all(&e.dir);
    assert_eq!(on_disk[0].source_id.as_deref(), Some(new.as_str()));
    // Flows: every reference moved, deployed and draft, in memory and on disk; armed unchanged.
    for (id, was) in ids.iter().zip(&armed) {
        let info = e.st.get_flow(id).unwrap();
        let mut refs = source_refs(&info.flow.nodes);
        if let Some(d) = &info.flow.draft {
            refs.extend(source_refs(&d.nodes));
        }
        assert!(!refs.is_empty(), "{id}");
        assert!(refs.iter().all(|r| r == &new), "{id}: {refs:?}");
        assert_eq!(info.flow.armed, *was, "{id}");
        assert!(info.warnings.is_empty(), "{id}: {:?}", info.warnings);
        let file = crate::core::flows::read_flow(&e.dir, id).unwrap().0;
        assert!(source_refs(&file.nodes).iter().all(|r| r == &new));
    }
    let draft = e.st.get_flow(&first).unwrap().flow.draft.unwrap();
    assert_eq!(
        source_refs(&draft.nodes).last(),
        Some(&new),
        "the draft stays"
    );
    assert_eq!(
        source_refs(&e.st.get_flow(&own.id).unwrap().flow.nodes).len(),
        4
    );
    // The recipe's layer bindings follow.
    let st = screen_state(&e, &inst.screen_id);
    let mut bound = 0;
    for l in &st.doc.meta.layers {
        let ScreenLayerKind::Template { bindings, .. } = &l.kind else {
            unreachable!()
        };
        for b in bindings.values() {
            assert_eq!(b.source_id, new);
            bound += 1;
        }
    }
    assert!(bound > 0);
    assert!(st.warnings.is_empty(), "{:?}", st.warnings);
    // The engine watches the new source: a wrap of the old one does nothing.
    e.st.rules_slideshow_wrapped(&old);
    e.st.rules_slideshow_wrapped(&new);
    wait_until("the new source stepped", || row_index(&e, &new) == 1);
    assert_eq!(row_index(&e, &old), 0);
    // Relinking to the current source changes nothing.
    assert_eq!(e.st.relink_recipe_source(&inst.id, &new).unwrap(), got);
}

#[test]
fn relink_repairs_a_recipe_whose_source_was_deleted() {
    let e = env();
    let inst = install(&e, "event-schedule", new_screen("Event")).unwrap();
    let old = inst.source_id.clone().unwrap();
    e.st.delete_source(&old).unwrap();
    let warned = |id: &String| {
        e.st.get_flow(id)
            .unwrap()
            .warnings
            .iter()
            .any(|w| w.message == "Source no longer exists")
    };
    assert!(inst.flow_ids.iter().any(warned));
    let new = e.st.create_schedule("Day 2").unwrap().id;
    let got = e.st.relink_recipe_source(&inst.id, &new).unwrap();
    assert_eq!(got.source_id.as_deref(), Some(new.as_str()));
    assert!(!inst.flow_ids.iter().any(warned));
    let st = screen_state(&e, &inst.screen_id);
    for l in &st.doc.meta.layers {
        let ScreenLayerKind::Template { bindings, .. } = &l.kind else {
            unreachable!()
        };
        assert!(bindings.values().all(|b| b.source_id == new));
    }
}

#[test]
fn relink_checks_ids_and_the_source_kind() {
    let e = env();
    let slides = install(&e, "sponsor-slideshow", new_screen("Main")).unwrap();
    let doors = install(&e, "countdown-to-doors", new_screen("Doors")).unwrap();
    let text = text_source(&e, "Partners");
    let schedule = e.st.create_schedule("Day 2").unwrap().id;
    assert!(matches!(
        e.st.relink_recipe_source("nope000001", &text),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.st.relink_recipe_source(&slides.id, "nope000001"),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        e.st.relink_recipe_source(&slides.id, &schedule),
        Err(AppError::Invalid(_))
    ));
    assert!(matches!(
        e.st.relink_recipe_source(&doors.id, &text),
        Err(AppError::Invalid(_))
    ));
    // Nothing changed.
    assert_eq!(e.st.rules().instance(&slides.id).unwrap(), slides);
    assert_eq!(e.st.rules().instance(&doors.id).unwrap(), doors);
    // The old source may still exist.
    assert!(e.st.relink_recipe_source(&doors.id, &schedule).is_ok());
}

#[test]
fn flow_warnings_are_readable() {
    let e = env();
    let inst = install(&e, "event-schedule", new_screen("Event")).unwrap();
    let s = inst.screen_id.clone();
    let l = inst.layer_ids[1].clone(); // "Now & Next"
    let src = inst.source_id.clone().unwrap();
    let mut fields = Fields::new();
    fields.insert("nope".into(), "x".into());
    let flow = Flow {
        id: "w000000001".into(),
        name: "W".into(),
        armed: false,
        retrigger: Retrigger::Restart,
        origin: None,
        nodes: vec![
            node(
                "a",
                NodeKind::LayerAir {
                    screen_id: s.clone(),
                    layer_id: "gone".into(),
                    on_air: true,
                },
            ),
            node(
                "b",
                NodeKind::Take {
                    screen_id: s.clone(),
                    layer_id: "gone".into(),
                    for_ms: None,
                },
            ),
            node(
                "c",
                NodeKind::Clear {
                    screen_id: s.clone(),
                    layer_id: "gone".into(),
                },
            ),
            node(
                "d",
                NodeKind::SetFields {
                    screen_id: s.clone(),
                    layer_id: l.clone(),
                    fields,
                },
            ),
            node(
                "e",
                NodeKind::TimerZero {
                    screen_id: s.clone(),
                    layer_id: l.clone(),
                    key: "clock".into(),
                },
            ),
            node(
                "f",
                NodeKind::ClearScreen {
                    screen_id: "gone".into(),
                },
            ),
            node(
                "g",
                NodeKind::StepSource {
                    source_id: "gone".into(),
                    step: SourceStep::By { delta: 1 },
                },
            ),
            node(
                "h",
                NodeKind::SourceRow {
                    source_id: src,
                    row: RowPick::Current,
                },
            ),
            node(
                "i",
                NodeKind::RunFlow {
                    flow_id: "gone".into(),
                    wait: false,
                },
            ),
        ],
        edges: vec![wire("w", ("h", "nocol"), ("d", "nope"))],
        draft: None,
        last_fired_at: None,
        updated_at: String::new(),
    };
    let got: Vec<(String, String)> =
        e.st.flow_warnings(&flow)
            .into_iter()
            .map(|i| (i.node.unwrap_or_default(), i.message))
            .collect();
    let want: Vec<(String, String)> = [
        (
            "a",
            "The layer it watches was deleted (screen \u{201c}Event\u{201d})",
        ),
        (
            "b",
            "The layer it takes was deleted (screen \u{201c}Event\u{201d})",
        ),
        (
            "c",
            "The layer it clears was deleted (screen \u{201c}Event\u{201d})",
        ),
        (
            "d",
            "Field \u{201c}nope\u{201d} isn't on the layer \u{201c}Now & Next\u{201d}",
        ),
        (
            "e",
            "Timer field \u{201c}clock\u{201d} isn't on the layer \u{201c}Now & Next\u{201d}",
        ),
        ("f", "Screen no longer exists"),
        ("g", "Source no longer exists"),
        (
            "h",
            "Column \u{201c}nocol\u{201d} isn't in the source \u{201c}Day 1 \u{b7} Schedule\u{201d}",
        ),
        ("i", "The flow it runs no longer exists"),
    ]
    .into_iter()
    .map(|(n, m)| (n.to_string(), m.to_string()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn manifest_layers_keep_their_rect_or_are_placed_from_the_design() {
    let layer = |json: &str| -> ManifestLayer { serde_json::from_str(json).unwrap() };
    let design = TemplateDesign {
        w: 480,
        h: 270,
        x: Some(1392.0),
        y: Some(48.0),
        sizing: ContentSizing::Fit,
    };
    // An explicit rect stays (scaled to the screen), sizing absent unless the manifest sets it.
    let l = layer(r#"{"name":"A","preset":"corner-bug","rect":{"x":0,"y":0,"w":1920,"h":1080}}"#);
    let (r, s) = layer_box(&l, Some(&design), (1920, 1080), (1280, 720));
    assert_eq!(
        (r, s),
        (
            Rect {
                x: 0,
                y: 0,
                w: 1280,
                h: 720
            },
            None
        )
    );
    let l = layer(
        r#"{"name":"A","preset":"corner-bug","rect":{"x":0,"y":0,"w":1920,"h":1080},"sizing":"fit"}"#,
    );
    assert_eq!(
        layer_box(&l, None, (1920, 1080), (1920, 1080)).1,
        Some(ContentSizing::Fit)
    );
    // No rect: the design box at its position (scaled to the screen, clamped) with its sizing.
    let l = layer(r#"{"name":"A","preset":"corner-bug"}"#);
    let (r, s) = layer_box(&l, Some(&design), (1920, 1080), (1280, 720));
    assert_eq!(
        r,
        Rect {
            x: 800,
            y: 32,
            w: 480,
            h: 270
        }
    );
    assert_eq!(s, Some(ContentSizing::Fit));
    // No rect, no design: the full screen, reflow.
    let (r, s) = layer_box(&l, None, (1920, 1080), (1280, 720));
    assert_eq!(
        (r, s),
        (
            Rect {
                x: 0,
                y: 0,
                w: 1280,
                h: 720
            },
            None
        )
    );
}

#[test]
fn installed_recipes_keep_reflow_layers() {
    let e = env();
    let inst = install(&e, "event-schedule", new_screen("Event")).unwrap();
    let screen = inst.screen_id.clone();
    let doc = e.st.get_screen(&screen).unwrap().doc;
    for l in &doc.meta.layers {
        let ScreenLayerKind::Template { sizing, .. } = &l.kind else {
            unreachable!()
        };
        assert_eq!(*sizing, None, "as before sizing existed");
    }
    let text =
        std::fs::read_to_string(e.dir.join("screens").join(format!("{screen}.json"))).unwrap();
    assert!(!text.contains("\"sizing\""), "{text}");
}

// ---- Sample images ---------------------------------------------------------------------------

/// The `logo` cells of a recipe's text source, in row order.
fn logos(e: &Env, inst: &RecipeInstance) -> Vec<String> {
    let doc =
        e.st.sources()
            .doc(inst.source_id.as_deref().unwrap())
            .unwrap();
    let SourceKind::Text { rows, .. } = doc.meta.kind else {
        panic!("a text source");
    };
    rows.iter()
        .map(|r| r.get("logo").cloned().unwrap_or_default())
        .collect()
}

fn asset_bytes(e: &Env, url: &str) -> Vec<u8> {
    let name = url.strip_prefix(assets::URL_PREFIX).unwrap();
    std::fs::read(assets::assets_root(&e.dir).join(name)).unwrap()
}

#[test]
fn sponsor_recipes_copy_their_sample_logos() {
    for id in ["sponsor-wall", "sponsor-slideshow"] {
        let e = env();
        let bundled = bundled_assets(id);
        assert_eq!(bundled.len(), 4, "{id}");
        let inst = install(&e, id, new_screen("Sponsors")).unwrap();
        let logos = logos(&e, &inst);
        assert_eq!(logos.len(), 4);
        // Every row points at a copied file with the bundled bytes, under its bundled name.
        for url in &logos {
            let name = url.strip_prefix(assets::URL_PREFIX).unwrap();
            let (_, want) = bundled.iter().find(|(n, _)| n == name).unwrap();
            assert_eq!(asset_bytes(&e, url), want.as_ref(), "{id}: {url}");
            assert!(crate::core::sources::is_image_value(url));
        }
        // The copies are own writes (the watcher ignores them) and listed as used by the source.
        for (n, _) in &bundled {
            assert!(fsx::is_own_change(&assets::assets_root(&e.dir).join(n)));
        }
        let listed = e.st.list_assets().unwrap();
        assert_eq!(listed.len(), 4);
        assert!(listed
            .iter()
            .all(|a| a.used_by == vec![inst.source_id.clone().unwrap()]));
    }
}

#[test]
fn sample_logos_never_overwrite_and_a_reinstall_gets_fresh_names() {
    let e = env();
    let root = assets::assets_root(&e.dir);
    std::fs::create_dir_all(&root).unwrap();
    let mine = b"<svg xmlns='http://www.w3.org/2000/svg'><rect width='1' height='1'/></svg>";
    std::fs::write(root.join("northwind-audio.svg"), mine).unwrap();

    let first = install(&e, "sponsor-wall", new_screen("A")).unwrap();
    assert_eq!(
        std::fs::read(root.join("northwind-audio.svg")).unwrap(),
        mine,
        "the user's file is untouched"
    );
    let a = logos(&e, &first);
    assert_eq!(a[0], "/assets/northwind-audio-2.svg");
    assert_eq!(a[1], "/assets/polaris-networks.svg");
    let bundled = bundled_assets("sponsor-wall");
    let northwind = &bundled
        .iter()
        .find(|(n, _)| n == "northwind-audio.svg")
        .unwrap()
        .1;
    assert_eq!(asset_bytes(&e, &a[0]), northwind.as_ref());

    // A second install copies again under free names; the first one's rows are unchanged.
    let second = install(&e, "sponsor-wall", new_screen("B")).unwrap();
    let b = logos(&e, &second);
    assert_eq!(
        b,
        vec![
            "/assets/northwind-audio-3.svg",
            "/assets/polaris-networks-2.svg",
            "/assets/fjord-energy-2.svg",
            "/assets/kite-and-key-2.svg",
        ]
    );
    for url in &b {
        assert!(!asset_bytes(&e, url).is_empty());
    }
    assert_eq!(logos(&e, &first), a);

    // Removing a recipe (even with what it created) leaves the images in assets/.
    e.st.remove_recipe(&second.id, true).unwrap();
    for url in &b {
        let name = url.strip_prefix(assets::URL_PREFIX).unwrap();
        assert!(root.join(name).is_file(), "{name} stays");
    }
}

#[test]
fn renamed_refs_are_rewritten_in_one_pass() {
    let renames = vec![
        ("a.svg".to_string(), "a-2.svg".to_string()),
        ("a-2.svg".to_string(), "a-3.svg".to_string()),
    ];
    assert_eq!(
        assets::rename_refs("/assets/a.svg /assets/a-2.svg /assets/b.svg", &renames),
        "/assets/a-2.svg /assets/a-3.svg /assets/b.svg"
    );
}

#[test]
fn the_sponsor_slideshow_lands_at_its_design_box() {
    let e = env();
    let inst = install(&e, "sponsor-slideshow", new_screen("Sponsors")).unwrap();
    let doc = screen_state(&e, &inst.screen_id).doc;
    let l = &doc.meta.layers[0];
    let design = crate::core::presets::load("sponsor-slideshow")
        .unwrap()
        .meta
        .design
        .unwrap();
    let (rect, sizing) = screens::place_layer(Some(&design), 1920, 1080);
    assert_eq!(l.rect, rect);
    assert_ne!((l.rect.w, l.rect.h), (1920, 1080));
    let ScreenLayerKind::Template { sizing: s, .. } = &l.kind else {
        unreachable!()
    };
    assert_eq!(*s, sizing);
    assert_eq!(*s, Some(ContentSizing::Fit));
    // The wall keeps the full screen.
    let wall = install(&e, "sponsor-wall", new_screen("Wall")).unwrap();
    let doc = screen_state(&e, &wall.screen_id).doc;
    assert_eq!(
        doc.meta.layers[0].rect,
        Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080
        }
    );
}

#[test]
fn bundled_images_get_the_import_checks() {
    let e = env();
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'/>";
    assert!(assets::add_bytes(&e.dir, "x.svg", b"not an image").is_err());
    assert!(assets::add_bytes(&e.dir, "../x.svg", svg).is_err());
    assert!(assets::add_bytes(&e.dir, "x.exe", svg).is_err());
    assert_eq!(assets::add_bytes(&e.dir, "x.svg", svg).unwrap(), "x.svg");
    assert_eq!(assets::add_bytes(&e.dir, "x.svg", svg).unwrap(), "x-2.svg");
    let left: Vec<String> = std::fs::read_dir(assets::assets_root(&e.dir))
        .unwrap()
        .map(|d| d.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(left.len(), 2, "no temp files left: {left:?}");
}
