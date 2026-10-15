//! The flow port table: each node type's event and data ports (see the port-id conventions in
//! `core::model`'s "Flows" section). Validation (`core::flows`) and the engine
//! (`core::flow_engine`) both read it, so a port exists in exactly one place.
//!
//! Dynamic ports: switch `case0`.. per case, sequence `s0`.., random `r0`.., one `setFields` data
//! input per field key of its config, and `data.sourceRow`'s column outputs. Columns depend on
//! the source, so [`output`] accepts any key-shaped name there as a text column; whether the
//! source has it is a soft warning (`flows::warnings`), like a missing target.

use crate::core::model::{Constant, DataType, NodeKind, WireKind, FLOW_BRANCHES_MAX};
use crate::core::templates::is_key;

/// One port of a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    pub wire: WireKind,
    /// The value type of a data port (`None` for event ports).
    pub ty: Option<DataType>,
}

impl Port {
    fn event(name: &str) -> Self {
        Self {
            name: name.to_string(),
            wire: WireKind::Event,
            ty: None,
        }
    }

    fn data(name: &str, ty: DataType) -> Self {
        Self {
            name: name.to_string(),
            wire: WireKind::Data,
            ty: Some(ty),
        }
    }
}

/// A node's ports (static and config-dependent ones; `data.sourceRow` columns excepted, see
/// [`Ports::column_outputs`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ports {
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    /// `data.sourceRow`: every column key of the source is also a text output.
    pub column_outputs: bool,
}

/// The four node categories (the `type` prefix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Trigger,
    Data,
    Logic,
    Action,
}

/// The node's `type` string (`trigger.cue`, `action.take`, …).
pub fn type_name(kind: &NodeKind) -> &'static str {
    use NodeKind as K;
    match kind {
        K::Cue { .. } => "trigger.cue",
        K::ItemStatus { .. } => "trigger.itemStatus",
        K::BeforeNext { .. } => "trigger.beforeNext",
        K::TimeOfDay { .. } => "trigger.timeOfDay",
        K::TimerZero { .. } => "trigger.timerZero",
        K::LayerAir { .. } => "trigger.layer",
        K::SlideshowWrap { .. } => "trigger.slideshowWrap",
        K::SourceRowChanged { .. } => "trigger.sourceRow",
        K::ApiChanged { .. } => "trigger.apiChanged",
        K::Osc { .. } => "trigger.osc",
        K::Midi { .. } => "trigger.midi",
        K::MediaEnded { .. } => "trigger.mediaEnded",
        K::RunByFlow {} => "trigger.runByFlow",
        K::Rest {} => "trigger.rest",
        K::SourceRow { .. } => "data.sourceRow",
        K::ScheduleItem { .. } => "data.scheduleItem",
        K::FieldValue { .. } => "data.fieldValue",
        K::LayerState { .. } => "data.layerState",
        K::Constant { .. } => "data.constant",
        K::TimeNow {} => "data.timeNow",
        K::Format { .. } => "data.format",
        K::Math { .. } => "data.math",
        K::Compare { .. } => "data.compare",
        K::Branch {} => "logic.branch",
        K::Switch { .. } => "logic.switch",
        K::Gate { .. } => "logic.gate",
        K::Delay { .. } => "logic.delay",
        K::WaitUntil { .. } => "logic.waitUntil",
        K::Debounce { .. } => "logic.debounce",
        K::Throttle { .. } => "logic.throttle",
        K::Sequence { .. } => "logic.sequence",
        K::Once {} => "logic.once",
        K::Counter { .. } => "logic.counter",
        K::Random { .. } => "logic.random",
        K::Repeat { .. } => "logic.repeat",
        K::Take { .. } => "action.take",
        K::Clear { .. } => "action.clear",
        K::ClearScreen { .. } => "action.clearScreen",
        K::ClearAll {} => "action.clearAll",
        K::SetFields { .. } => "action.setFields",
        K::SetTimer { .. } => "action.setTimer",
        K::StepSource { .. } => "action.stepSource",
        K::PauseSource { .. } => "action.pauseSource",
        K::Schedule { .. } => "action.schedule",
        K::RunFlow { .. } => "action.runFlow",
        K::StopFlow { .. } => "action.stopFlow",
        K::SetVariant { .. } => "action.setVariant",
        K::TakePreview { .. } => "action.takePreview",
        K::Log { .. } => "action.log",
        K::Media { .. } => "action.media",
    }
}

