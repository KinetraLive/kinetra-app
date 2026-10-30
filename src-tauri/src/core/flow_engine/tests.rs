//! The flow engine through `AppState`: node semantics, retrigger, limits, deploy, arming,
//! Automation off, PANIC, Run flow, the source-row trigger, and the budget (tasks at rest,
//! dispatch, compile time, memory). Flows go through the store (`create_flow`, validated), as in
//! the app; tasks run on a runtime injected into the hub.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::*;
use crate::core::events::EventSink;
use crate::core::live::FieldDef;
use crate::core::localtime::{parse_local, Zone};
use crate::core::model::{
    AutoAdvance, AutoTransition, CompareOp, Constant, FlowEdge, FlowNode, ItemState, MathOp,
    NewFlow, NewScreen, PortRef, Rect, SaveFlow, ScheduleItem, ScreenDoc, ScreenLayer,
    ScreenLayerKind, SourceBinding, SourceColumn, SourceDoc, TemplateDoc, TemplateSource, UiState,
    VariantTransition,
};
use crate::core::protocol::{FieldType, TimerDir, TimerEnd};

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
    rt: tokio::runtime::Runtime,
    st: AppState,
    rec: Arc<Recorder>,
    dir: PathBuf,
}

const Z: Zone = Zone::Fixed(0);

/// Epoch ms of `HH:MM[:SS]` on 2026-10-05 (UTC, a Monday).
fn t(hm: &str) -> i64 {
    Z.to_epoch(parse_local(&format!("2026-10-05T{hm}")).unwrap())
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
    let e = Env {
        _tmp: tmp,
        rt,
        st,
        rec,
        dir,
    };
    e.set_now("14:00", 0);
    e
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn fields(pairs: &[(&str, &str)]) -> Fields {
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

fn node(id: &str, kind: NodeKind) -> FlowNode {
    FlowNode {
        id: id.into(),
        position: Default::default(),
        kind,
    }
}

static EDGE: AtomicU64 = AtomicU64::new(0);

fn edge(from: &str, fp: &str, to: &str, tp: &str, kind: WireKind) -> FlowEdge {
    FlowEdge {
        id: format!("e{}", EDGE.fetch_add(1, AtomicOrdering::Relaxed)),
        from: PortRef {
            node: from.into(),
            port: fp.into(),
        },
        to: PortRef {
            node: to.into(),
            port: tp.into(),
        },
        kind,
    }
}

fn ev(from: &str, fp: &str, to: &str, tp: &str) -> FlowEdge {
    edge(from, fp, to, tp, WireKind::Event)
}

fn data(from: &str, fp: &str, to: &str, tp: &str) -> FlowEdge {
    edge(from, fp, to, tp, WireKind::Data)
}

fn log(id: &str) -> FlowNode {
    node(
        id,
        NodeKind::Log {
            message: id.into(),
            toast: false,
        },
    )
}

fn cue_node() -> FlowNode {
    node("cue", NodeKind::Cue { label: None })
}

impl Env {
    /// Schedule time is now `HH:MM` minus `before_ms`.
    fn set_now(&self, hm: &str, before_ms: i64) {
        let real = i64::try_from(fsx::now_epoch_ms()).unwrap();
        self.st
            .sources()
            .clock()
            .set_test(t(hm) - before_ms - real, Z);
        self.st.replan();
    }

    /// Fields `name`, `title` (text), `rows` (list: name) and `timer`.
    fn template(&self) -> TemplateDoc {
        let mut doc = self
            .st
            .templates()
            .create(&self.dir, "Board", &TemplateSource::Blank)
            .unwrap();
        let mut rows = field("rows", FieldType::List);
        rows.columns = vec![col("name")];
        doc.meta.fields = vec![
            field("name", FieldType::Text),
            field("title", FieldType::Text),
            rows,
            field("timer", FieldType::Timer),
        ];
        doc.variants[0].html = "<div><b>{{name}}</b><i>{{title}}</i><ol><li data-each=\"rows\">{{.name}}</li></ol></div>".into();
        self.st.templates().save(&self.dir, doc).unwrap()
    }

    /// A text source with rows `a`, `b`, `c` (columns `name`, `score`).
    fn source(&self) -> SourceDoc {
        let mut doc = self.st.create_source("Players").unwrap();
        doc.meta.kind = SourceKind::Text {
            columns: vec![col("name"), col("score")],
            rows: ["a", "b", "c"]
                .iter()
                .enumerate()
                .map(|(i, n)| fields(&[("name", n), ("score", &(i * 10).to_string())]))
                .collect(),
            index: 0,
            auto: None,
        };
        self.st.save_source(doc).unwrap()
    }

    /// A screen with layers `l1` (`name` bound to `src` if given) and `l2`.
    fn screen(&self, tpl: &str, src: Option<&str>) -> ScreenDoc {
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
        let mut b = BTreeMap::new();
        if let Some(s) = src {
            b.insert(
                "name".to_string(),
                SourceBinding {
                    source_id: s.into(),
                    column: "name".into(),
                    sort: None,
                    limit: None,
                    row: None,
                },
            );
        }
        for (id, bindings) in [("l1", b), ("l2", BTreeMap::new())] {
            doc.meta.layers.push(ScreenLayer {
                id: id.into(),
                name: id.into(),
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 1920,
                    h: 1080,
                },
                kind: ScreenLayerKind::Template {
                    template_id: tpl.into(),
                    fields: Fields::new(),
                    vars: Fields::new(),
                    bindings,
                    variant: None,
                    sizing: None,
                    opacity: None,
                },
                anchor: None,
            });
        }
        self.st.save_screen(doc).unwrap().doc
    }

    fn create(&self, name: &str, nodes: Vec<FlowNode>, edges: Vec<FlowEdge>) -> String {
        self.create_with(name, Retrigger::Restart, nodes, edges)
    }

    fn create_with(
        &self,
        name: &str,
        retrigger: Retrigger,
        nodes: Vec<FlowNode>,
        edges: Vec<FlowEdge>,
    ) -> String {
        self.st
            .create_flow(NewFlow {
                name: name.into(),
                armed: true,
                retrigger,
                origin: None,
                nodes,
                edges,
            })
            .unwrap()
            .flow
            .id
    }

    fn events(&self, flow: &str) -> Vec<Value> {
        self.rec
            .named(names::FLOW_STATE)
            .into_iter()
            .filter(|v| v["id"] == flow)
            .collect()
    }

    fn ends(&self, flow: &str) -> Vec<Value> {
        self.events(flow)
            .into_iter()
            .filter(|v| v["state"] == "finished" || v["state"] == "failed")
            .collect()
    }

    /// Runs a cue flow and waits for the instance to end.
    fn run(&self, flow: &str) -> Value {
        let n = self.ends(flow).len();
        self.st.run_flow(flow, None).unwrap();
        wait_until("the run's end", || self.ends(flow).len() > n);
        self.ends(flow).pop().unwrap()
    }

    /// The action nodes that ran (in order), from `running` events.
    fn ran(&self, flow: &str) -> Vec<String> {
        self.events(flow)
            .iter()
            .filter(|v| v["state"] == "running")
            .filter_map(|v| v["node"].as_str().map(str::to_string))
            .filter(|n| n != "cue" && n != "t")
            .collect()
    }

    fn field(&self, screen: &str, layer: &str, key: &str) -> String {
        self.st
            .live()
            .with_layer(screen, layer, |l| {
                l.fields.get(key).cloned().unwrap_or_default()
            })
            .unwrap()
    }

    fn on_air(&self, screen: &str, layer: &str) -> bool {
        self.st.live().is_on_air(screen, layer)
    }

    fn automation_off(&self, off: bool) {
        let mut ui: UiState = self.st.ui_state().unwrap();
        ui.control.automation_off = off;
        self.st.set_ui_state(ui).unwrap();
    }
}

fn set_title(screen: &str, layer: &str) -> NodeKind {
    NodeKind::SetFields {
        screen_id: screen.into(),
        layer_id: layer.into(),
        fields: fields(&[("title", "")]),
    }
}

// ---- Walk order --------------------------------------------------------------------------------

#[test]
fn fan_out_is_depth_first_in_edge_order() {
    let e = env();
    let id = e.create(
        "fan",
        vec![cue_node(), log("l1"), log("l2"), log("l3")],
        vec![
            ev("cue", "fired", "l1", "in"),
            ev("cue", "fired", "l2", "in"),
            ev("l1", "done", "l3", "in"),
        ],
    );
    let end = e.run(&id);
    assert_eq!(end["state"], "finished");
    assert_eq!(e.ran(&id), ["l1", "l3", "l2"]);
}

#[test]
fn sequence_runs_branches_in_order_and_moves_on_when_one_parks() {
    let e = env();
    let id = e.create(
        "seq",
        vec![
            cue_node(),
            node("s", NodeKind::Sequence { count: 3 }),
            node("d", NodeKind::Delay { ms: 120 }),
            log("a"),
            log("b"),
            log("c"),
            log("a2"),
        ],
        vec![
            ev("cue", "fired", "s", "in"),
            ev("s", "s0", "a", "in"),
            ev("a", "done", "d", "in"),
            ev("d", "out", "a2", "in"),
            ev("s", "s1", "b", "in"),
            ev("s", "s2", "c", "in"),
        ],
    );
    let started = Instant::now();
    let end = e.run(&id);
    assert_eq!(end["state"], "finished");
    assert!(started.elapsed() >= Duration::from_millis(120));
    assert_eq!(
        e.ran(&id),
        ["a", "b", "c", "a2"],
        "s0 parked at its delay, so s1 and s2 ran first"
    );
}

// ---- Logic -------------------------------------------------------------------------------------

#[test]
fn branch_compare_and_switch() {
    let e = env();
    let constant = |id: &str, v: Constant| node(id, NodeKind::Constant { value: v });
    let cmp = |id: &str, op: CompareOp, a: &str, b: &str| {
        node(
            id,
            NodeKind::Compare {
                op,
                a: a.into(),
                b: b.into(),
            },
        )
    };
    // 10 > 9 as numbers (as text "10" < "9"); "b" > "a"; contains; empty.
    for (op, a, b, expect) in [
        (CompareOp::Gt, "10", "9", "yes"),
        (CompareOp::Lt, "10", "9", "no"),
        (CompareOp::Gt, "b", "a", "yes"),
        (CompareOp::Contains, "hello", "ell", "yes"),
        (CompareOp::Empty, " ", "", "yes"),
        (CompareOp::Eq, "1.0", "1", "yes"),
        (CompareOp::Ne, "x", "x", "no"),
    ] {
        let id = e.create(
            "branch",
            vec![
                cue_node(),
                cmp("c", op, a, b),
                node("br", NodeKind::Branch {}),
                log("yes"),
                log("no"),
            ],
            vec![
                ev("cue", "fired", "br", "in"),
                data("c", "bool", "br", "condition"),
                ev("br", "then", "yes", "in"),
                ev("br", "else", "no", "in"),
            ],
        );
        e.run(&id);
        assert_eq!(e.ran(&id), [expect], "{op:?} {a} {b}");
    }
    // Switch on a constant: case equality, else default.
    for (v, expect) in [("b", "cb"), ("z", "dflt")] {
        let id = e.create(
            "switch",
            vec![
                cue_node(),
                constant("k", Constant::Text(v.into())),
                node(
                    "sw",
                    NodeKind::Switch {
                        cases: vec!["a".into(), "b".into()],
                    },
                ),
                log("ca"),
                log("cb"),
                log("dflt"),
            ],
            vec![
                ev("cue", "fired", "sw", "in"),
                data("k", "value", "sw", "value"),
                ev("sw", "case0", "ca", "in"),
                ev("sw", "case1", "cb", "in"),
                ev("sw", "default", "dflt", "in"),
            ],
        );
        e.run(&id);
        assert_eq!(e.ran(&id), [expect]);
    }
}

#[test]
fn gate_once_counter_keep_state_until_a_deploy() {
    let e = env();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    // Gate: starts closed; `in` before `open` in the first run, so only the second run passes.
    let gate = e.create(
        "gate",
        vec![
            cue_node(),
            node("q", NodeKind::Sequence { count: 2 }),
            node("g", NodeKind::Gate { open: false }),
            log("pass"),
        ],
        vec![
            ev("cue", "fired", "q", "in"),
            ev("q", "s0", "g", "in"),
            ev("q", "s1", "g", "open"),
            ev("g", "out", "pass", "in"),
        ],
    );
    e.run(&gate);
    assert!(e.ran(&gate).is_empty());
    e.run(&gate);
    assert_eq!(e.ran(&gate), ["pass"]);
    // Once, until a deploy of another graph.
    let once_nodes = vec![cue_node(), node("o", NodeKind::Once {}), log("first")];
    let once_edges = vec![ev("cue", "fired", "o", "in"), ev("o", "out", "first", "in")];
    let once = e.create("once", once_nodes.clone(), once_edges.clone());
    for _ in 0..3 {
        e.run(&once);
    }
    assert_eq!(e.ran(&once), ["first"]);
    // The same graph again: a no-op (state kept).
    e.st.save_flow(SaveFlow {
        id: once.clone(),
        name: "once".into(),
        retrigger: Retrigger::Restart,
        nodes: once_nodes.clone(),
        edges: once_edges.clone(),
    })
    .unwrap();
    e.run(&once);
    assert_eq!(e.ran(&once).len(), 1);
    // Another graph: state resets.
    let mut nodes = once_nodes;
    nodes.push(log("spare"));
    e.st.save_flow(SaveFlow {
        id: once.clone(),
        name: "once".into(),
        retrigger: Retrigger::Restart,
        nodes,
        edges: once_edges,
    })
    .unwrap();
    e.run(&once);
    assert_eq!(e.ran(&once).len(), 2);
    // Counter: every 2nd event, count as data.
    let counter = e.create(
        "counter",
        vec![
            cue_node(),
            node("n", NodeKind::Counter { every: 2 }),
            node("set", set_title(&s.id, "l2")),
        ],
        vec![
            ev("cue", "fired", "n", "in"),
            ev("n", "out", "set", "in"),
            data("n", "count", "set", "title"),
        ],
    );
    for _ in 0..5 {
        e.run(&counter);
    }
    assert_eq!(e.ran(&counter).len(), 2);
    assert_eq!(e.field(&s.id, "l2", "title"), "4");
}

#[test]
fn random_never_repeats_and_throttle_drops() {
    let e = env();
    let rnd = e.create(
        "random",
        vec![
            cue_node(),
            node(
                "r",
                NodeKind::Random {
                    count: 2,
                    no_repeat: true,
                },
            ),
            log("x"),
            log("y"),
        ],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "r0", "x", "in"),
            ev("r", "r1", "y", "in"),
        ],
    );
    for _ in 0..6 {
        e.run(&rnd);
    }
    let picks = e.ran(&rnd);
    assert_eq!(picks.len(), 6);
    for w in picks.windows(2) {
        assert_ne!(w[0], w[1], "{picks:?}");
    }
    let thr = e.create(
        "throttle",
        vec![
            cue_node(),
            node("t", NodeKind::Throttle { ms: 60_000 }),
            log("x"),
        ],
        vec![ev("cue", "fired", "t", "in"), ev("t", "out", "x", "in")],
    );
    for _ in 0..3 {
        e.run(&thr);
    }
    assert_eq!(e.ran(&thr), ["x"]);
}

