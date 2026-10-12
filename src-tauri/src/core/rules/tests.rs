//! Schedules and rules through `AppState`, with a skewed clock (schedule time = real time +
//! skew, zone UTC) so boundaries can be put a few hundred ms away. Tasks run on a runtime
//! injected into the hub, as in the app.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::*;
use crate::core::events::EventSink;
use crate::core::live::FieldDef;
use crate::core::localtime::{parse_local, Zone};
use crate::core::model::{
    AutoAdvance, AutoTransition, FlowEdge, FlowNode, ItemPick, ItemState, NewFlow, NewScreen,
    NodeKind, PortRef, Rect, Retrigger, RowSelector, SaveFlow, ScheduleItem, ScheduleOp, ScreenDoc,
    ScreenLayer, ScreenLayerKind, SourceBinding, SourceColumn, SourceDoc, TemplateDoc,
    TemplateSource, TimerAction, WireKind,
};
use crate::core::protocol::{FieldType, Fields, TimerEnd};

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

    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
    st: AppState,
    rec: Arc<Recorder>,
    dir: PathBuf,
}

const Z: Zone = Zone::Fixed(0);

/// Epoch ms of `HH:MM[:SS]` on 2026-10-05 (UTC).
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
        _rt: rt,
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
        std::thread::sleep(Duration::from_millis(10));
    }
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

fn field(key: &str, kind: FieldType) -> FieldDef {
    FieldDef {
        key: key.into(),
        label: key.to_uppercase(),
        kind,
        ..Default::default()
    }
}

fn item(id: &str, start: &str, end: &str) -> ScheduleItem {
    ScheduleItem {
        id: id.into(),
        title: format!("Item {id}"),
        subtitle: String::new(),
        location: String::new(),
        start: format!("2026-10-05T{start}"),
        end: format!("2026-10-05T{end}"),
        delay_min: 0,
        state: ItemState::Auto,
        started_at: None,
        ended_at: None,
    }
}

fn bind(source: &str, column: &str, sel: Option<RowSelector>) -> SourceBinding {
    SourceBinding {
        source_id: source.into(),
        column: column.into(),
        sort: None,
        limit: None,
        row: sel,
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
        id: format!("e{}", EDGE.fetch_add(1, Ordering::Relaxed)),
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

    /// A/B/C: 13:00–13:30, 13:30–14:30, 14:30–15:00.
    fn schedule(&self) -> SourceDoc {
        self.schedule_of(vec![
            item("a", "13:00", "13:30"),
            item("b", "13:30", "14:30"),
            item("c", "14:30", "15:00"),
        ])
    }

    fn schedule_of(&self, items: Vec<ScheduleItem>) -> SourceDoc {
        let mut doc = self.st.create_schedule("Run of show").unwrap();
        let SourceKind::Schedule { items: slot, .. } = &mut doc.meta.kind else {
            panic!("not a schedule")
        };
        *slot = items;
        self.st.save_source(doc).unwrap()
    }

    /// Fields: `items` (list: title, status), `now_title`, `next_title`, `cd` (timer), `t`
    /// (timer, unbound).
    fn template(&self) -> TemplateDoc {
        let mut doc = self
            .st
            .templates()
            .create(&self.dir, "Board", &TemplateSource::Blank)
            .unwrap();
        let mut items = field("items", FieldType::List);
        items.columns = vec![col("title"), col("status")];
        doc.meta.fields = vec![
            items,
            field("now_title", FieldType::Text),
            field("next_title", FieldType::Text),
            field("cd", FieldType::Timer),
            field("t", FieldType::Timer),
        ];
        doc.variants[0].html =
            "<div>{{now_title}}<ol><li data-each=\"items\">{{.title}}</li></ol></div>".into();
        self.st.templates().save(&self.dir, doc).unwrap()
    }

    /// A screen with layers `l1` (bound to `src` if given) and `l2` (unbound).
    fn screen(&self, t: &str, src: Option<&str>) -> ScreenDoc {
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
        if let Some(s) = src {
            bindings.insert("items".to_string(), bind(s, "", None));
            bindings.insert(
                "now_title".to_string(),
                bind(s, "title", Some(RowSelector::Now)),
            );
            bindings.insert(
                "next_title".to_string(),
                bind(s, "title", Some(RowSelector::Next)),
            );
            bindings.insert(
                "cd".to_string(),
                bind(s, "countdown", Some(RowSelector::Next)),
            );
        }
        for (id, b) in [("l1", bindings), ("l2", BTreeMap::new())] {
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
                    template_id: t.into(),
                    fields: Fields::new(),
                    vars: Fields::new(),
                    bindings: b,
                    variant: None,
                    sizing: None,
                    opacity: None,
                },
                anchor: None,
            });
        }
        self.st.save_screen(doc).unwrap().doc
    }

    fn field(&self, screen: &str, layer: &str, key: &str) -> String {
        self.st
            .live()
            .with_layer(screen, layer, |l| l.fields[key].clone())
            .unwrap()
    }

    fn on_air(&self, screen: &str, layer: &str) -> bool {
        self.st.live().is_on_air(screen, layer)
    }

    /// A flow (validated and deployed through the store), armed.
    fn flow(&self, name: &str, trigger: NodeKind, steps: Vec<NodeKind>) -> String {
        self.flow_armed(name, trigger, steps, true)
    }

    /// A trigger wired through `steps` in a line (a rule or a macro as a flow).
    fn flow_armed(
        &self,
        name: &str,
        trigger: NodeKind,
        steps: Vec<NodeKind>,
        armed: bool,
    ) -> String {
        let (nodes, edges) = crate::core::flows::linear(trigger, steps);
        self.st
            .create_flow(NewFlow {
                name: name.into(),
                armed,
                retrigger: Retrigger::Restart,
                origin: None,
                nodes,
                edges,
            })
            .unwrap()
            .flow
            .id
    }

    /// A flow from nodes and edges, armed.
    fn create(&self, name: &str, nodes: Vec<FlowNode>, edges: Vec<FlowEdge>) -> String {
        self.st
            .create_flow(NewFlow {
                name: name.into(),
                armed: true,
                retrigger: Retrigger::Restart,
                origin: None,
                nodes,
                edges,
            })
            .unwrap()
            .flow
            .id
    }

    /// The end states (`finished`, `failed`) of a flow's instances.
    fn fired(&self, flow: &str) -> Vec<Value> {
        self.rec
            .named(names::FLOW_STATE)
            .into_iter()
            .filter(|v| v["id"] == flow && (v["state"] == "finished" || v["state"] == "failed"))
            .collect()
    }
}