pub fn category(kind: &NodeKind) -> Category {
    match type_name(kind).split_once('.').map(|(c, _)| c) {
        Some("trigger") => Category::Trigger,
        Some("data") => Category::Data,
        Some("logic") => Category::Logic,
        _ => Category::Action,
    }
}

pub fn is_trigger(kind: &NodeKind) -> bool {
    category(kind) == Category::Trigger
}

pub fn is_data(kind: &NodeKind) -> bool {
    category(kind) == Category::Data
}

pub fn is_logic(kind: &NodeKind) -> bool {
    category(kind) == Category::Logic
}

pub fn is_action(kind: &NodeKind) -> bool {
    category(kind) == Category::Action
}

/// Nodes that wait before firing on (an event cycle must pass through one of these). A Repeat
/// paces its rounds, so a wire from its `body` back to its `stop` is fine (one back to its own
/// `in` is an error of its own, see `flows`).
pub fn is_wait(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Delay { .. }
            | NodeKind::WaitUntil { .. }
            | NodeKind::Debounce { .. }
            | NodeKind::Throttle { .. }
            | NodeKind::Repeat { .. }
    )
}

/// The value type of a constant.
pub fn constant_type(c: &Constant) -> DataType {
    match c {
        Constant::Text(_) => DataType::Text,
        Constant::Number(_) => DataType::Number,
        Constant::Bool(_) => DataType::Bool,
    }
}

/// Whether a data wire may carry `from` into an input of type `to`: equal types, or `number`,
/// `bool`, `time` into `text` (implicit conversion).
pub fn compatible(from: DataType, to: DataType) -> bool {
    from == to
        || (to == DataType::Text
            && matches!(from, DataType::Number | DataType::Bool | DataType::Time))
}

/// `"a"`..`"h"`: the format inputs.
pub const FORMAT_INPUTS: [&str; FLOW_BRANCHES_MAX as usize] =
    ["a", "b", "c", "d", "e", "f", "g", "h"];

/// The data outputs of `data.sourceRow` besides its columns.
pub const SOURCE_ROW_OUTPUTS: [&str; 3] = ["row", "rows", "index"];

/// The text outputs of `data.scheduleItem`.
pub const SCHEDULE_ITEM_TEXT: [&str; 7] = [
    "title", "subtitle", "location", "start", "end", "status", "label",
];