#[test]
fn debounce_fires_once_after_a_burst() {
    let e = env();
    let id = e.create(
        "debounce",
        vec![
            cue_node(),
            // Long against the 30 ms gaps, so a stalled test thread cannot split the burst.
            node("d", NodeKind::Debounce { ms: 600 }),
            log("x"),
        ],
        vec![ev("cue", "fired", "d", "in"), ev("d", "out", "x", "in")],
    );
    for _ in 0..3 {
        e.st.run_flow(&id, None).unwrap();
        std::thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(e.st.flow_engine().task_count(), 1, "one parked instance");
    wait_until("the debounced log", || !e.ends(&id).is_empty());
    assert_eq!(e.ran(&id), ["x"]);
    assert_eq!(e.st.flow_engine().task_count(), 0);
}

#[test]
fn delay_cancel_and_wait_until() {
    let e = env();
    // Delay cancelled by its `cancel` input: the branch never resumes, the instance ends.
    let cancel = e.create(
        "cancel",
        vec![
            cue_node(),
            node("q", NodeKind::Sequence { count: 2 }),
            node("d", NodeKind::Delay { ms: 3000 }),
            log("late"),
        ],
        vec![
            ev("cue", "fired", "q", "in"),
            ev("q", "s0", "d", "in"),
            ev("q", "s1", "d", "cancel"),
            ev("d", "out", "late", "in"),
        ],
    );
    let started = Instant::now();
    assert_eq!(e.run(&cancel)["state"], "finished");
    assert!(started.elapsed() < Duration::from_millis(3000));
    assert!(e.ran(&cancel).is_empty());
    // Wait until `release` (from a delayed branch of the same instance).
    let release = e.create(
        "release",
        vec![
            cue_node(),
            node("q", NodeKind::Sequence { count: 2 }),
            node(
                "w",
                NodeKind::WaitUntil {
                    at: None,
                    timeout_ms: None,
                },
            ),
            node("d", NodeKind::Delay { ms: 80 }),
            log("after"),
        ],
        vec![
            ev("cue", "fired", "q", "in"),
            ev("q", "s0", "w", "in"),
            ev("q", "s1", "d", "in"),
            ev("d", "out", "w", "release"),
            ev("w", "out", "after", "in"),
        ],
    );
    let started = Instant::now();
    e.run(&release);
    assert!(started.elapsed() >= Duration::from_millis(80));
    assert_eq!(e.ran(&release), ["after"]);
    // Timeout; and a time of day (the test clock: 14:00 is 300 ms away).
    let timeout = e.create(
        "timeout",
        vec![
            cue_node(),
            node(
                "w",
                NodeKind::WaitUntil {
                    at: None,
                    timeout_ms: Some(60),
                },
            ),
            log("out"),
            log("late"),
        ],
        vec![
            ev("cue", "fired", "w", "in"),
            ev("w", "out", "out", "in"),
            ev("w", "timeout", "late", "in"),
        ],
    );
    e.run(&timeout);
    assert_eq!(e.ran(&timeout), ["late"]);
    let at = e.create(
        "at",
        vec![
            cue_node(),
            node(
                "w",
                NodeKind::WaitUntil {
                    at: Some("14:00".into()),
                    timeout_ms: Some(5_000),
                },
            ),
            log("out"),
            log("late"),
        ],
        vec![
            ev("cue", "fired", "w", "in"),
            ev("w", "out", "out", "in"),
            ev("w", "timeout", "late", "in"),
        ],
    );
    e.set_now("14:00", 300);
    let started = Instant::now();
    e.run(&at);
    assert_eq!(e.ran(&at), ["out"]);
    assert!(started.elapsed() >= Duration::from_millis(100));
    // A wait for `release` with nothing left to release it ends the instance.
    let stuck = e.create(
        "stuck",
        vec![
            cue_node(),
            node(
                "w",
                NodeKind::WaitUntil {
                    at: None,
                    timeout_ms: None,
                },
            ),
            log("never"),
        ],
        vec![ev("cue", "fired", "w", "in"), ev("w", "out", "never", "in")],
    );
    assert_eq!(e.run(&stuck)["state"], "finished");
    assert_eq!(e.st.flow_engine().task_count(), 0);
}

// ---- Data --------------------------------------------------------------------------------------

#[test]
fn data_nodes_are_pulled_when_an_action_runs() {
    let e = env();
    let tpl = e.template();
    let src = e.source();
    let s = e.screen(&tpl.id, Some(&src.id));
    let set = |key: &str| {
        node(
            "set",
            NodeKind::SetFields {
                screen_id: s.id.clone(),
                layer_id: "l2".into(),
                fields: fields(&[(key, "")]),
            },
        )
    };
    let run_with = |name: &str, extra: Vec<FlowNode>, wires: Vec<FlowEdge>| {
        let mut nodes = vec![cue_node(), set("title")];
        nodes.extend(extra);
        let mut edges = vec![ev("cue", "fired", "set", "in")];
        edges.extend(wires);
        let id = e.create(name, nodes, edges);
        let end = e.run(&id);
        assert_eq!(end["state"], "finished", "{end}");
        e.field(&s.id, "l2", "title")
    };
    // Source row: current, last, by index input; a column and the index.
    let src_row = |pick: RowPick| {
        node(
            "r",
            NodeKind::SourceRow {
                source_id: src.id.clone(),
                row: pick,
            },
        )
    };
    e.st.step_source(&src.id, SourceStep::Index { index: 1 })
        .unwrap();
    assert_eq!(
        run_with(
            "cur",
            vec![src_row(RowPick::Current)],
            vec![data("r", "name", "set", "title")]
        ),
        "b"
    );
    assert_eq!(
        run_with(
            "last",
            vec![src_row(RowPick::Last)],
            vec![data("r", "score", "set", "title")]
        ),
        "20"
    );
    // A wired row number counts from 1: row 3 is "c".
    assert_eq!(
        run_with(
            "idx",
            vec![
                src_row(RowPick::First),
                node(
                    "k",
                    NodeKind::Constant {
                        value: Constant::Number(3.0)
                    }
                )
            ],
            vec![
                data("k", "value", "r", "index"),
                data("r", "name", "set", "title")
            ]
        ),
        "c"
    );
    // Math (number → text), format with a time, field value, layer state.
    assert_eq!(
        run_with(
            "math",
            vec![node(
                "m",
                NodeKind::Math {
                    op: MathOp::Mul,
                    a: 2.5,
                    b: 4.0
                }
            )],
            vec![data("m", "number", "set", "title")]
        ),
        "10"
    );
    e.set_now("14:07", 0);
    assert_eq!(
        run_with(
            "fmt",
            vec![
                node("now", NodeKind::TimeNow {}),
                node(
                    "f",
                    NodeKind::Format {
                        pattern: "At {a:HH:mm} ({a}) {b} {z}".into()
                    }
                ),
                node(
                    "k",
                    NodeKind::Constant {
                        value: Constant::Bool(true)
                    }
                )
            ],
            vec![
                data("now", "now", "f", "a"),
                data("k", "value", "f", "b"),
                data("f", "text", "set", "title")
            ]
        ),
        "At 14:07 (14:07) true {z}"
    );
    e.st.live().show(&s.id, "l1").unwrap();
    assert_eq!(
        run_with(
            "fv",
            vec![node(
                "v",
                NodeKind::FieldValue {
                    screen_id: s.id.clone(),
                    layer_id: "l1".into(),
                    key: "name".into()
                }
            )],
            vec![data("v", "value", "set", "title")]
        ),
        "b",
        "the bound field's live value"
    );
    assert_eq!(
        run_with(
            "ls",
            vec![node(
                "ls",
                NodeKind::LayerState {
                    screen_id: s.id.clone(),
                    layer_id: "l1".into()
                }
            )],
            vec![data("ls", "onAir", "set", "title")]
        ),
        "true"
    );
    // Rows into a list field.
    let id = e.create(
        "rows",
        vec![cue_node(), set("rows"), src_row(RowPick::Current)],
        vec![
            ev("cue", "fired", "set", "in"),
            data("r", "rows", "set", "rows"),
        ],
    );
    e.run(&id);
    let rows: Value = serde_json::from_str(&e.field(&s.id, "l2", "rows")).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 3);
    assert_eq!(rows[2]["name"], "c");
}

