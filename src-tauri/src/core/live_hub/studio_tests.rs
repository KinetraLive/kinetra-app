//! Studio mode through the hub: the preview bus, T-bar pacing, PANIC, `preview-changed`, the
//! exits and the single entry point.

use serde_json::{json, Value};
use tokio::sync::broadcast::Receiver;

use super::*;
use crate::core::live::LayerPlan;
use crate::core::protocol::TakeTransitionKind;

/// A hub with the test screen and a user screen `main` (layers `a`, `b`), Studio mode on/off.
fn hub(studio: bool) -> LiveHub {
    let mut live = LiveState::with_test_screen();
    let mut main = live.screens[0].clone();
    main.info.id = "main".into();
    main.info.name = "Main".into();
    main.layers[0].id = "a".into();
    let mut b = main.layers[0].clone();
    b.id = "b".into();
    b.z = 1;
    main.layers.push(b);
    live.screens.push(main);
    live.studio = studio;
    LiveHub::new(live)
}

const CUT: TakeTransition = TakeTransition {
    kind: TakeTransitionKind::Cut,
    ms: 300,
};
const KEEP: PreviewAfterTake = PreviewAfterTake::Keep;

/// `(preview bus?, message)` for `main`, received so far.
fn drain(rx: &mut Receiver<Arc<HubMsg>>) -> Vec<(bool, Value)> {
    let mut out = Vec::new();
    while let Ok(m) = rx.try_recv() {
        if &*m.screen_id == "main" {
            out.push((m.preview, serde_json::from_str(&m.json).unwrap()));
        }
    }
    out
}

fn kinds(msgs: &[(bool, Value)]) -> Vec<(bool, String)> {
    msgs.iter()
        .map(|(p, v)| (*p, v["t"].as_str().unwrap().to_string()))
        .collect()
}

fn cue<'a>(layer_id: &'a str, visible: Option<bool>) -> StudioAction<'a> {
    StudioAction::Cue { layer_id, visible }
}

/// The plan of `main` with `name` of layer `layer` set (the document holds staged edits too).
fn edit_plan(hub: &LiveHub, layer: &str, name: &str) -> ScreenPlan {
    let state = hub.read();
    let s = state.screen("main").unwrap();
    ScreenPlan {
        info: s.info.clone(),
        layers: s
            .layers
            .iter()
            .map(|l| {
                let mut content = s.preview.staged(&l.id).cloned().unwrap_or(l.content());
                if l.id == layer {
                    content.fields.insert("name".into(), name.into());
                }
                LayerPlan {
                    id: l.id.clone(),
                    name: l.name.clone(),
                    content,
                }
            })
            .collect(),
    }
}

#[test]
fn preview_bus_gets_program_plus_diff_only_while_subscribed() {
    let hub = hub(true);
    let mut rx = hub.subscribe();
    assert_eq!(hub.preview_buses(), 0);
    hub.studio_op("main", cue("b", Some(true))).unwrap();
    assert!(
        drain(&mut rx).is_empty(),
        "no preview subscriber: nothing sent"
    );
    assert_eq!(hub.preview_buses(), 0, "nothing derived");

    let (sub, snap) = hub.preview_attach("main");
    assert_eq!(hub.preview_buses(), 1);
    let snap: Value = serde_json::from_str(&snap.unwrap()).unwrap();
    assert_eq!(snap["t"], "snapshot");
    assert_eq!(snap["layers"][0]["state"], "out");
    assert_eq!(snap["layers"][1]["state"], "in", "cued in: on in Preview");
    assert_eq!(
        hub.preview_snapshot_json("main").map(|s| s.len()),
        Some(serde_json::to_string(&snap).unwrap().len())
    );
    // Program's snapshot is unchanged.
    let prog: Value = serde_json::from_str(&hub.snapshot_json("main").unwrap()).unwrap();
    assert_eq!(prog["layers"][1]["state"], "out");

    hub.studio_op("main", cue("a", Some(true))).unwrap();
    assert_eq!(
        drain(&mut rx),
        [(true, json!({"t": "state", "layerId": "a", "state": "in"}))],
        "the preview bus only"
    );
    // A staged edit reaches the preview bus as a `fields` diff; Program gets nothing.
    hub.apply_screen(edit_plan(&hub, "a", "Ada"), LiveEdit::Immediate)
        .unwrap();
    assert_eq!(
        drain(&mut rx),
        [(
            true,
            json!({"t": "fields", "layerId": "a", "fields": {"name": "Ada"}})
        )]
    );
    // A take: one `take` on Program; the Preview (keep) already showed it: nothing there.
    hub.studio_op(
        "main",
        StudioAction::Take {
            transition: CUT,
            after: KEEP,
        },
    )
    .unwrap();
    let got = drain(&mut rx);
    assert_eq!(kinds(&got), [(false, "take".to_string())], "{got:?}");
    // Program changes show on the preview bus too (Preview = Program + diff).
    hub.hide("main", "b").unwrap();
    assert_eq!(
        kinds(&drain(&mut rx)),
        [(false, "state".to_string()), (true, "state".to_string())]
    );

    // Second subscriber shares the derived screen; the last one gone stops it.
    let (sub2, _) = hub.preview_attach("main");
    drop(sub);
    assert_eq!(hub.preview_buses(), 1);
    drop(sub2);
    assert_eq!(hub.preview_buses(), 0);
    hub.studio_op("main", cue("b", Some(true))).unwrap();
    assert!(drain(&mut rx).is_empty());
}

