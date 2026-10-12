use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::core::events::EventSink;
use crate::core::model::{
    Constant, DataType, ItemStatus, MathOp, NewScreen, RecipeInstance, RecipePanel, RowPick,
    SourceStep,
};

// ---- Helpers ---------------------------------------------------------------------------------

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Value)>>);

impl EventSink for Recorder {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

impl Recorder {
    fn take(&self, name: &str) -> Vec<Value> {
        let mut g = self.0.lock().unwrap();
        let (hit, rest): (Vec<_>, Vec<_>) = g.drain(..).partition(|(n, _)| n == name);
        *g = rest;
        hit.into_iter().map(|(_, v)| v).collect()
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
    st: AppState,
    dir: PathBuf,
    events: Arc<Recorder>,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let events = Arc::new(Recorder::default());
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        events.clone(),
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
        events,
    }
}

fn n(id: &str, kind: NodeKind) -> FlowNode {
    FlowNode {
        id: id.into(),
        position: NodePos::default(),
        kind,
    }
}

fn ev(id: &str, from: &str, fp: &str, to: &str, tp: &str) -> FlowEdge {
    FlowEdge {
        id: id.into(),
        from: PortRef {
            node: from.into(),
            port: fp.into(),
        },
        to: PortRef {
            node: to.into(),
            port: tp.into(),
        },
        kind: WireKind::Event,
    }
}

fn dw(id: &str, from: &str, fp: &str, to: &str, tp: &str) -> FlowEdge {
    FlowEdge {
        kind: WireKind::Data,
        ..ev(id, from, fp, to, tp)
    }
}

fn cue() -> NodeKind {
    NodeKind::Cue { label: None }
}

fn take(s: &str, l: &str) -> NodeKind {
    NodeKind::Take {
        screen_id: s.into(),
        layer_id: l.into(),
        for_ms: None,
    }
}

fn delay(ms: u64) -> NodeKind {
    NodeKind::Delay { ms }
}

fn log() -> NodeKind {
    NodeKind::Log {
        message: "hi".into(),
        toast: false,
    }
}

/// Cue → take.
fn simple() -> (Vec<FlowNode>, Vec<FlowEdge>) {
    (
        vec![n("c", cue()), n("t", take("scr", "lay"))],
        vec![ev("e1", "c", "fired", "t", "in")],
    )
}

fn new_flow(name: &str) -> NewFlow {
    let (nodes, edges) = simple();
    NewFlow {
        name: name.into(),
        armed: false,
        retrigger: Retrigger::Restart,
        origin: None,
        nodes,
        edges,
    }
}

fn err(r: AppResult<()>) -> String {
    match r {
        Err(AppError::Invalid(m)) => m,
        other => panic!("expected invalid, got {other:?}"),
    }
}

fn file_json(e: &Env, id: &str) -> Value {
    serde_json::from_slice(&std::fs::read(flows_root(&e.dir).join(format!("{id}.json"))).unwrap())
        .unwrap()
}

// ---- Validation ------------------------------------------------------------------------------

#[test]
fn a_linear_flow_is_valid_and_names_are_checked() {
    let (nodes, edges) = simple();
    validate_graph(&nodes, &edges).unwrap();
    assert_eq!(validate_name("  Go  ").unwrap(), "Go");
    assert!(validate_name("   ").is_err());
    assert!(validate_name(&"x".repeat(FLOW_NAME_MAX + 1)).is_err());
    assert!(validate_name(&"é".repeat(FLOW_NAME_MAX)).is_ok());
    // No trigger is valid (it never runs).
    validate_graph(&[n("t", take("s", "l"))], &[]).unwrap();
}

#[test]
fn limits_and_ids() {
    let many: Vec<FlowNode> = (0..=FLOW_NODES_MAX)
        .map(|i| n(&format!("n{i}"), log()))
        .collect();
    assert!(err(validate_graph(&many, &[])).contains("Too many nodes"));
    let nodes = vec![n("a", cue()), n("b", log())];
    let edges: Vec<FlowEdge> = (0..=FLOW_EDGES_MAX)
        .map(|i| ev(&format!("e{i}"), "a", "fired", "b", "in"))
        .collect();
    assert!(err(validate_graph(&nodes, &edges)).contains("Too many wires"));
    assert!(err(validate_graph(&[n("a", log()), n("a", log())], &[])).contains("Duplicate node"));
    assert!(err(validate_graph(&[n(&"x".repeat(33), log())], &[])).contains("Invalid node id"));
    assert!(err(validate_graph(&[n("a b", log())], &[])).contains("Invalid node id"));
    let dup = vec![
        ev("e", "a", "fired", "b", "in"),
        ev("e", "a", "fired", "b", "in"),
    ];
    assert!(err(validate_graph(&nodes, &dup)).contains("Duplicate wire"));
    assert!(validate_graph(&[n(&"x".repeat(32), log())], &[]).is_ok());
}

#[test]
fn node_configs_are_checked() {
    let bad = [
        NodeKind::TimeOfDay {
            at: "9:30".into(),
            days: vec![],
        },
        NodeKind::TimeOfDay {
            at: "24:00".into(),
            days: vec![],
        },
        NodeKind::TimeOfDay {
            at: "09:30".into(),
            days: vec![7],
        },
        NodeKind::TimeOfDay {
            at: "09:30".into(),
            days: vec![1, 1],
        },
        NodeKind::BeforeNext {
            source_id: "s".into(),
            minutes: BEFORE_NEXT_MAX_MIN + 1,
        },
        delay(FLOW_DELAY_MAX_MS + 1),
        NodeKind::Debounce { ms: 0 },
        NodeKind::Throttle {
            ms: FLOW_DELAY_MAX_MS + 1,
        },
        NodeKind::WaitUntil {
            at: Some("25:00".into()),
            timeout_ms: None,
        },
        NodeKind::Sequence { count: 1 },
        NodeKind::Sequence { count: 9 },
        NodeKind::Random {
            count: 9,
            no_repeat: false,
        },
        NodeKind::Switch {
            cases: vec!["x".into(); FLOW_CASES_MAX + 1],
        },
        NodeKind::Switch { cases: vec![] },
        NodeKind::Counter { every: 0 },
        NodeKind::Take {
            screen_id: "s".into(),
            layer_id: "l".into(),
            for_ms: Some(0),
        },
        take("", "l"),
        take("a/b", "l"),
        NodeKind::SetFields {
            screen_id: "s".into(),
            layer_id: "l".into(),
            fields: Default::default(),
        },
        NodeKind::SetFields {
            screen_id: "s".into(),
            layer_id: "l".into(),
            fields: [("Bad".to_string(), "x".to_string())].into(),
        },
        NodeKind::Schedule {
            source_id: "s".into(),
            item: ItemPick::Now,
            op: ScheduleOp::SetStart {
                start: "2026-10-04 09:30".into(),
            },
        },
        NodeKind::Schedule {
            source_id: "s".into(),
            item: ItemPick::Now,
            op: ScheduleOp::Delay { minutes: 0 },
        },
        NodeKind::Math {
            op: MathOp::Add,
            a: f64::NAN,
            b: 0.0,
        },
        NodeKind::Cue {
            label: Some("x".repeat(CUE_LABEL_MAX + 1)),
        },
        NodeKind::RunFlow {
            flow_id: String::new(),
            wait: false,
        },
    ];
    for (i, k) in bad.into_iter().enumerate() {
        let name = type_name(&k);
        let r = validate_graph(&[n("x", k)], &[]);
        assert!(matches!(r, Err(AppError::Invalid(_))), "case {i} ({name})");
    }
    let good = [
        NodeKind::TimeOfDay {
            at: "23:59".into(),
            days: vec![0, 6],
        },
        delay(FLOW_DELAY_MAX_MS),
        delay(0),
        NodeKind::Sequence { count: 8 },
        NodeKind::Schedule {
            source_id: "s".into(),
            item: ItemPick::Item {
                item_id: "abc".into(),
            },
            op: ScheduleOp::SetNext {
                start: Some("2026-10-04T09:30".into()),
            },
        },
        NodeKind::Schedule {
            source_id: "s".into(),
            item: ItemPick::Input,
            op: ScheduleOp::Delay { minutes: -5 },
        },
        NodeKind::Constant {
            value: Constant::Number(3.0),
        },
    ];
    for k in good {
        validate_graph(&[n("x", k)], &[]).unwrap();
    }
}

#[test]
fn wires_need_existing_ports_of_the_same_kind_and_compatible_types() {
    let nodes = vec![
        n("c", cue()),
        n("t", take("s", "l")),
        n(
            "k",
            NodeKind::Constant {
                value: Constant::Number(2.0),
            },
        ),
        n(
            "row",
            NodeKind::SourceRow {
                source_id: "src".into(),
                row: RowPick::Current,
            },
        ),
        n("lg", log()),
        n("d", delay(10)),
    ];
    let ok = |edges: Vec<FlowEdge>| validate_graph(&nodes, &edges);
    // Number → text is implicit; a column of a source row is text.
    ok(vec![dw("w", "k", "value", "lg", "text")]).unwrap();
    ok(vec![dw("w", "row", "title", "lg", "text")]).unwrap();
    ok(vec![dw("w", "k", "value", "d", "ms")]).unwrap();
    let m = err(ok(vec![dw("w", "row", "row", "d", "ms")]));
    assert!(m.contains("row can't go into a number input"), "{m}");
    assert!(err(ok(vec![ev("w", "c", "fired", "nope", "in")])).contains("does not exist"));
    assert!(err(ok(vec![ev("w", "c", "nope", "t", "in")])).contains("no output \"nope\""));
    assert!(err(ok(vec![ev("w", "c", "fired", "t", "nope")])).contains("no input \"nope\""));
    // Event output into a data input, data wire on event ports.
    assert!(err(ok(vec![ev("w", "c", "fired", "lg", "text")])).contains("own kind"));
    assert!(err(ok(vec![dw("w", "c", "fired", "t", "in")])).contains("own kind"));
    // A data input takes one wire; event inputs take many.
    let m = err(ok(vec![
        dw("w1", "k", "value", "lg", "text"),
        dw("w2", "row", "title", "lg", "text"),
    ]));
    assert!(m.contains("already has a wire"), "{m}");
    ok(vec![
        ev("w1", "c", "fired", "t", "in"),
        ev("w2", "c", "fired", "lg", "in"),
        ev("w3", "t", "done", "lg", "in"),
    ])
    .unwrap();
    assert!(err(ok(vec![
        ev("w1", "c", "fired", "t", "in"),
        ev("w2", "c", "fired", "t", "in"),
    ]))
    .contains("already connected"));
    // setFields: one input per configured key, rows allowed.
    let sf = n(
        "sf",
        NodeKind::SetFields {
            screen_id: "s".into(),
            layer_id: "l".into(),
            fields: [("items".to_string(), "[]".to_string())].into(),
        },
    );
    let nodes2 = vec![nodes[3].clone(), sf];
    validate_graph(&nodes2, &[dw("w", "row", "rows", "sf", "items")]).unwrap();
    assert!(validate_graph(&nodes2, &[dw("w", "row", "rows", "sf", "other")]).is_err());
}

#[test]
fn at_most_one_cue() {
    let r = validate_graph(&[n("a", cue()), n("b", cue())], &[]);
    assert!(err(r).contains("at most one Cue"));
    validate_graph(
        &[
            n("a", cue()),
            n(
                "b",
                NodeKind::TimeOfDay {
                    at: "10:00".into(),
                    days: vec![],
                },
            ),
        ],
        &[],
    )
    .unwrap();
}

#[test]
fn cycles() {
    // Data cycle: math a ← math b ← math a.
    let math = || NodeKind::Math {
        op: MathOp::Add,
        a: 0.0,
        b: 0.0,
    };
    let m = err(validate_graph(
        &[n("m1", math()), n("m2", math())],
        &[
            dw("w1", "m1", "number", "m2", "a"),
            dw("w2", "m2", "number", "m1", "a"),
        ],
    ));
    assert!(m.starts_with("Data loop"), "{m}");
    assert!(m.contains("m1") && m.contains("m2"), "{m}");
    // Event cycle through plain nodes: rejected with the node ids.
    let m = err(validate_graph(
        &[n("c", cue()), n("a", log()), n("b", log())],
        &[
            ev("w0", "c", "fired", "a", "in"),
            ev("w1", "a", "done", "b", "in"),
            ev("w2", "b", "done", "a", "in"),
        ],
    ));
    assert!(m.starts_with("Event loop"), "{m}");
    assert!(m.contains("a → b → a") || m.contains("b → a → b"), "{m}");
    // The same loop through a Delay (or debounce, throttle, waitUntil) is allowed.
    for wait in [
        delay(1000),
        NodeKind::Debounce { ms: 10 },
        NodeKind::Throttle { ms: 10 },
        NodeKind::WaitUntil {
            at: None,
            timeout_ms: Some(10),
        },
    ] {
        validate_graph(
            &[n("c", cue()), n("a", log()), n("d", wait)],
            &[
                ev("w0", "c", "fired", "a", "in"),
                ev("w1", "a", "done", "d", "in"),
                ev("w2", "d", "out", "a", "in"),
            ],
        )
        .unwrap();
    }
    // A self-loop on a plain node is a cycle.
    assert!(validate_graph(&[n("a", log())], &[ev("w", "a", "done", "a", "in")]).is_err());
}

// ---- check_flow (every error with its node) --------------------------------------------------

/// (node, message) of each issue.
fn issues(list: &[FlowIssue]) -> Vec<(Option<&str>, &str)> {
    list.iter()
        .map(|i| (i.node.as_deref(), i.message.as_str()))
        .collect()
}

fn on<'a>(list: &'a [FlowIssue], node: &str) -> Vec<&'a str> {
    list.iter()
        .filter(|i| i.node.as_deref() == Some(node))
        .map(|i| i.message.as_str())
        .collect()
}