#[test]
fn schedule_item_data() {
    let e = env();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    let mut doc = e.st.create_schedule("Run of show").unwrap();
    let item = |id: &str, a: &str, b: &str| ScheduleItem {
        id: id.into(),
        title: format!("Item {id}"),
        subtitle: String::new(),
        location: "Hall".into(),
        start: format!("2026-10-05T{a}"),
        end: format!("2026-10-05T{b}"),
        delay_min: 0,
        state: ItemState::Auto,
        started_at: None,
        ended_at: None,
    };
    if let SourceKind::Schedule { items, .. } = &mut doc.meta.kind {
        *items = vec![item("a", "13:30", "14:30"), item("b", "14:30", "15:00")];
    }
    let src = e.st.save_source(doc).unwrap();
    let id = e.create(
        "sched",
        vec![
            cue_node(),
            node(
                "n",
                NodeKind::ScheduleItem {
                    source_id: src.id.clone(),
                    which: NowNext::Next,
                },
            ),
            node(
                "f",
                NodeKind::Format {
                    pattern: "{a} {b} {c}".into(),
                },
            ),
            node("set", set_title(&s.id, "l2")),
        ],
        vec![
            ev("cue", "fired", "set", "in"),
            data("n", "title", "f", "a"),
            data("n", "start", "f", "b"),
            data("n", "exists", "f", "c"),
            data("f", "text", "set", "title"),
        ],
    );
    e.run(&id);
    assert_eq!(e.field(&s.id, "l2", "title"), "Item b 14:30 true");
}

// ---- Retrigger, limits -------------------------------------------------------------------------

