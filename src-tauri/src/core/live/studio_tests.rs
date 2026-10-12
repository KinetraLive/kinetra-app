//! Studio mode in the pure live state: the Preview diff, takes, the T-bar, rebasing.

use serde_json::{json, Value};

use super::*;
use crate::core::protocol::TakeTransitionKind;

/// A state with the test screen and a user screen `main` with layers `a` and `b` (the test lower
/// third), all off air.
fn state(studio: bool) -> LiveState {
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
    live
}

fn main(live: &LiveState) -> &ScreenLive {
    live.screen("main").unwrap()
}

/// `main`'s plan as its document would give it: the staged content where there is some (the
/// file holds every edit), else Program's.
fn plan(live: &LiveState) -> ScreenPlan {
    plan_from(live, true)
}

fn plan_from(live: &LiveState, staged: bool) -> ScreenPlan {
    let s = main(live);
    ScreenPlan {
        info: s.info.clone(),
        layers: s
            .layers
            .iter()
            .map(|l| LayerPlan {
                id: l.id.clone(),
                name: l.name.clone(),
                content: match s.preview.staged(&l.id) {
                    Some(c) if staged => c.clone(),
                    _ => l.content(),
                },
            })
            .collect(),
    }
}

/// The plan of `main` with `key = value` on layer `layer`.
fn edited(live: &LiveState, layer: &str, key: &str, value: &str) -> ScreenPlan {
    let mut p = plan(live);
    let l = p.layers.iter_mut().find(|l| l.id == layer).unwrap();
    l.content.fields.insert(key.into(), value.into());
    p
}

fn program_field(live: &LiveState, layer: &str, key: &str) -> String {
    main(live)
        .layers
        .iter()
        .find(|l| l.id == layer)
        .unwrap()
        .fields[key]
        .clone()
}

fn on_air(live: &LiveState, layer: &str) -> bool {
    main(live)
        .layers
        .iter()
        .find(|l| l.id == layer)
        .unwrap()
        .on_air
}

fn wire(msgs: &[ServerMsg]) -> Vec<Value> {
    msgs.iter()
        .map(|m| serde_json::to_value(m).unwrap())
        .collect()
}

const CUT: TakeTransition = TakeTransition {
    kind: TakeTransitionKind::Cut,
    ms: 500,
};

#[test]
fn on_take_staging_is_a_content_only_cue() {
    let mut live = state(false);
    live.show("main", "a").unwrap();
    let out = live.apply_screen(edited(&live, "a", "name", "Ada"), LiveEdit::OnTake);
    assert!(out.msgs.is_empty(), "staged: nothing sent");
    let cue = &main(&live).preview.layers["a"];
    assert_eq!(cue.visible, None);
    assert!(cue.content.is_some());
    // The Program view reports the staged values (as before Studio mode existed).
    let v = live.layer_view("main", "a").unwrap();
    assert!(v.pending);
    assert_eq!(v.fields["name"], "Ada");
    // And the Preview shows them too.
    let pv = live.preview_view("main").unwrap();
    assert_eq!(pv.cues.len(), 1);
    assert!(pv.cues[0].staged && pv.cues[0].visible.is_none());
    assert_eq!(pv.layers[0].fields["name"], "Ada");
    // TAKE (show) consumes it.
    live.show("main", "a").unwrap();
    assert!(main(&live).preview.is_empty());
    assert_eq!(program_field(&live, "a", "name"), "Ada");
}