/// Every port of a node (see [`Ports`]).
pub fn ports(kind: &NodeKind) -> Ports {
    use DataType as T;
    use NodeKind as K;
    let ev = Port::event;
    let d = Port::data;
    let mut p = Ports::default();
    let cat = category(kind);
    // Shared event ports per category.
    match cat {
        Category::Trigger => p.outputs.push(ev("fired")),
        Category::Action => {
            p.inputs.push(ev("in"));
            p.outputs.push(ev("done"));
            p.outputs.push(ev("failed"));
        }
        Category::Logic => p.inputs.push(ev("in")),
        Category::Data => {}
    }
    match kind {
        // Triggers.
        K::Cue { .. } | K::RunByFlow {} | K::Rest {} => p.outputs.push(d("value", T::Text)),
        K::ItemStatus { .. } | K::BeforeNext { .. } => p.outputs.push(d("item", T::Row)),
        K::TimeOfDay { .. } => p.outputs.push(d("at", T::Time)),
        K::SourceRowChanged { .. } => {
            p.outputs.push(d("row", T::Row));
            p.outputs.push(d("index", T::Number));
        }
        K::ApiChanged { .. } => {
            p.outputs.push(d("value", T::Text));
            p.outputs.push(d("row", T::Row));
        }
        K::Osc { .. } => p.outputs.push(d("value", T::Text)),
        K::Midi { .. } => p.outputs.push(d("value", T::Number)),
        K::TimerZero { .. }
        | K::LayerAir { .. }
        | K::SlideshowWrap { .. }
        | K::MediaEnded { .. } => {}
        // Data.
        K::SourceRow { .. } => {
            p.inputs.push(d("index", T::Number));
            p.outputs.push(d("row", T::Row));
            p.outputs.push(d("rows", T::Rows));
            p.outputs.push(d("index", T::Number));
            p.column_outputs = true;
        }
        K::ScheduleItem { .. } => {
            for n in SCHEDULE_ITEM_TEXT {
                p.outputs.push(d(n, T::Text));
            }
            p.outputs.push(d("exists", T::Bool));
            p.outputs.push(d("item", T::Row));
        }
        K::FieldValue { .. } => p.outputs.push(d("value", T::Text)),
        K::LayerState { .. } => {
            p.outputs.push(d("onAir", T::Bool));
            p.outputs.push(d("pending", T::Bool));
        }
        K::Constant { value } => p.outputs.push(d("value", constant_type(value))),
        K::TimeNow {} => p.outputs.push(d("now", T::Time)),
        K::Format { .. } => {
            for n in FORMAT_INPUTS {
                p.inputs.push(d(n, T::Text));
            }
            p.outputs.push(d("text", T::Text));
        }
        K::Math { .. } => {
            p.inputs.push(d("a", T::Number));
            p.inputs.push(d("b", T::Number));
            p.outputs.push(d("number", T::Number));
        }
        K::Compare { .. } => {
            p.inputs.push(d("a", T::Text));
            p.inputs.push(d("b", T::Text));
            p.outputs.push(d("bool", T::Bool));
        }
        // Logic.
        K::Branch {} => {
            p.inputs.push(d("condition", T::Bool));
            p.outputs.push(ev("then"));
            p.outputs.push(ev("else"));
        }
        K::Switch { cases } => {
            p.inputs.push(d("value", T::Text));
            for i in 0..cases.len() {
                p.outputs.push(ev(&format!("case{i}")));
            }
            p.outputs.push(ev("default"));
        }
        K::Gate { .. } => {
            p.inputs.push(ev("open"));
            p.inputs.push(ev("close"));
            p.outputs.push(ev("out"));
        }
        K::Delay { .. } => {
            p.inputs.push(ev("cancel"));
            p.inputs.push(d("ms", T::Number));
            p.outputs.push(ev("out"));
        }
        K::WaitUntil { .. } => {
            p.inputs.push(ev("release"));
            p.outputs.push(ev("out"));
            p.outputs.push(ev("timeout"));
        }
        K::Debounce { .. } | K::Throttle { .. } => p.outputs.push(ev("out")),
        K::Sequence { count } => {
            for i in 0..*count {
                p.outputs.push(ev(&format!("s{i}")));
            }
        }
        K::Once {} => {
            p.inputs.push(ev("reset"));
            p.outputs.push(ev("out"));
        }
        K::Counter { .. } => {
            p.inputs.push(ev("reset"));
            p.outputs.push(ev("out"));
            p.outputs.push(d("count", T::Number));
        }
        K::Random { count, .. } => {
            for i in 0..*count {
                p.outputs.push(ev(&format!("r{i}")));
            }
        }
        K::Repeat { .. } => {
            p.inputs.push(ev("stop"));
            p.outputs.push(ev("body"));
            p.outputs.push(ev("done"));
            p.outputs.push(d("round", T::Number));
        }
        // Actions.
        K::Take { .. }
        | K::Clear { .. }
        | K::ClearScreen { .. }
        | K::ClearAll {}
        | K::PauseSource { .. }
        | K::SetVariant { .. }
        | K::TakePreview { .. }
        | K::StopFlow { .. } => {}
        K::SetFields { fields, .. } => {
            for k in fields.keys() {
                p.inputs.push(d(k, T::Text));
            }
        }
        K::SetTimer { .. } => p.inputs.push(d("ms", T::Number)),
        K::StepSource { .. } => p.inputs.push(d("index", T::Number)),
        K::Schedule { .. } => p.inputs.push(d("item", T::Row)),
        K::RunFlow { .. } => p.inputs.push(d("value", T::Text)),
        K::Log { .. } => p.inputs.push(d("text", T::Text)),
        K::Media { .. } => p.inputs.push(d("ms", T::Number)),
    }
    p
}

/// An input port by name.
pub fn input(kind: &NodeKind, name: &str) -> Option<Port> {
    ports(kind).inputs.into_iter().find(|p| p.name == name)
}

/// An output port by name. On `data.sourceRow`, any key-shaped name other than its fixed outputs
/// is a text column (checked against the source by `flows::warnings`).
pub fn output(kind: &NodeKind, name: &str) -> Option<Port> {
    let p = ports(kind);
    if let Some(port) = p.outputs.iter().find(|o| o.name == name) {
        return Some(port.clone());
    }
    (p.column_outputs && is_key(name)).then(|| Port::data(name, DataType::Text))
}