#[test]
fn retrigger_ignore_drops_firings_while_running() {
    let e = env();
    let id = e.create_with(
        "ignore",
        Retrigger::Ignore,
        // Long enough that the second firing surely comes while the first still runs.
        vec![cue_node(), node("d", NodeKind::Delay { ms: 600 }), log("x")],
        vec![ev("cue", "fired", "d", "in"), ev("d", "out", "x", "in")],
    );
    e.st.run_flow(&id, None).unwrap();
    e.st.run_flow(&id, None).unwrap();
    wait_until("ended", || !e.ends(&id).is_empty());
    std::thread::sleep(Duration::from_millis(50));
    let states: Vec<String> = e
        .events(&id)
        .iter()
        .map(|v| v["state"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        states,
        ["running", "running", "finished"],
        "start, the log, end"
    );
}

#[test]
fn too_many_steps_stop_the_instance() {
    let e = env();
    // An event loop through a Delay (allowed) runs forever without the step limit.
    let id = e.create(
        "loop",
        vec![cue_node(), node("d", NodeKind::Delay { ms: 0 }), log("x")],
        vec![
            ev("cue", "fired", "x", "in"),
            ev("x", "done", "d", "in"),
            ev("d", "out", "x", "in"),
        ],
    );
    let end = e.run(&id);
    assert_eq!(end["state"], "failed");
    assert!(end["message"].as_str().unwrap().contains("Too many steps"));
    assert_eq!(e.ran(&id).len(), (FLOW_STEPS_MAX as usize).div_ceil(2));
    assert_eq!(e.st.flow_engine().task_count(), 0);
}

#[test]
fn firings_per_second_are_limited_and_reported_once() {
    let e = env();
    let id = e.create(
        "burst",
        vec![cue_node(), log("x")],
        vec![ev("cue", "fired", "x", "in")],
    );
    for _ in 0..15 {
        e.st.run_flow(&id, None).unwrap();
    }
    std::thread::sleep(Duration::from_millis(200));
    let ev = e.events(&id);
    let starts = ev
        .iter()
        .filter(|v| v["state"] == "running" && v["node"] == "cue")
        .count();
    assert_eq!(starts, 10, "the first ten fire (each restarting the last)");
    let failed: Vec<&Value> = ev.iter().filter(|v| v["state"] == "failed").collect();
    assert_eq!(failed.len(), 1, "reported once per window");
    assert_eq!(failed[0]["node"], "cue", "on the trigger");
    assert!(failed[0]["message"]
        .as_str()
        .unwrap()
        .contains("more than 10"));
    // A second later the flow fires again.
    std::thread::sleep(Duration::from_millis(1000));
    e.run(&id);
}

#[test]
fn running_instances_are_capped() {
    let e = env();
    let mut ids = Vec::new();
    for i in 0..=FLOW_INSTANCES_MAX {
        ids.push(e.create(
            &format!("wait {i}"),
            vec![cue_node(), node("d", NodeKind::Delay { ms: 60_000 })],
            vec![ev("cue", "fired", "d", "in")],
        ));
    }
    for id in &ids {
        e.st.run_flow(id, None).unwrap();
    }
    assert_eq!(e.st.flow_engine().task_count(), FLOW_INSTANCES_MAX);
    let last = e.events(ids.last().unwrap()).pop().unwrap();
    assert_eq!(last["state"], "failed");
    assert!(last["message"].as_str().unwrap().contains("Too many flows"));
    e.st.live_panic().unwrap();
    assert_eq!(e.st.flow_engine().task_count(), 0);
}

// ---- Deploy, arm, Automation off, PANIC --------------------------------------------------------

#[test]
fn deploy_stops_a_running_instance_unless_the_graph_is_the_same() {
    let e = env();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    let nodes = vec![
        cue_node(),
        node("d", NodeKind::Delay { ms: 1500 }),
        node(
            "take",
            NodeKind::Take {
                screen_id: s.id.clone(),
                layer_id: "l1".into(),
                for_ms: None,
            },
        ),
    ];
    let edges = vec![ev("cue", "fired", "d", "in"), ev("d", "out", "take", "in")];
    let id = e.create("deploy", nodes.clone(), edges.clone());
    let save = |nodes: Vec<FlowNode>, name: &str| {
        e.st.save_flow(SaveFlow {
            id: id.clone(),
            name: name.into(),
            retrigger: Retrigger::Restart,
            nodes,
            edges: edges.clone(),
        })
        .unwrap();
    };
    // The same graph (a rename, moved nodes): the instance keeps running.
    e.st.run_flow(&id, None).unwrap();
    let mut moved = nodes.clone();
    moved[1].position.x = 500.0;
    save(moved, "renamed");
    assert!(e.st.flow_engine().is_running(&id));
    wait_until("the take", || e.on_air(&s.id, "l1"));
    e.st.live().hide(&s.id, "l1").unwrap();
    // Another graph: stopped, the take never happens.
    e.st.run_flow(&id, None).unwrap();
    let mut other = nodes;
    other.push(log("spare"));
    save(other, "renamed");
    assert!(!e.st.flow_engine().is_running(&id));
    let last = e.events(&id).pop().unwrap();
    assert_eq!(
        (&last["state"], &last["message"]),
        (&Value::from("stopped"), &Value::from("Stopped by deploy"))
    );
    std::thread::sleep(Duration::from_millis(1700));
    assert!(!e.on_air(&s.id, "l1"));
}

#[test]
fn disarming_stops_and_refuses_cues_and_deleting_stops() {
    let e = env();
    let id = e.create(
        "arm",
        vec![cue_node(), node("d", NodeKind::Delay { ms: 60_000 })],
        vec![ev("cue", "fired", "d", "in")],
    );
    e.st.run_flow(&id, None).unwrap();
    e.st.set_flow_armed(&id, false).unwrap();
    assert!(!e.st.flow_engine().is_running(&id));
    assert_eq!(e.events(&id).pop().unwrap()["message"], "Stopped: disarmed");
    assert!(matches!(
        e.st.run_flow(&id, None),
        Err(AppError::Invalid(_))
    ));
    e.st.set_flow_armed(&id, true).unwrap();
    e.st.run_flow(&id, None).unwrap();
    e.st.delete_flow(&id).unwrap();
    assert_eq!(e.st.flow_engine().task_count(), 0);
    assert_eq!(
        e.events(&id).pop().unwrap()["message"],
        "Stopped: flow deleted"
    );
    // No Cue trigger.
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    let layer = e.create(
        "layer",
        vec![
            node(
                "t",
                NodeKind::LayerAir {
                    screen_id: s.id.clone(),
                    layer_id: "l1".into(),
                    on_air: true,
                },
            ),
            log("x"),
        ],
        vec![ev("t", "fired", "x", "in")],
    );
    assert!(matches!(
        e.st.run_flow(&layer, None),
        Err(AppError::Invalid(_))
    ));
}

#[test]
fn automation_off_blocks_automatic_triggers_but_not_cues() {
    let e = env();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    let auto = e.create(
        "auto",
        vec![
            node(
                "t",
                NodeKind::LayerAir {
                    screen_id: s.id.clone(),
                    layer_id: "l1".into(),
                    on_air: true,
                },
            ),
            node("d", NodeKind::Delay { ms: 60_000 }),
            log("x"),
        ],
        vec![ev("t", "fired", "d", "in"), ev("d", "out", "x", "in")],
    );
    let clear = e.create(
        "clear",
        vec![
            cue_node(),
            node(
                "take",
                NodeKind::Take {
                    screen_id: s.id.clone(),
                    layer_id: "l2".into(),
                    for_ms: Some(60_000),
                },
            ),
        ],
        vec![ev("cue", "fired", "take", "in")],
    );
    // A running automatic instance and a pending timed clear.
    e.st.live().show(&s.id, "l1").unwrap();
    wait_until("auto running", || e.st.flow_engine().is_running(&auto));
    e.run(&clear);
    assert_eq!(e.st.rules().task_count(), 1, "the timed clear");
    e.automation_off(true);
    assert!(e.st.flow_engine().automation_off());
    assert!(!e.st.flow_engine().is_running(&auto));
    assert_eq!(
        e.events(&auto).pop().unwrap()["message"],
        "Stopped: automation off"
    );
    assert_eq!(e.st.rules().task_count(), 0, "the timed clear is cancelled");
    assert!(e.on_air(&s.id, "l2"), "nothing changes on air");
    // Automatic triggers are blocked; a cue still runs.
    e.st.live().hide(&s.id, "l1").unwrap();
    e.st.live().show(&s.id, "l1").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(!e.st.flow_engine().is_running(&auto));
    let n = e.ends(&clear).len();
    e.run(&clear);
    assert_eq!(e.ends(&clear).len(), n + 1);
    // On again: nothing catches up; the next change fires.
    e.automation_off(false);
    std::thread::sleep(Duration::from_millis(100));
    assert!(!e.st.flow_engine().is_running(&auto));
    e.st.live().hide(&s.id, "l1").unwrap();
    e.st.live().show(&s.id, "l1").unwrap();
    wait_until("auto running", || e.st.flow_engine().is_running(&auto));
    // Persisted with the project and loaded on open.
    e.automation_off(true);
    e.st.close_project();
    e.st.open_project(&e.dir).unwrap();
    assert!(e.st.flow_engine().automation_off());
}

#[test]
fn automation_off_holds_auto_advance_and_countdown_ends() {
    let e = env();
    e.st.automation()
        .set_test_timing(Some(Duration::from_millis(60)), Duration::from_millis(30));
    let tpl = e.template();
    let src = e.source();
    let s = e.screen(&tpl.id, Some(&src.id));
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
    e.st.live().show(&s.id, "l1").unwrap();
    wait_until("a step", || {
        sources::position(&e.st.get_source(&src.id).unwrap()).1 >= 1
    });
    e.automation_off(true);
    assert!(e.st.automation().armed().is_empty(), "held");
    let at = sources::position(&e.st.get_source(&src.id).unwrap()).1;
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(sources::position(&e.st.get_source(&src.id).unwrap()).1, at);
    // A countdown with end: hide stays on air at zero.
    let set = |a| e.st.set_timer(&s.id, "l2", "timer", a).unwrap();
    e.st.live().show(&s.id, "l2").unwrap();
    set(TimerAction::Configure {
        dir: TimerDir::Down,
        end: TimerEnd::Hide,
    });
    set(TimerAction::Set { ms: 300 });
    set(TimerAction::Start);
    assert_eq!(e.st.live().pending_countdowns(), 0);
    std::thread::sleep(Duration::from_millis(1600));
    assert!(e.on_air(&s.id, "l2"), "stays on air at zero");
    // On again: the rotation resumes from where it was; the countdown (past zero) holds.
    e.automation_off(false);
    assert_eq!(e.st.automation().armed(), std::slice::from_ref(&src.id));
    wait_until("resumed", || {
        sources::position(&e.st.get_source(&src.id).unwrap()).1 != at
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(e.on_air(&s.id, "l2"));
    assert_eq!(e.st.live().pending_countdowns(), 0);
    // A countdown still ahead is scheduled again when automation comes back.
    e.automation_off(true);
    set(TimerAction::Set { ms: 5_000 });
    set(TimerAction::Start);
    assert_eq!(e.st.live().pending_countdowns(), 0);
    e.automation_off(false);
    assert_eq!(e.st.live().pending_countdowns(), 1);
}

#[test]
fn panic_stops_automatic_instances_and_fires_no_off_air_trigger() {
    let e = env();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    let on = e.create(
        "on",
        vec![
            node(
                "t",
                NodeKind::LayerAir {
                    screen_id: s.id.clone(),
                    layer_id: "l1".into(),
                    on_air: true,
                },
            ),
            node("d", NodeKind::Delay { ms: 60_000 }),
        ],
        vec![ev("t", "fired", "d", "in")],
    );
    let off = e.create(
        "off",
        vec![
            node(
                "t",
                NodeKind::LayerAir {
                    screen_id: s.id.clone(),
                    layer_id: "l1".into(),
                    on_air: false,
                },
            ),
            log("x"),
        ],
        vec![ev("t", "fired", "x", "in")],
    );
    e.st.live().show(&s.id, "l1").unwrap();
    wait_until("running", || e.st.flow_engine().is_running(&on));
    e.st.live_panic().unwrap();
    assert_eq!(e.st.flow_engine().task_count(), 0);
    assert_eq!(e.events(&on).pop().unwrap()["message"], "PANIC");
    std::thread::sleep(Duration::from_millis(100));
    assert!(e.events(&off).is_empty());
}

// ---- Run flow ----------------------------------------------------------------------------------

#[test]
fn run_flow_passes_a_value_waits_and_is_depth_limited() {
    let e = env();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    // B: run by flow → delay → set title = value.
    let b = e.create(
        "B",
        vec![
            node("t", NodeKind::RunByFlow {}),
            node("d", NodeKind::Delay { ms: 120 }),
            node("set", set_title(&s.id, "l2")),
        ],
        vec![
            ev("t", "fired", "d", "in"),
            ev("d", "out", "set", "in"),
            data("t", "value", "set", "title"),
        ],
    );
    // A: cue → run B (wait) with "hello" → log.
    let a = e.create(
        "A",
        vec![
            cue_node(),
            node(
                "k",
                NodeKind::Constant {
                    value: Constant::Text("hello".into()),
                },
            ),
            node(
                "run",
                NodeKind::RunFlow {
                    flow_id: b.clone(),
                    wait: true,
                },
            ),
            log("after"),
        ],
        vec![
            ev("cue", "fired", "run", "in"),
            data("k", "value", "run", "value"),
            ev("run", "done", "after", "in"),
        ],
    );
    let end = e.run(&a);
    assert_eq!(end["state"], "finished");
    assert_eq!(
        e.field(&s.id, "l2", "title"),
        "hello",
        "B ran with the value"
    );
    assert_eq!(e.ends(&b).len(), 1, "B ended before A went on");
    assert_eq!(e.ran(&a), ["run", "after"]);
    // A chain started from a cue runs while Automation is off.
    e.automation_off(true);
    e.run(&a);
    assert_eq!(e.ends(&b).len(), 2);
    e.automation_off(false);
    // Depth: F0 (cue) → F1 → … each runs the next; F4 (depth 4) can't run F5.
    let mut ids: Vec<String> = Vec::new();
    for i in (0..6).rev() {
        let next = ids.last().cloned();
        let trigger = if i == 0 {
            cue_node()
        } else {
            node("t", NodeKind::RunByFlow {})
        };
        let mut nodes = vec![trigger, log("x")];
        let first = if i == 0 { "cue" } else { "t" };
        let mut edges = vec![ev(first, "fired", "x", "in")];
        if let Some(n) = next {
            nodes.push(node(
                "run",
                NodeKind::RunFlow {
                    flow_id: n,
                    wait: false,
                },
            ));
            edges.push(ev("x", "done", "run", "in"));
        }
        ids.push(e.create(&format!("F{i}"), nodes, edges));
    }
    ids.reverse();
    e.st.run_flow(&ids[0], None).unwrap();
    wait_until("F4 ended", || !e.ends(&ids[4]).is_empty());
    std::thread::sleep(Duration::from_millis(100));
    let f4 = e.ends(&ids[4]).pop().unwrap();
    assert_eq!(f4["state"], "failed");
    assert!(f4["message"].as_str().unwrap().contains("nested too deep"));
    assert!(e.events(&ids[5]).is_empty(), "F5 never ran");
}

// ---- Source row trigger ------------------------------------------------------------------------

#[test]
fn source_row_trigger_fires_from_every_path() {
    let e = env();
    e.st.automation()
        .set_test_timing(Some(Duration::from_millis(80)), Duration::from_millis(30));
    let tpl = e.template();
    let src = e.source();
    let s = e.screen(&tpl.id, Some(&src.id));
    let id = e.create(
        "row",
        vec![
            node(
                "t",
                NodeKind::SourceRowChanged {
                    source_id: src.id.clone(),
                },
            ),
            node("set", set_title(&s.id, "l2")),
        ],
        vec![
            ev("t", "fired", "set", "in"),
            data("t", "index", "set", "title"),
        ],
    );
    let title = || e.field(&s.id, "l2", "title");
    // Next / Prev.
    e.st.step_source(&src.id, SourceStep::By { delta: 1 })
        .unwrap();
    wait_until("step", || title() == "2");
    // No change, no firing (the trigger's index is a row number, from 1).
    e.st.step_source(&src.id, SourceStep::Index { index: 1 })
        .unwrap();
    // A save that moves the row.
    let mut doc = e.st.get_source(&src.id).unwrap();
    if let SourceKind::Text { index, .. } = &mut doc.meta.kind {
        *index = 2;
    }
    e.st.save_source(doc).unwrap();
    wait_until("save", || title() == "3");
    // A flow's step.
    let stepper = e.create(
        "stepper",
        vec![
            cue_node(),
            node(
                "s",
                NodeKind::StepSource {
                    source_id: src.id.clone(),
                    step: SourceStep::Index { index: 0 },
                },
            ),
        ],
        vec![ev("cue", "fired", "s", "in")],
    );
    e.run(&stepper);
    wait_until("flow step", || title() == "1");
    // The watcher's reload of an edited file.
    let mut doc = e.st.get_source(&src.id).unwrap();
    if let SourceKind::Text { index, .. } = &mut doc.meta.kind {
        *index = 1;
    }
    crate::core::sources::save(&e.dir, doc, false).unwrap();
    e.st.reload_source(&e.dir, &src.id);
    wait_until("reload", || title() == "2");
    // Auto-advance.
    e.st.set_auto_advance(
        &src.id,
        Some(AutoAdvance {
            every_ms: 1000,
            looped: false,
            transition: AutoTransition::Update,
            shuffle: false,
            paused: false,
            item_durations: false,
        }),
    )
    .unwrap();
    e.st.live().show(&s.id, "l1").unwrap();
    wait_until("auto-advance", || title() == "3");
    // The field is set before the run reports its end.
    wait_until("five firings", || e.ends(&id).len() >= 5);
    assert_eq!(e.ends(&id).len(), 5, "one firing per change");
}

// ---- Budget ------------------------------------------------------------------------------------

#[test]
fn no_task_at_rest_with_cue_and_event_flows() {
    let e = env();
    let tpl = e.template();
    let src = e.source();
    let s = e.screen(&tpl.id, Some(&src.id));
    e.create(
        "cue",
        vec![cue_node(), log("x")],
        vec![ev("cue", "fired", "x", "in")],
    );
    e.create(
        "layer",
        vec![
            node(
                "t",
                NodeKind::LayerAir {
                    screen_id: s.id.clone(),
                    layer_id: "l1".into(),
                    on_air: true,
                },
            ),
            log("x"),
        ],
        vec![ev("t", "fired", "x", "in")],
    );
    e.create(
        "row",
        vec![
            node(
                "t",
                NodeKind::SourceRowChanged {
                    source_id: src.id.clone(),
                },
            ),
            log("x"),
        ],
        vec![ev("t", "fired", "x", "in")],
    );
    e.st.close_project();
    e.st.open_project(&e.dir).unwrap();
    assert_eq!(e.st.list_flows().unwrap().len(), 3);
    assert_eq!(e.st.flow_engine().task_count(), 0);
    assert_eq!(e.st.rules().task_count(), 0, "no planner: nothing timed");
    assert_eq!(e.st.rules().next_wake(), None);
    assert_eq!(e.st.automation().task_count(), 0);
    // Tasks aborted by the close are reaped by the runtime shortly after.
    wait_until("nothing on the runtime", || {
        e.rt.metrics().num_alive_tasks() == 0
    });
}

#[test]
fn dispatch_reaches_the_first_action_fast() {
    let e = env();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    let other = e.screen(&tpl.id, None);
    e.create(
        "fast",
        vec![
            node(
                "t",
                NodeKind::LayerAir {
                    screen_id: s.id.clone(),
                    layer_id: "l1".into(),
                    on_air: true,
                },
            ),
            node(
                "take",
                NodeKind::Take {
                    screen_id: other.id.clone(),
                    layer_id: "l1".into(),
                    for_ms: None,
                },
            ),
        ],
        vec![ev("t", "fired", "take", "in")],
    );
    let mut best = Duration::MAX;
    for _ in 0..5 {
        e.st.live().hide(&s.id, "l1").unwrap();
        e.st.live().hide(&other.id, "l1").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let started = Instant::now();
        // The event (a take; the hook runs inside it) …
        e.st.live().show(&s.id, "l1").unwrap();
        let hook = started.elapsed();
        // … to the first action on air.
        while !e.on_air(&other.id, "l1") {
            std::hint::spin_loop();
            assert!(started.elapsed() < Duration::from_secs(2));
        }
        let total = started.elapsed();
        println!("dispatch: event {hook:?}, event → first action {total:?}");
        best = best.min(total);
    }
    println!("dispatch best: {best:?}");
    // Debug build, a loaded test machine: generous. Release: well under 1 ms.
    assert!(best < Duration::from_millis(25), "{best:?}");
}

/// A realistic 200-node flow: a trigger, then actions and logic with 10-character ids.
fn big_flow(seed: usize) -> Flow {
    let id = |i: usize| format!("n{seed:03}{i:05}");
    let mut nodes = vec![node(&id(0), NodeKind::Cue { label: None })];
    let mut edges = Vec::new();
    for i in 1..200 {
        let kind = match i % 5 {
            0 => NodeKind::Take {
                screen_id: "screen0001".into(),
                layer_id: format!("layer{:05}", i % 20),
                for_ms: Some(5000),
            },
            1 => NodeKind::Delay { ms: 1000 },
            2 => NodeKind::SetFields {
                screen_id: "screen0001".into(),
                layer_id: "layer00001".into(),
                fields: fields(&[("title", "Hello")]),
            },
            3 => NodeKind::Clear {
                screen_id: "screen0001".into(),
                layer_id: format!("layer{:05}", i % 20),
            },
            _ => NodeKind::Log {
                message: "step".into(),
                toast: false,
            },
        };
        let prev = &nodes[i - 1];
        let port = match prev.kind {
            NodeKind::Cue { .. } => "fired",
            NodeKind::Delay { .. } => "out",
            _ => "done",
        };
        edges.push(FlowEdge {
            id: format!("e{seed:03}{i:05}"),
            from: PortRef {
                node: prev.id.clone(),
                port: port.into(),
            },
            to: PortRef {
                node: id(i),
                port: "in".into(),
            },
            kind: WireKind::Event,
        });
        nodes.push(node(&id(i), kind));
    }
    Flow {
        id: format!("flow{seed:06}"),
        name: format!("Flow {seed}"),
        armed: true,
        retrigger: Retrigger::Restart,
        origin: None,
        nodes,
        edges,
        draft: None,
        last_fired_at: None,
        updated_at: "2026-10-04T00:00:00Z".into(),
    }
}

#[test]
fn compiling_is_fast_and_compact() {
    let f = big_flow(0);
    let mut best = Duration::MAX;
    for _ in 0..5 {
        let started = Instant::now();
        let c = compile(&f);
        best = best.min(started.elapsed());
        assert_eq!(c.nodes.len(), 200);
    }
    println!("compile 200 nodes: {best:?}");
    assert!(
        best < Duration::from_millis(50),
        "debug build, generous: {best:?}"
    );
    let total: usize = (0..128).map(|i| compile(&big_flow(i)).approx_bytes()).sum();
    println!(
        "compiled 128 × 200 nodes: {} bytes ({:.2} MB); Op {} B, node {} B, edge {} B",
        total,
        total as f64 / 1_048_576.0,
        std::mem::size_of::<Op>(),
        std::mem::size_of::<CNode>(),
        std::mem::size_of::<Edge>()
    );
    assert!(total <= 2 * 1024 * 1024, "{total} bytes");
    // Every op stays small.
    assert!(std::mem::size_of::<Op>() <= 24);
}

#[test]
fn the_engine_holds_compiled_graphs_and_an_index_only_of_armed_flows() {
    let e = env();
    let mut f = big_flow(1);
    e.st.engine_deploy(&f);
    assert!(e.st.flow_engine().compiled_bytes() > 0);
    let index = e.st.flow_engine().index();
    assert_eq!(index.get(&TriggerKey::Cue(f.id.clone())).len(), 1);
    f.armed = false;
    e.st.engine_set_armed(&f.id, false);
    let index = e.st.flow_engine().index();
    assert!(index.get(&TriggerKey::Cue(f.id.clone())).is_empty());
    e.st.engine_remove(&f.id);
    assert_eq!(e.st.flow_engine().compiled_bytes(), 0);
}

// ---- Live debug ----------------------------------------------------------------------------------

impl Env {
    /// `flow-debug` batches of a flow so far.
    fn debug(&self, flow: &str) -> Vec<FlowDebug> {
        self.rec
            .named(names::FLOW_DEBUG)
            .into_iter()
            .filter(|v| v["id"] == flow)
            .map(|v| serde_json::from_value(v).unwrap())
            .collect()
    }

    /// Waits until no instance and no flusher is left.
    fn settle(&self) {
        wait_until("no engine task", || self.st.flow_engine().task_count() == 0);
    }
}

fn node_ids(b: &FlowDebug) -> Vec<&str> {
    b.nodes.iter().map(|n| n.node.as_str()).collect()
}

#[test]
fn debug_subscriptions_are_counted_and_cleared_on_close_and_reload() {
    let e = env();
    let id = e.create(
        "go",
        vec![cue_node(), log("a")],
        vec![ev("cue", "fired", "a", "in")],
    );
    let eng = e.st.flow_engine();
    assert!(matches!(
        e.st.subscribe_flow_debug("nope"),
        Err(AppError::NotFound(_))
    ));
    e.st.subscribe_flow_debug(&id).unwrap();
    e.st.subscribe_flow_debug(&id).unwrap();
    assert_eq!(eng.debug_subscribers(&id), 2);
    e.st.unsubscribe_flow_debug(&id).unwrap();
    assert_eq!(eng.debug_subscribers(&id), 1);
    e.st.unsubscribe_flow_debug(&id).unwrap();
    e.st.unsubscribe_flow_debug(&id).unwrap();
    e.st.unsubscribe_flow_debug("nope").unwrap();
    assert_eq!(eng.debug_subscribers(&id), 0, "never below zero");
    // Back to zero: nothing is recorded.
    e.run(&id);
    e.settle();
    assert!(e.debug(&id).is_empty());
    // Reload and close clear every subscription.
    e.st.subscribe_flow_debug(&id).unwrap();
    e.st.reload_project().unwrap();
    assert_eq!(eng.debug_subscribers(&id), 0);
    e.st.flows_project_opened();
    e.st.subscribe_flow_debug(&id).unwrap();
    e.st.close_project();
    assert_eq!(eng.debug_subscribers(&id), 0);
    assert!(matches!(
        e.st.subscribe_flow_debug(&id),
        Err(AppError::NoProject)
    ));
    e.st.unsubscribe_flow_debug(&id).unwrap();
}

#[test]
fn debug_batches_hold_the_latest_run_per_node_deduped_edges_and_errors() {
    let e = env();
    // cue → seq: s0, s1, s2 → a → b; a third branch also takes a missing layer.
    let edges = vec![
        ev("cue", "fired", "seq", "in"),
        ev("seq", "s0", "a", "in"),
        ev("seq", "s1", "a", "in"),
        ev("seq", "s2", "a", "in"),
        ev("a", "done", "b", "in"),
        ev("seq", "s2", "t", "in"),
    ];
    let ids: Vec<String> = edges.iter().map(|x| x.id.clone()).collect();
    let id = e.create(
        "debug",
        vec![
            cue_node(),
            node("seq", NodeKind::Sequence { count: 3 }),
            log("a"),
            log("b"),
            node(
                "t",
                NodeKind::Take {
                    screen_id: "noscreen".into(),
                    layer_id: "l".into(),
                    for_ms: None,
                },
            ),
        ],
        edges,
    );
    e.st.subscribe_flow_debug(&id).unwrap();
    e.run(&id);
    e.settle();
    let batches = e.debug(&id);
    assert_eq!(
        batches.len(),
        1,
        "one run within 100 ms: one batch {batches:?}"
    );
    let b = &batches[0];
    assert_eq!(node_ids(b), ["cue", "seq", "a", "b", "t"], "each node once");
    assert!(b.nodes.iter().all(|n| !n.at.is_empty()));
    let mut want = ids.clone();
    want.sort();
    let mut got = b.edges.clone();
    got.sort();
    assert_eq!(got, want, "every event edge once (a → b ran three times)");
    assert_eq!(b.errors.len(), 1, "{:?}", b.errors);
    assert_eq!(b.errors[0].node.as_deref(), Some("t"));
    assert!(b.errors[0].message.starts_with("Skipped"), "{:?}", b.errors);
    // Nothing more at rest.
    assert_eq!(e.st.flow_engine().task_count(), 0);
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(e.debug(&id).len(), 1);
}

#[test]
fn debug_is_throttled_to_ten_batches_a_second() {
    let e = env();
    // A loop through a short Delay runs nodes ~250 times a second (128 loops within the step
    // limit: longer than the burst).
    let id = e.create(
        "loop",
        vec![cue_node(), log("a"), node("d", NodeKind::Delay { ms: 8 })],
        vec![
            ev("cue", "fired", "a", "in"),
            ev("a", "done", "d", "in"),
            ev("d", "out", "a", "in"),
        ],
    );
    e.st.subscribe_flow_debug(&id).unwrap();
    // 200 firings a second for a second (the first ten start, the rest are dropped).
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(1) {
        let _ = e.st.run_flow(&id, None);
        std::thread::sleep(Duration::from_millis(5));
    }
    e.st.stop_flow(&id).unwrap();
    e.settle();
    let batches = e.debug(&id);
    println!("flow-debug batches in a 1 s burst: {}", batches.len());
    assert!(
        (7..=11).contains(&batches.len()),
        "≤ 10 batches a second: {}",
        batches.len()
    );
    let errors: Vec<&FlowIssue> = batches.iter().flat_map(|b| &b.errors).collect();
    assert!(
        errors
            .iter()
            .any(|i| i.node.as_deref() == Some("cue") && i.message.contains("more than 10")),
        "{errors:?}"
    );
    assert!(batches.iter().any(|b| node_ids(b).contains(&"d")));
}

#[test]
fn debug_records_nothing_and_runs_no_task_without_a_subscriber() {
    let e = env();
    let a = e.create(
        "a",
        vec![cue_node(), log("x")],
        vec![ev("cue", "fired", "x", "in")],
    );
    let b = e.create(
        "b",
        vec![cue_node(), log("y")],
        vec![ev("cue", "fired", "y", "in")],
    );
    // Subscribed but nothing fired: no task.
    e.st.subscribe_flow_debug(&a).unwrap();
    assert_eq!(e.st.flow_engine().task_count(), 0);
    // Another flow runs: nothing recorded for it, no flusher.
    e.run(&b);
    e.settle();
    assert_eq!(e.st.flow_engine().task_count(), 0);
    assert!(e.rec.named(names::FLOW_DEBUG).is_empty());
    // The subscribed one: a flusher while a batch is pending, none after.
    e.run(&a);
    e.settle();
    assert_eq!(e.debug(&a).len(), 1);
    assert_eq!(e.st.flow_engine().task_count(), 0);
    // Unsubscribing drops what is pending.
    e.st.run_flow(&a, None).unwrap();
    e.st.unsubscribe_flow_debug(&a).unwrap();
    e.settle();
    std::thread::sleep(Duration::from_millis(150));
    assert!(e.debug(&a).len() <= 2);
    let n = e.debug(&a).len();
    e.run(&a);
    e.settle();
    assert_eq!(e.debug(&a).len(), n, "unsubscribed: nothing new");
}

#[test]
fn deploy_keeps_the_subscription_and_batches_use_the_new_graph() {
    let e = env();
    let id = e.create(
        "go",
        vec![cue_node(), log("old")],
        vec![ev("cue", "fired", "old", "in")],
    );
    e.st.subscribe_flow_debug(&id).unwrap();
    e.run(&id);
    e.settle();
    assert_eq!(node_ids(&e.debug(&id)[0]), ["cue", "old"]);
    let edge = ev("go", "fired", "new", "in");
    let edge_id = edge.id.clone();
    e.st.save_flow(SaveFlow {
        id: id.clone(),
        name: "go".into(),
        retrigger: Retrigger::Restart,
        nodes: vec![node("go", NodeKind::Cue { label: None }), log("new")],
        edges: vec![edge],
    })
    .unwrap();
    assert_eq!(e.st.flow_engine().debug_subscribers(&id), 1);
    e.run(&id);
    e.settle();
    let batches = e.debug(&id);
    let last = batches.last().unwrap();
    assert_eq!(node_ids(last), ["go", "new"]);
    assert_eq!(last.edges, [edge_id]);
    // Deleting the flow drops its subscription.
    e.st.delete_flow(&id).unwrap();
    assert_eq!(e.st.flow_engine().debug_subscribers(&id), 0);
}

// ---- action.setVariant ---------------------------------------------------------------------------

fn set_variant(screen: &str, layer: &str, variant: &str) -> NodeKind {
    NodeKind::SetVariant {
        screen_id: screen.into(),
        layer_id: layer.into(),
        variant: variant.into(),
        transition: VariantTransition::OutIn,
    }
}

#[test]
fn set_variant_swaps_on_air_at_once_bypassing_on_take() {
    let e = env();
    let t = e.template();
    let d =
        e.st.templates()
            .create_variant(&e.dir, &t.id, "default", "Compact")
            .unwrap();
    e.st.template_saved(&d);
    let d =
        e.st.templates()
            .save_variant(
                &e.dir,
                &t.id,
                "compact",
                "<p>{{name}}</p>",
                "",
                Fields::new(),
            )
            .unwrap();
    e.st.template_saved(&d);
    let s = e.screen(&t.id, None);
    let sid = s.id.clone();
    // On air under onTake (the default), with an operator edit staged for the next take.
    e.st.live().show(&sid, "l1").unwrap();
    e.st.set_layer_fields(&sid, "l1", fields(&[("title", "Staged")]))
        .unwrap();
    assert!(e.st.live().layer_view(&sid, "l1").unwrap().pending);
    let mut rx = e.st.live().subscribe();

    let id = e.create(
        "swap",
        vec![cue_node(), node("v", set_variant(&sid, "l1", "compact"))],
        vec![ev("cue", "fired", "v", "in")],
    );
    let end = e.run(&id);
    assert_eq!(end["state"], "finished", "{end}");
    let mut msgs = Vec::new();
    while let Ok(m) = rx.try_recv() {
        if &*m.screen_id == sid.as_str() {
            msgs.push(serde_json::from_str::<Value>(&m.json).unwrap());
        }
    }
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    let wl = &msgs[0]["layer"];
    assert_eq!(msgs[0]["t"], "layer");
    assert_eq!(wl["swap"], "outIn");
    assert_eq!(wl["state"], "in");
    assert_eq!(wl["html"], "<p>{{name}}</p>");
    assert_eq!(wl["fields"]["title"], "", "the staged edit stays staged");
    let v = e.st.live().layer_view(&sid, "l1").unwrap();
    assert!(v.pending && v.on_air);
    assert_eq!(v.variant, "compact");
    assert_eq!(
        e.st.live()
            .with_layer(&sid, "l1", |l| l.variant.clone())
            .unwrap(),
        "compact"
    );
    let ScreenLayerKind::Template { variant, .. } =
        &e.st.get_screen(&sid).unwrap().doc.meta.layers[0].kind
    else {
        unreachable!()
    };
    assert_eq!(variant.as_deref(), Some("compact"), "saved with the screen");

    // A variant the template doesn't have: skipped (failed), nothing on air changes.
    let bad = e.create(
        "bad",
        vec![cue_node(), node("v", set_variant(&sid, "l1", "ghost"))],
        vec![ev("cue", "fired", "v", "in")],
    );
    e.run(&bad);
    assert_eq!(
        e.st.live()
            .with_layer(&sid, "l1", |l| l.variant.clone())
            .unwrap(),
        "compact"
    );

    // Warnings name the layer's template; validation wants a variant.
    let c =
        e.st.check_flow(
            vec![cue_node(), node("v", set_variant(&sid, "l1", "ghost"))],
            vec![ev("cue", "fired", "v", "in")],
        )
        .unwrap();
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(c.warnings.len(), 1, "{:?}", c.warnings);
    assert_eq!(
        c.warnings[0].message,
        "Variant \u{201c}ghost\u{201d} isn't in the template \u{201c}Board\u{201d}"
    );
    let c =
        e.st.check_flow(
            vec![cue_node(), node("v", set_variant(&sid, "gone", "compact"))],
            vec![ev("cue", "fired", "v", "in")],
        )
        .unwrap();
    assert!(
        c.warnings[0].message.contains("was deleted"),
        "{:?}",
        c.warnings
    );
    let c =
        e.st.check_flow(
            vec![cue_node(), node("v", set_variant(&sid, "l1", ""))],
            vec![ev("cue", "fired", "v", "in")],
        )
        .unwrap();
    assert_eq!(c.errors.len(), 1, "{:?}", c.errors);
}

// ---- Studio mode: Take preview ---------------------------------------------------------------

#[test]
fn take_preview_goes_through_the_studio_entry_point() {
    let e = env();
    let t = e.template();
    let s = e.screen(&t.id, None);
    let sid = s.id.clone();
    let take = NodeKind::TakePreview {
        screen_id: sid.clone(),
        transition: Some(crate::core::model::TakeTransition {
            kind: crate::core::model::TakeTransitionKind::Cut,
            ms: 100,
        }),
    };
    let id = e.create(
        "take",
        vec![cue_node(), node("tp", take), log("ok"), log("off")],
        vec![
            ev("cue", "fired", "tp", "in"),
            ev("tp", "done", "ok", "in"),
            ev("tp", "failed", "off", "in"),
        ],
    );
    // Studio mode off: the action fails.
    e.run(&id);
    assert_eq!(e.ran(&id), ["tp", "off"]);
    // On: the flow takes the Preview (a cued-in layer) to Program.
    let settings = crate::core::model::StudioSettings {
        enabled: true,
        ..Default::default()
    };
    e.st.set_studio(settings, None).unwrap();
    e.st.apply_studio_op(
        &sid,
        StudioOp::Cue {
            layer_id: "l1".into(),
            visible: Some(true),
        },
    )
    .unwrap();
    let mut rx = e.st.live().subscribe();
    let n0 = e.st.live().studio_op_count();
    e.run(&id);
    assert!(e.ran(&id).ends_with(&["tp".to_string(), "ok".to_string()]));
    assert!(e.on_air(&sid, "l1"));
    assert_eq!(e.st.live().studio_op_count(), n0 + 1, "one studio op");
    let mut takes = Vec::new();
    while let Ok(m) = rx.try_recv() {
        if &*m.screen_id == sid.as_str() {
            takes.push(serde_json::from_str::<Value>(&m.json).unwrap());
        }
    }
    assert_eq!(takes.len(), 1, "{takes:?}");
    assert_eq!(takes[0]["t"], "take");
    assert_eq!(takes[0]["transition"]["kind"], "cut");
    assert!(e.st.preview_view(&sid).unwrap().cues.is_empty());
    // A missing screen is a warning; a bad id an error.
    let missing = NodeKind::TakePreview {
        screen_id: "gone12345".into(),
        transition: None,
    };
    let c =
        e.st.check_flow(
            vec![cue_node(), node("tp", missing)],
            vec![ev("cue", "fired", "tp", "in")],
        )
        .unwrap();
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert_eq!(c.warnings.len(), 1, "{:?}", c.warnings);
    assert_eq!(c.warnings[0].message, "Screen no longer exists");
    let bad = NodeKind::TakePreview {
        screen_id: "../x".into(),
        transition: None,
    };
    let c =
        e.st.check_flow(
            vec![cue_node(), node("tp", bad)],
            vec![ev("cue", "fired", "tp", "in")],
        )
        .unwrap();
    assert_eq!(c.errors.len(), 1, "{:?}", c.errors);
}

// ---- Repeat and Stop flow ----------------------------------------------------------------------

const PACED: &str = "Looping faster than every 100 ms — paced.";

fn repeat(id: &str, count: Option<u32>, delay_ms: u64) -> FlowNode {
    node(id, NodeKind::Repeat { count, delay_ms })
}

fn stop_flow(id: &str, flow: &str) -> FlowNode {
    node(
        id,
        NodeKind::StopFlow {
            flow_id: flow.into(),
        },
    )
}

impl Env {
    /// The action nodes that ran, in order (`running` events; the pacing warning is a
    /// `running` event on the Repeat, named `r`, `o` or `i` here).
    fn actions(&self, flow: &str) -> Vec<String> {
        self.events(flow)
            .iter()
            .filter(|v| v["state"] == "running")
            .filter_map(|v| v["node"].as_str().map(str::to_string))
            .filter(|n| !["cue", "r", "o", "i"].contains(&n.as_str()))
            .collect()
    }

    fn count_ran(&self, flow: &str, node: &str) -> usize {
        self.actions(flow).iter().filter(|n| *n == node).count()
    }

    /// The pacing warnings of a flow in `flow-state` (on its Repeat `r`).
    fn paced(&self, flow: &str) -> Vec<Value> {
        self.events(flow)
            .into_iter()
            .filter(|v| v["message"] == PACED && v["node"] == "r")
            .collect()
    }

    fn last_state(&self, flow: &str) -> Value {
        self.events(flow).pop().unwrap()
    }
}

#[test]
fn repeat_runs_its_body_n_times_with_the_round_then_done_once() {
    let e = env();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    let take = |id: &str| {
        node(
            id,
            NodeKind::Take {
                screen_id: s.id.clone(),
                layer_id: "l1".into(),
                for_ms: None,
            },
        )
    };
    let clear = node(
        "cl",
        NodeKind::Clear {
            screen_id: s.id.clone(),
            layer_id: "l1".into(),
        },
    );
    let id = e.create(
        "three",
        vec![
            cue_node(),
            repeat("r", Some(3), 0),
            take("tk"),
            clear,
            node(
                "sw",
                NodeKind::Switch {
                    cases: vec!["1".into(), "2".into(), "3".into()],
                },
            ),
            log("one"),
            log("two"),
            log("three"),
            log("end"),
        ],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "body", "tk", "in"),
            ev("tk", "done", "cl", "in"),
            ev("cl", "done", "sw", "in"),
            data("r", "round", "sw", "value"),
            ev("sw", "case0", "one", "in"),
            ev("sw", "case1", "two", "in"),
            ev("sw", "case2", "three", "in"),
            ev("r", "done", "end", "in"),
        ],
    );
    let started = Instant::now();
    let end = e.run(&id);
    assert_eq!(end["state"], "finished", "{end}");
    assert_eq!(
        e.actions(&id),
        ["tk", "cl", "one", "tk", "cl", "two", "tk", "cl", "three", "end"]
    );
    // Rounds start at most every 100 ms (two waits between three rounds).
    assert!(started.elapsed() >= Duration::from_millis(200));
    assert_eq!(e.paced(&id).len(), 1, "one warning per run");
    assert!(!e.on_air(&s.id, "l1"));
    e.settle();
}

#[test]
fn a_forever_repeat_ends_on_its_stop_input_and_fires_done() {
    let e = env();
    let id = e.create(
        "forever",
        vec![
            cue_node(),
            repeat("r", None, 0),
            log("x"),
            node("n", NodeKind::Counter { every: 3 }),
            log("end"),
        ],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "body", "x", "in"),
            ev("x", "done", "n", "in"),
            ev("n", "out", "r", "stop"),
            ev("r", "done", "end", "in"),
        ],
    );
    let end = e.run(&id);
    assert_eq!(end["state"], "finished", "{end}");
    assert_eq!(e.actions(&id), ["x", "x", "x", "end"]);
    // A stop cancels the body's pending delay (parked first, then the stop runs).
    let id = e.create(
        "cancel",
        vec![
            cue_node(),
            repeat("r", None, 0),
            node("d", NodeKind::Delay { ms: 60_000 }),
            log("never"),
            log("x"),
            log("end"),
        ],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "body", "d", "in"),
            ev("d", "out", "never", "in"),
            ev("r", "body", "x", "in"),
            ev("x", "done", "r", "stop"),
            ev("r", "done", "end", "in"),
        ],
    );
    let started = Instant::now();
    let end = e.run(&id);
    assert_eq!(end["state"], "finished", "{end}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(e.actions(&id), ["x", "end"]);
    e.settle();
}