fn take(screen: &str, layer: &str, for_ms: Option<u32>) -> NodeKind {
    NodeKind::Take {
        screen_id: screen.into(),
        layer_id: layer.into(),
        for_ms,
    }
}

// ---- Bindings ----------------------------------------------------------------------------------

#[test]
fn schedule_bindings_rows_selectors_and_countdown() {
    let e = env();
    let tpl = e.template();
    let src = e.schedule();
    let s = e.screen(&tpl.id, Some(&src.id));
    assert!(
        e.st.get_screen(&s.id).unwrap().warnings.is_empty(),
        "{:?}",
        e.st.get_screen(&s.id).unwrap().warnings
    );
    let items: Value = serde_json::from_str(&e.field(&s.id, "l1", "items")).unwrap();
    assert_eq!(items.as_array().unwrap().len(), 3);
    assert_eq!(items[1]["title"], "Item b");
    assert_eq!(items[1]["status"], "now");
    assert_eq!(items[1]["_status"], "now");
    assert_eq!(items[0]["_status"], "previous");
    assert_eq!(items[2]["_status"], "next");
    assert_eq!(e.field(&s.id, "l1", "now_title"), "Item b");
    assert_eq!(e.field(&s.id, "l1", "next_title"), "Item c");
    let cd = fieldvals::parse_timer(&e.field(&s.id, "l1", "cd")).unwrap();
    let now = e.st.sources().clock().now();
    assert!(cd.run);
    assert_eq!(cd.at + cd.base, now.to_real(t("14:30")) as u64);
    // Summary: NOW and UP NEXT, the NOW item as the current row, schedule columns.
    let sum = e.st.list_sources().unwrap();
    let sum = sum.iter().find(|x| x.id == src.id).unwrap();
    assert_eq!(sum.now_id.as_deref(), Some("b"));
    assert_eq!(sum.next_id.as_deref(), Some("c"));
    assert_eq!(sum.current["title"], "Item b");
    assert_eq!(sum.row_count, 3);
    assert!(sum.columns.iter().any(|c| c.key == "countdown"));
    // A schedule can't step or auto-advance.
    assert!(matches!(
        e.st.step_source(&src.id, crate::core::model::SourceStep::By { delta: 1 }),
        Err(AppError::Invalid(_))
    ));
    assert!(matches!(
        e.st.set_auto_advance(
            &src.id,
            Some(AutoAdvance {
                every_ms: 1000,
                looped: true,
                transition: AutoTransition::Update,
                shuffle: false,
                paused: false,
                item_durations: false,
            })
        ),
        Err(AppError::Invalid(_))
    ));
    // After the last item: no NOW / NEXT, the fields are empty, the countdown stopped.
    e.set_now("16:00", 0);
    e.st.save_screen(e.st.get_screen(&s.id).unwrap().doc)
        .unwrap();
    assert_eq!(e.field(&s.id, "l1", "now_title"), "");
    assert_eq!(e.field(&s.id, "l1", "next_title"), "");
    assert_eq!(e.field(&s.id, "l1", "cd"), schedule::countdown_zero());
    assert!(e.st.get_screen(&s.id).unwrap().warnings.is_empty());
}