#[test]
fn studio_edits_go_to_the_preview_on_and_off_air() {
    let mut live = state(true);
    live.show("main", "a").unwrap();
    // Off air (b) and on air (a): both staged, nothing sent, Program unchanged.
    let out = live.apply_screen(edited(&live, "b", "name", "Bea"), LiveEdit::Immediate);
    assert!(out.msgs.is_empty(), "{:?}", out.msgs);
    let out = live.apply_screen(edited(&live, "a", "name", "Ada"), LiveEdit::Immediate);
    assert!(out.msgs.is_empty(), "{:?}", out.msgs);
    let rect = Rect {
        x: 10,
        y: 10,
        w: 100,
        h: 100,
    };
    assert!(live
        .set_rect("main", "b", rect, LiveEdit::Immediate)
        .unwrap()
        .is_empty());
    assert_eq!(program_field(&live, "a", "name"), "");
    assert_eq!(program_field(&live, "b", "name"), "");
    assert_eq!(main(&live).layers[1].rect, None, "Program keeps its rect");
    // Program views are Program only in Studio mode.
    let v = live.screen_view("main").unwrap();
    assert!(v
        .layers
        .iter()
        .all(|l| !l.pending && l.fields["name"].is_empty()));
    let pv = live.preview_view("main").unwrap();
    assert_eq!(pv.layers[0].fields["name"], "Ada");
    assert_eq!(pv.layers[1].fields["name"], "Bea");
    assert_eq!(pv.cues.len(), 2);
    // The staged b keeps both edits (the rect drag started from the staged content).
    assert_eq!(
        main(&live).preview.staged("b").unwrap().rect,
        Some(rect),
        "rect staged"
    );
    assert_eq!(
        main(&live).preview.staged("b").unwrap().fields["name"],
        "Bea"
    );
    // An edit back to Program drops the staged content.
    let out = live.apply_screen(plan_with_program(&live), LiveEdit::Immediate);
    assert!(out.msgs.is_empty());
    assert!(main(&live).preview.is_empty());
}

/// `main`'s plan with Program's content (what a revert would save).
fn plan_with_program(live: &LiveState) -> ScreenPlan {
    plan_from(live, false)
}

#[test]
fn structural_changes_apply_at_once_in_studio_mode() {
    let mut live = state(true);
    live.cue("main", "b", Some(true)).unwrap();
    let mut p = plan(&live);
    p.layers.reverse();
    let out = live.apply_screen(p.clone(), LiveEdit::OnTake);
    assert!(
        out.msgs
            .iter()
            .any(|m| matches!(m, ServerMsg::Layer { layer } if layer.id == "b" && layer.z == 0)),
        "reorder applies to Program: {:?}",
        out.msgs
    );
    // Removing a layer drops its cue.
    p.layers.retain(|l| l.id != "b");
    live.apply_screen(p, LiveEdit::OnTake);
    assert!(main(&live).preview.is_empty());
}

#[test]
fn cues_bump_the_rev_and_a_cue_equal_to_program_is_none() {
    let mut live = state(true);
    let r0 = main(&live).preview.rev;
    assert!(live.cue("main", "a", Some(true)).unwrap());
    let r1 = main(&live).preview.rev;
    assert!(r1 > r0);
    assert!(
        !live.cue("main", "a", Some(true)).unwrap(),
        "same cue: no change"
    );
    // Cueing out an off-air layer is no cue.
    assert!(!live.cue("main", "b", Some(false)).unwrap());
    assert!(!main(&live).preview.layers.contains_key("b"));
    assert!(live.cue("main", "a", None).unwrap());
    assert!(main(&live).preview.is_empty());
    assert!(matches!(
        live.cue("main", "nope", Some(true)),
        Err(LiveError::UnknownLayer(_))
    ));
    // The Preview shows the cued-in layer on air, Program does not.
    live.cue("main", "b", Some(true)).unwrap();
    let pv = live.preview_view("main").unwrap();
    assert!(pv.layers[1].on_air && !on_air(&live, "b"));
    assert_eq!(
        serde_json::to_value(&pv.cues[0]).unwrap(),
        json!({"layerId": "b", "visible": true, "staged": false, "programChanged": false})
    );
}