#[test]
fn stop_flow_from_a_cue_stops_a_loop_without_done() {
    let e = env();
    let lp = e.create(
        "Loop",
        vec![cue_node(), repeat("r", None, 0), log("x"), log("end")],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "body", "x", "in"),
            ev("r", "done", "end", "in"),
        ],
    );
    let stopper = e.create(
        "Stop loop",
        vec![cue_node(), stop_flow("s", &lp), log("after")],
        vec![
            ev("cue", "fired", "s", "in"),
            ev("s", "done", "after", "in"),
        ],
    );
    e.st.run_flow(&lp, None).unwrap();
    wait_until("two rounds", || e.count_ran(&lp, "x") >= 2);
    let end = e.run(&stopper);
    assert_eq!(end["state"], "finished");
    assert_eq!(e.actions(&stopper), ["s", "after"]);
    assert!(!e.st.flow_engine().is_running(&lp));
    let last = e.last_state(&lp);
    assert_eq!(last["state"], "stopped");
    assert_eq!(last["message"], "Stopped by Stop loop");
    // Not running: nothing to stop, `done` all the same.
    let end = e.run(&stopper);
    assert_eq!(end["state"], "finished");
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(
        e.count_ran(&lp, "end"),
        0,
        "a stopped loop never fires done"
    );
    e.settle();
}