#[test]
fn view_and_validation() {
    let e = env();
    let src = e.schedule();
    let v = e.st.schedule_view(&src.id).unwrap();
    assert_eq!(v.now_id.as_deref(), Some("b"));
    assert_eq!(v.items[2].title, "Item c");
    assert!(!v.can_undo);
    // Invalid schedules are refused.
    let mut doc = e.st.get_source(&src.id).unwrap();
    let SourceKind::Schedule { items, .. } = &mut doc.meta.kind else {
        panic!()
    };
    items[0].end = items[0].start.clone();
    assert!(matches!(e.st.save_source(doc), Err(AppError::Invalid(_))));
    // A text source has no schedule view.
    let text = e.st.create_source("Text").unwrap();
    assert!(matches!(
        e.st.schedule_view(&text.id),
        Err(AppError::Invalid(_))
    ));
}

// ---- Boundaries ------------------------------------------------------------------------------

#[test]
fn a_boundary_wakes_the_planner_and_updates_on_air_layers() {
    let e = env();
    let tpl = e.template();
    let src = e.schedule();
    let s = e.screen(&tpl.id, Some(&src.id));
    e.st.live().show(&s.id, "l1").unwrap();
    e.set_now("14:30", 400);
    assert_eq!(e.st.rules().next_wake(), Some(t("14:30")));
    assert_eq!(e.st.rules().task_count(), 1, "one planner task");
    e.rec.clear();
    wait_until("the boundary", || {
        e.field(&s.id, "l1", "now_title") == "Item c"
    });
    // On air under onTake: forced (not staged).
    assert!(!e.st.live().layer_view(&s.id, "l1").unwrap().pending);
    assert_eq!(e.field(&s.id, "l1", "next_title"), "");
    let views = e.rec.named(names::SCHEDULE_CHANGED);
    assert!(!views.is_empty());
    assert_eq!(views.last().unwrap()["nowId"], "c");
    assert!(views.last().unwrap()["nextId"].is_null());
    // Next boundary: C's end; after it nothing is ahead and no task is left. The tick updates
    // the layer before it re-plans, so wait for the new wake.
    wait_until("the next wake", || {
        e.st.rules().next_wake() == Some(t("15:00"))
    });
    e.set_now("15:00", 300);
    wait_until("the last boundary", || {
        e.field(&s.id, "l1", "now_title").is_empty()
    });
    wait_until("no planner", || e.st.rules().task_count() == 0);
    assert_eq!(e.st.rules().next_wake(), None);
}

#[test]
fn unwatched_schedules_and_idle_projects_have_no_task() {
    let e = env();
    assert_eq!(e.st.rules().task_count(), 0);
    let src = e.schedule();
    assert_eq!(
        e.st.rules().task_count(),
        0,
        "not bound and not in a recipe: not watched"
    );
    // Binding it to a screen arms the planner at once (B ends at 14:30).
    let tpl = e.template();
    e.screen(&tpl.id, Some(&src.id));
    assert_eq!(e.st.rules().next_wake(), Some(t("14:30")));
}

// ---- Actions ---------------------------------------------------------------------------------