#[test]
fn check_graph_reports_every_error_with_its_node() {
    let math = || NodeKind::Math {
        op: MathOp::Add,
        a: 0.0,
        b: 0.0,
    };
    let nodes = vec![
        n("c1", cue()),
        n("c2", cue()),
        n("t", take("", "lay")),
        n(
            "k",
            NodeKind::Constant {
                value: Constant::Number(2.0),
            },
        ),
        n(
            "row",
            NodeKind::SourceRow {
                source_id: "src".into(),
                row: RowPick::Current,
            },
        ),
        n("lg", log()),
        n("d", delay(10)),
        n("m1", math()),
        n("m2", math()),
        n("a", log()),
        n("b", log()),
        n("x", log()),
    ];
    let edges = vec![
        ev("w1", "c1", "nope", "t", "in"),        // unknown output → c1
        ev("w2", "c1", "fired", "t", "nope"),     // unknown input → t
        ev("w3", "c1", "fired", "lg", "text"),    // event into a data input → lg
        dw("w4", "row", "row", "d", "ms"),        // row into a number input → d
        dw("w5", "k", "value", "lg", "text"),     // ok
        dw("w6", "row", "title", "lg", "text"),   // second wire into a data input → lg
        ev("w7", "c1", "fired", "gone", "in"),    // target missing → c1
        ev("w8", "gone", "fired", "lg", "in"),    // source missing → lg
        ev("w9", "gone", "fired", "gone2", "in"), // neither → no node
        dw("w10", "m1", "number", "m2", "a"),     // data cycle m1 ⇄ m2
        dw("w11", "m2", "number", "m1", "a"),
        ev("w12", "a", "done", "b", "in"), // event cycle a → b → x → a
        ev("w13", "b", "done", "x", "in"),
        ev("w14", "x", "done", "a", "in"),
        ev("w15", "c1", "fired", "a", "in"),
        ev("w16", "c1", "fired", "a", "in"), // the same ports again → a
    ];
    let list = check_graph(&nodes, &edges);
    // Deploy still fails with the first error, as before.
    let first = err(validate_graph(&nodes, &edges));
    assert!(
        first.starts_with("Node t (action.take): pick a screen"),
        "{first}"
    );
    assert_eq!(list[0].node.as_deref(), Some("t"));
    assert_eq!(
        on(&list, "t"),
        ["pick a screen", "action.take has no input \"nope\""]
    );
    assert_eq!(on(&list, "c2"), ["A flow has at most one Cue trigger"]);
    let c1 = on(&list, "c1");
    assert!(c1.contains(&"trigger.cue has no output \"nope\""), "{c1:?}");
    assert!(c1.contains(&"node gone does not exist"), "{c1:?}");
    let lg = on(&list, "lg");
    assert!(
        lg.contains(&"event and data ports only connect to their own kind"),
        "{lg:?}"
    );
    assert!(
        lg.contains(&"input \"text\" of node lg already has a wire"),
        "{lg:?}"
    );
    assert!(lg.contains(&"node gone does not exist"), "{lg:?}");
    assert_eq!(on(&list, "d"), ["row can't go into a number input"]);
    assert!(issues(&list).contains(&(None, "node gone does not exist")));
    for m in ["m1", "m2"] {
        let got = on(&list, m);
        assert_eq!(got.len(), 1, "{m}: {got:?}");
        assert!(got[0].starts_with("Data loop: "), "{got:?}");
    }
    for m in ["b", "x"] {
        let got = on(&list, m);
        assert_eq!(got.len(), 1, "{m}: {got:?}");
        assert!(got[0].starts_with("Event loop without a Delay"), "{got:?}");
    }
    let a = on(&list, "a");
    assert!(a.contains(&"these ports are already connected"), "{a:?}");
    assert!(a.iter().any(|m| m.starts_with("Event loop")), "{a:?}");
    // Nodes outside a cycle or any problem carry nothing.
    assert!(on(&list, "k").is_empty() && on(&list, "row").is_empty());
}