#[test]
fn preview_bus_follows_screen_removal_and_reset() {
    let hub = hub(true);
    let mut rx = hub.subscribe();
    let (_sub, _) = hub.preview_attach("main");
    assert!(hub.remove_screen("main"));
    assert_eq!(
        drain(&mut rx),
        [
            (false, json!({"t": "error", "code": "unknownScreen"})),
            (true, json!({"t": "error", "code": "unknownScreen"})),
        ]
    );
    // An unknown screen: no snapshot yet; it arrives when the screen does.
    let (_sub2, snap) = hub.preview_attach("nope");
    assert!(snap.is_none());
    let mut next = LiveState::with_test_screen();
    let mut nope = next.screens[0].clone();
    nope.info.id = "nope".into();
    next.screens.push(nope);
    let mut rx2 = hub.subscribe();
    hub.reset(next);
    let mut snaps = 0;
    while let Ok(m) = rx2.try_recv() {
        if &*m.screen_id == "nope" && m.preview {
            assert!(m.json.contains(r#""t":"snapshot""#));
            snaps += 1;
        }
    }
    assert_eq!(snaps, 1);
}

#[tokio::test]
async fn tbar_moves_are_paced_and_the_latest_wins() {
    let hub = hub(true);
    hub.studio_op("main", cue("b", Some(true))).unwrap();
    let mut rx = hub.subscribe();
    hub.studio_op("main", StudioAction::TbarBegin).unwrap();
    assert_eq!(kinds(&drain(&mut rx)), [(false, "prepare".to_string())]);
    hub.studio_op("main", StudioAction::TbarSet { p: 0.1 })
        .unwrap();
    assert_eq!(
        drain(&mut rx),
        [(false, json!({"t": "tbar", "p": 0.1}))],
        "the first move goes out at once"
    );
    for p in [0.2, 0.3, 7.0] {
        let v = hub.studio_op("main", StudioAction::TbarSet { p }).unwrap();
        assert_eq!(v.tbar, Some(0.1), "the view reports what was sent");
    }
    assert!(drain(&mut rx).is_empty(), "paced");
    tokio::time::sleep(TBAR_INTERVAL * 3).await;
    assert_eq!(
        drain(&mut rx),
        [(false, json!({"t": "tbar", "p": 1.0}))],
        "one message, the latest position (clamped)"
    );
    assert_eq!(hub.preview_view("main").unwrap().tbar, Some(1.0));
    tokio::time::sleep(TBAR_INTERVAL * 3).await;
    assert!(drain(&mut rx).is_empty(), "nothing at rest");
    assert!(lock(&hub.tbar).values().all(|c| c.task.is_none()));
    // Released at the end: commit, Program takes the diff, no pacing state left.
    hub.studio_op("main", StudioAction::TbarSet { p: 0.9 })
        .unwrap();
    hub.studio_op(
        "main",
        StudioAction::TbarEnd {
            commit: true,
            after: KEEP,
        },
    )
    .unwrap();
    let got = drain(&mut rx);
    assert_eq!(
        kinds(got.last().map(std::slice::from_ref).unwrap()),
        [(false, "commit".to_string())]
    );
    assert!(hub.is_on_air("main", "b"));
    assert!(lock(&hub.tbar).is_empty());
    tokio::time::sleep(TBAR_INTERVAL * 3).await;
    assert!(
        drain(&mut rx).is_empty(),
        "a pending move never follows the commit"
    );
}

#[test]
fn tbar_abort_keeps_program_and_the_diff_and_a_cue_change_aborts() {
    let hub = hub(true);
    hub.studio_op("main", cue("b", Some(true))).unwrap();
    let mut rx = hub.subscribe();
    hub.studio_op("main", StudioAction::TbarSet { p: 0.4 })
        .unwrap();
    assert_eq!(
        kinds(&drain(&mut rx)),
        [(false, "prepare".to_string()), (false, "tbar".to_string())],
        "a move starts the T-bar"
    );
    let v = hub
        .studio_op(
            "main",
            StudioAction::TbarEnd {
                commit: false,
                after: KEEP,
            },
        )
        .unwrap();
    assert_eq!(drain(&mut rx), [(false, json!({"t": "abort"}))]);
    assert!(!hub.is_on_air("main", "b"));
    assert_eq!(v.cues.len(), 1);
    assert_eq!(v.tbar, None);
    // Changing the diff mid-T-bar ends it (what was prepared would be stale).
    hub.studio_op("main", StudioAction::TbarBegin).unwrap();
    drain(&mut rx);
    hub.studio_op("main", cue("a", Some(true))).unwrap();
    assert_eq!(drain(&mut rx), [(false, json!({"t": "abort"}))]);
    // A take during a T-bar: abort first, then the take.
    hub.studio_op("main", StudioAction::TbarBegin).unwrap();
    drain(&mut rx);
    hub.studio_op(
        "main",
        StudioAction::Take {
            transition: CUT,
            after: KEEP,
        },
    )
    .unwrap();
    assert_eq!(
        kinds(&drain(&mut rx)),
        [(false, "abort".to_string()), (false, "take".to_string())]
    );
}

#[test]
fn panic_aborts_tbars_and_clears_every_diff_clear_keeps_them() {
    let hub = hub(true);
    hub.show("main", "a").unwrap();
    hub.studio_op("main", cue("a", Some(false))).unwrap();
    hub.studio_op("main", cue("b", Some(true))).unwrap();
    let mut rx = hub.subscribe();
    hub.clear("main").unwrap();
    assert_eq!(kinds(&drain(&mut rx)), [(false, "state".to_string())]);
    let v = hub.preview_view("main").unwrap();
    assert_eq!(v.cues.len(), 2, "Clear leaves the Preview");
    assert!(v.cues[0].program_changed, "Program changed under a's cue");
    assert!(!v.cues[1].program_changed, "b is untouched");

    hub.studio_op("main", StudioAction::TbarBegin).unwrap();
    drain(&mut rx);
    hub.panic();
    assert_eq!(
        drain(&mut rx),
        [(false, json!({"t": "abort"})), (false, json!({"t": "cut"}))]
    );
    let v = hub.preview_view("main").unwrap();
    assert!(v.cues.is_empty() && v.tbar.is_none());
}

#[derive(Default)]
struct Events(Mutex<Vec<(String, Value)>>);

impl EventSink for Events {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

impl Events {
    fn take(&self, name: &str) -> Vec<Value> {
        let all = std::mem::take(&mut *self.0.lock().unwrap());
        all.into_iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v)
            .collect()
    }
}

#[test]
fn preview_changed_only_in_studio_mode_and_only_on_change() {
    let off = hub(false);
    let ev = Arc::new(Events::default());
    off.set_notifier(ev.clone());
    off.show("main", "a").unwrap();
    off.apply_screen(edit_plan(&off, "a", "Ada"), LiveEdit::OnTake)
        .unwrap();
    assert!(ev.take(names::PREVIEW_CHANGED).is_empty());

    let on = hub(true);
    let ev = Arc::new(Events::default());
    on.set_notifier(ev.clone());
    on.studio_op("main", cue("a", Some(true))).unwrap();
    let got = ev.take(names::PREVIEW_CHANGED);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["screenId"], "main");
    assert_eq!(got[0]["cues"][0]["layerId"], "a");
    assert_eq!(got[0]["layers"][0]["onAir"], true);
    on.studio_op("main", cue("a", Some(true))).unwrap();
    assert!(ev.take(names::PREVIEW_CHANGED).is_empty(), "no-op");
    // A staged edit: Preview changes, Program's view doesn't.
    let before = ev.take(names::LIVE_CHANGED).len();
    on.apply_screen(edit_plan(&on, "a", "Ada"), LiveEdit::Immediate)
        .unwrap();
    let all = std::mem::take(&mut *ev.0.lock().unwrap());
    let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, [names::PREVIEW_CHANGED], "{before}");
    assert_eq!(all[0].1["layers"][0]["fields"]["name"], "Ada");
}