#[test]
fn actions_apply_at_once_save_coalesced_and_undo_one_level() {
    let e = env();
    let tpl = e.template();
    let src = e.schedule();
    let s = e.screen(&tpl.id, Some(&src.id));
    e.st.live().show(&s.id, "l1").unwrap();
    e.rec.clear();
    let v =
        e.st.schedule_action(
            &src.id,
            ScheduleAction::StartNow {
                item_id: "c".into(),
            },
        )
        .unwrap();
    assert_eq!(v.now_id.as_deref(), Some("c"));
    assert!(v.can_undo);
    assert_eq!(
        e.field(&s.id, "l1", "now_title"),
        "Item c",
        "on air at once"
    );
    assert!(!e.st.live().layer_view(&s.id, "l1").unwrap().pending);
    assert!(!e.rec.named(names::SCHEDULE_CHANGED).is_empty());
    let changed = e.rec.named(names::SOURCE_CHANGED);
    assert_eq!(changed.last().unwrap()["nowId"], "c");
    // In memory until the content flusher writes it.
    assert!(e.st.sources().is_dirty(&src.id));
    e.st.flush_all();
    let file = crate::core::sources::get(&e.dir, &src.id).unwrap();
    let SourceKind::Schedule { items, .. } = &file.meta.kind else {
        panic!()
    };
    assert!(items[2].started_at.is_some());
    assert!(items[1].ended_at.is_some());
    // Undo: back to the clock; one level only.
    let v = e.st.schedule_action(&src.id, ScheduleAction::Undo).unwrap();
    assert_eq!(v.now_id.as_deref(), Some("b"));
    assert!(!v.can_undo);
    assert_eq!(e.field(&s.id, "l1", "now_title"), "Item b");
    assert!(matches!(
        e.st.schedule_action(&src.id, ScheduleAction::Undo),
        Err(AppError::Invalid(_))
    ));
    // Delay and set next.
    let v =
        e.st.schedule_action(
            &src.id,
            ScheduleAction::Delay {
                item_id: "c".into(),
                minutes: 10,
            },
        )
        .unwrap();
    let c = v.items.iter().find(|i| i.id == "c").unwrap();
    assert_eq!(c.status, ItemStatus::Delayed);
    assert_eq!(c.start, "2026-10-05T14:40");
    assert!(c.moved);
    // An edit drops the undo.
    let doc = e.st.get_source(&src.id).unwrap();
    e.st.save_source(doc).unwrap();
    assert!(!e.st.schedule_view(&src.id).unwrap().can_undo);
    // Unknown items and text sources.
    assert!(matches!(
        e.st.schedule_action(
            &src.id,
            ScheduleAction::End {
                item_id: "zz".into()
            }
        ),
        Err(AppError::NotFound(_))
    ));
}

// ---- Triggers (recipe rules are flows) ----------------------------------------------------------

fn item_status(src: &str, status: ItemStatus, item: Option<&str>) -> NodeKind {
    NodeKind::ItemStatus {
        source_id: src.into(),
        status,
        item_id: item.map(str::to_string),
    }
}

#[test]
fn item_status_triggers_fire_once_take_for_a_while_and_persist_last_fired() {
    let e = env();
    let tpl = e.template();
    let src = e.schedule();
    let s = e.screen(&tpl.id, Some(&src.id));
    let now = e.flow(
        "now",
        item_status(&src.id, ItemStatus::Now, None),
        vec![take(&s.id, "l2", Some(300))],
    );
    let only_a = e.flow(
        "only-a",
        item_status(&src.id, ItemStatus::Now, Some("a")),
        vec![take(&s.id, "l1", None)],
    );
    let off = e.flow_armed(
        "off",
        item_status(&src.id, ItemStatus::Now, None),
        vec![take(&s.id, "l1", None)],
        false,
    );
    assert!(e.fired(&now).is_empty(), "loading is a baseline");
    e.set_now("14:30", 300);
    wait_until("the flow's take", || e.on_air(&s.id, "l2"));
    wait_until("finished", || !e.fired(&now).is_empty());
    let end = &e.fired(&now)[0];
    assert_eq!(end["state"], "finished");
    assert!(end["message"].is_null());
    wait_until("the timed clear", || !e.on_air(&s.id, "l2"));
    assert!(
        !e.on_air(&s.id, "l1"),
        "disarmed and other-item triggers never fire"
    );
    assert!(e.fired(&off).is_empty() && e.fired(&only_a).is_empty());
    // lastFiredAt: in memory at once, on disk with the next flush.
    assert!(e.st.get_flow(&now).unwrap().flow.last_fired_at.is_some());
    e.st.flush_all();
    let (file, _) = crate::core::flows::read_flow(&e.dir, &now).unwrap();
    assert!(file.last_fired_at.is_some());
    // The same transition again (undo + redo): never twice for the same item.
    e.st.schedule_action(
        &src.id,
        ScheduleAction::Auto {
            item_id: "c".into(),
        },
    )
    .unwrap();
    e.st.schedule_action(
        &src.id,
        ScheduleAction::End {
            item_id: "c".into(),
        },
    )
    .unwrap();
    e.st.schedule_action(&src.id, ScheduleAction::Undo).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(e.fired(&now).len(), 1);
    assert!(e.st.rules().task_count() <= 1, "no clear pending");
    assert_eq!(e.st.flow_engine().task_count(), 0, "no instance left");
}

