use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::core::events::EventSink;
use crate::core::model::{NodeKind, FLOWS_MAX};

// ---- Helpers ---------------------------------------------------------------------------------

struct Quiet;

impl EventSink for Quiet {
    fn emit(&self, _event: &str, _payload: Value) {}
}

struct Env {
    _tmp: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
    st: AppState,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        Arc::new(Quiet),
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
    Env {
        _tmp: tmp,
        _rt: rt,
        st,
    }
}

/// A well-formed value for every slot of a template.
fn sample_slots(m: &Manifest) -> BTreeMap<String, String> {
    m.slots
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let v = match s.kind {
                FlowSlotKind::Screen => format!("scr{i}"),
                FlowSlotKind::Layer => format!("scr{i}/lay{i}"),
                FlowSlotKind::Timer => format!("scr{i}/lay{i}/clock"),
                FlowSlotKind::Field => format!("scr{i}/lay{i}/headline"),
                FlowSlotKind::TextSource | FlowSlotKind::ScheduleSource => format!("src{i}"),
                FlowSlotKind::Flow => format!("flw{i}"),
            };
            (s.key.clone(), v)
        })
        .collect()
}

fn req(id: &str, slots: BTreeMap<String, String>) -> AddFlowTemplate {
    AddFlowTemplate {
        template_id: id.into(),
        name: None,
        slots,
    }
}

fn ids(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("flow{i}")).collect()
}

fn files() -> Vec<(String, Vec<u8>)> {
    FlowFiles::iter()
        .map(|p| {
            let data = FlowFiles::get(&p).unwrap().data.into_owned();
            (p.to_string(), data)
        })
        .collect()
}

// ---- Manifests -------------------------------------------------------------------------------

#[test]
fn every_manifest_parses_and_order_covers_them() {
    let files = files();
    assert_eq!(files.len(), ORDER.len(), "one file per ORDER entry");
    let mut seen = HashSet::new();
    for (path, data) in &files {
        let m = parse(data).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(path, &format!("{}.json", m.id), "file name is the id");
        assert!(seen.insert(m.id.clone()), "duplicate id {}", m.id);
        assert!(ORDER.contains(&m.id.as_str()), "{} is not in ORDER", m.id);
        assert!(!m.description.is_empty(), "{} has no description", m.id);
    }
    let listed: Vec<String> = list().into_iter().map(|t| t.id).collect();
    assert_eq!(
        listed,
        ORDER.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    );
}

#[test]
fn manifests_stay_small() {
    let total: usize = files().iter().map(|(_, d)| d.len()).sum();
    assert!(total <= 40 * 1024, "flow templates are {total} bytes");
}

#[test]
fn every_template_resolves_and_passes_check_graph() {
    for t in list() {
        let m = manifest(&t.id).unwrap();
        let flows = build(m, &req(&t.id, sample_slots(m)), &ids(m.flows.len()))
            .unwrap_or_else(|e| panic!("{}: {e}", t.id));
        assert_eq!(flows.len(), t.flows as usize);
        for f in &flows {
            let text = serde_json::to_string(f).unwrap();
            assert!(!text.contains("{{"), "{}: placeholder left in {text}", t.id);
            assert!(check_graph(&f.nodes, &f.edges).is_empty(), "{}", t.id);
            assert!(f.origin.is_none());
            // Fresh ids: no readable manifest id survives.
            for n in &f.nodes {
                assert_eq!(n.id.len(), 10, "{}: node id {}", t.id, n.id);
            }
            for e in &f.edges {
                assert_eq!(e.id.len(), 10, "{}: edge id {}", t.id, e.id);
            }
            // Armed exactly when it only runs on demand.
            let automatic = f.nodes.iter().any(|n| {
                crate::core::flow_nodes::is_trigger(&n.kind)
                    && !matches!(n.kind, NodeKind::Cue { .. } | NodeKind::RunByFlow {})
            });
            assert_eq!(f.armed, !automatic, "{}: {}", t.id, f.name);
        }
    }
}

#[test]
fn categories_and_chains() {
    let list = list();
    assert_eq!(list.len(), 18);
    for t in &list {
        let m = manifest(&t.id).unwrap();
        let first = m.flows[0]
            .nodes
            .iter()
            .find(|n| str_at(n, &["type"]).unwrap().starts_with("trigger."))
            .unwrap();
        let label = node_label(str_at(first, &["type"]).unwrap());
        assert!(t.chain.starts_with(label), "{}: {}", t.id, t.chain);
        assert!(t.chain.contains(" → "), "{}: {}", t.id, t.chain);
        assert!((1..=3).contains(&t.flows));
    }
    let chain = |id: &str| list.iter().find(|t| t.id == id).unwrap().chain.clone();
    assert_eq!(
        chain("show-open"),
        "Cue → Take → Delay → Clear layer → Take → Set timer"
    );
    assert_eq!(
        chain("break-countdown"),
        "Cue → Sequence → Clear screen → Take"
    );
    assert_eq!(
        chain("five-minute-warning"),
        "Before next item → Set fields → Take"
    );
    assert_eq!(chain("manual-hold"), "Schedule item status → Gate → Take");
    let cat = |id: &str| list.iter().find(|t| t.id == id).unwrap().category;
    assert_eq!(cat("show-open"), FlowTemplateCategory::Show);
    assert_eq!(cat("auto-delay"), FlowTemplateCategory::Schedule);
    assert_eq!(cat("sponsor-reel-between"), FlowTemplateCategory::Sponsors);
    assert_eq!(cat("score-reaction"), FlowTemplateCategory::Sports);
    assert_eq!(cat("manual-hold"), FlowTemplateCategory::Blocks);
}