#[test]
fn check_graph_of_a_valid_graph_and_of_names_and_limits() {
    let (nodes, edges) = simple();
    assert!(check_graph(&nodes, &edges).is_empty());
    // Ids and limits carry no node.
    let list = check_graph(&[n("a b", log()), n("x", log()), n("x", log())], &[]);
    assert_eq!(
        issues(&list),
        [
            (None, "Invalid node id: a b"),
            (None, "Duplicate node id: x")
        ]
    );
    let many: Vec<FlowNode> = (0..=FLOW_NODES_MAX)
        .map(|i| n(&format!("n{i}"), log()))
        .collect();
    let list = check_graph(&many, &[]);
    assert_eq!(list.len(), 1);
    assert!(list[0].node.is_none() && list[0].message.starts_with("Too many nodes"));
    // A self-loop names its node.
    let list = check_graph(&[n("a", log())], &[ev("w", "a", "done", "a", "in")]);
    assert_eq!(on(&list, "a").len(), 1);
    // Two separate event cycles: every node of each.
    let list = check_graph(
        &[n("a", log()), n("b", log()), n("p", log()), n("q", log())],
        &[
            ev("w1", "a", "done", "b", "in"),
            ev("w2", "b", "done", "a", "in"),
            ev("w3", "p", "done", "q", "in"),
            ev("w4", "q", "done", "p", "in"),
        ],
    );
    let nodes: Vec<Option<&str>> = list.iter().map(|i| i.node.as_deref()).collect();
    assert_eq!(nodes, [Some("a"), Some("b"), Some("p"), Some("q")]);
    assert!(list[0].message.contains("a → b → a"), "{list:?}");
    assert!(list[2].message.contains("p → q → p"), "{list:?}");
}

#[test]
fn check_flow_adds_warnings_and_saves_nothing() {
    let e = env();
    let id = e.st.create_flow(new_flow("Go")).unwrap().flow.id;
    let before = std::fs::read(flow_path(&e.dir, &id).unwrap()).unwrap();
    // A valid structure: warnings only (missing screen; a node with no problem has none).
    let (nodes, edges) = simple();
    let c = e.st.check_flow(nodes, edges).unwrap();
    assert!(c.errors.is_empty(), "{c:?}");
    assert_eq!(c.warnings.len(), 1, "{c:?}");
    assert_eq!(c.warnings[0].node.as_deref(), Some("t"));
    assert_eq!(c.warnings[0].message, "Screen no longer exists");
    // Errors and warnings together; no trigger is a warning without a node.
    let c =
        e.st.check_flow(
            vec![
                n("t", take("", "lay")),
                n(
                    "r",
                    NodeKind::RunFlow {
                        flow_id: id.clone(),
                        wait: false,
                    },
                ),
            ],
            vec![],
        )
        .unwrap();
    assert_eq!(issues(&c.errors), [(Some("t"), "pick a screen")]);
    assert_eq!(
        c.warnings
            .last()
            .map(|w| (w.node.as_deref(), w.message.as_str())),
        Some((None, "No trigger: this flow never runs"))
    );
    assert!(on(&c.warnings, "r").is_empty(), "the flow exists");
    assert!(on(&c.warnings, "t").is_empty(), "its config error only");
    // Nothing was written or deployed.
    assert_eq!(
        std::fs::read(flow_path(&e.dir, &id).unwrap()).unwrap(),
        before
    );
    assert_eq!(e.st.flows().len(), 1);
    assert!(e.st.flows().flow(&id).unwrap().draft.is_none());
    // No project: noProject.
    e.st.close_project();
    let (nodes, edges) = simple();
    assert!(matches!(
        e.st.check_flow(nodes, edges),
        Err(AppError::NoProject)
    ));
}

#[test]
fn drafts_check_limits_and_ids_only() {
    // Half-wired: an edge to a node that doesn't exist yet, a Take without a target.
    let nodes = vec![n("c", cue()), n("t", take("", ""))];
    let edges = vec![ev("e", "c", "fired", "gone", "in")];
    validate_draft(&nodes, &edges).unwrap();
    assert!(validate_graph(&nodes, &edges).is_err());
    assert!(validate_draft(&[n("a", log()), n("a", log())], &[]).is_err());
}

#[test]
fn rules_and_macros_convert_to_valid_linear_flows() {
    let rule = Rule {
        id: "r1".into(),
        name: " Start ".into(),
        enabled: true,
        when: Trigger::ItemStatus {
            source_id: "src".into(),
            status: ItemStatus::Now,
            item_id: None,
        },
        then: vec![
            RuleAction::Take {
                screen_id: "s".into(),
                layer_id: "l".into(),
                for_ms: Some(5000),
            },
            RuleAction::PauseSource {
                source_id: "src".into(),
                paused: false,
            },
            RuleAction::RunMacro {
                macro_id: "mac".into(),
            },
        ],
        last_fired_at: Some("2026-10-01T10:00:00Z".into()),
    };
    let f = rule_to_flow(&rule, "flow1".into(), Some("inst".into()));
    validate_flow(&f).unwrap();
    assert_eq!(f.name, "Start");
    assert!(f.armed);
    assert_eq!(f.origin.as_deref(), Some("inst"));
    assert_eq!(f.last_fired_at, rule.last_fired_at);
    assert_eq!(f.nodes.len(), 4);
    assert!(matches!(f.nodes[0].kind, NodeKind::ItemStatus { .. }));
    assert_eq!(
        f.nodes[2].kind,
        NodeKind::PauseSource {
            source_id: "src".into(),
            mode: PauseMode::Resume
        }
    );
    assert_eq!(
        f.nodes[3].kind,
        NodeKind::RunFlow {
            flow_id: "mac".into(),
            wait: false
        }
    );
    let ports: Vec<(&str, &str)> = f
        .edges
        .iter()
        .map(|e| (e.from.port.as_str(), e.to.port.as_str()))
        .collect();
    assert_eq!(ports, [("fired", "in"), ("done", "in"), ("done", "in")]);
    assert_eq!(f.nodes[2].position.x, 2.0 * LINEAR_STEP_X);

    let m = Macro {
        id: "mac".into(),
        name: "Open".into(),
        steps: vec![
            MacroStep::Take {
                screen_id: "s".into(),
                layer_id: "l".into(),
            },
            MacroStep::Wait { ms: 500 },
            MacroStep::ClearScreen {
                screen_id: "s".into(),
            },
        ],
    };
    let f = macro_to_flow(&m);
    validate_flow(&f).unwrap();
    assert_eq!(f.id, "mac");
    assert!(f.armed);
    assert_eq!(f.origin, None);
    assert_eq!(f.retrigger, Retrigger::Restart);
    let kinds: Vec<&str> = f.nodes.iter().map(|n| type_name(&n.kind)).collect();
    assert_eq!(
        kinds,
        [
            "trigger.cue",
            "action.take",
            "logic.delay",
            "action.clearScreen"
        ]
    );
    assert_eq!(f.nodes[2].kind, delay(500));
    let ports: Vec<&str> = f.edges.iter().map(|e| e.from.port.as_str()).collect();
    assert_eq!(ports, ["fired", "done", "out"]);
    // An empty macro is a lone cue.
    let empty = macro_to_flow(&Macro {
        id: "m2".into(),
        name: "Nothing".into(),
        steps: vec![],
    });
    assert_eq!(empty.nodes.len(), 1);
    validate_flow(&empty).unwrap();
}