#[test]
fn before_next_and_time_of_day_triggers() {
    let e = env();
    let tpl = e.template();
    let src = e.schedule();
    let s = e.screen(&tpl.id, None);
    let before = e.flow(
        "before",
        NodeKind::BeforeNext {
            source_id: src.id.clone(),
            minutes: 1,
        },
        vec![take(&s.id, "l1", None)],
    );
    let tod = e.flow(
        "tod",
        NodeKind::TimeOfDay {
            at: "14:29".into(),
            days: vec![0],
        },
        vec![take(&s.id, "l2", None)],
    );
    let other = e.flow(
        "tod-other-day",
        NodeKind::TimeOfDay {
            at: "14:29".into(),
            days: vec![3],
        },
        vec![NodeKind::Clear {
            screen_id: s.id.clone(),
            layer_id: "l2".into(),
        }],
    );
    // 2026-10-05 is a Monday (0). At 14:29 (one minute before C at 14:30) both fire.
    e.set_now("14:29", 300);
    assert_eq!(e.st.rules().next_wake(), Some(t("14:29")));
    wait_until("beforeNext", || e.on_air(&s.id, "l1"));
    wait_until("timeOfDay", || e.on_air(&s.id, "l2"));
    wait_until("both finished", || {
        e.fired(&before).len() == 1 && e.fired(&tod).len() == 1
    });
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(e.fired(&before).len(), 1);
    assert_eq!(e.fired(&tod).len(), 1);
    assert!(e.fired(&other).is_empty());
    // Re-planned past them: the next wake is C's start, not 14:29 again.
    assert_eq!(e.st.rules().next_wake(), Some(t("14:30")));
}

#[test]
fn editing_a_time_of_day_plans_the_new_time() {
    let e = env();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    e.set_now("14:00", 0);
    let tod = |at: &str| NodeKind::TimeOfDay {
        at: at.into(),
        days: vec![],
    };
    let id = e.flow("tod", tod("16:00"), vec![take(&s.id, "l1", None)]);
    assert_eq!(e.st.rules().next_wake(), Some(t("16:00")));
    // Earlier and later than the old time: the wake follows the edit (it used to keep 16:00).
    // Same node ids, as the editor saves it.
    for at in ["15:00", "17:00"] {
        let f = e.st.get_flow(&id).unwrap().flow;
        let mut nodes = f.nodes;
        nodes[0].kind = tod(at);
        e.st.save_flow(SaveFlow {
            id: id.clone(),
            name: "tod".into(),
            retrigger: Retrigger::Restart,
            nodes,
            edges: f.edges,
        })
        .unwrap();
        assert_eq!(e.st.rules().next_wake(), Some(t(at)), "{at}");
    }
    // It fires at the new time.
    e.set_now("17:00", 300);
    wait_until("timeOfDay", || e.on_air(&s.id, "l1"));
}