// ---- Slots -----------------------------------------------------------------------------------

#[test]
fn missing_or_malformed_slots_name_the_label() {
    let m = manifest("show-open").unwrap();
    let mut slots = sample_slots(m);
    slots.remove("bug");
    match build(m, &req("show-open", slots), &ids(1)) {
        Err(AppError::Invalid(msg)) => assert!(msg.contains("Logo bug layer"), "{msg}"),
        other => panic!("{other:?}"),
    }
    for (key, bad_value, label) in [
        ("bug", "scr1", "Logo bug layer"),
        ("bug", "scr1/lay1/x", "Logo bug layer"),
        ("bug", "scr 1/lay1", "Logo bug layer"),
        ("clock", "scr1/lay1", "Show clock timer"),
        ("clock", "scr1/lay1/Bad Key", "Show clock timer"),
        ("opener", "   ", "Opener layer"),
    ] {
        let mut slots = sample_slots(m);
        slots.insert(key.into(), bad_value.into());
        match build(m, &req("show-open", slots), &ids(1)) {
            Err(AppError::Invalid(msg)) => assert!(msg.contains(label), "{bad_value}: {msg}"),
            other => panic!("{bad_value}: {other:?}"),
        }
    }
    let mut slots = sample_slots(m);
    slots.insert("nope".into(), "x".into());
    assert!(matches!(
        build(m, &req("show-open", slots), &ids(1)),
        Err(AppError::Invalid(_))
    ));
}

#[test]
fn set_fields_slot_sets_the_field_key_and_the_wire_port() {
    let m = manifest("five-minute-warning").unwrap();
    let mut slots = sample_slots(m);
    slots.insert("text".into(), "main/strap/subline".into());
    let flows = build(m, &req(&m.id, slots), &ids(1)).unwrap();
    let f = &flows[0];
    let set = f
        .nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::SetFields { .. }))
        .unwrap();
    let NodeKind::SetFields {
        screen_id,
        layer_id,
        fields,
    } = &set.kind
    else {
        unreachable!()
    };
    assert_eq!((screen_id.as_str(), layer_id.as_str()), ("main", "strap"));
    assert_eq!(fields.keys().collect::<Vec<_>>(), vec!["subline"]);
    assert!(f
        .edges
        .iter()
        .any(|e| e.to.node == set.id && e.to.port == "subline"));
    let take = f
        .nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Take { .. }))
        .unwrap();
    assert!(
        matches!(&take.kind, NodeKind::Take { screen_id, layer_id, .. }
        if screen_id == "main" && layer_id == "strap")
    );
}