// ---- Store -----------------------------------------------------------------------------------

#[test]
fn create_save_and_reload_round_trip() {
    let e = env();
    let info = e.st.create_flow(new_flow("  Opener ")).unwrap();
    let id = info.flow.id.clone();
    assert_eq!(info.flow.name, "Opener");
    assert!(!info.read_only && !info.running);
    // Missing targets are soft warnings.
    assert!(info.warnings.iter().any(|w| w.node.as_deref() == Some("t")));
    let text = std::fs::read_to_string(flows_root(&e.dir).join(format!("{id}.json"))).unwrap();
    assert!(text.starts_with("{\n  \"schemaVersion\": 1,"), "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["nodes"][0]["type"], "trigger.cue");
    assert!(v.get("draft").is_none());
    // Invalid input is refused and nothing is written.
    let mut bad = new_flow("Bad");
    bad.nodes.push(n("c2", cue()));
    assert!(matches!(e.st.create_flow(bad), Err(AppError::Invalid(_))));
    assert_eq!(e.st.list_flows().unwrap().len(), 1);
    // save_flow replaces the graph.
    let (mut nodes, mut edges) = simple();
    nodes.push(n("lg", log()));
    edges.push(ev("e2", "t", "done", "lg", "in"));
    let saved =
        e.st.save_flow(SaveFlow {
            id: id.clone(),
            name: "Opener 2".into(),
            retrigger: Retrigger::Ignore,
            nodes,
            edges,
        })
        .unwrap();
    assert_eq!(saved.flow.nodes.len(), 3);
    assert_eq!(file_json(&e, &id)["retrigger"], "ignore");
    // Reopen: same flow.
    let before = e.st.get_flow(&id).unwrap().flow;
    let path = e.dir.clone();
    e.st.close_project();
    assert!(e.st.flows().is_empty());
    assert!(matches!(e.st.list_flows(), Err(AppError::NoProject)));
    e.st.open_project(&path).unwrap();
    assert_eq!(e.st.get_flow(&id).unwrap().flow, before);
    assert!(matches!(e.st.get_flow("nope"), Err(AppError::NotFound(_))));
}

#[test]
fn list_order_is_creation_order_within_a_session() {
    let e = env();
    let b = e.st.create_flow(new_flow("B")).unwrap().flow.id;
    let a = e.st.create_flow(new_flow("A")).unwrap().flow.id;
    let ids: Vec<String> =
        e.st.list_flows()
            .unwrap()
            .into_iter()
            .map(|f| f.flow.id)
            .collect();
    assert_eq!(ids, [b.clone(), a.clone()]);
    // Loaded by name.
    let path = e.dir.clone();
    e.st.open_project(&path).unwrap();
    let ids: Vec<String> =
        e.st.list_flows()
            .unwrap()
            .into_iter()
            .map(|f| f.flow.id)
            .collect();
    assert_eq!(ids, [a, b]);
}

#[test]
fn drafts_deploy_and_discard() {
    let e = env();
    let id = e.st.create_flow(new_flow("F")).unwrap().flow.id;
    let (mut nodes, edges) = simple();
    nodes.push(n("x", take("", "")));
    // A half-done draft is stored in memory, written by the flusher.
    let info =
        e.st.save_flow_draft(&id, nodes.clone(), edges.clone())
            .unwrap();
    assert!(info.flow.draft.is_some());
    assert!(e.st.flows().is_dirty(&id));
    assert!(file_json(&e, &id).get("draft").is_none());
    e.st.flush_all();
    assert!(file_json(&e, &id)["draft"]["nodes"].is_array());
    // Deploying it fails validation; the draft stays.
    assert!(matches!(e.st.deploy_flow(&id), Err(AppError::Invalid(_))));
    assert!(e.st.get_flow(&id).unwrap().flow.draft.is_some());
    // A draft equal to the deployed graph is no draft.
    let (n0, e0) = simple();
    let info = e.st.save_flow_draft(&id, n0, e0).unwrap();
    assert!(info.flow.draft.is_none());
    // A valid draft deploys.
    nodes.pop();
    nodes.push(n("lg", log()));
    e.st.save_flow_draft(&id, nodes, edges).unwrap();
    let info = e.st.deploy_flow(&id).unwrap();
    assert!(info.flow.draft.is_none());
    assert_eq!(info.flow.nodes.len(), 3);
    assert_eq!(file_json(&e, &id)["nodes"].as_array().unwrap().len(), 3);
    // Deploy without a draft does nothing.
    e.st.deploy_flow(&id).unwrap();
    // Discard.
    let (n1, e1) = simple();
    e.st.save_flow_draft(&id, n1, e1).unwrap();
    let info = e.st.discard_flow_draft(&id).unwrap();
    assert!(info.flow.draft.is_none());
    assert_eq!(info.flow.nodes.len(), 3);
    assert!(file_json(&e, &id).get("draft").is_none());
}

#[test]
fn armed_and_last_fired_are_written_coalesced() {
    let e = env();
    let id = e.st.create_flow(new_flow("F")).unwrap().flow.id;
    let writes = e.st.content_write_count();
    let info = e.st.set_flow_armed(&id, true).unwrap();
    assert!(info.flow.armed);
    assert_eq!(file_json(&e, &id)["armed"], false, "not written yet");
    e.st.flow_fired(&id, "2026-10-04T10:00:00Z");
    e.st.flow_fired(&id, "2026-10-04T10:00:01Z");
    assert_eq!(
        e.st.get_flow(&id).unwrap().flow.last_fired_at.as_deref(),
        Some("2026-10-04T10:00:01Z")
    );
    e.st.flush_all();
    let v = file_json(&e, &id);
    assert_eq!(v["armed"], true);
    assert_eq!(v["lastFiredAt"], "2026-10-04T10:00:01Z");
    assert_eq!(e.st.content_write_count(), writes + 1, "one write for both");
    // Nothing dirty: nothing written.
    e.st.flush_all();
    assert_eq!(e.st.content_write_count(), writes + 1);
    // A structural write keeps lastFiredAt.
    e.st.rename_flow(&id, "G").unwrap();
    assert_eq!(file_json(&e, &id)["lastFiredAt"], "2026-10-04T10:00:01Z");
}

#[test]
fn the_content_flusher_writes_armed_on_its_own() {
    let e = env();
    e.st.set_content_flush_delay(Duration::from_millis(20));
    let id = e.st.create_flow(new_flow("F")).unwrap().flow.id;
    e.st.set_flow_armed(&id, true).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while file_json(&e, &id)["armed"] != true {
        assert!(std::time::Instant::now() < deadline, "flusher never wrote");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn newer_schema_is_read_only_and_never_written() {
    let e = env();
    let root = flows_root(&e.dir);
    std::fs::create_dir_all(&root).unwrap();
    let mut v = serde_json::to_value(rule_to_flow(
        &Rule {
            id: "r".into(),
            name: "Future".into(),
            enabled: true,
            when: Trigger::SlideshowWrap {
                source_id: "s".into(),
            },
            then: vec![RuleAction::Clear {
                screen_id: "s".into(),
                layer_id: "l".into(),
            }],
            last_fired_at: None,
        },
        "future0001".into(),
        None,
    ))
    .unwrap();
    v["schemaVersion"] = json!(7);
    v["somethingNew"] = json!(true);
    let path = root.join("future0001.json");
    let text = serde_json::to_string(&v).unwrap();
    std::fs::write(&path, &text).unwrap();
    let dir = e.dir.clone();
    e.st.open_project(&dir).unwrap();
    let info = e.st.get_flow("future0001").unwrap();
    assert!(info.read_only);
    for r in [
        e.st.rename_flow("future0001", "X").map(drop),
        e.st.set_flow_armed("future0001", false).map(drop),
        e.st.delete_flow("future0001"),
        e.st.discard_flow_draft("future0001").map(drop),
    ] {
        assert!(matches!(r, Err(AppError::Invalid(_))), "{r:?}");
    }
    e.st.flow_fired("future0001", "2026-10-04T10:00:00Z");
    e.st.flush_all();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    // write_flow refuses to overwrite it too.
    assert!(matches!(
        write_flow(&dir, &info.flow),
        Err(AppError::NewerVersion { .. })
    ));
    // Duplicating gives an ordinary, writable flow.
    let copy = e.st.duplicate_flow("future0001").unwrap();
    assert!(!copy.read_only);
}

#[test]
fn corrupt_files_are_set_aside_and_invalid_graphs_are_listed_not_run() {
    let e = env();
    let root = flows_root(&e.dir);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("broken0001.json"), "{{").unwrap();
    std::fs::write(
        root.join("shape00001.json"),
        r#"{"schemaVersion":1,"name":3}"#,
    )
    .unwrap();
    let mut f = macro_to_flow(&Macro {
        id: "cyc0000001".into(),
        name: "Two cues".into(),
        steps: vec![],
    });
    f.nodes.push(n("c2", cue()));
    write_flow(&e.dir, &f).unwrap();
    let dir = e.dir.clone();
    e.st.open_project(&dir).unwrap();
    let list = e.st.list_flows().unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].warnings[0].message.contains("at most one Cue"));
    let names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|d| d.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(names
        .iter()
        .any(|n| n.starts_with("broken0001.json.corrupt-")));
    assert!(names
        .iter()
        .any(|n| n.starts_with("shape00001.json.corrupt-")));
    assert_eq!(e.events.take(names::STORE_RECOVERED).len(), 2);
    // Fixing it through save_flow makes it valid.
    let fixed =
        e.st.save_flow(SaveFlow {
            id: "cyc0000001".into(),
            name: "Fixed".into(),
            retrigger: Retrigger::Restart,
            nodes: vec![n("c", cue())],
            edges: vec![],
        })
        .unwrap();
    assert!(fixed.warnings.is_empty(), "{:?}", fixed.warnings);
}