#[test]
fn take_sends_one_take_message_per_transition() {
    for kind in [
        TakeTransitionKind::Cut,
        TakeTransitionKind::Crossfade,
        TakeTransitionKind::OutIn,
    ] {
        let mut live = state(true);
        live.show("main", "a").unwrap();
        live.cue("main", "a", Some(false)).unwrap();
        live.cue("main", "b", Some(true)).unwrap();
        live.apply_screen(edited(&live, "b", "name", "Bea"), LiveEdit::OnTake);
        let rev = main(&live).layers[1].template_rev;
        let tr = TakeTransition { kind, ms: 99_999 };
        let out = live
            .take_preview("main", tr, PreviewAfterTake::Keep)
            .unwrap();
        let w = wire(&out.msgs);
        assert_eq!(w.len(), 1, "{w:?}");
        let ms = 5000;
        assert_eq!(w[0]["t"], "take");
        assert_eq!(
            w[0]["transition"],
            serde_json::to_value(TakeTransition { kind, ms }).unwrap(),
            "ms clamped"
        );
        assert_eq!(w[0]["hide"], json!(["a"]));
        let layers = w[0]["layers"].as_array().unwrap();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0]["id"], "b");
        assert_eq!(layers[0]["state"], "in");
        assert_eq!(layers[0]["fields"]["name"], "Bea");
        assert_eq!(layers[0]["templateRev"], rev, "fields only: same rev");
        assert!(!on_air(&live, "a") && on_air(&live, "b"));
        assert_eq!(program_field(&live, "b", "name"), "Bea");
        assert!(main(&live).preview.is_empty(), "keep: empty diff");
        let mut touched = out.touched.clone();
        touched.sort();
        assert_eq!(touched, ["a", "b"]);
    }
}