#[test]
fn stop_flow_of_its_own_flow_ends_the_instance_after_the_node() {
    let e = env();
    let id = e.create(
        "Self stop",
        vec![
            cue_node(),
            repeat("r", None, 0),
            log("x"),
            node("n", NodeKind::Counter { every: 2 }),
            log("never"),
            log("end"),
        ],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "body", "x", "in"),
            ev("x", "done", "n", "in"),
            ev("r", "done", "end", "in"),
        ],
    );
    // Wire the stop of its own id (known only after create).
    let mut nodes = e.st.get_flow(&id).unwrap().flow.nodes;
    nodes.push(stop_flow("s", &id));
    let mut edges = e.st.get_flow(&id).unwrap().flow.edges;
    edges.push(ev("n", "out", "s", "in"));
    edges.push(ev("s", "done", "never", "in"));
    e.st.save_flow(SaveFlow {
        id: id.clone(),
        name: "Self stop".into(),
        retrigger: Retrigger::Restart,
        nodes,
        edges,
    })
    .unwrap();
    e.st.run_flow(&id, None).unwrap();
    wait_until("stopped", || e.last_state(&id)["state"] == "stopped");
    assert_eq!(e.last_state(&id)["message"], "Stopped by Self stop");
    assert_eq!(e.actions(&id), ["x", "x", "s"]);
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(e.actions(&id), ["x", "x", "s"], "no done, no more rounds");
    e.settle();
}