#[test]
fn before_next_does_not_fire_for_what_is_already_due_on_load() {
    let e = env();
    let src = e.schedule();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    e.set_now("14:29", -100);
    let before = e.flow(
        "before",
        NodeKind::BeforeNext {
            source_id: src.id.clone(),
            minutes: 5,
        },
        vec![take(&s.id, "l1", None)],
    );
    std::thread::sleep(Duration::from_millis(200));
    assert!(e.fired(&before).is_empty());
    // Moving the next item doesn't make it fire: once per item.
    e.st.schedule_action(
        &src.id,
        ScheduleAction::SetNext {
            item_id: "c".into(),
            start: Some("2026-10-05T14:31".into()),
        },
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(e.fired(&before).is_empty(), "same item: once per item");
}

#[test]
fn timer_zero_and_layer_triggers() {
    let e = env();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    let other = e.screen(&tpl.id, None);
    let zero = e.flow(
        "zero",
        NodeKind::TimerZero {
            screen_id: s.id.clone(),
            layer_id: "l1".into(),
            key: "t".into(),
        },
        vec![take(&other.id, "l1", None)],
    );
    let on = e.flow(
        "on",
        NodeKind::LayerAir {
            screen_id: s.id.clone(),
            layer_id: "l2".into(),
            on_air: true,
        },
        vec![NodeKind::SetFields {
            screen_id: other.id.clone(),
            layer_id: "l2".into(),
            fields: row(&[("now_title", "fired")]),
        }],
    );
    let off = e.flow(
        "off",
        NodeKind::LayerAir {
            screen_id: s.id.clone(),
            layer_id: "l2".into(),
            on_air: false,
        },
        vec![take(&other.id, "l2", None)],
    );
    // A countdown (end: stop) reaching zero.
    let set = |a| e.st.set_timer(&s.id, "l1", "t", a).unwrap();
    set(TimerAction::Configure {
        dir: TimerDir::Down,
        end: TimerEnd::Stop,
    });
    set(TimerAction::Set { ms: 600 });
    set(TimerAction::Start);
    assert!(e.st.rules().next_wake().is_some());
    wait_until("timerZero", || e.on_air(&other.id, "l1"));
    wait_until("zero finished", || e.fired(&zero).len() == 1);
    // Layer on air.
    e.st.live().show(&s.id, "l2").unwrap();
    wait_until("layer on", || {
        e.field(&other.id, "l2", "now_title") == "fired"
    });
    wait_until("on finished", || e.fired(&on).len() == 1);
    assert!(e.fired(&off).is_empty());
    // PANIC: off air, but layer triggers don't fire for it.
    e.st.live_panic().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(e.fired(&off).is_empty());
    assert!(!e.on_air(&other.id, "l2"));
    // A normal clear does fire it.
    e.st.live().show(&s.id, "l2").unwrap();
    e.st.live().hide(&s.id, "l2").unwrap();
    wait_until("layer off", || e.on_air(&other.id, "l2"));
}

#[test]
fn missing_targets_are_skipped_and_reported() {
    let e = env();
    let src = e.schedule();
    let tpl = e.template();
    let s = e.screen(&tpl.id, Some(&src.id));
    let id = e.flow(
        "r",
        item_status(&src.id, ItemStatus::Now, None),
        vec![
            take(&s.id, "nolayer01", None),
            NodeKind::RunFlow {
                flow_id: "noflow0001".into(),
                wait: false,
            },
            take(&s.id, "l2", None),
        ],
    );
    e.st.schedule_action(
        &src.id,
        ScheduleAction::StartNow {
            item_id: "c".into(),
        },
    )
    .unwrap();
    wait_until("fired", || !e.fired(&id).is_empty());
    let end = &e.fired(&id)[0];
    assert_eq!(end["state"], "finished", "{end}");
    let msg = end["message"].as_str().unwrap().to_string();
    assert_eq!(msg.matches("skipped").count(), 2, "{msg}");
    assert!(e.on_air(&s.id, "l2"), "later actions still run");
}

#[test]
fn panic_cancels_pending_clears_and_close_stops_everything() {
    let e = env();
    let src = e.schedule();
    let tpl = e.template();
    let s = e.screen(&tpl.id, Some(&src.id));
    e.flow(
        "r",
        item_status(&src.id, ItemStatus::Now, None),
        vec![take(&s.id, "l2", Some(60_000))],
    );
    e.st.schedule_action(
        &src.id,
        ScheduleAction::StartNow {
            item_id: "c".into(),
        },
    )
    .unwrap();
    wait_until("taken", || e.on_air(&s.id, "l2"));
    // Planner (C's end) + the pending clear.
    wait_until("clear pending", || e.st.rules().task_count() == 2);
    e.st.live_panic().unwrap();
    assert_eq!(e.st.rules().task_count(), 1, "the clear is cancelled");
    e.st.close_project();
    assert_eq!(e.st.rules().task_count(), 0);
    assert_eq!(e.st.flow_engine().task_count(), 0);
    assert!(e.st.rules().instances().is_empty());
}

#[test]
fn deploy_keeps_timed_clears_and_arming_does_not_catch_up() {
    let e = env();
    let src = e.schedule();
    let tpl = e.template();
    let s = e.screen(&tpl.id, None);
    let id = e.flow(
        "r",
        item_status(&src.id, ItemStatus::Now, None),
        vec![take(&s.id, "l2", Some(400))],
    );
    e.st.schedule_action(
        &src.id,
        ScheduleAction::StartNow {
            item_id: "c".into(),
        },
    )
    .unwrap();
    wait_until("taken", || e.on_air(&s.id, "l2"));
    wait_until("clear pending", || e.st.rules().task_count() == 2);
    // A redeploy (another graph) keeps the layer's pending clear.
    let flow = e.st.get_flow(&id).unwrap().flow;
    let mut nodes = flow.nodes.clone();
    nodes.push(node(
        "extra00001",
        NodeKind::Log {
            message: "x".into(),
            toast: false,
        },
    ));
    e.st.save_flow(SaveFlow {
        id: id.clone(),
        name: flow.name.clone(),
        retrigger: flow.retrigger,
        nodes,
        edges: flow.edges.clone(),
    })
    .unwrap();
    wait_until("the timed clear still happens", || !e.on_air(&s.id, "l2"));
    // Disarmed while another item becomes NOW; arming again doesn't catch up.
    e.st.set_flow_armed(&id, false).unwrap();
    e.st.schedule_action(
        &src.id,
        ScheduleAction::StartNow {
            item_id: "a".into(),
        },
    )
    .unwrap();
    e.st.set_flow_armed(&id, true).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(!e.on_air(&s.id, "l2"), "nothing caught up");
    assert_eq!(e.fired(&id).len(), 1);
}

#[test]
fn schedule_actions_of_flows_are_undoable() {
    let e = env();
    let src = e.schedule();
    let id = e.flow(
        "cue",
        NodeKind::Cue { label: None },
        vec![NodeKind::Schedule {
            source_id: src.id.clone(),
            item: ItemPick::Next,
            op: ScheduleOp::StartNow,
        }],
    );
    e.st.run_flow(&id, None).unwrap();
    wait_until("finished", || !e.fired(&id).is_empty());
    let v = e.st.schedule_view(&src.id).unwrap();
    assert_eq!(v.now_id.as_deref(), Some("c"), "UP NEXT started");
    assert!(v.can_undo);
    let v = e.st.schedule_action(&src.id, ScheduleAction::Undo).unwrap();
    assert_eq!(
        v.now_id.as_deref(),
        Some("b"),
        "the operator's Undo reverts it"
    );
    // NOW.
    let delay = e.flow(
        "delay",
        NodeKind::Cue { label: None },
        vec![NodeKind::Schedule {
            source_id: src.id.clone(),
            item: ItemPick::Now,
            op: ScheduleOp::Delay { minutes: 5 },
        }],
    );
    e.st.run_flow(&delay, None).unwrap();
    wait_until("finished", || !e.fired(&delay).is_empty());
    let v = e.st.schedule_view(&src.id).unwrap();
    assert!(v.items.iter().any(|i| i.id == "b" && i.moved));
    // The item row of an itemStatus trigger (`_id`).
    let by_row = e.create(
        "row",
        vec![
            node("t", item_status(&src.id, ItemStatus::Now, None)),
            node(
                "a",
                NodeKind::Schedule {
                    source_id: src.id.clone(),
                    item: ItemPick::Input,
                    op: ScheduleOp::Cancel,
                },
            ),
        ],
        vec![ev("t", "fired", "a", "in"), data("t", "item", "a", "item")],
    );
    e.st.schedule_action(
        &src.id,
        ScheduleAction::StartNow {
            item_id: "c".into(),
        },
    )
    .unwrap();
    wait_until("finished", || !e.fired(&by_row).is_empty());
    let v = e.st.schedule_view(&src.id).unwrap();
    assert_eq!(
        v.items.iter().find(|i| i.id == "c").unwrap().status,
        ItemStatus::Cancelled
    );
}

#[test]
fn next_time_of_day_respects_days() {
    let now = Now::at(t("14:00"), Z);
    assert_eq!(next_time_of_day("15:00", &[], &now), Some(t("15:00")));
    assert_eq!(
        next_time_of_day("13:00", &[], &now),
        Some(t("13:00") + 86_400_000),
        "tomorrow"
    );
    // Monday 5 Oct; Wednesday = 2.
    assert_eq!(
        next_time_of_day("09:00", &[2], &now),
        Some(t("09:00") + 2 * 86_400_000)
    );
    assert_eq!(next_time_of_day("9:00", &[], &now), None);
}