#[test]
fn take_orders_off_air_content_before_and_outgoing_content_after() {
    let mut live = state(true);
    live.show("main", "a").unwrap();
    // a: cued out with a content change; b: content change, stays off air.
    live.apply_screen(edited(&live, "a", "name", "A2"), LiveEdit::OnTake);
    live.apply_screen(edited(&live, "b", "name", "B2"), LiveEdit::OnTake);
    live.cue("main", "a", Some(false)).unwrap();
    let out = live
        .take_preview("main", CUT, PreviewAfterTake::Keep)
        .unwrap();
    let w = wire(&out.msgs);
    let kinds: Vec<&str> = w.iter().map(|m| m["t"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["fields", "take", "fields"], "{w:?}");
    assert_eq!(w[0]["layerId"], "b");
    assert_eq!(w[1]["layers"], json!([]));
    assert_eq!(w[1]["hide"], json!(["a"]));
    assert_eq!(w[2]["layerId"], "a");
    // An empty diff takes nothing.
    assert!(live
        .take_preview("main", CUT, PreviewAfterTake::Keep)
        .unwrap()
        .msgs
        .is_empty());
}

#[test]
fn preview_after_take_keep_or_clear() {
    let mut live = state(true);
    live.cue("main", "a", Some(true)).unwrap();
    live.take_preview("main", CUT, PreviewAfterTake::Clear)
        .unwrap();
    assert!(on_air(&live, "a"));
    // Clear: every on-air layer cued out: a blank Preview.
    let pv = live.preview_view("main").unwrap();
    assert_eq!(pv.cues.len(), 1);
    assert_eq!(pv.cues[0].visible, Some(false));
    assert!(pv.layers.iter().all(|l| !l.on_air));
    // Keep: Preview mirrors the new Program.
    live.cue("main", "b", Some(true)).unwrap();
    live.take_preview("main", CUT, PreviewAfterTake::Keep)
        .unwrap();
    assert!(!on_air(&live, "a") && on_air(&live, "b"));
    assert!(main(&live).preview.is_empty());
    let pv = live.preview_view("main").unwrap();
    assert_eq!(
        pv.layers.iter().map(|l| l.on_air).collect::<Vec<_>>(),
        [false, true]
    );
}

#[test]
fn tbar_prepares_clamps_commits_and_aborts() {
    let mut live = state(true);
    live.show("main", "a").unwrap();
    live.cue("main", "a", Some(false)).unwrap();
    live.cue("main", "b", Some(true)).unwrap();
    let w = wire(&live.tbar_begin("main").unwrap());
    assert_eq!(w.len(), 1);
    assert_eq!(w[0]["t"], "prepare");
    assert_eq!(w[0]["hide"], json!(["a"]));
    assert_eq!(w[0]["layers"][0]["id"], "b");
    assert!(
        on_air(&live, "a") && !on_air(&live, "b"),
        "Program unchanged"
    );
    assert!(
        live.tbar_begin("main").unwrap().is_empty(),
        "already prepared"
    );
    for (p, want) in [(1.5, 1.0), (-1.0, 0.0), (f64::NAN, 0.0), (0.25, 0.25)] {
        assert!(live.tbar_set("main", p).unwrap().is_empty());
        assert_eq!(main(&live).preview.tbar.as_ref().unwrap().p, want);
    }
    assert_eq!(live.tbar_unsent("main"), Some(0.25));
    assert_eq!(live.tbar_unsent("main"), None, "sent");
    assert_eq!(live.preview_view("main").unwrap().tbar, Some(0.25));
    // Abort: Program and the diff unchanged.
    let out = live
        .tbar_end("main", false, PreviewAfterTake::Keep)
        .unwrap();
    assert_eq!(wire(&out.msgs), [json!({"t": "abort"})]);
    assert!(on_air(&live, "a") && !on_air(&live, "b"));
    assert_eq!(main(&live).preview.layers.len(), 2);
    assert_eq!(live.preview_view("main").unwrap().tbar, None);
    assert!(live
        .tbar_end("main", true, PreviewAfterTake::Keep)
        .unwrap()
        .msgs
        .is_empty());
    // Commit: like a take, but `commit` instead of `take`.
    live.tbar_set("main", 0.5).unwrap();
    let out = live.tbar_end("main", true, PreviewAfterTake::Keep).unwrap();
    assert_eq!(wire(&out.msgs), [json!({"t": "commit"})]);
    assert!(!on_air(&live, "a") && on_air(&live, "b"));
    assert!(main(&live).preview.is_empty());
}

#[test]
fn tbar_commit_after_program_changed_underneath_sends_a_snapshot() {
    let mut live = state(true);
    live.cue("main", "b", Some(true)).unwrap();
    live.tbar_begin("main").unwrap();
    // Automation changes the prepared layer's Program content meanwhile.
    live.set_fields("main", "b", [("name".to_string(), "X".to_string())].into())
        .unwrap();
    let out = live.tbar_end("main", true, PreviewAfterTake::Keep).unwrap();
    let w = wire(&out.msgs);
    let kinds: Vec<&str> = w.iter().map(|m| m["t"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["commit", "snapshot"]);
}

#[test]
fn automation_rebases_the_diff_and_marks_program_changed() {
    let mut live = state(true);
    live.show("main", "a").unwrap();
    // The operator stages a title.
    live.apply_screen(edited(&live, "a", "title", "Staged"), LiveEdit::OnTake);
    // Automation changes name (untouched by the operator) and title (staged) on Program.
    let old = live.content_if_staged("main", "a").unwrap();
    let f: Fields = [
        ("name".to_string(), "Auto".to_string()),
        ("title".to_string(), "AutoTitle".to_string()),
    ]
    .into();
    let msgs = live.set_fields("main", "a", f).unwrap();
    live.program_changed("main", &msgs, Some(("a", &old)));
    let staged = main(&live).preview.staged("a").unwrap();
    assert_eq!(staged.fields["name"], "Auto", "follows Program");
    assert_eq!(staged.fields["title"], "Staged", "the diff wins");
    let pv = live.preview_view("main").unwrap();
    assert!(pv.cues[0].program_changed);
    assert_eq!(pv.layers[0].fields["name"], "Auto");
    assert_eq!(pv.layers[0].fields["title"], "Staged");
    // A take clears the marker.
    live.take_preview("main", CUT, PreviewAfterTake::Keep)
        .unwrap();
    assert_eq!(program_field(&live, "a", "title"), "Staged");
    assert!(main(&live).preview.is_empty());

    // A visibility cue: an automated hide marks it; clearing the cue clears the marker.
    live.cue("main", "b", Some(true)).unwrap();
    live.show("main", "b").unwrap();
    let msgs = vec![ServerMsg::State {
        layer_id: "b".into(),
        state: LayerState::In,
    }];
    live.program_changed("main", &msgs, None);
    assert!(live.preview_view("main").unwrap().cues[0].program_changed);
    live.cue("main", "b", None).unwrap();
    assert!(live.preview_view("main").unwrap().cues.is_empty());
    // Staged content equal to the new Program is dropped.
    live.apply_screen(edited(&live, "a", "name", "Same"), LiveEdit::OnTake);
    let old = live.content_if_staged("main", "a").unwrap();
    let msgs = live
        .set_fields(
            "main",
            "a",
            [("name".to_string(), "Same".to_string())].into(),
        )
        .unwrap();
    live.program_changed("main", &msgs, Some(("a", &old)));
    assert!(main(&live).preview.is_empty());
}

#[test]
fn automation_show_in_studio_mode_leaves_staged_content_in_the_diff() {
    let mut live = state(true);
    live.apply_screen(edited(&live, "a", "name", "Ada"), LiveEdit::OnTake);
    let msgs = live.show("main", "a").unwrap();
    assert_eq!(
        wire(&msgs),
        [json!({"t": "state", "layerId": "a", "state": "in"})]
    );
    assert_eq!(program_field(&live, "a", "name"), "");
    assert!(main(&live).preview.staged("a").is_some());
}

#[test]
fn cut_clears_diffs_in_studio_mode_only() {
    let mut live = state(false);
    live.show("main", "a").unwrap();
    live.apply_screen(edited(&live, "a", "name", "Ada"), LiveEdit::OnTake);
    live.cut();
    assert!(
        !main(&live).preview.is_empty(),
        "onTake staging survives PANIC"
    );
    live.studio = true;
    live.cue("main", "b", Some(true)).unwrap();
    live.tbar_begin("main").unwrap();
    live.cut();
    assert!(main(&live).preview.is_empty());
    assert!(!live.tbar_active("main"));
}

#[test]
fn leaving_studio_keeping_staged_content() {
    let mut live = state(true);
    live.show("main", "a").unwrap();
    live.apply_screen(edited(&live, "a", "name", "A2"), LiveEdit::OnTake);
    live.apply_screen(edited(&live, "b", "name", "B2"), LiveEdit::OnTake);
    live.cue("main", "b", Some(true)).unwrap();
    let out = live.studio_off_keep_staged();
    assert!(!live.studio);
    let (id, msgs) = out.iter().find(|(id, _)| id == "main").unwrap();
    assert_eq!(id, "main");
    // b is off air: its content applies now; its cue is dropped.
    assert_eq!(
        wire(msgs),
        [json!({"t": "fields", "layerId": "b", "fields": {"name": "B2"}})]
    );
    assert!(!on_air(&live, "b"));
    // a is on air: "Changes on next take".
    let v = live.layer_view("main", "a").unwrap();
    assert!(v.pending);
    assert_eq!(v.fields["name"], "A2");
    assert_eq!(main(&live).preview.layers.len(), 1);
}

#[test]
fn persisted_cues_round_trip() {
    let mut live = state(true);
    live.show("main", "a").unwrap();
    live.cue("main", "a", Some(false)).unwrap();
    live.cue("main", "b", Some(true)).unwrap();
    let cues = live.studio_cues();
    // Only cue-ins: after a restart everything is off air.
    assert_eq!(
        serde_json::to_value(&cues).unwrap(),
        json!({"main": {"b": true}})
    );
    let mut fresh = state(true);
    let mut with_gone = cues.clone();
    with_gone
        .get_mut("main")
        .unwrap()
        .insert("gone".into(), true);
    fresh.restore_cues(&with_gone);
    let pv = fresh.preview_view("main").unwrap();
    assert_eq!(pv.cues.len(), 1);
    assert_eq!(pv.cues[0].layer_id, "b");
}

#[test]
fn bus_diff_keeps_revs_growing_on_the_preview_bus() {
    let mut live = state(true);
    let mut old = preview_of(main(&live));
    let rev = old.layers[0].template_rev;
    // A staged look change: the bus sees a new rev.
    let mut p = plan(&live);
    let mut t = (*p.layers[0].content.template).clone();
    t.html = "<div>{{name}}</div>".into();
    p.layers[0].content.template = Arc::new(t);
    live.apply_screen(p, LiveEdit::OnTake);
    let mut new = preview_of(main(&live));
    let msgs = bus_diff(&old, &mut new);
    assert_eq!(msgs.len(), 1);
    let ServerMsg::Layer { layer } = &msgs[0] else {
        panic!("{msgs:?}")
    };
    assert_eq!(layer.template_rev, rev + 1);
    old = new;
    // Another look, still staged (Program's rev unchanged): rev + 2 on the bus, not rev + 1.
    let mut p = plan(&live);
    let mut t = (*p.layers[0].content.template).clone();
    t.html = "<p>{{name}}</p>".into();
    p.layers[0].content.template = Arc::new(t);
    live.apply_screen(p, LiveEdit::OnTake);
    let mut new = preview_of(main(&live));
    let msgs = bus_diff(&old, &mut new);
    let ServerMsg::Layer { layer } = &msgs[0] else {
        panic!("{msgs:?}")
    };
    assert_eq!(layer.template_rev, rev + 2);
    old = new;
    // A cue: one state message; same look keeps the rev.
    live.cue("main", "b", Some(true)).unwrap();
    let mut new = preview_of(main(&live));
    let msgs = bus_diff(&old, &mut new);
    assert_eq!(
        wire(&msgs),
        [json!({"t": "state", "layerId": "b", "state": "in"})]
    );
    assert_eq!(new.layers[0].template_rev, rev + 2);
}

/// `main`'s plan with layer `layer`'s sizing set.
fn sized(live: &LiveState, layer: &str, sizing: ContentSizing) -> ScreenPlan {
    let mut p = plan(live);
    p.layers
        .iter_mut()
        .find(|l| l.id == layer)
        .unwrap()
        .content
        .sizing = sizing;
    p
}

fn program_sizing(live: &LiveState, layer: &str) -> ContentSizing {
    main(live)
        .layers
        .iter()
        .find(|l| l.id == layer)
        .unwrap()
        .sizing
}

#[test]
fn sizing_is_content_off_air_on_take_and_immediate() {
    let mut live = state(false);
    let rev = main(&live).layers[0].template_rev;
    // Off air: one same-rev `layer` carrying `sizing: fit`.
    let out = live.apply_screen(sized(&live, "a", ContentSizing::Fit), LiveEdit::OnTake);
    let w = wire(&out.msgs);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0]["t"], "layer");
    assert_eq!(w[0]["layer"]["sizing"], "fit");
    assert_eq!(w[0]["layer"]["templateRev"], rev, "no recompile");
    // Back to reflow: `sizing` is omitted on the wire.
    let out = live.apply_screen(sized(&live, "a", ContentSizing::Reflow), LiveEdit::OnTake);
    let w = wire(&out.msgs);
    assert_eq!(w.len(), 1);
    assert!(w[0]["layer"].get("sizing").is_none(), "{w:?}");
    // Unchanged: nothing.
    let out = live.apply_screen(sized(&live, "a", ContentSizing::Reflow), LiveEdit::OnTake);
    assert!(out.msgs.is_empty());

    // On air under onTake: staged, nothing sent; the show applies it in place.
    live.show("main", "a").unwrap();
    let out = live.apply_screen(sized(&live, "a", ContentSizing::Fit), LiveEdit::OnTake);
    assert!(out.msgs.is_empty(), "{:?}", out.msgs);
    assert!(live.layer_view("main", "a").unwrap().pending);
    assert_eq!(program_sizing(&live, "a"), ContentSizing::Reflow);
    let msgs = live.show("main", "a").unwrap();
    let w = wire(&msgs);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0]["layer"]["sizing"], "fit");
    assert_eq!(w[0]["layer"]["state"], "in");
    assert_eq!(w[0]["layer"]["templateRev"], rev);
    assert_eq!(program_sizing(&live, "a"), ContentSizing::Fit);

    // On air under immediate: one same-rev `layer` now.
    let out = live.apply_screen(
        sized(&live, "a", ContentSizing::Reflow),
        LiveEdit::Immediate,
    );
    let w = wire(&out.msgs);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0]["layer"]["templateRev"], rev);
    assert!(w[0]["layer"].get("sizing").is_none());
    assert_eq!(program_sizing(&live, "a"), ContentSizing::Reflow);
}