/// Whether a data input accepts a value of type `from`. `setFields` inputs are text, and also
/// take `rows` (for list fields).
pub fn accepts(kind: &NodeKind, port: &Port, from: DataType) -> bool {
    let Some(to) = port.ty else { return false };
    compatible(from, to) || (matches!(kind, NodeKind::SetFields { .. }) && from == DataType::Rows)
}

/// A data type's name as the UI shows it.
pub fn data_type_name(t: DataType) -> &'static str {
    match t {
        DataType::Text => "text",
        DataType::Number => "number",
        DataType::Bool => "bool",
        DataType::Time => "time",
        DataType::Row => "row",
        DataType::Rows => "rows",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{MathOp, PauseMode};

    fn names(ports: &[Port]) -> Vec<&str> {
        ports.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn type_names_match_serde_tags() {
        let kinds = [
            NodeKind::Cue { label: None },
            NodeKind::RunByFlow {},
            NodeKind::Rest {},
            NodeKind::Once {},
            NodeKind::PauseSource {
                source_id: "s".into(),
                mode: PauseMode::Toggle,
            },
            NodeKind::Math {
                op: MathOp::Add,
                a: 0.0,
                b: 0.0,
            },
        ];
        for k in kinds {
            let v = serde_json::to_value(&k).unwrap();
            assert_eq!(v["type"], type_name(&k));
        }
    }

    #[test]
    fn categories_and_shared_ports() {
        let cue = NodeKind::Cue { label: None };
        assert!(is_trigger(&cue));
        let p = ports(&cue);
        assert!(p.inputs.is_empty());
        assert_eq!(names(&p.outputs), ["fired", "value"]);
        let take = NodeKind::Take {
            screen_id: "s".into(),
            layer_id: "l".into(),
            for_ms: None,
        };
        assert!(is_action(&take));
        assert_eq!(names(&ports(&take).inputs), ["in"]);
        assert_eq!(names(&ports(&take).outputs), ["done", "failed"]);
        let delay = NodeKind::Delay { ms: 5 };
        assert!(is_logic(&delay) && is_wait(&delay));
        assert_eq!(names(&ports(&delay).inputs), ["in", "cancel", "ms"]);
        assert!(is_data(&NodeKind::TimeNow {}));
    }

    #[test]
    fn dynamic_ports() {
        let sw = NodeKind::Switch {
            cases: vec!["a".into(), "b".into()],
        };
        assert_eq!(names(&ports(&sw).outputs), ["case0", "case1", "default"]);
        let seq = NodeKind::Sequence { count: 3 };
        assert_eq!(names(&ports(&seq).outputs), ["s0", "s1", "s2"]);
        let rnd = NodeKind::Random {
            count: 2,
            no_repeat: true,
        };
        assert_eq!(names(&ports(&rnd).outputs), ["r0", "r1"]);
        let sf = NodeKind::SetFields {
            screen_id: "s".into(),
            layer_id: "l".into(),
            fields: [("name".to_string(), String::new())].into(),
        };
        assert_eq!(names(&ports(&sf).inputs), ["in", "name"]);
        let name = input(&sf, "name").unwrap();
        assert!(accepts(&sf, &name, DataType::Rows));
        assert!(accepts(&sf, &name, DataType::Number));
        assert!(!accepts(&sf, &name, DataType::Row));
        let sr = NodeKind::SourceRow {
            source_id: "s".into(),
            row: crate::core::model::RowPick::Current,
        };
        assert_eq!(output(&sr, "title").unwrap().ty, Some(DataType::Text));
        assert_eq!(output(&sr, "rows").unwrap().ty, Some(DataType::Rows));
        assert!(output(&sr, "Bad Column").is_none());
        let fmt = NodeKind::Format {
            pattern: "{a}".into(),
        };
        assert_eq!(ports(&fmt).inputs.len(), 8);
        let c = NodeKind::Constant {
            value: Constant::Bool(true),
        };
        assert_eq!(output(&c, "value").unwrap().ty, Some(DataType::Bool));
    }

    // ---- Port table fixture for the TypeScript editor (`ports.ts` parity) --------------------

    const ALL_TYPES: [DataType; 6] = [
        DataType::Text,
        DataType::Number,
        DataType::Bool,
        DataType::Time,
        DataType::Row,
        DataType::Rows,
    ];

    /// One sample per node type (dynamic configs non-trivial). Every node type must be here: the
    /// test fails when the count of distinct types differs from [`NODE_TYPES`].
    fn samples() -> Vec<NodeKind> {
        use crate::core::model::{
            CompareOp, ItemPick, ItemStatus, NowNext, RowPick, ScheduleOp, SourceStep, TimerAction,
        };
        use crate::core::protocol::{TimerDir, TimerEnd};
        let s = |v: &str| v.to_string();
        vec![
            NodeKind::Cue {
                label: Some(s("Go")),
            },
            NodeKind::ItemStatus {
                source_id: s("sched"),
                status: ItemStatus::Now,
                item_id: None,
            },
            NodeKind::BeforeNext {
                source_id: s("sched"),
                minutes: 5,
            },
            NodeKind::TimeOfDay {
                at: s("09:30"),
                days: vec![0, 4],
            },
            NodeKind::TimerZero {
                screen_id: s("main"),
                layer_id: s("l1"),
                key: s("clock"),
            },
            NodeKind::LayerAir {
                screen_id: s("main"),
                layer_id: s("l1"),
                on_air: true,
            },
            NodeKind::SlideshowWrap {
                source_id: s("slides"),
            },
            NodeKind::SourceRowChanged {
                source_id: s("guests"),
            },
            NodeKind::RunByFlow {},
            NodeKind::Rest {},
            NodeKind::ApiChanged {
                source_id: s("scores"),
                column: Some(s("home")),
            },
            NodeKind::Osc {
                address: s("/stage/*/go"),
            },
            NodeKind::Midi {
                device: None,
                channel: Some(1),
                kind: crate::core::model::MidiKind::Note,
                number: 36,
            },
            NodeKind::MediaEnded {
                screen_id: s("main"),
                layer_id: s("l1"),
            },
            NodeKind::SourceRow {
                source_id: s("guests"),
                row: RowPick::Current,
            },
            NodeKind::ScheduleItem {
                source_id: s("sched"),
                which: NowNext::Next,
            },
            NodeKind::FieldValue {
                screen_id: s("main"),
                layer_id: s("l1"),
                key: s("name"),
            },
            NodeKind::LayerState {
                screen_id: s("main"),
                layer_id: s("l1"),
            },
            NodeKind::Constant {
                value: Constant::Number(3.0),
            },
            NodeKind::TimeNow {},
            NodeKind::Format {
                pattern: s("{a} at {b:HH:mm}"),
            },
            NodeKind::Math {
                op: MathOp::Add,
                a: 1.0,
                b: 2.0,
            },
            NodeKind::Compare {
                op: CompareOp::Eq,
                a: s("x"),
                b: s("y"),
            },
            NodeKind::Branch {},
            NodeKind::Switch {
                cases: vec![s("red"), s("green"), s("blue")],
            },
            NodeKind::Gate { open: true },
            NodeKind::Delay { ms: 1000 },
            NodeKind::WaitUntil {
                at: Some(s("10:00")),
                timeout_ms: Some(60_000),
            },
            NodeKind::Debounce { ms: 500 },
            NodeKind::Throttle { ms: 500 },
            NodeKind::Sequence { count: 3 },
            NodeKind::Once {},
            NodeKind::Counter { every: 2 },
            NodeKind::Random {
                count: 3,
                no_repeat: true,
            },
            NodeKind::Repeat {
                count: Some(3),
                delay_ms: 500,
            },
            NodeKind::Take {
                screen_id: s("main"),
                layer_id: s("l1"),
                for_ms: Some(5000),
            },
            NodeKind::Clear {
                screen_id: s("main"),
                layer_id: s("l1"),
            },
            NodeKind::ClearScreen {
                screen_id: s("main"),
            },
            NodeKind::ClearAll {},
            NodeKind::SetFields {
                screen_id: s("main"),
                layer_id: s("l1"),
                fields: [(s("title"), s("Host")), (s("name"), s("Ada"))].into(),
            },
            NodeKind::SetTimer {
                screen_id: s("main"),
                layer_id: s("l1"),
                key: s("clock"),
                action: TimerAction::Configure {
                    dir: TimerDir::Down,
                    end: TimerEnd::Hide,
                },
            },
            NodeKind::StepSource {
                source_id: s("guests"),
                step: SourceStep::By { delta: 1 },
            },
            NodeKind::PauseSource {
                source_id: s("slides"),
                mode: PauseMode::Toggle,
            },
            NodeKind::Schedule {
                source_id: s("sched"),
                item: ItemPick::Input,
                op: ScheduleOp::Delay { minutes: 5 },
            },
            NodeKind::RunFlow {
                flow_id: s("other"),
                wait: true,
            },
            NodeKind::StopFlow {
                flow_id: s("other"),
            },
            NodeKind::SetVariant {
                screen_id: s("main"),
                layer_id: s("l1"),
                variant: s("compact"),
                transition: crate::core::model::VariantTransition::OutIn,
            },
            NodeKind::TakePreview {
                screen_id: s("main"),
                transition: None,
            },
            NodeKind::Log {
                message: s("hello"),
                toast: true,
            },
            NodeKind::Media {
                screen_id: s("main"),
                layer_id: s("l1"),
                action: crate::core::model::MediaAction::Seek,
                ms: Some(1500),
            },
        ]
    }

    /// Distinct node types (the `NodeKind` variants).
    const NODE_TYPES: usize = 50;

    fn port_json(kind: &NodeKind, p: &Port, input: bool) -> serde_json::Value {
        let mut v = serde_json::json!({
            "port": p.name,
            "kind": p.wire,
        });
        if let Some(t) = p.ty {
            v["dataType"] = serde_json::json!(t);
            if input {
                let accepts: Vec<DataType> = ALL_TYPES
                    .into_iter()
                    .filter(|from| accepts(kind, p, *from))
                    .collect();
                v["accepts"] = serde_json::json!(accepts);
            }
        }
        v
    }

    fn fixture() -> String {
        let nodes: Vec<serde_json::Value> = samples()
            .iter()
            .map(|k| {
                let p = ports(k);
                let tagged = serde_json::to_value(k).unwrap();
                serde_json::json!({
                    "type": type_name(k),
                    "config": tagged["config"],
                    "inputs": p.inputs.iter().map(|x| port_json(k, x, true)).collect::<Vec<_>>(),
                    "outputs": p.outputs.iter().map(|x| port_json(k, x, false)).collect::<Vec<_>>(),
                    "columnOutputs": p.column_outputs,
                })
            })
            .collect();
        let compatible: Vec<serde_json::Value> = ALL_TYPES
            .iter()
            .flat_map(|from| {
                ALL_TYPES.iter().map(move |to| {
                    serde_json::json!({ "from": from, "to": to, "ok": compatible(*from, *to) })
                })
            })
            .collect();
        let doc = serde_json::json!({
            "about": "Generated by `cargo test ports_fixture` (src-tauri/src/core/flow_nodes.rs) from the Rust port table; regenerate with UPDATE_PORTS_FIXTURE=1. One sample config per node type. Ports in order: { port, kind: event|data, dataType? (data only), accepts? (data inputs only: every source type a wire may carry into it, i.e. compatible(from, dataType) plus action.setFields taking rows) }. columnOutputs: data.sourceRow also has a text data output for every source column key ([a-z][a-z0-9_-]{0,31}, other than its fixed outputs), not listed. compatible: the implicit-conversion table for every DataType pair.",
            "nodes": nodes,
            "compatible": compatible,
        });
        let mut text = serde_json::to_string_pretty(&doc).unwrap();
        text.push('\n');
        text
    }

    #[test]
    fn ports_fixture() {
        let kinds = samples();
        let types: std::collections::HashSet<&str> = kinds.iter().map(type_name).collect();
        assert_eq!(kinds.len(), NODE_TYPES, "one sample per node type");
        assert_eq!(types.len(), NODE_TYPES, "every node type once");
        for k in &kinds {
            crate::core::flows::check_config(k).unwrap();
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../src/lib/components/flows/graph/ports.fixture.json");
        let want = fixture();
        if std::env::var("UPDATE_PORTS_FIXTURE").is_ok_and(|v| v == "1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &want).unwrap();
            return;
        }
        let have = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            have == want,
            "{} is out of date with the Rust port table: run UPDATE_PORTS_FIXTURE=1 cargo test ports_fixture",
            path.display()
        );
    }

    #[test]
    fn implicit_conversion_only_to_text() {
        use DataType as T;
        for t in [T::Number, T::Bool, T::Time, T::Text] {
            assert!(compatible(t, T::Text));
        }
        assert!(!compatible(T::Row, T::Text));
        assert!(!compatible(T::Text, T::Number));
        assert!(!compatible(T::Number, T::Bool));
        assert!(compatible(T::Rows, T::Rows));
    }
}