#[test]
fn duplicate_and_delete() {
    let e = env();
    let mut req = new_flow("Show");
    req.armed = true;
    let id = e.st.create_flow(req).unwrap().flow.id;
    let copy = e.st.duplicate_flow(&id).unwrap();
    assert_ne!(copy.flow.id, id);
    assert_eq!(copy.flow.name, "Show copy");
    assert!(!copy.flow.armed);
    assert_eq!(copy.flow.origin, None);
    assert!(flows_root(&e.dir)
        .join(format!("{}.json", copy.flow.id))
        .is_file());
    // A long name stays within the limit.
    let long =
        e.st.create_flow(new_flow(&"n".repeat(FLOW_NAME_MAX)))
            .unwrap();
    let c = e.st.duplicate_flow(&long.flow.id).unwrap();
    assert!(c.flow.name.ends_with(" copy"));
    assert_eq!(c.flow.name.chars().count(), FLOW_NAME_MAX);
    // Delete drops the file and the hotkey.
    let mut ui = e.st.ui_state().unwrap();
    ui.control.hotkeys.insert(format!("flow/{id}"), "F3".into());
    ui.control.hotkeys.insert("s1/l1".into(), "F4".into());
    e.st.set_ui_state(ui).unwrap();
    e.st.delete_flow(&id).unwrap();
    assert!(!flows_root(&e.dir).join(format!("{id}.json")).exists());
    assert!(matches!(e.st.get_flow(&id), Err(AppError::NotFound(_))));
    let hk = e.st.ui_state().unwrap().control.hotkeys;
    assert!(!hk.contains_key(&format!("flow/{id}")));
    assert!(hk.contains_key("s1/l1"));
    assert!(matches!(e.st.delete_flow(&id), Err(AppError::NotFound(_))));
}