#[test]
fn disarm_panic_and_automation_off_stop_a_loop() {
    let e = env();
    let id = e.create(
        "loop",
        vec![cue_node(), repeat("r", None, 0), log("x"), log("end")],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "body", "x", "in"),
            ev("r", "done", "end", "in"),
        ],
    );
    let start = |e: &Env| {
        let n = e.count_ran(&id, "x");
        e.st.run_flow(&id, None).unwrap();
        wait_until("a round", || e.count_ran(&id, "x") > n);
    };
    start(&e);
    e.st.set_flow_armed(&id, false).unwrap();
    assert_eq!(e.st.flow_engine().task_count(), 0);
    assert_eq!(e.last_state(&id)["message"], "Stopped: disarmed");
    e.st.set_flow_armed(&id, true).unwrap();
    start(&e);
    e.st.live_panic().unwrap();
    assert_eq!(e.st.flow_engine().task_count(), 0);
    assert_eq!(e.last_state(&id)["message"], "PANIC");
    start(&e);
    e.automation_off(true);
    assert_eq!(e.st.flow_engine().task_count(), 0);
    assert_eq!(e.last_state(&id)["message"], "Stopped: automation off");
    e.automation_off(false);
    start(&e);
    e.st.stop_flow(&id).unwrap();
    assert_eq!(e.st.flow_engine().task_count(), 0);
    assert_eq!(e.last_state(&id)["message"], "Stopped");
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(e.count_ran(&id, "end"), 0, "no done on any of these");
    e.settle();
    wait_until("nothing on the runtime", || {
        e.rt.metrics().num_alive_tasks() == 0
    });
}

#[test]
fn the_next_round_waits_for_the_body_incl_its_delay_then_delay_ms() {
    let e = env();
    let id = e.create(
        "slow",
        vec![
            cue_node(),
            repeat("r", Some(3), 100),
            node("d", NodeKind::Delay { ms: 150 }),
            log("x"),
            log("end"),
        ],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "body", "d", "in"),
            ev("d", "out", "x", "in"),
            ev("r", "done", "end", "in"),
        ],
    );
    let started = Instant::now();
    let end = e.run(&id);
    let took = started.elapsed();
    assert_eq!(end["state"], "finished");
    assert_eq!(e.actions(&id), ["x", "x", "x", "end"]);
    // 3 × 150 ms of body + 2 × 100 ms between rounds.
    assert!(took >= Duration::from_millis(650), "{took:?}");
    assert!(took < Duration::from_millis(3000), "{took:?}");
    assert!(e.paced(&id).is_empty(), "never faster than the guard");
}

#[test]
fn the_next_round_waits_for_a_run_flow_the_body_waits_for() {
    let e = env();
    let target = e.create(
        "target",
        vec![
            node("t", NodeKind::RunByFlow {}),
            node("d", NodeKind::Delay { ms: 150 }),
            log("inner"),
        ],
        vec![ev("t", "fired", "d", "in"), ev("d", "out", "inner", "in")],
    );
    let id = e.create(
        "caller",
        vec![
            cue_node(),
            repeat("r", Some(2), 0),
            node(
                "rf",
                NodeKind::RunFlow {
                    flow_id: target.clone(),
                    wait: true,
                },
            ),
            log("x"),
            log("end"),
        ],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "body", "rf", "in"),
            ev("rf", "done", "x", "in"),
            ev("r", "done", "end", "in"),
        ],
    );
    let started = Instant::now();
    let end = e.run(&id);
    assert_eq!(end["state"], "finished");
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert_eq!(e.actions(&id), ["rf", "x", "rf", "x", "end"]);
    assert_eq!(e.count_ran(&target, "inner"), 2);
    e.settle();
}

#[test]
fn a_fast_loop_is_paced_to_ten_rounds_a_second_with_one_warning() {
    let e = env();
    let id = e.create(
        "fast",
        vec![cue_node(), repeat("r", None, 0), log("x")],
        vec![ev("cue", "fired", "r", "in"), ev("r", "body", "x", "in")],
    );
    e.st.subscribe_flow_debug(&id).unwrap();
    e.st.run_flow(&id, None).unwrap();
    std::thread::sleep(Duration::from_millis(1000));
    e.st.stop_flow(&id).unwrap();
    let rounds = e.count_ran(&id, "x");
    assert!((5..=11).contains(&rounds), "{rounds} rounds in a second");
    let paced = e.paced(&id);
    assert_eq!(paced.len(), 1, "one warning per run");
    assert_eq!(paced[0]["state"], "running");
    assert_eq!(paced[0]["node"], "r");
    // From the warning on, every `running` of the run carries it (the UI keeps the warning
    // until a `running` without a message: a new run).
    let ev = e.events(&id);
    let at = ev.iter().position(|v| v["node"] == "r").unwrap();
    assert!(ev[..at].iter().all(|v| v["message"].is_null()), "{ev:?}");
    let later: Vec<&Value> = ev[at..]
        .iter()
        .filter(|v| v["state"] == "running")
        .collect();
    assert!(later.len() >= 4, "{later:?}");
    assert!(later.iter().all(|v| v["message"] == PACED), "{later:?}");
    e.settle();
    let warnings: Vec<FlowIssue> = e
        .debug(&id)
        .into_iter()
        .flat_map(|b| b.errors)
        .filter(|i| i.message == PACED)
        .collect();
    assert_eq!(
        warnings,
        [FlowIssue {
            node: Some("r".into()),
            message: PACED.into()
        }]
    );
    // A new run warns again (once).
    e.st.run_flow(&id, None).unwrap();
    wait_until("the second warning", || e.paced(&id).len() == 2);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(e.paced(&id).len(), 2);
    e.st.stop_flow(&id).unwrap();
    e.st.unsubscribe_flow_debug(&id).unwrap();
    e.settle();
}