#[test]
fn studio_exits() {
    // Off without exit while the Preview has changes: refused, nothing changes.
    let h = hub(true);
    h.studio_op("main", cue("b", Some(true))).unwrap();
    assert_eq!(h.set_studio(false, None, CUT), Err(PreviewHasChanges));
    assert!(h.is_studio());
    // Take: the Preview goes to Program with the transition, then Studio mode is off.
    let mut rx = h.subscribe();
    h.set_studio(false, Some(StudioExit::Take), CUT).unwrap();
    let got = drain(&mut rx);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1["t"], "take");
    assert_eq!(got[0].1["transition"]["kind"], "cut");
    assert!(!h.is_studio() && h.is_on_air("main", "b"));
    assert!(h.preview_view("main").unwrap().cues.is_empty());

    // Discard.
    let h = hub(true);
    h.studio_op("main", cue("b", Some(true))).unwrap();
    h.set_studio(false, Some(StudioExit::Discard), CUT).unwrap();
    assert!(!h.is_on_air("main", "b"));
    assert!(h.preview_view("main").unwrap().cues.is_empty());

    // Keep staged: on-air content → "Changes on next take", cues dropped.
    let h = hub(true);
    h.show("main", "a").unwrap();
    h.apply_screen(edit_plan(&h, "a", "Ada"), LiveEdit::OnTake)
        .unwrap();
    h.studio_op("main", cue("b", Some(true))).unwrap();
    h.set_studio(false, Some(StudioExit::KeepStaged), CUT)
        .unwrap();
    let v = h.layer_view("main", "a").unwrap();
    assert!(v.pending && v.fields["name"] == "Ada");
    assert!(!h.is_on_air("main", "b"));

    // An empty Preview needs no exit; turning it on again is plain.
    let h = hub(true);
    h.set_studio(false, None, CUT).unwrap();
    assert!(!h.is_studio());
    h.set_studio(true, None, CUT).unwrap();
    assert!(h.is_studio());
}