#[test]
fn deleting_a_recipe_flow_updates_its_instance() {
    let e = env();
    let inst =
        e.st.install_recipe(&crate::core::model::InstallRecipe {
            recipe_id: "event-schedule".into(),
            name: "Day".into(),
            screen: crate::core::model::RecipeScreen::New {
                name: "Event".into(),
                width: 1920,
                height: 1080,
            },
        })
        .unwrap();
    assert!(inst.flow_ids.len() >= 2);
    let gone = inst.flow_ids[0].clone();
    e.st.delete_flow(&gone).unwrap();
    let now = e.st.list_recipe_instances().unwrap();
    assert!(!now[0].flow_ids.contains(&gone));
    let v: Value = serde_json::from_slice(
        &std::fs::read(recipes::recipes_root(&e.dir).join(format!("{}.json", inst.id))).unwrap(),
    )
    .unwrap();
    assert_eq!(v["schemaVersion"], 2);
    assert!(!v["flowIds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x == gone.as_str()));
}

#[test]
fn a_new_flow_can_belong_to_an_installed_recipe() {
    let e = env();
    let inst =
        e.st.install_recipe(&crate::core::model::InstallRecipe {
            recipe_id: "event-schedule".into(),
            name: "Day".into(),
            screen: crate::core::model::RecipeScreen::New {
                name: "Event".into(),
                width: 1920,
                height: 1080,
            },
        })
        .unwrap();
    let mut req = new_flow("Own rule");
    req.origin = Some(inst.id.clone());
    let f = e.st.create_flow(req).unwrap();
    assert_eq!(f.flow.origin.as_deref(), Some(inst.id.as_str()));
    let now = e.st.list_recipe_instances().unwrap();
    assert_eq!(now[0].flow_ids.last(), Some(&f.flow.id));
    let v: Value = serde_json::from_slice(
        &std::fs::read(recipes::recipes_root(&e.dir).join(format!("{}.json", inst.id))).unwrap(),
    )
    .unwrap();
    assert!(v["flowIds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x == f.flow.id.as_str()));
    // Deleting it drops it from the recipe again.
    e.st.delete_flow(&f.flow.id).unwrap();
    assert!(!e.st.list_recipe_instances().unwrap()[0]
        .flow_ids
        .contains(&f.flow.id));

    // An origin must be an installed recipe.
    let mut bad = new_flow("Stray");
    bad.origin = Some("nope000001".into());
    let n = e.st.list_flows().unwrap().len();
    assert!(matches!(e.st.create_flow(bad), Err(AppError::Invalid(_))));
    assert_eq!(e.st.list_flows().unwrap().len(), n);
}

#[test]
fn warnings_name_missing_targets() {
    let e = env();
    let s =
        e.st.create_screen(&NewScreen {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap();
    let flow = Flow {
        id: "w000000001".into(),
        name: "W".into(),
        armed: false,
        retrigger: Retrigger::Restart,
        origin: None,
        nodes: vec![
            n("a", take(&s.doc.id, "nolayer")),
            n(
                "b",
                NodeKind::ClearScreen {
                    screen_id: s.doc.id.clone(),
                },
            ),
            n(
                "c",
                NodeKind::StepSource {
                    source_id: "nosource".into(),
                    step: SourceStep::By { delta: 1 },
                },
            ),
            n(
                "d",
                NodeKind::RunFlow {
                    flow_id: "noflow".into(),
                    wait: false,
                },
            ),
        ],
        edges: vec![],
        draft: None,
        last_fired_at: None,
        updated_at: String::new(),
    };
    let w = e.st.flow_warnings(&flow);
    let nodes: Vec<Option<&str>> = w.iter().map(|i| i.node.as_deref()).collect();
    assert_eq!(nodes, [Some("a"), Some("c"), Some("d"), None], "{w:?}");
    assert!(w[3].message.contains("No trigger"));
}

// ---- Migration -------------------------------------------------------------------------------

fn legacy_project(e: &Env) {
    let dir = &e.dir;
    std::fs::write(
        dir.join("macros.json"),
        serde_json::to_string(&json!({
            "schemaVersion": 1,
            "macros": [{
                "id": "mac0000001",
                "name": "Open show",
                "steps": [
                    {"do": "take", "screenId": "s1", "layerId": "l1"},
                    {"do": "wait", "ms": 1500},
                    {"do": "clearScreen", "screenId": "s1"}
                ]
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    let inst = RecipeInstance {
        id: "inst000001".into(),
        recipe_id: "event-schedule".into(),
        name: "Day 1".into(),
        panel: RecipePanel::Schedule,
        source_id: Some("src".into()),
        screen_id: "s1".into(),
        layer_ids: vec!["l1".into()],
        template_ids: vec![],
        rules: vec![
            Rule {
                id: "r1".into(),
                name: "On NOW".into(),
                enabled: true,
                when: Trigger::ItemStatus {
                    source_id: "src".into(),
                    status: ItemStatus::Now,
                    item_id: None,
                },
                then: vec![RuleAction::Take {
                    screen_id: "s1".into(),
                    layer_id: "l1".into(),
                    for_ms: Some(10_000),
                }],
                last_fired_at: Some("2026-09-01T12:00:00Z".into()),
            },
            Rule {
                id: "r2".into(),
                name: "Run the opener".into(),
                enabled: false,
                when: Trigger::TimeOfDay {
                    at: "09:00".into(),
                    days: vec![],
                },
                then: vec![RuleAction::RunMacro {
                    macro_id: "mac0000001".into(),
                }],
                last_fired_at: None,
            },
        ],
        flow_ids: vec![],
        created_at: "2026-09-01T10:00:00Z".into(),
    };
    let mut v = serde_json::to_value(&inst).unwrap();
    v["schemaVersion"] = json!(1);
    v.as_object_mut().unwrap().remove("flowIds");
    let root = recipes::recipes_root(dir);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("inst000001.json"), v.to_string()).unwrap();
    std::fs::write(
        dir.join("ui-state.json"),
        json!({
            "schemaVersion": 1,
            "control": { "hotkeys": { "macro/mac0000001": "F6", "s1/l1": "F1" } }
        })
        .to_string(),
    )
    .unwrap();
}

fn pre_migrate_backups(e: &Env) -> usize {
    crate::core::backup::list(&e.dir)
        .unwrap()
        .iter()
        .filter(|b| b.reason == BackupReason::PreMigrate)
        .count()
}

#[test]
fn migrates_rules_macros_and_hotkeys_once() {
    let e = env();
    let dir = e.dir.clone();
    e.st.close_project();
    legacy_project(&e);
    e.st.open_project(&dir).unwrap();

    // A pre-migrate backup holding the old files.
    assert_eq!(pre_migrate_backups(&e), 1);
    let b = crate::core::backup::list(&dir).unwrap();
    let snap = dir.join(".backups").join(
        &b.iter()
            .find(|b| b.reason == BackupReason::PreMigrate)
            .unwrap()
            .id,
    );
    assert!(snap.join("macros.json").is_file());
    assert!(snap.join("recipes/inst000001.json").is_file());

    let list = e.st.list_flows().unwrap();
    assert_eq!(list.len(), 3, "{list:?}");
    // The macro: same id, a cue, armed, no origin.
    let mac = e.st.get_flow("mac0000001").unwrap().flow;
    assert!(mac.armed);
    assert_eq!(mac.origin, None);
    let kinds: Vec<&str> = mac.nodes.iter().map(|n| type_name(&n.kind)).collect();
    assert_eq!(
        kinds,
        [
            "trigger.cue",
            "action.take",
            "logic.delay",
            "action.clearScreen"
        ]
    );
    // The rules: origin, armed = enabled, lastFiredAt kept, runMacro → runFlow.
    let inst = &e.st.list_recipe_instances().unwrap()[0];
    assert!(inst.rules.is_empty());
    assert_eq!(inst.flow_ids.len(), 2);
    let on_now = list.iter().find(|f| f.flow.name == "On NOW").unwrap();
    assert_eq!(on_now.flow.origin.as_deref(), Some("inst000001"));
    assert!(on_now.flow.armed);
    assert_eq!(
        on_now.flow.last_fired_at.as_deref(),
        Some("2026-09-01T12:00:00Z")
    );
    let opener = list
        .iter()
        .find(|f| f.flow.name == "Run the opener")
        .unwrap();
    assert!(!opener.flow.armed);
    assert_eq!(
        opener.flow.nodes[1].kind,
        NodeKind::RunFlow {
            flow_id: "mac0000001".into(),
            wait: false
        }
    );
    assert!(inst.flow_ids.contains(&on_now.flow.id) && inst.flow_ids.contains(&opener.flow.id));
    // Files: recipe v2 with flowIds and no rules; macros.json is the marker.
    let rv: Value = serde_json::from_slice(
        &std::fs::read(recipes::recipes_root(&dir).join("inst000001.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(rv["schemaVersion"], 2);
    assert!(rv.get("rules").is_none());
    assert_eq!(rv["flowIds"].as_array().unwrap().len(), 2);
    let mv: Value =
        serde_json::from_slice(&std::fs::read(dir.join("macros.json")).unwrap()).unwrap();
    assert_eq!(mv, json!({"schemaVersion": 2, "migratedTo": "flows"}));
    // Hotkeys.
    let hk = e.st.ui_state().unwrap().control.hotkeys;
    assert_eq!(hk.get("flow/mac0000001").map(String::as_str), Some("F6"));
    assert_eq!(hk.get("s1/l1").map(String::as_str), Some("F1"));
    assert!(!hk.contains_key("macro/mac0000001"));
    e.st.flush_all();
    let uv: Value =
        serde_json::from_slice(&std::fs::read(dir.join("ui-state.json")).unwrap()).unwrap();
    assert_eq!(uv["control"]["hotkeys"]["flow/mac0000001"], "F6");

    // Second open: nothing new.
    let ids: Vec<String> = list.iter().map(|f| f.flow.id.clone()).collect();
    e.st.close_project();
    e.st.open_project(&dir).unwrap();
    let again: Vec<String> =
        e.st.list_flows()
            .unwrap()
            .into_iter()
            .map(|f| f.flow.id)
            .collect();
    let (mut a, mut b) = (ids, again);
    a.sort();
    b.sort();
    assert_eq!(a, b);
    assert_eq!(pre_migrate_backups(&e), 1);
}

#[test]
fn an_interrupted_migration_does_not_duplicate_rule_flows() {
    let e = env();
    let dir = e.dir.clone();
    e.st.close_project();
    legacy_project(&e);
    // As if a previous run wrote the rule flows but not the recipe file.
    let plan = migration_plan(&dir);
    let inst = plan.recipes[0].clone();
    let f = rule_to_flow(&inst.rules[0], "earlier001".into(), Some(inst.id.clone()));
    write_flow(&dir, &f).unwrap();
    e.st.open_project(&dir).unwrap();
    let list = e.st.list_flows().unwrap();
    assert_eq!(
        list.iter().filter(|f| f.flow.name == "On NOW").count(),
        1,
        "{list:?}"
    );
    assert!(e.st.list_recipe_instances().unwrap()[0]
        .flow_ids
        .contains(&"earlier001".to_string()));
}

#[test]
fn a_newer_macros_file_or_recipe_is_left_alone() {
    let e = env();
    let dir = e.dir.clone();
    e.st.close_project();
    std::fs::write(dir.join("macros.json"), r#"{"schemaVersion": 5, "x": 1}"#).unwrap();
    let root = recipes::recipes_root(&dir);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("new0000001.json"), r#"{"schemaVersion": 9}"#).unwrap();
    e.st.open_project(&dir).unwrap();
    assert_eq!(pre_migrate_backups(&e), 0);
    assert_eq!(
        std::fs::read_to_string(dir.join("macros.json")).unwrap(),
        r#"{"schemaVersion": 5, "x": 1}"#
    );
    assert_eq!(
        std::fs::read_to_string(root.join("new0000001.json")).unwrap(),
        r#"{"schemaVersion": 9}"#
    );
    assert!(e.st.list_flows().unwrap().is_empty());
}

// ---- Recipes ---------------------------------------------------------------------------------

#[test]
fn removing_a_recipe_deletes_or_keeps_its_flows() {
    let e = env();
    let install = |name: &str| {
        e.st.install_recipe(&crate::core::model::InstallRecipe {
            recipe_id: "event-schedule".into(),
            name: name.into(),
            screen: crate::core::model::RecipeScreen::New {
                name: name.into(),
                width: 1920,
                height: 1080,
            },
        })
        .unwrap()
    };
    let keep = install("A");
    for id in &keep.flow_ids {
        let f = e.st.get_flow(id).unwrap();
        assert_eq!(f.flow.origin.as_deref(), Some(keep.id.as_str()));
        assert!(!f.flow.armed, "recipes install disarmed");
    }
    e.st.remove_recipe(&keep.id, false).unwrap();
    for id in &keep.flow_ids {
        let f = e.st.get_flow(id).unwrap();
        assert_eq!(f.flow.origin, None, "kept as own flows");
        assert_eq!(file_json(&e, id)["origin"], Value::Null);
    }
    let gone = install("B");
    e.st.remove_recipe(&gone.id, true).unwrap();
    for id in &gone.flow_ids {
        assert!(matches!(e.st.get_flow(id), Err(AppError::NotFound(_))));
        assert!(!flows_root(&e.dir).join(format!("{id}.json")).exists());
    }
    assert_eq!(e.st.list_flows().unwrap().len(), keep.flow_ids.len());
}

// ---- Watcher ---------------------------------------------------------------------------------

fn changes(e: &Env) -> Vec<FilesChanged> {
    e.events
        .take(names::PROJECT_FILES_CHANGED)
        .into_iter()
        .map(|v| serde_json::from_value(v).unwrap())
        .collect()
}

#[test]
fn watcher_reloads_valid_files_and_keeps_the_last_good_one() {
    let e = env();
    let id = e.st.create_flow(new_flow("F")).unwrap().flow.id;
    e.events.take(names::PROJECT_FILES_CHANGED);
    let path = flows_root(&e.dir).join(format!("{id}.json"));
    let batch = || vec![(FileKind::Flow, id.clone())];

    // Unchanged (e.g. our own write seen late): nothing announced.
    e.st.handle_file_changes(&e.dir, batch());
    assert!(changes(&e).is_empty());

    // A valid edit replaces the flow.
    let mut v = file_json(&e, &id);
    v["name"] = json!("Edited");
    v["armed"] = json!(true);
    std::fs::write(&path, v.to_string()).unwrap();
    e.st.handle_file_changes(&e.dir, batch());
    let c = changes(&e);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].kind, FileKind::Flow);
    assert_eq!(c[0].change, FileChange::Changed);
    let f = e.st.get_flow(&id).unwrap().flow;
    assert_eq!(f.name, "Edited");
    assert!(f.armed);

    // Invalid JSON or an invalid graph keeps the last good flow.
    std::fs::write(&path, "{ nope").unwrap();
    e.st.handle_file_changes(&e.dir, batch());
    let c = changes(&e);
    assert_eq!(c[0].change, FileChange::Invalid);
    assert!(c[0].message.is_some());
    assert_eq!(e.st.get_flow(&id).unwrap().flow.name, "Edited");
    assert!(path.exists(), "an external edit is never set aside");
    v["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "c2", "type": "trigger.cue", "config": {}}));
    std::fs::write(&path, v.to_string()).unwrap();
    e.st.handle_file_changes(&e.dir, batch());
    let c = changes(&e);
    assert_eq!(c[0].change, FileChange::Invalid);
    assert!(c[0].message.as_deref().unwrap().contains("Cue"));
    assert_eq!(e.st.get_flow(&id).unwrap().flow.nodes.len(), 2);

    // A new file appears.
    let other = macro_to_flow(&Macro {
        id: "ext0000001".into(),
        name: "External".into(),
        steps: vec![],
    });
    std::fs::write(
        flows_root(&e.dir).join("ext0000001.json"),
        serde_json::to_string(&other).unwrap(),
    )
    .unwrap();
    e.st.handle_file_changes(&e.dir, vec![(FileKind::Flow, "ext0000001".into())]);
    assert_eq!(changes(&e)[0].change, FileChange::Changed);
    assert_eq!(e.st.get_flow("ext0000001").unwrap().flow.name, "External");

    // Removed.
    std::fs::remove_file(&path).unwrap();
    e.st.handle_file_changes(&e.dir, batch());
    assert_eq!(changes(&e)[0].change, FileChange::Removed);
    assert!(matches!(e.st.get_flow(&id), Err(AppError::NotFound(_))));
    // Removed again: nothing to say.
    e.st.handle_file_changes(&e.dir, batch());
    assert!(changes(&e).is_empty());
}

#[test]
fn an_external_edit_of_the_deployed_graph_keeps_the_draft() {
    let e = env();
    let id = e.st.create_flow(new_flow("F")).unwrap().flow.id;
    let path = flows_root(&e.dir).join(format!("{id}.json"));
    let batch = || vec![(FileKind::Flow, id.clone())];
    let (mut dn, de) = simple();
    dn.push(n("lg", log()));
    e.st.save_flow_draft(&id, dn.clone(), de.clone()).unwrap();
    e.events.take(names::PROJECT_FILES_CHANGED);

    // The deployed graph is edited outside (the file has no draft: not flushed yet, or removed).
    let mut v = file_json(&e, &id);
    v["nodes"][1]["config"]["layerId"] = json!("other");
    v.as_object_mut().unwrap().remove("draft");
    std::fs::write(&path, v.to_string()).unwrap();
    e.st.handle_file_changes(&e.dir, batch());
    assert_eq!(changes(&e)[0].change, FileChange::Changed);
    let f = e.st.get_flow(&id).unwrap().flow;
    assert_eq!(
        f.nodes[1].kind,
        take("scr", "other"),
        "the new deployed graph"
    );
    let d = f.draft.expect("the draft is kept");
    assert_eq!((d.nodes, d.edges), (dn.clone(), de.clone()));
    // ... and written back by the content flusher.
    assert!(e.st.flows().is_dirty(&id));
    e.st.flush_all();
    assert_eq!(
        file_json(&e, &id)["draft"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(file_json(&e, &id)["nodes"][1]["config"]["layerId"], "other");

    // A file that only lacks the draft announces nothing but writes it back.
    let mut v = file_json(&e, &id);
    v.as_object_mut().unwrap().remove("draft");
    std::fs::write(&path, v.to_string()).unwrap();
    e.st.handle_file_changes(&e.dir, batch());
    assert!(changes(&e).is_empty());
    assert!(e.st.get_flow(&id).unwrap().flow.draft.is_some());
    e.st.flush_all();
    assert!(file_json(&e, &id)["draft"].is_object());

    // A file with a draft of its own wins.
    let mut v = file_json(&e, &id);
    v["draft"]["nodes"].as_array_mut().unwrap().pop();
    std::fs::write(&path, v.to_string()).unwrap();
    e.st.handle_file_changes(&e.dir, batch());
    assert_eq!(changes(&e)[0].change, FileChange::Changed);
    assert_eq!(
        e.st.get_flow(&id).unwrap().flow.draft.unwrap().nodes.len(),
        2
    );

    // A deployed graph that equals the draft (deployed outside) drops it.
    e.st.save_flow_draft(&id, dn.clone(), de.clone()).unwrap();
    e.st.flush_all();
    let mut v = file_json(&e, &id);
    v["nodes"] = serde_json::to_value(&dn).unwrap();
    v.as_object_mut().unwrap().remove("draft");
    std::fs::write(&path, v.to_string()).unwrap();
    e.st.handle_file_changes(&e.dir, batch());
    assert_eq!(changes(&e)[0].change, FileChange::Changed);
    let f = e.st.get_flow(&id).unwrap().flow;
    assert!(f.draft.is_none());
    assert_eq!(f.nodes.len(), 3);
}

#[test]
fn the_watcher_classifies_flow_files() {
    let root = Path::new("C:/p");
    let c = |p: &str| crate::core::watch::classify(root, &root.join(p));
    assert_eq!(
        c("flows/abc0000001.json"),
        Some((FileKind::Flow, "abc0000001".into()))
    );
    for ignored in [
        "flows/abc.json.tmp-123",
        "flows/abc.json.corrupt-20260101T000000Z",
        "flows/sub/x.json",
        "flows",
    ] {
        assert_eq!(c(ignored), None, "{ignored}");
    }
}

#[test]
fn the_port_table_is_used_for_data_types() {
    // Guards the table's compatibility rule through validation (time → text ok, text → bool no).
    let nodes = vec![
        n("t", NodeKind::TimeNow {}),
        n("lg", log()),
        n(
            "k",
            NodeKind::Constant {
                value: Constant::Text("x".into()),
            },
        ),
        n("b", NodeKind::Branch {}),
    ];
    validate_graph(&nodes, &[dw("w", "t", "now", "lg", "text")]).unwrap();
    assert!(validate_graph(&nodes, &[dw("w", "k", "value", "b", "condition")]).is_err());
    assert_eq!(
        flow_nodes::output(&nodes[0].kind, "now").unwrap().ty,
        Some(DataType::Time)
    );
}

// ---- Repeat and Stop flow ----------------------------------------------------------------------

fn repeat(count: Option<u32>, delay_ms: u64) -> NodeKind {
    NodeKind::Repeat { count, delay_ms }
}

#[test]
fn repeat_and_stop_flow_configs_are_checked() {
    check_config(&repeat(Some(1), 0)).unwrap();
    check_config(&repeat(Some(FLOW_REPEAT_MAX), FLOW_DELAY_MAX_MS)).unwrap();
    check_config(&repeat(None, 500)).unwrap();
    assert!(check_config(&repeat(Some(0), 0)).is_err());
    assert!(check_config(&repeat(Some(FLOW_REPEAT_MAX + 1), 0)).is_err());
    assert!(check_config(&repeat(None, FLOW_DELAY_MAX_MS + 1)).is_err());
    check_config(&NodeKind::StopFlow {
        flow_id: "abcdefghij".into(),
    })
    .unwrap();
    assert_eq!(
        check_config(&NodeKind::StopFlow {
            flow_id: String::new()
        }),
        Err("pick a flow".into())
    );
    // The JSON shape of the contract (`count: null` = forever).
    let v = serde_json::to_value(repeat(None, 250)).unwrap();
    assert_eq!(
        v,
        json!({"type": "logic.repeat", "config": {"count": null, "delayMs": 250}})
    );
    let k: NodeKind =
        serde_json::from_value(json!({"type": "action.stopFlow", "config": {"flowId": "x"}}))
            .unwrap();
    assert_eq!(
        k,
        NodeKind::StopFlow {
            flow_id: "x".into()
        }
    );
}

#[test]
fn a_repeat_body_may_loop_to_stop_but_not_back_into_its_own_in() {
    let nodes = || {
        vec![
            n("c", cue()),
            n("r", repeat(None, 0)),
            n("a", log()),
            n("b", log()),
            n("d", delay(10)),
        ]
    };
    // body → a → b → stop: fine (a Repeat breaks event cycles like a wait).
    validate_graph(
        &nodes(),
        &[
            ev("w0", "c", "fired", "r", "in"),
            ev("w1", "r", "body", "a", "in"),
            ev("w2", "a", "done", "b", "in"),
            ev("w3", "b", "done", "r", "stop"),
            ev("w4", "r", "done", "d", "in"),
        ],
    )
    .unwrap();
    // body → a → b → in (even through a Delay): an error on the Repeat.
    for via_delay in [false, true] {
        let mut edges = vec![
            ev("w0", "c", "fired", "r", "in"),
            ev("w1", "r", "body", "a", "in"),
            ev("w2", "a", "done", "b", "in"),
        ];
        if via_delay {
            edges.push(ev("w3", "b", "done", "d", "in"));
            edges.push(ev("w4", "d", "out", "r", "in"));
        } else {
            edges.push(ev("w3", "b", "done", "r", "in"));
        }
        let m = err(validate_graph(&nodes(), &edges));
        assert_eq!(m, "Repeat already loops; wire to stop instead");
        let list = check_graph(&nodes(), &edges);
        assert_eq!(
            issues(&list),
            [(Some("r"), "Repeat already loops; wire to stop instead")]
        );
    }
    // body straight into its own in.
    let m = err(validate_graph(
        &[n("r", repeat(Some(2), 0))],
        &[ev("w", "r", "body", "r", "in")],
    ));
    assert_eq!(m, "Repeat already loops; wire to stop instead");
    // done → in is not the body: allowed (a new loop each time, paced and step-limited).
    validate_graph(
        &[n("c", cue()), n("r", repeat(Some(2), 0))],
        &[
            ev("w0", "c", "fired", "r", "in"),
            ev("w1", "r", "done", "r", "in"),
        ],
    )
    .unwrap();
    // A nested Repeat's body may go back to the inner Repeat's stop, and the outer's body path
    // through the inner one into the outer's in is still an error.
    let nested = vec![
        n("c", cue()),
        n("o", repeat(Some(2), 0)),
        n("i", repeat(Some(3), 0)),
        n("a", log()),
    ];
    validate_graph(
        &nested,
        &[
            ev("w0", "c", "fired", "o", "in"),
            ev("w1", "o", "body", "i", "in"),
            ev("w2", "i", "body", "a", "in"),
            ev("w3", "a", "done", "i", "stop"),
        ],
    )
    .unwrap();
    let list = check_graph(
        &nested,
        &[
            ev("w0", "c", "fired", "o", "in"),
            ev("w1", "o", "body", "i", "in"),
            ev("w2", "i", "done", "a", "in"),
            ev("w3", "a", "done", "o", "in"),
        ],
    );
    assert_eq!(
        issues(&list),
        [(Some("o"), "Repeat already loops; wire to stop instead")]
    );
}

#[test]
fn a_flow_cant_run_itself_but_may_stop_itself() {
    let e = env();
    let id = e.st.create_flow(new_flow("Loop")).unwrap().flow.id;
    let me = |kind: NodeKind| {
        (
            vec![n("c", cue()), n("r", kind)],
            vec![ev("w", "c", "fired", "r", "in")],
        )
    };
    let (nodes, edges) = me(NodeKind::RunFlow {
        flow_id: id.clone(),
        wait: false,
    });
    // Save and deploy refuse it; check_flow with the id names the node.
    let m = err(e
        .st
        .save_flow(SaveFlow {
            id: id.clone(),
            name: "Loop".into(),
            retrigger: Retrigger::Restart,
            nodes: nodes.clone(),
            edges: edges.clone(),
        })
        .map(drop));
    assert_eq!(m, "A flow can't run itself; use Repeat");
    e.st.save_flow_draft(&id, nodes.clone(), edges.clone())
        .unwrap();
    assert_eq!(
        err(e.st.deploy_flow(&id).map(drop)),
        "A flow can't run itself; use Repeat"
    );
    e.st.discard_flow_draft(&id).unwrap();
    let c =
        e.st.check_flow_of(Some(&id), nodes.clone(), edges.clone())
            .unwrap();
    assert_eq!(
        issues(&c.errors),
        [(Some("r"), "A flow can't run itself; use Repeat")]
    );
    // Without the id (another flow's graph), running that flow is fine.
    assert!(e.st.check_flow(nodes, edges).unwrap().errors.is_empty());
    // Stop flow of itself is valid; of a missing flow, a soft warning.
    let (nodes, edges) = me(NodeKind::StopFlow {
        flow_id: id.clone(),
    });
    let c =
        e.st.check_flow_of(Some(&id), nodes.clone(), edges.clone())
            .unwrap();
    assert!(c.errors.is_empty() && c.warnings.is_empty(), "{c:?}");
    e.st.save_flow(SaveFlow {
        id: id.clone(),
        name: "Loop".into(),
        retrigger: Retrigger::Restart,
        nodes,
        edges,
    })
    .unwrap();
    let (nodes, edges) = me(NodeKind::StopFlow {
        flow_id: "gone000000".into(),
    });
    let c = e.st.check_flow_of(Some(&id), nodes, edges).unwrap();
    assert!(c.errors.is_empty(), "{c:?}");
    assert_eq!(
        issues(&c.warnings),
        [(Some("r"), "The flow it stops no longer exists")]
    );
}