#[test]
fn the_step_budget_is_per_round() {
    let e = env();
    e.st.flow_engine().pace_ms.store(1, AtomicOrdering::Relaxed);
    // A long loop: far more than 256 node runs in total, never "Too many steps".
    let id = e.create(
        "long",
        vec![cue_node(), repeat("r", None, 0), log("x"), log("y")],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "body", "x", "in"),
            ev("x", "done", "y", "in"),
        ],
    );
    e.st.run_flow(&id, None).unwrap();
    wait_until("300 rounds", || e.count_ran(&id, "x") >= 300);
    assert!(e.st.flow_engine().is_running(&id));
    e.st.stop_flow(&id).unwrap();
    assert!(e.ends(&id).is_empty(), "{:?}", e.ends(&id));
    // One runaway round (an event loop through a Delay inside the body) still trips it.
    let runaway = e.create(
        "runaway",
        vec![
            cue_node(),
            repeat("r", None, 0),
            log("x"),
            node("d", NodeKind::Delay { ms: 0 }),
        ],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "body", "x", "in"),
            ev("x", "done", "d", "in"),
            ev("d", "out", "x", "in"),
        ],
    );
    let end = e.run(&runaway);
    assert_eq!(end["state"], "failed");
    assert_eq!(
        end["message"],
        "Too many steps (more than 256 node runs in one Repeat round)"
    );
    e.settle();
}

#[test]
fn a_long_loop_holds_flat_memory() {
    let e = env();
    e.st.flow_engine().pace_ms.store(1, AtomicOrdering::Relaxed);
    let id = e.create(
        "flat",
        vec![
            cue_node(),
            repeat("r", None, 0),
            node(
                "f",
                NodeKind::Format {
                    pattern: "Round {a}".into(),
                },
            ),
            node(
                "x",
                NodeKind::Log {
                    message: String::new(),
                    toast: false,
                },
            ),
        ],
        vec![
            ev("cue", "fired", "r", "in"),
            ev("r", "body", "x", "in"),
            data("r", "round", "f", "a"),
            data("f", "text", "x", "text"),
        ],
    );
    let compiled = e.st.flow_engine().compiled_bytes();
    e.st.run_flow(&id, None).unwrap();
    // Each round waits for the (1 ms) pacing timer: slow timers on Windows, so a longer wait.
    // Count walk segments, not rounds: a round whose pacing time has already passed when the
    // previous one settles starts in the same segment (common on a loaded machine).
    let segments = || e.st.flow_engine().walk_bytes.lock().unwrap().len();
    let deadline = Instant::now() + Duration::from_secs(40);
    while segments() < 500 {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for 500 segments ({} rounds)",
            e.count_ran(&id, "x")
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    e.st.stop_flow(&id).unwrap();
    e.settle();
    let sizes = std::mem::take(&mut *e.st.flow_engine().walk_bytes.lock().unwrap());
    assert!(sizes.len() >= 500, "{} segments", sizes.len());
    let early = sizes[..50].iter().copied().max().unwrap();
    let late = sizes[50..].iter().copied().max().unwrap();
    assert!(late <= early, "the walk grew: {early} → {late} bytes");
    assert_eq!(e.st.flow_engine().compiled_bytes(), compiled);
}

#[test]
fn nested_repeats_run_the_inner_loop_each_outer_round() {
    let e = env();
    e.st.flow_engine().pace_ms.store(5, AtomicOrdering::Relaxed);
    let id = e.create(
        "nested",
        vec![
            cue_node(),
            repeat("o", Some(2), 0),
            repeat("i", Some(3), 0),
            node(
                "sw",
                NodeKind::Switch {
                    cases: vec!["1".into(), "2".into()],
                },
            ),
            log("x"),
            log("o1"),
            log("o2"),
            log("y"),
            log("end"),
        ],
        vec![
            ev("cue", "fired", "o", "in"),
            ev("o", "body", "i", "in"),
            ev("i", "body", "x", "in"),
            ev("i", "done", "sw", "in"),
            data("o", "round", "sw", "value"),
            ev("sw", "case0", "o1", "in"),
            ev("sw", "case1", "o2", "in"),
            ev("o1", "done", "y", "in"),
            ev("o2", "done", "y", "in"),
            ev("o", "done", "end", "in"),
        ],
    );
    let end = e.run(&id);
    assert_eq!(end["state"], "finished", "{end}");
    assert_eq!(
        e.actions(&id),
        ["x", "x", "x", "o1", "y", "x", "x", "x", "o2", "y", "end"]
    );
    e.settle();
}

#[test]
fn a_nested_loop_counts_in_the_outer_rounds_budget() {
    let e = env();
    e.st.flow_engine().pace_ms.store(1, AtomicOrdering::Relaxed);
    // 1 outer round of an inner loop of 200 rounds × 2 node runs: over 256 in that round.
    let id = e.create(
        "deep",
        vec![
            cue_node(),
            repeat("o", Some(2), 0),
            repeat("i", Some(200), 0),
            log("x"),
            log("y"),
        ],
        vec![
            ev("cue", "fired", "o", "in"),
            ev("o", "body", "i", "in"),
            ev("i", "body", "x", "in"),
            ev("x", "done", "y", "in"),
        ],
    );
    let end = e.run(&id);
    assert_eq!(end["state"], "failed");
    assert!(end["message"]
        .as_str()
        .unwrap()
        .contains("in one Repeat round"));
    assert_eq!(e.count_ran(&id, "x"), 128, "1 (in) + 128 × 2 = 257th run");
    e.settle();
}

#[test]
fn media_action_and_media_ended_trigger() {
    use crate::core::model::{
        MediaAction, MediaEndAction, MediaFit, MediaLayerConfig, PlaybackState,
    };
    let clip = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../e2e/media/clip-vp9.webm");
    if !clip.exists() {
        eprintln!("e2e clip missing; skipped");
        return;
    }
    let e = env();
    std::fs::create_dir_all(e.dir.join("assets")).unwrap();
    std::fs::copy(&clip, e.dir.join("assets/clip.webm")).unwrap();
    let mut doc =
        e.st.create_screen(&NewScreen {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap()
        .doc;
    doc.meta.layers.push(ScreenLayer {
        id: "m1".into(),
        name: "Clip".into(),
        rect: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        kind: ScreenLayerKind::Media {
            media: MediaLayerConfig {
                asset: "/assets/clip.webm".into(),
                fit: MediaFit::Fit,
                in_ms: 0,
                out_ms: Some(300),
                volume: 100,
                muted: false,
                end_action: MediaEndAction::Clear,
                autoplay: false,
            },
            opacity: None,
        },
        anchor: None,
    });
    let s = e.st.save_screen(doc).unwrap().doc.id;
    let ended = e.create(
        "Ended",
        vec![
            node(
                "t",
                NodeKind::MediaEnded {
                    screen_id: s.clone(),
                    layer_id: "m1".into(),
                },
            ),
            log("after"),
        ],
        vec![ev("t", "fired", "after", "in")],
    );
    let play = e.create(
        "Play",
        vec![
            cue_node(),
            node(
                "go",
                NodeKind::Media {
                    screen_id: s.clone(),
                    layer_id: "m1".into(),
                    action: MediaAction::Play,
                    ms: None,
                },
            ),
        ],
        vec![ev("cue", "fired", "go", "in")],
    );
    // Take without autoplay: paused at the in point; the flow's Media action plays it.
    e.st.live().show(&s, "m1").unwrap();
    let pb = |e: &Env| {
        e.st.live()
            .layer_view(&s, "m1")
            .unwrap()
            .media
            .unwrap()
            .playback
    };
    assert_eq!(pb(&e).state, PlaybackState::Paused);
    assert_eq!(e.run(&play)["state"], "finished");
    assert_eq!(pb(&e).state, PlaybackState::Playing);
    // At the out point the core clears the layer and fires "Media ends".
    wait_until("the clear at the out point", || !e.on_air(&s, "m1"));
    wait_until("Media ends", || e.ran(&ended) == ["after"]);
}

#[test]
fn an_api_call_runs_its_flow_with_the_value_even_with_automation_off() {
    let e = env();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    let set = node(
        "set",
        NodeKind::SetFields {
            screen_id: s.id.clone(),
            layer_id: "l2".into(),
            fields: fields(&[("title", "")]),
        },
    );
    let id = e.create(
        "From Companion",
        vec![node("rest", NodeKind::Rest {}), set],
        vec![
            ev("rest", "fired", "set", "in"),
            data("rest", "value", "set", "title"),
        ],
    );
    e.automation_off(true);
    e.st.run_flow_rest(&id, Some("Ingrid Vale".into())).unwrap();
    wait_until("the run's end", || !e.ends(&id).is_empty());
    assert_eq!(e.field(&s.id, "l2", "title"), "Ingrid Vale");

    // Not a Cue: run_flow (cue buttons) refuses it, and the API refuses flows without one.
    assert!(e.st.run_flow(&id, None).is_err());
    let cue = e.create("Cue only", vec![cue_node()], vec![]);
    assert_eq!(
        e.st.run_flow_rest(&cue, None),
        Err(RestRunRefusal::NotFound)
    );
    assert_eq!(
        e.st.run_flow_rest("nope", None),
        Err(RestRunRefusal::NotFound)
    );
    e.st.set_flow_armed(&id, false).unwrap();
    assert_eq!(e.st.run_flow_rest(&id, None), Err(RestRunRefusal::Disarmed));
}

#[test]
fn a_flow_has_at_most_one_api_call_trigger() {
    let e = env();
    let err =
        e.st.create_flow(NewFlow {
            name: "Two".into(),
            armed: true,
            retrigger: Retrigger::Restart,
            origin: None,
            nodes: vec![node("a", NodeKind::Rest {}), node("b", NodeKind::Rest {})],
            edges: vec![],
        })
        .unwrap_err();
    assert!(
        err.to_string().contains("at most one API call trigger"),
        "{err}"
    );
}

/// Step source with a row number wired into `index` goes to that row (counted from 1, like the UI),
/// whatever its own mode says, and stays there on every run. A tester's "Next row" node with a
/// Constant 75 wired in seemed to step to the next row: that flow wasn't deployed.
#[test]
fn a_wired_index_overrides_the_step_mode() {
    let e = env();
    let src = e.source();
    let index = || match e.st.get_source(&src.id).unwrap().meta.kind {
        SourceKind::Text { index, .. } => index,
        _ => unreachable!(),
    };
    // Row 3 is index 2; row 1 index 0; anything below 1 is row 1.
    for (value, want) in [
        (Constant::Number(3.0), 2),
        (Constant::Number(1.0), 0),
        (Constant::Number(-4.0), 0),
    ] {
        let id = e.create(
            "goto",
            vec![
                cue_node(),
                node(
                    "s",
                    NodeKind::StepSource {
                        source_id: src.id.clone(),
                        step: SourceStep::By { delta: 1 },
                    },
                ),
                node("k", NodeKind::Constant { value }),
            ],
            vec![
                ev("cue", "fired", "s", "in"),
                data("k", "value", "s", "index"),
            ],
        );
        e.run(&id);
        wait_until("wired index", || index() == want);
        // Again: still that row, not the next one.
        e.run(&id);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(index(), want);
    }
}