#[test]
fn every_studio_path_goes_through_the_entry_point() {
    let h = hub(true);
    let n0 = h.studio_op_count();
    h.studio_op("main", cue("a", Some(true))).unwrap();
    assert_eq!(h.studio_op_count(), n0 + 1, "studio op");
    h.apply_screen(edit_plan(&h, "b", "Bea"), LiveEdit::Immediate)
        .unwrap();
    assert_eq!(h.studio_op_count(), n0 + 2, "edit (save / Control edit)");
    let rect = Rect {
        x: 0,
        y: 0,
        w: 10,
        h: 10,
    };
    h.set_rect("main", "b", rect, LiveEdit::Immediate).unwrap();
    assert_eq!(h.studio_op_count(), n0 + 3, "drag");
    // Outside Studio mode edits follow the live rules without it.
    let off = hub(false);
    off.apply_screen(edit_plan(&off, "b", "Bea"), LiveEdit::Immediate)
        .unwrap();
    off.set_rect("main", "b", rect, LiveEdit::Immediate)
        .unwrap();
    assert_eq!(off.studio_op_count(), 0);
}

#[test]
fn forced_automation_goes_to_program_and_rebases_the_diff() {
    let h = hub(true);
    h.show("main", "a").unwrap();
    // The operator stages a title on a.
    {
        let mut p = edit_plan(&h, "a", "");
        p.layers[0]
            .content
            .fields
            .insert("title".into(), "Staged".into());
        h.apply_screen(p, LiveEdit::OnTake).unwrap();
    }
    let mut rx = h.subscribe();
    let f: Fields = [("name".to_string(), "Auto".to_string())].into();
    h.force_fields("main", "a", &f).unwrap();
    assert_eq!(
        drain(&mut rx),
        [(
            false,
            json!({"t": "fields", "layerId": "a", "fields": {"name": "Auto"}})
        )]
    );
    let v = h.preview_view("main").unwrap();
    assert!(v.cues[0].program_changed && v.cues[0].staged);
    assert_eq!(v.layers[0].fields["name"], "Auto");
    assert_eq!(v.layers[0].fields["title"], "Staged");
    // Outside Studio mode the forced keys are patched into the staged content (as before).
    let off = hub(false);
    off.show("main", "a").unwrap();
    off.apply_screen(edit_plan(&off, "a", "Staged"), LiveEdit::OnTake)
        .unwrap();
    off.force_fields("main", "a", &f).unwrap();
    let v = off.layer_view("main", "a").unwrap();
    assert!(
        !v.pending,
        "staged content equal to Program after the patch is dropped"
    );
}
