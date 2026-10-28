//! Protocol 2 commands (§5) against the mock relay: timers, variants, rows and Studio, each
//! through the local path, with its permission, lock, target and stale checks.

use super::state_v2::{setup, ALL};
use super::*;

const EVERY: &[&str] = &[
    "take_clear",
    "edit_fields",
    "edit_live_fields",
    "run_cues",
    "studio",
    "timers",
    "variants",
    "rows",
    "button_panel",
];

impl Env {
    fn p2(&self, corr: &str, action: &str, args: Value) -> Value {
        self.command_p(corr, action, args, EVERY, self.seq())
    }

    fn row_ids(&self, source: &str) -> Vec<String> {
        self.tree()["sources"][source]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect()
    }

    fn field(&self, layer: &str, key: &str) -> String {
        self.st
            .live()
            .layer_view(&self.screen, layer)
            .unwrap()
            .fields
            .get(key)
            .cloned()
            .unwrap_or_default()
    }
}

#[test]
fn timer_starts_pauses_and_resets_like_control() {
    let (e, _) = setup(ALL);
    e.live();
    let r = e.p2(
        "t1",
        "timer",
        json!({"layer": "l2", "field": "clock", "op": "start"}),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(
        e.field("l2", "clock").contains("\"run\":true"),
        "{}",
        e.field("l2", "clock")
    );
    let r = e.p2(
        "t2",
        "timer",
        json!({"layer": "l2", "field": "clock", "op": "pause"}),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(e.field("l2", "clock").contains("\"run\":false"));
    let r = e.p2(
        "t3",
        "timer",
        json!({"layer": "l2", "field": "clock", "op": "reset"}),
    );
    assert_eq!(r["ok"], true, "{r}");

    // Only timer fields; a bad op is incomplete; the permission is checked.
    let r = e.p2(
        "t4",
        "timer",
        json!({"layer": "l1", "field": "name", "op": "start"}),
    );
    assert_eq!(
        (r["code"].as_str(), r["details"]["field"].as_str()),
        (Some("invalid"), Some("name"))
    );
    let r = e.p2(
        "t5",
        "timer",
        json!({"layer": "l2", "field": "clock", "op": "jump"}),
    );
    assert_eq!(r["code"], "invalid");
    let r = e.command_p(
        "t6",
        "timer",
        json!({"layer": "l2", "field": "clock", "op": "start"}),
        &["take_clear"],
        e.seq(),
    );
    assert_eq!(r["code"], "permission");

    // Stale: the value changed after what the browser saw.
    let seen = e.seq();
    e.st.set_timer(
        &e.screen,
        "l2",
        "clock",
        crate::core::model::TimerAction::Start,
    )
    .unwrap();
    wait_until("timer diff", || e.seq() > seen);
    let r = e.command_p(
        "t7",
        "timer",
        json!({"layer": "l2", "field": "clock", "op": "pause"}),
        EVERY,
        seen,
    );
    assert_eq!(r["code"], "stale");

    // Locked: only while the layer is on air.
    e.st.remote_lock(true).unwrap();
    let r = e.p2(
        "t8",
        "timer",
        json!({"layer": "l2", "field": "clock", "op": "pause"}),
    );
    assert_eq!(r["ok"], true, "off air: {r}");
    e.st.live().show(&e.screen, "l2").unwrap();
    wait_until("on air", || e.tree()["layers"]["l2"]["on_air"] == true);
    let r = e.p2(
        "t9",
        "timer",
        json!({"layer": "l2", "field": "clock", "op": "start"}),
    );
    assert_eq!(r["code"], "locked");
}

#[test]
fn set_variant_switches_the_look_like_control() {
    let (e, _) = setup(ALL);
    let tree = e.live();
    let l1 = &tree["layers"]["l1"];
    let other = tree["templates"][l1["template"].as_str().unwrap()]["variants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] != l1["variant"])
        .unwrap()
        .clone();
    let id = other["id"].as_str().unwrap();
    let r = e.p2("v1", "set_variant", json!({"layer": "l1", "variant": id}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(e.st.live().layer_view(&e.screen, "l1").unwrap().variant, id);
    let activity = e.st.remote_activity();
    assert!(
        activity.iter().any(|a| a
            .text
            .ends_with(&format!("to {}", other["name"].as_str().unwrap()))),
        "{activity:?}"
    );
    let r = e.p2(
        "v2",
        "set_variant",
        json!({"layer": "l1", "variant": "nope"}),
    );
    assert_eq!(r["code"], "not_found");
    let r = e.command_p(
        "v3",
        "set_variant",
        json!({"layer": "l1", "variant": id}),
        &["edit_fields"],
        e.seq(),
    );
    assert_eq!(r["code"], "permission");
}

#[test]
fn rows_are_picked_and_stepped_like_the_sources_strip() {
    let (e, sid) = setup(ALL);
    e.live();
    let ids = e.row_ids(&sid);
    let current = |e: &Env| e.tree()["sources"][&sid]["current"].clone();

    let r = e.p2("r1", "step_row", json!({"source": sid, "delta": 1}));
    assert_eq!(r["ok"], true, "{r}");
    wait_until("stepped", || current(&e) == ids[1]);
    assert_eq!(e.field("l1", "title"), "Kari Nordmann");
    let r = e.p2("r2", "select_row", json!({"source": sid, "row": ids[2]}));
    assert_eq!(r["ok"], true, "{r}");
    wait_until("picked", || current(&e) == ids[2]);
    // At the last row a step forward is ok and changes nothing (no wrap).
    let r = e.p2("r3", "step_row", json!({"source": sid, "delta": 1}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(current(&e), ids[2]);

    // A row that isn't there (or was edited), a bad delta, an unknown source.
    let r = e.p2(
        "r4",
        "select_row",
        json!({"source": sid, "row": "000000000000"}),
    );
    assert_eq!(r["code"], "not_found");
    let r = e.p2("r5", "step_row", json!({"source": sid, "delta": 2}));
    assert_eq!(r["code"], "invalid");
    let r = e.p2("r6", "step_row", json!({"source": "nope", "delta": 1}));
    assert_eq!(r["code"], "not_found");

    // Stale when `current` moved after what the browser saw.
    let seen = e.seq();
    e.st.step_source(&sid, crate::core::model::SourceStep::By { delta: -1 })
        .unwrap();
    wait_until("local step", || current(&e) == ids[1]);
    let r = e.command_p(
        "r7",
        "step_row",
        json!({"source": sid, "delta": 1}),
        EVERY,
        seen,
    );
    assert_eq!(r["code"], "stale");

    // Locked only while a layer it feeds is on air.
    e.st.remote_lock(true).unwrap();
    let r = e.p2("r8", "step_row", json!({"source": sid, "delta": -1}));
    assert_eq!(r["ok"], true, "nothing on air: {r}");
    e.st.live().show(&e.screen, "l1").unwrap();
    wait_until("on air", || e.tree()["layers"]["l1"]["on_air"] == true);
    let r = e.p2("r9", "step_row", json!({"source": sid, "delta": 1}));
    assert_eq!(r["code"], "locked");
}

#[test]
fn studio_commands_follow_the_local_studio_controls() {
    let (e, _) = setup(ALL);
    e.live();
    let r = e.p2(
        "s0",
        "studio_cue",
        json!({"screen": e.screen, "layer": "l1"}),
    );
    assert_eq!(
        (r["code"].as_str(), r["message"].as_str()),
        (
            Some("busy"),
            Some("Studio mode is off on the show machine.")
        )
    );

    e.st.set_studio(
        StudioSettings {
            enabled: true,
            preview_after_take: PreviewAfterTake::Keep,
            transition: TakeTransition {
                kind: TakeTransitionKind::Crossfade,
                ms: 300,
            },
        },
        None,
    )
    .unwrap();
    wait_until("studio", || e.tree()["studio_mode"] == true);
    let r = e.p2(
        "s1",
        "studio_cue",
        json!({"screen": e.screen, "layer": "l1"}),
    );
    assert_eq!(r["ok"], true, "{r}");
    wait_until("cued", || e.tree()["preview"][&e.screen] == json!(["l1"]));
    let r = e.p2(
        "s2",
        "studio_uncue",
        json!({"screen": e.screen, "layer": "l1"}),
    );
    assert_eq!(r["ok"], true, "{r}");
    wait_until("uncued", || e.tree()["preview"][&e.screen] == json!([]));
    // The layer must be on that screen.
    let r = e.p2(
        "s3",
        "studio_cue",
        json!({"screen": "other", "layer": "l1"}),
    );
    assert_eq!(r["code"], "not_found");

    // Locked: cueing changes only Preview; a take is refused.
    e.st.remote_lock(true).unwrap();
    let r = e.p2(
        "s4",
        "studio_cue",
        json!({"screen": e.screen, "layer": "l1"}),
    );
    assert_eq!(r["ok"], true, "{r}");
    let r = e.p2(
        "s5",
        "studio_take",
        json!({"screen": e.screen, "mode": "cut"}),
    );
    assert_eq!(r["code"], "locked");
    e.st.remote_lock(false).unwrap();

    // A cut takes Preview to Program at once.
    let r = e.p2(
        "s6",
        "studio_take",
        json!({"screen": e.screen, "mode": "cut"}),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(e.on_air("l1"));
    let r = e.command_p(
        "s7",
        "studio_take",
        json!({"screen": e.screen, "mode": "take"}),
        &["take_clear"],
        e.seq(),
    );
    assert_eq!(r["code"], "permission");
}

#[test]
fn protocol_2_actions_need_their_feature() {
    let (e, sid) = setup(&["permissions"]);
    e.live();
    for (action, args) in [
        (
            "timer",
            json!({"layer": "l2", "field": "clock", "op": "start"}),
        ),
        ("set_variant", json!({"layer": "l1", "variant": "minimal"})),
        ("step_row", json!({"source": sid, "delta": 1})),
        ("studio_take", json!({"screen": e.screen, "mode": "cut"})),
    ] {
        let r = e.p2(action, action, args);
        assert_eq!(r["code"], "unsupported", "{action}: {r}");
    }
}

#[test]
fn the_panel_shows_labels_only_and_press_button_runs_the_apps_definition() {
    use crate::core::model::{PanelAction, PanelButton, PanelColor, PanelDoc};
    let mut features = ALL.to_vec();
    features.push("panel");
    let features: &'static [&'static str] = Box::leak(features.into_boxed_slice());
    let (e, _) = setup(features);
    let set = |name: &str| PanelAction::SetFields {
        screen: e.screen.clone(),
        layer: "l1".into(),
        values: Fields::from([("name".to_string(), name.to_string())]),
    };
    e.st.save_panel(PanelDoc {
        buttons: vec![
            PanelButton {
                id: "sponsor".into(),
                label: "Sponsor in".into(),
                color: PanelColor::Blue,
                confirm: true,
                actions: vec![
                    set("Nordlys"),
                    PanelAction::Take {
                        screen: e.screen.clone(),
                        layer: "l1".into(),
                    },
                ],
            },
            PanelButton {
                id: "fix".into(),
                label: "Fix name".into(),
                color: PanelColor::Grey,
                confirm: false,
                actions: vec![
                    set("Fixed"),
                    PanelAction::Take {
                        screen: e.screen.clone(),
                        layer: "gone".into(),
                    },
                ],
            },
            PanelButton {
                id: "edit".into(),
                label: "Edit name".into(),
                color: PanelColor::Green,
                confirm: false,
                actions: vec![set("Edited")],
            },
        ],
    })
    .unwrap();
    let tree = e.live();
    state::check_refs(&tree).unwrap();
    assert_eq!(tree["panel"]["order"], json!(["sponsor", "fix", "edit"]));
    assert_eq!(
        tree["panel"]["buttons"]["sponsor"],
        json!({"label": "Sponsor in", "color": "blue", "confirm": true, "on_air": true})
    );
    assert_eq!(
        tree["panel"]["buttons"]["edit"]["on_air"], false,
        "an edit of a layer off air"
    );
    assert!(
        !tree.to_string().contains("Nordlys"),
        "actions never leave the app"
    );

    // `button_panel` alone is enough: the actions aren't checked against other permissions.
    let r = e.command_p(
        "b1",
        "press_button",
        json!({"button": "sponsor"}),
        &["button_panel"],
        e.seq(),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(e.on_air("l1"));
    assert_eq!(e.field("l1", "name"), "Nordlys");
    assert!(e
        .st
        .remote_activity()
        .iter()
        .any(|a| a.text.ends_with("pressed Sponsor in")));

    // The first failing step stops it; the steps before stay done.
    let r = e.p2("b2", "press_button", json!({"button": "fix"}));
    assert_eq!(
        (r["ok"].as_bool(), r["code"].as_str()),
        (Some(false), Some("not_found"))
    );
    assert_eq!(r["details"], json!({"step": 2, "steps": 2}));
    assert_eq!(e.field("l1", "name"), "Fixed");

    // Locked: a button that can change what's on air is refused; without the permission too.
    e.st.remote_lock(true).unwrap();
    let r = e.p2("b3", "press_button", json!({"button": "sponsor"}));
    assert_eq!(r["code"], "locked");
    let r = e.command_p(
        "b4",
        "press_button",
        json!({"button": "sponsor"}),
        &["take_clear"],
        e.seq(),
    );
    assert_eq!(r["code"], "permission");
    let r = e.p2("b5", "press_button", json!({"button": "nope"}));
    assert_eq!(r["code"], "not_found");
}