#[test]
fn a_design_change_resends_the_layer_without_a_recompile() {
    let mut live = state(false);
    let rev = main(&live).layers[0].template_rev;
    let mut p = sized(&live, "a", ContentSizing::Fit);
    let mut t = (*p.layers[0].content.template).clone();
    t.design = Some(DesignSize { w: 640, h: 360 });
    p.layers[0].content.template = Arc::new(t);
    let out = live.apply_screen(p, LiveEdit::Immediate);
    let w = wire(&out.msgs);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0]["layer"]["design"], json!({"w": 640, "h": 360}));
    assert_eq!(w[0]["layer"]["templateRev"], rev);
    // The snapshot carries both.
    let ServerMsg::Snapshot { layers, .. } = main(&live).snapshot() else {
        panic!()
    };
    assert_eq!(layers[0].sizing, Some(ContentSizing::Fit));
    assert_eq!(layers[0].design, Some(DesignSize { w: 640, h: 360 }));
    assert_eq!(layers[1].sizing, None, "reflow is omitted");
}

#[test]
fn studio_stages_sizing_in_the_preview_and_the_take_applies_it() {
    let mut live = state(true);
    live.show("main", "a").unwrap();
    let rev = main(&live).layers[0].template_rev;
    let old = preview_of(main(&live));
    let out = live.apply_screen(sized(&live, "a", ContentSizing::Fit), LiveEdit::Immediate);
    assert!(out.msgs.is_empty(), "{:?}", out.msgs);
    assert_eq!(program_sizing(&live, "a"), ContentSizing::Reflow);
    // The Preview bus shows it: a same-rev `layer`.
    let mut new = preview_of(main(&live));
    let msgs = bus_diff(&old, &mut new);
    let w = wire(&msgs);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0]["layer"]["sizing"], "fit");
    assert_eq!(w[0]["layer"]["templateRev"], old.layers[0].template_rev);
    // TAKE: one `take` with the layer (same rev), Program follows.
    let out = live
        .take_preview("main", CUT, PreviewAfterTake::Keep)
        .unwrap();
    let w = wire(&out.msgs);
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0]["t"], "take");
    assert_eq!(w[0]["layers"][0]["sizing"], "fit");
    assert_eq!(w[0]["layers"][0]["templateRev"], rev);
    assert_eq!(program_sizing(&live, "a"), ContentSizing::Fit);
}

#[test]
fn rebase_keeps_a_staged_sizing_and_follows_an_untouched_one() {
    let live = state(true);
    let a = main(&live).layers[0].content();
    let mut staged = a.clone();
    staged.sizing = ContentSizing::Fit;
    let mut program = a.clone();
    program.fields.insert("name".into(), "Ada".into());
    let mut cue = staged.clone();
    rebase(&mut cue, &a, &program);
    assert_eq!(cue.sizing, ContentSizing::Fit, "the operator's sizing wins");
    let mut cue = a.clone();
    cue.fields.insert("title".into(), "x".into());
    let mut moved = a.clone();
    moved.sizing = ContentSizing::Fit;
    rebase(&mut cue, &a, &moved);
    assert_eq!(cue.sizing, ContentSizing::Fit, "untouched: follows Program");
}