#[test]
fn optional_slot_left_empty_resolves_to_empty() {
    let m = parse(
        json!({
            "schemaVersion": 1, "id": "opt", "name": "Opt", "category": "blocks",
            "slots": [{"key": "note", "label": "Note flow", "kind": "flow", "optional": true}],
            "flows": [{"armed": "cue", "nodes": [
                {"id": "cue", "type": "trigger.cue", "config": {"label": "x{{note}}"}}
            ], "edges": []}]
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let flows = build(&m, &req("opt", BTreeMap::new()), &ids(1)).unwrap();
    assert!(matches!(&flows[0].nodes[0].kind, NodeKind::Cue { label: Some(l) } if l == "x"));
}

#[test]
fn bad_manifests_are_rejected() {
    let base = |flows: Value| {
        json!({"schemaVersion": 1, "id": "x", "name": "X", "category": "show", "slots": [],
               "flows": flows})
        .to_string()
    };
    let cue = json!({"armed": "cue", "nodes": [], "edges": []});
    // Two flows without suffixes, four flows, an unknown armed value.
    assert!(parse(base(json!([cue, cue])).as_bytes()).is_err());
    assert!(parse(base(json!([cue, cue, cue, cue])).as_bytes()).is_err());
    assert!(parse(base(json!([{"armed": true, "nodes": [], "edges": []}])).as_bytes()).is_err());
    // An unknown placeholder fails at build.
    let m = parse(
        base(json!([{"armed": "cue", "nodes": [
            {"id": "c", "type": "trigger.cue", "config": {"label": "{{ghost}}"}}
        ], "edges": []}]))
        .as_bytes(),
    )
    .unwrap();
    assert!(build(&m, &req("x", BTreeMap::new()), &ids(1)).is_err());
}

// ---- Adding ----------------------------------------------------------------------------------

#[test]
fn adding_creates_named_flows_armed_by_kind() {
    let e = env();
    let m = manifest("break-countdown").unwrap();
    let infos =
        e.st.add_flow_template(req("break-countdown", sample_slots(m)))
            .unwrap();
    assert_eq!(infos.len(), 2);
    assert_eq!(infos[0].flow.name, "Break with countdown · start");
    assert_eq!(infos[1].flow.name, "Break with countdown · end");
    assert!(infos[0].flow.armed);
    assert!(!infos[1].flow.armed);
    assert!(infos.iter().all(|i| i.flow.origin.is_none()));
    // Missing targets are soft warnings, not errors.
    assert!(!infos[0].warnings.is_empty());
    let listed = e.st.list_flows().unwrap();
    assert_eq!(listed.len(), 2);

    // A custom name; a single-flow template has no suffix.
    let m = manifest("stinger-once").unwrap();
    let mut r = req("stinger-once", sample_slots(m));
    r.name = Some("  Big stinger ".into());
    let infos = e.st.add_flow_template(r).unwrap();
    assert_eq!(infos[0].flow.name, "Big stinger");
    assert!(infos[0].flow.armed);
    // Files are on disk.
    let dir = e.st.project_dir().unwrap();
    assert!(flows::flow_path(&dir, &infos[0].flow.id).unwrap().exists());
}

#[test]
fn unknown_template_and_missing_slot() {
    let e = env();
    assert!(matches!(
        e.st.add_flow_template(req("nope", BTreeMap::new())),
        Err(AppError::NotFound(_))
    ));
    match e.st.add_flow_template(req("next-guest", BTreeMap::new())) {
        Err(AppError::Invalid(msg)) => assert!(msg.contains("Lower third layer"), "{msg}"),
        other => panic!("{other:?}"),
    }
    assert!(e.st.list_flows().unwrap().is_empty());
}

#[test]
fn cross_flow_references_resolve() {
    let e = env();
    let m = manifest("manual-hold").unwrap();
    let infos =
        e.st.add_flow_template(req("manual-hold", sample_slots(m)))
            .unwrap();
    assert_eq!(infos.len(), 3);
    let names: Vec<&str> = infos.iter().map(|i| i.flow.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Manual hold · take",
            "Manual hold · hold",
            "Manual hold · release"
        ]
    );
    assert_eq!(
        infos.iter().map(|i| i.flow.armed).collect::<Vec<_>>(),
        [false, true, true]
    );
    let main = &infos[0].flow.id;
    for i in &infos[1..] {
        let target = i
            .flow
            .nodes
            .iter()
            .find_map(|n| match &n.kind {
                NodeKind::RunFlow { flow_id, .. } => Some(flow_id.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(&target, main);
        // The target exists, so no "flow does not exist" warning.
        assert!(
            i.warnings
                .iter()
                .all(|w| w.message != "The flow it runs no longer exists"),
            "{:?}",
            i.warnings
        );
    }
}

#[test]
fn all_or_nothing() {
    let e = env();
    let m = parse(
        json!({
            "schemaVersion": 1, "id": "broken", "name": "Broken", "category": "blocks",
            "slots": [],
            "flows": [
                {"suffix": "ok", "armed": "cue", "nodes": [
                    {"id": "cue", "type": "trigger.cue", "config": {}}
                ], "edges": []},
                {"suffix": "bad", "armed": "cue", "nodes": [
                    {"id": "cue", "type": "trigger.cue", "config": {}},
                    {"id": "wait", "type": "logic.delay", "config": {"ms": 999_999_999_u64}}
                ], "edges": [
                    {"id": "e1", "from": {"node": "cue", "port": "fired"},
                     "to": {"node": "wait", "port": "in"}, "kind": "event"}
                ]}
            ]
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    match e
        .st
        .add_flow_template_from(&m, &req("broken", BTreeMap::new()))
    {
        Err(AppError::Invalid(msg)) => assert!(msg.contains("Broken · bad"), "{msg}"),
        other => panic!("{other:?}"),
    }
    assert!(e.st.list_flows().unwrap().is_empty());
    let dir = e.st.project_dir().unwrap();
    let on_disk = std::fs::read_dir(flows::flows_root(&dir))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(on_disk, 0);
}

#[test]
fn the_flow_limit_holds_for_the_whole_template() {
    let e = env();
    let m = manifest("clean-all-log").unwrap();
    for _ in 0..FLOWS_MAX - 1 {
        e.st.add_flow_template(req("clean-all-log", sample_slots(m)))
            .unwrap();
    }
    // One slot left, two flows needed: nothing is created.
    let m2 = manifest("break-countdown").unwrap();
    assert!(e
        .st
        .add_flow_template(req("break-countdown", sample_slots(m2)))
        .is_err());
    assert_eq!(e.st.list_flows().unwrap().len(), FLOWS_MAX - 1);
}
