//! Display outputs: clamping, the plan (matching, fallback, missing, spanning pairs, patterns,
//! no project, missing screens) and the runtime against a recording window host and the test
//! display backend. No real window is ever created.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use serde_json::Value;

use super::*;
use crate::core::displays::TestBackend;
use crate::core::events::EventSink;
use crate::core::model::{NdiOutputSettings, NewScreen, OutputDisplayRef, OutputKind, Prefs};
use crate::core::state::clamp_prefs;

// ---- Helpers ---------------------------------------------------------------------------------

fn display(id: &str, x: i32, y: i32, w: u32, h: u32, hz: u32) -> DisplayInfo {
    DisplayInfo {
        id: id.into(),
        name: format!("Display {id}"),
        edid_key: format!("DEL-A0B1-{id}"),
        x,
        y,
        width: w,
        height: h,
        refresh_hz: hz,
        primary: x == 0 && y == 0,
        has_main_window: false,
    }
}

fn fill(id: &str, d: &DisplayInfo, screen: &str) -> OutputConfig {
    OutputConfig {
        id: id.into(),
        kind: OutputKind::Display,
        name: format!("Output {id}"),
        enabled: true,
        display: OutputDisplayRef {
            id: d.id.clone(),
            name: d.name.clone(),
            edid_key: d.edid_key.clone(),
            width: d.width,
            height: d.height,
        },
        screen_id: Some(screen.into()),
        mode: OutputMode::Fill,
        background: OutputBackground::Black,
        key_of: None,
        always_on_top: false,
        audio: None,
        ndi: None,
    }
}

fn key(id: &str, d: &DisplayInfo, of: &str) -> OutputConfig {
    OutputConfig {
        mode: OutputMode::Key,
        key_of: Some(of.into()),
        screen_id: None,
        ..fill(id, d, "x")
    }
}

fn screens() -> HashMap<String, (u32, u32)> {
    HashMap::from([("main".to_string(), (1920, 1080))])
}

const RUNNING: ServerState = ServerState::Running { port: 7770 };

fn plan_running(
    configs: &[OutputConfig],
    displays: &[DisplayInfo],
    patterns: &HashMap<String, TestPattern>,
) -> (Vec<WindowPlan>, Vec<OutputStatus>) {
    let s = screens();
    plan(
        configs,
        displays,
        Some(PlanProject {
            server: &RUNNING,
            screens: &s,
        }),
        patterns,
    )
}

fn http(w: &WindowPlan) -> &str {
    match &w.url {
        WindowUrl::Http(u) => u,
        WindowUrl::App(u) => panic!("expected the server, got {u}"),
    }
}

fn app(w: &WindowPlan) -> &str {
    match &w.url {
        WindowUrl::App(u) => u,
        WindowUrl::Http(u) => panic!("expected the idle page, got {u}"),
    }
}

// ---- Clamp -----------------------------------------------------------------------------------

#[test]
fn clamp_rules() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let b = display("b", 1920, 0, 1920, 1080, 60);
    let c = display("c", 3840, 0, 1920, 1080, 60);
    let mut list = vec![
        fill("o1", &a, "main"),
        fill("o1", &b, "main"), // repeated id: dropped
        fill("bad id!", &b, "main"),
        OutputConfig {
            name: "  output O1 ".into(), // same name ignoring case
            background: OutputBackground::Color {
                color: "#00FF7F".into(),
            },
            ..fill("o2", &b, "main")
        },
        OutputConfig {
            name: "\u{7}".into(),
            background: OutputBackground::Color {
                color: "green".into(),
            },
            screen_id: Some("../x".into()),
            ..fill("o3", &a, "main") // same display as o1, enabled: turned off
        },
        key("k1", &c, "o1"),
        key("k2", &b, "o1"), // o1 already has a key
        key("k3", &b, "nope"),
        key("k4", &b, "k1"), // a key output isn't a fill
    ];
    list[0].name = "Output O1".into();
    list.push(OutputConfig {
        key_of: Some("o1".into()),
        ..fill("f9", &a, "main")
    });
    let out = clamp_outputs(list);
    let ids: Vec<&str> = out.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["o1", "o2", "o3", "k1", "k2", "k3", "k4", "f9"]);
    assert_eq!(out[0].name, "Output O1");
    assert_eq!(out[1].name, "output O1 2");
    assert_eq!(
        out[1].background,
        OutputBackground::Color {
            color: "#00ff7f".into()
        }
    );
    assert_eq!(out[2].name, "Output 3");
    assert_eq!(out[2].background, OutputBackground::Black);
    assert_eq!(out[2].screen_id, None);
    assert!(!out[2].enabled, "second enabled output on display a");
    assert!(out[3].enabled && out[3].key_of.as_deref() == Some("o1"));
    for k in &out[4..7] {
        assert!(!k.enabled && k.key_of.is_none(), "{k:?}");
    }
    assert_eq!(out[7].key_of, None, "fill outputs have no keyOf");

    // ≤ 8, long names are cut, suffixes stay within 40 characters.
    let long = "x".repeat(60);
    let many: Vec<OutputConfig> = (0..12)
        .map(|i| OutputConfig {
            name: long.clone(),
            enabled: false,
            ..fill(&format!("o{i}"), &a, "main")
        })
        .collect();
    let out = clamp_outputs(many);
    assert_eq!(out.len(), OUTPUTS_MAX);
    assert_eq!(out[0].name.chars().count(), 40);
    assert_eq!(out[1].name.chars().count(), 40);
    assert!(out[1].name.ends_with(" 2"));
    let lower: HashSet<String> = out.iter().map(|c| c.name.to_lowercase()).collect();
    assert_eq!(lower.len(), out.len());
}

#[test]
fn prefs_keep_outputs_and_drop_bad_entries() {
    let json = r#"{"theme":"dark","outputs":[
        {"id":"o1","kind":"display","name":"Program","enabled":true,
         "display":{"id":"\\\\?\\DISPLAY#DEL1#1","name":"DELL","edidKey":"DEL-0001-00000001","width":1920,"height":1080},
         "screenId":"main","mode":"fill","background":{"kind":"green"},"keyOf":null,"alwaysOnTop":true},
        {"id":"o2","kind":"decklink","name":"Later"},
        {"id":"o3","name":"Minimal"},
        42
    ]}"#;
    let p: Prefs = serde_json::from_str(json).unwrap();
    let p = clamp_prefs(p);
    assert_eq!(
        p.outputs.len(),
        2,
        "unknown kinds and non-objects are dropped"
    );
    assert_eq!(p.outputs[0].background, OutputBackground::Green);
    assert!(p.outputs[0].always_on_top);
    assert_eq!(p.outputs[1].mode, OutputMode::Fill);
    assert!(!p.outputs[1].enabled);
    // Round trip; empty outputs are omitted.
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["outputs"][0]["display"]["edidKey"], "DEL-0001-00000001");
    assert_eq!(v["outputs"][0]["keyOf"], Value::Null);
    let back: Prefs = serde_json::from_value(v).unwrap();
    assert_eq!(back.outputs, p.outputs);
    let none = serde_json::to_value(Prefs::default()).unwrap();
    assert!(none.get("outputs").is_none());
    let bad: Prefs = serde_json::from_str(r#"{"outputs":"nope"}"#).unwrap();
    assert!(bad.outputs.is_empty());
}

#[test]
fn query_values_are_encoded() {
    assert_eq!(enc("DELL U2720Q"), "DELL%20U2720Q");
    assert_eq!(enc("a&b=c#d"), "a%26b%3Dc%23d");
    assert_eq!(enc("Ø"), "%C3%98");
    assert_eq!(enc("ok-_.~9"), "ok-_.~9");
    assert_eq!(bg_hex(&OutputBackground::Green), "00b140");
    assert_eq!(
        bg_hex(&OutputBackground::Color {
            color: "#ABCDEF".into()
        }),
        "abcdef"
    );
}

// ---- Plan ------------------------------------------------------------------------------------

#[test]
fn a_fill_output_shows_its_screen_on_its_display() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let b = display("b", 1920, 0, 2560, 1440, 144);
    let mut o = fill("o1", &b, "main");
    o.background = OutputBackground::Green;
    o.always_on_top = true;
    let (w, s) = plan_running(&[o], &[a, b], &HashMap::new());
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].label, "output-o1");
    assert_eq!(
        w[0].rect,
        PlanRect {
            x: 1920,
            y: 0,
            width: 2560,
            height: 1440
        }
    );
    assert!(w[0].always_on_top);
    assert_eq!(
        http(&w[0]),
        "http://127.0.0.1:7770/screen/main?output=o1&bg=00b140&mode=fill"
    );
    assert_eq!(s[0].state, OutputState::Showing);
    assert_eq!(s[0].display_id.as_deref(), Some("b"));
    assert_eq!(s[0].frame_locked, None);
    assert_eq!(s[0].message, None);
}

#[test]
fn off_outputs_have_no_window() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let mut o = fill("o1", &a, "main");
    o.enabled = false;
    let (w, s) = plan_running(&[o], &[a], &HashMap::new());
    assert!(w.is_empty());
    assert_eq!(
        s[0],
        OutputStatus {
            id: "o1".into(),
            state: OutputState::Off,
            display_id: None,
            frame_locked: None,
            pattern: None,
            message: None,
            ndi: None,
        }
    );
}

#[test]
fn a_display_on_another_port_is_matched_by_edid_and_size() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let old = display("old-port", 1920, 0, 1920, 1080, 60);
    let o = fill("o1", &old, "main");
    let mut moved = old.clone();
    moved.id = "new-port".into();
    moved.x = -1920;
    let (w, s) = plan_running(
        std::slice::from_ref(&o),
        &[a.clone(), moved.clone()],
        &HashMap::new(),
    );
    assert_eq!(w[0].rect.x, -1920);
    assert_eq!(s[0].state, OutputState::Showing);
    assert_eq!(s[0].display_id.as_deref(), Some("new-port"));
    assert_eq!(s[0].message.as_deref(), Some("Moved to another port"));

    // Another size (or no EDID key): missing.
    let mut resized = moved.clone();
    resized.width = 2560;
    let (w, s) = plan_running(
        std::slice::from_ref(&o),
        &[a.clone(), resized],
        &HashMap::new(),
    );
    assert!(w.is_empty());
    assert_eq!(s[0].state, OutputState::DisplayMissing);
    assert_eq!(s[0].display_id, None);
    let mut no_edid = o.clone();
    no_edid.display.edid_key.clear();
    let mut blank = moved;
    blank.edid_key.clear();
    let (w, s) = plan_running(&[no_edid], &[a, blank], &HashMap::new());
    assert!(w.is_empty());
    assert_eq!(s[0].state, OutputState::DisplayMissing);
}

#[test]
fn the_fallback_never_takes_a_display_another_output_has() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let b = display("b", 1920, 0, 1920, 1080, 60);
    // o2 remembers a display with b's EDID key, but o1 is on b by id.
    let o1 = fill("o1", &b, "main");
    let mut o2 = fill("o2", &b, "main");
    o2.display.id = "gone".into();
    let (w, s) = plan_running(&[o2, o1], &[a, b], &HashMap::new());
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].label, "output-o1");
    assert_eq!(s[0].state, OutputState::DisplayMissing);
    assert_eq!(s[1].state, OutputState::Showing);
}

#[test]
fn two_outputs_on_one_display() {
    // Clamp turns the later enabled one off; a pattern can still turn an off one on.
    let a = display("a", 0, 0, 1920, 1080, 60);
    let o1 = fill("o1", &a, "main");
    let o2 = fill("o2", &a, "main");
    let list = clamp_outputs(vec![o1, o2]);
    assert!(!list[1].enabled);
    let (w, s) = plan_running(&list, std::slice::from_ref(&a), &HashMap::new());
    assert_eq!(w.len(), 1);
    assert_eq!(s[1].state, OutputState::Off);
    let patterns = HashMap::from([("o2".to_string(), TestPattern::Bars)]);
    let (w, s) = plan_running(&list, &[a], &patterns);
    assert_eq!(w.len(), 1);
    assert_eq!(s[1].state, OutputState::Failed);
    assert_eq!(
        s[1].message.as_deref(),
        Some("Display used by another output")
    );
    assert_eq!(s[1].pattern, Some(TestPattern::Bars));
}

#[test]
fn a_key_pair_on_adjacent_matching_displays_is_one_window() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let b = display("b", 1920, 0, 1920, 1080, 60);
    let c = display("c", 0, 1080, 1920, 1080, 60);
    let mut f = fill("f", &a, "main");
    f.background = OutputBackground::Color {
        color: "#112233".into(),
    };
    let k = OutputConfig {
        always_on_top: true,
        ..key("k", &b, "f")
    };
    let (w, s) = plan_running(
        &[f.clone(), k],
        &[a.clone(), b.clone(), c.clone()],
        &HashMap::new(),
    );
    assert_eq!(w.len(), 1, "{w:#?}");
    assert_eq!(w[0].label, "output-f");
    assert_eq!(w[0].outputs, ["f", "k"]);
    assert!(w[0].always_on_top, "either output's always on top");
    assert_eq!(
        w[0].rect,
        PlanRect {
            x: 0,
            y: 0,
            width: 3840,
            height: 1080
        }
    );
    assert_eq!(
        http(&w[0]),
        "http://127.0.0.1:7770/screen/main?output=f&bg=112233&mode=fill&dual=h"
    );
    assert!(s.iter().all(|s| s.frame_locked == Some(true)));
    assert!(s.iter().all(|s| s.state == OutputState::Showing));
    assert_eq!(s[1].display_id.as_deref(), Some("b"));

    // Stacked: dual=v.
    let (w, s) = plan_running(&[f, key("k", &c, "f")], &[a, b, c], &HashMap::new());
    assert_eq!(w.len(), 1);
    assert_eq!(
        w[0].rect,
        PlanRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 2160
        }
    );
    assert!(http(&w[0]).ends_with("&dual=v"));
    assert!(s.iter().all(|s| s.frame_locked == Some(true)));
}

#[test]
fn a_key_left_of_or_above_its_fill_spans_too() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let left = display("l", -1920, 0, 1920, 1080, 60);
    let above = display("t", 0, -1080, 1920, 1080, 60);
    let f = fill("f", &a, "main");
    // Key left of the fill: dual=hr, the window starts at the key's display.
    let (w, s) = plan_running(
        &[f.clone(), key("k", &left, "f")],
        &[a.clone(), left.clone(), above.clone()],
        &HashMap::new(),
    );
    assert_eq!(w.len(), 1, "{w:#?}");
    assert_eq!(w[0].label, "output-f");
    assert_eq!(w[0].outputs, ["f", "k"]);
    assert_eq!(
        w[0].rect,
        PlanRect {
            x: -1920,
            y: 0,
            width: 3840,
            height: 1080
        }
    );
    assert_eq!(
        http(&w[0]),
        "http://127.0.0.1:7770/screen/main?output=f&bg=000000&mode=fill&dual=hr"
    );
    assert!(s.iter().all(|s| s.frame_locked == Some(true)));
    assert!(s.iter().all(|s| s.state == OutputState::Showing));
    assert_eq!(s[0].display_id.as_deref(), Some("a"));
    assert_eq!(s[1].display_id.as_deref(), Some("l"));

    // Key above the fill: dual=vr.
    let (w, s) = plan_running(
        &[f.clone(), key("k", &above, "f")],
        &[a.clone(), left, above],
        &HashMap::new(),
    );
    assert_eq!(w.len(), 1);
    assert_eq!(
        w[0].rect,
        PlanRect {
            x: 0,
            y: -1080,
            width: 1920,
            height: 2160
        }
    );
    assert!(http(&w[0]).ends_with("&dual=vr"));
    assert!(s.iter().all(|s| s.frame_locked == Some(true)));

    // A pattern on the pair: one spanning pattern window, reversed too.
    let k = display("k1", 1920, 0, 1920, 1080, 60);
    let f2 = fill("f", &k, "main");
    let patterns = HashMap::from([("f".to_string(), TestPattern::Bars)]);
    let (w, _) = plan_running(&[f2, key("k", &a, "f")], &[a, k], &patterns);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].rect.x, 0);
    assert!(http(&w[0]).starts_with("http://127.0.0.1:7770/pattern?kind=bars&"));
    assert!(http(&w[0]).ends_with("&mode=fill&dual=hr"));
}

#[test]
fn a_key_pair_that_cant_span_is_two_windows() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let f = fill("f", &a, "main");
    let cases = [
        ("different refresh", display("b", 1920, 0, 1920, 1080, 50)),
        ("different size", display("b", 1920, 0, 2560, 1440, 60)),
        ("not adjacent", display("b", 3840, 0, 1920, 1080, 60)),
        ("offset edge", display("b", 1920, 200, 1920, 1080, 60)),
        ("gap below", display("b", 0, 1100, 1920, 1080, 60)),
    ];
    for (what, b) in cases {
        let (w, s) = plan_running(
            &[f.clone(), key("k", &b, "f")],
            &[a.clone(), b.clone()],
            &HashMap::new(),
        );
        assert_eq!(w.len(), 2, "{what}");
        assert_eq!(w[0].label, "output-f");
        assert_eq!(w[1].label, "output-k");
        assert_eq!(
            http(&w[0]),
            "http://127.0.0.1:7770/screen/main?output=f&bg=000000&mode=fill",
            "{what}: the fill's window has no key"
        );
        assert_eq!(
            http(&w[1]),
            "http://127.0.0.1:7770/screen/main?output=k&bg=000000&mode=key&audio=0",
            "{what}: the key shows its fill's screen"
        );
        assert!(s.iter().all(|s| s.frame_locked == Some(false)), "{what}");
    }
}

#[test]
fn a_key_without_its_fill_shows_alone() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let b = display("b", 1920, 0, 1920, 1080, 60);
    let mut f = fill("f", &a, "main");
    f.enabled = false;
    let (w, s) = plan_running(
        &[f.clone(), key("k", &b, "f")],
        &[a.clone(), b.clone()],
        &HashMap::new(),
    );
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].label, "output-k");
    assert!(http(&w[0]).contains("mode=key"));
    assert_eq!(s[1].frame_locked, None);
    // Its fill's display missing: two outputs, no pair.
    f.enabled = true;
    let (w, s) = plan_running(&[f, key("k", &b, "f")], &[b], &HashMap::new());
    assert_eq!(w.len(), 1);
    assert_eq!(s[0].state, OutputState::DisplayMissing);
    assert_eq!(s[0].frame_locked, None);
}

#[test]
fn test_patterns_replace_the_screen_on_both_of_a_pair() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let b = display("b", 1920, 0, 1920, 1080, 60);
    let mut f = fill("f", &a, "main");
    f.name = "Program A&B".into();
    let mut k = key("k", &b, "f");
    // The partner is off: it stays off; only the asked-for output shows the pattern.
    k.enabled = false;
    let patterns = HashMap::from([("f".to_string(), TestPattern::Safe)]);
    let (w, s) = plan_running(&[f.clone(), k.clone()], &[a.clone(), b.clone()], &patterns);
    assert_eq!(w.len(), 1);
    assert!(!http(&w[0]).contains("dual="), "{}", http(&w[0]));
    assert_eq!(s[0].pattern, Some(TestPattern::Safe));
    assert_eq!((s[1].pattern, s[1].state), (None, OutputState::Off));
    // The same from the key's side: its fill, off, stays off.
    let mut f_off = f.clone();
    f_off.enabled = false;
    let k_pat = HashMap::from([("k".to_string(), TestPattern::Bars)]);
    let (w, s) = plan_running(&[f_off, k.clone()], &[a.clone(), b.clone()], &k_pat);
    assert_eq!(w.len(), 1);
    assert_eq!((s[0].pattern, s[0].state), (None, OutputState::Off));
    assert_eq!(s[1].pattern, Some(TestPattern::Bars));
    // Both on: the pattern shows on the pair (one spanning window).
    k.enabled = true;
    let (w, s) = plan_running(&[f.clone(), k.clone()], &[a.clone(), b.clone()], &patterns);
    assert_eq!(w.len(), 1);
    assert_eq!(
        http(&w[0]),
        "http://127.0.0.1:7770/pattern?kind=safe&w=1920&h=1080&name=Program%20A%26B&display=Display%20a&res=1920x1080&hz=60&mode=fill&dual=h"
    );
    assert!(s.iter().all(|s| s.pattern == Some(TestPattern::Safe)));
    assert!(s.iter().all(|s| s.state == OutputState::Showing));

    // An off output with a pattern, its screen missing: the display's size.
    let mut solo = fill("o", &b, "gone");
    solo.enabled = false;
    let patterns = HashMap::from([("o".to_string(), TestPattern::Name)]);
    let (w, s) = plan_running(&[solo.clone()], std::slice::from_ref(&b), &patterns);
    assert!(http(&w[0]).starts_with("http://127.0.0.1:7770/pattern?kind=name&w=1920&h=1080&"));
    assert!(http(&w[0]).ends_with("&mode=fill"));
    assert_eq!(s[0].state, OutputState::Showing);
    // Without it, the off output has no window.
    let (w, _) = plan_running(&[solo], &[b], &HashMap::new());
    assert!(w.is_empty());
}

#[test]
fn no_project_shows_the_idle_page() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let b = display("b", 3840, 0, 1920, 1080, 60);
    let mut f = fill("f", &a, "main");
    f.background = OutputBackground::Green;
    let (w, s) = plan(
        &[f.clone(), key("k", &b, "f")],
        &[a.clone(), b.clone()],
        None,
        &HashMap::new(),
    );
    assert_eq!(app(&w[0]), "output-idle.html?bg=00b140");
    assert_eq!(app(&w[1]), "output-idle.html?bg=000000&mode=key");
    assert!(s.iter().all(|s| s.state == OutputState::NoProject));
    // A pattern without a server: the idle page with the name.
    let patterns = HashMap::from([("f".to_string(), TestPattern::Bars)]);
    let (w, s) = plan(&[f], &[a], None, &patterns);
    assert_eq!(
        app(&w[0]),
        "output-idle.html?bg=00b140&name=Output%20f&res=1920x1080&hz=60"
    );
    assert_eq!(s[0].pattern, Some(TestPattern::Bars));
}

#[test]
fn a_missing_screen_or_server_shows_the_idle_page() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let o = fill("o", &a, "elsewhere");
    let (w, s) = plan_running(
        std::slice::from_ref(&o),
        std::slice::from_ref(&a),
        &HashMap::new(),
    );
    assert_eq!(app(&w[0]), "output-idle.html?bg=000000");
    assert_eq!(s[0].state, OutputState::ScreenMissing);
    let mut none = o.clone();
    none.screen_id = None;
    let (_, s) = plan_running(&[none], std::slice::from_ref(&a), &HashMap::new());
    assert_eq!(s[0].state, OutputState::ScreenMissing);

    let sc = screens();
    let failed = ServerState::Failed {
        message: "Port 7770 is already in use".into(),
    };
    let ok = fill("o", &a, "main");
    let (w, s) = plan(
        std::slice::from_ref(&ok),
        std::slice::from_ref(&a),
        Some(PlanProject {
            server: &failed,
            screens: &sc,
        }),
        &HashMap::new(),
    );
    assert_eq!(app(&w[0]), "output-idle.html?bg=000000");
    assert_eq!(s[0].state, OutputState::Failed);
    assert_eq!(s[0].message.as_deref(), Some("Port 7770 is already in use"));
    // Starting: black for a moment, reported as showing (no flicker in the status).
    let (w, s) = plan(
        &[ok],
        &[a],
        Some(PlanProject {
            server: &ServerState::Starting,
            screens: &sc,
        }),
        &HashMap::new(),
    );
    assert_eq!(app(&w[0]), "output-idle.html?bg=000000");
    assert_eq!(s[0].state, OutputState::Showing);
}

#[test]
fn identify_shows_every_display() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let mut b = display("b", 1920, 0, 3840, 2160, 30);
    b.name = "LG HDR 4K".into();
    let w = identify_plan(&[a, b]);
    assert_eq!(w.len(), 2);
    assert_eq!(w[1].label, "identify-2");
    assert!(w[1].always_on_top);
    assert_eq!(
        app(&w[1]),
        "output-idle.html?bg=000000&name=LG%20HDR%204K&res=3840x2160&hz=30"
    );
}

// ---- Runtime ---------------------------------------------------------------------------------

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

/// Records what it was asked to show; never creates a window.
#[derive(Default)]
struct Host {
    applied: Mutex<Vec<(Instant, Vec<WindowPlan>)>>,
    identified: Mutex<Vec<Vec<WindowPlan>>>,
    fail: Mutex<HashSet<String>>,
    /// Labels whose page "keeps crashing" (the Tauri host's give-up failure).
    crashing: Mutex<HashSet<String>>,
    closed: Mutex<usize>,
}

impl OutputHost for Host {
    fn apply(&self, windows: &[WindowPlan]) -> HashMap<String, String> {
        self.applied
            .lock()
            .unwrap()
            .push((Instant::now(), windows.to_vec()));
        let fail = self.fail.lock().unwrap();
        let crashing = self.crashing.lock().unwrap();
        windows
            .iter()
            .filter_map(|w| {
                if crashing.contains(&w.label) {
                    Some((w.label.clone(), PAGE_CRASHING.to_string()))
                } else {
                    fail.contains(&w.label)
                        .then(|| (w.label.clone(), "boom".to_string()))
                }
            })
            .collect()
    }
    fn identify(&self, windows: Vec<WindowPlan>, _duration: Duration) {
        self.identified.lock().unwrap().push(windows);
    }
    fn close_all(&self) {
        *self.closed.lock().unwrap() += 1;
    }
}

impl Host {
    fn last(&self) -> Vec<WindowPlan> {
        self.applied
            .lock()
            .unwrap()
            .last()
            .map(|(_, w)| w.clone())
            .unwrap_or_default()
    }
    fn count(&self) -> usize {
        self.applied.lock().unwrap().len()
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    st: AppState,
    rec: Arc<Recorder>,
    host: Arc<Host>,
    displays: Arc<TestBackend>,
    dir: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        rec.clone(),
        Duration::from_millis(20),
    )
    .unwrap();
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    let displays = Arc::new(TestBackend::new(
        vec![
            display("a", 0, 0, 1920, 1080, 60),
            display("b", 1920, 0, 1920, 1080, 60),
        ],
        Duration::from_millis(50),
    ));
    st.set_display_backend(Some(displays.clone()));
    let host = Arc::new(Host::default());
    st.set_output_host(Some(host.clone()));
    let dir = tmp.path().to_path_buf();
    Env {
        _tmp: tmp,
        st,
        rec,
        host,
        displays,
        dir,
    }
}

fn set_outputs(st: &AppState, outputs: Vec<OutputConfig>) {
    let mut p = st.prefs();
    p.outputs = outputs;
    st.set_prefs(p);
}

fn open_with_screen(e: &Env) -> String {
    e.st.create_project(&e.dir, "Show").unwrap();
    e.st.create_screen(&NewScreen {
        name: "Main".into(),
        width: 1280,
        height: 720,
        background: "transparent".into(),
    })
    .unwrap()
    .doc
    .id
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn nothing_runs_without_outputs() {
    let e = env();
    e.st.outputs_reconcile_now();
    assert_eq!(e.host.count(), 0, "no window work");
    assert!(!e.st.displays_rt().watching(), "no displays thread");
    assert_eq!(e.displays.threads(), 0);
    assert_eq!(
        e.displays.enumerations(),
        0,
        "displays aren't even enumerated"
    );
    assert!(e.st.list_outputs().is_empty());
    assert!(!e.st.any_output_showing());
    assert!(e.rec.named(names::OUTPUTS_CHANGED).is_empty());
}

#[test]
fn outputs_follow_prefs_project_screens_and_server() {
    let e = env();
    let a = display("a", 0, 0, 1920, 1080, 60);
    set_outputs(
        &e.st,
        vec![OutputConfig {
            screen_id: None,
            ..fill("o1", &a, "main")
        }],
    );
    e.st.outputs_reconcile_now();
    // No project: black, the watcher runs while the output is on.
    assert_eq!(e.host.last().len(), 1);
    assert_eq!(app(&e.host.last()[0]), "output-idle.html?bg=000000");
    assert_eq!(e.st.list_outputs()[0].state, OutputState::NoProject);
    assert!(e.st.any_output_showing());
    assert!(e.st.displays_rt().watching());
    assert_eq!(e.displays.threads(), 1);

    let screen = open_with_screen(&e);
    let mut p = e.st.prefs();
    p.outputs[0].screen_id = Some(screen.clone());
    e.st.set_prefs(p);
    e.st.outputs_reconcile_now();
    assert_eq!(
        e.st.list_outputs()[0].state,
        OutputState::Showing,
        "server starting"
    );
    e.st.outputs_set_server(&ServerStatus::Running {
        port: 7771,
        url: "http://127.0.0.1:7771".into(),
        lan_url: None,
    });
    e.st.outputs_reconcile_now();
    assert_eq!(
        http(&e.host.last()[0]),
        format!("http://127.0.0.1:7771/screen/{screen}?output=o1&bg=000000&mode=fill")
    );
    // Unchanged: no window work, no event.
    let applied = e.host.count();
    let events = e.rec.named(names::OUTPUTS_CHANGED).len();
    e.st.outputs_reconcile_now();
    assert_eq!(e.host.count(), applied);
    assert_eq!(e.rec.named(names::OUTPUTS_CHANGED).len(), events);

    // Deleting the screen: black + screenMissing.
    e.st.delete_screen(&screen).unwrap();
    e.st.outputs_reconcile_now();
    assert_eq!(app(&e.host.last()[0]), "output-idle.html?bg=000000");
    assert_eq!(e.st.list_outputs()[0].state, OutputState::ScreenMissing);
    let last = e.rec.named(names::OUTPUTS_CHANGED).pop().unwrap();
    assert_eq!(last[0]["state"], "screenMissing");

    // Closing the project: black, the window stays.
    e.st.close_project();
    e.st.outputs_reconcile_now();
    assert_eq!(e.host.last().len(), 1);
    assert_eq!(e.st.list_outputs()[0].state, OutputState::NoProject);

    // Off: the window closes and the watcher stops.
    e.st.all_outputs_off();
    e.st.outputs_reconcile_now();
    assert!(e.host.last().is_empty());
    assert_eq!(e.st.list_outputs()[0].state, OutputState::Off);
    assert!(!e.st.any_output_showing());
    assert!(!e.st.displays_rt().watching());
    assert_eq!(e.displays.threads(), 0);
}

#[test]
fn test_patterns_come_and_go() {
    let e = env();
    let a = display("a", 0, 0, 1920, 1080, 60);
    let mut o = fill("o1", &a, "main");
    o.enabled = false;
    set_outputs(&e.st, vec![o]);
    e.st.outputs_reconcile_now();
    assert!(e.host.last().is_empty());
    assert!(matches!(
        e.st.output_test_pattern("nope", Some(TestPattern::Bars)),
        Err(AppError::NotFound(_))
    ));
    e.st.output_test_pattern("o1", Some(TestPattern::Grid))
        .unwrap();
    e.st.outputs_reconcile_now();
    assert_eq!(e.host.last().len(), 1, "the pattern turns it on");
    let s = &e.st.list_outputs()[0];
    assert_eq!(s.pattern, Some(TestPattern::Grid));
    assert!(e.st.displays_rt().watching());
    e.st.output_test_pattern("o1", None).unwrap();
    e.st.outputs_reconcile_now();
    assert!(e.host.last().is_empty(), "off again");
    assert_eq!(e.st.list_outputs()[0].pattern, None);
    // Deleting an output drops its pattern.
    e.st.output_test_pattern("o1", Some(TestPattern::Name))
        .unwrap();
    set_outputs(&e.st, vec![]);
    e.st.outputs_reconcile_now();
    assert!(e.host.last().is_empty());
    assert!(e.st.list_outputs().is_empty());
    assert!(!e.st.displays_rt().watching());
}

#[test]
fn a_window_that_fails_is_reported_and_retried() {
    let e = env();
    let a = display("a", 0, 0, 1920, 1080, 60);
    e.host.fail.lock().unwrap().insert("output-o1".into());
    set_outputs(&e.st, vec![fill("o1", &a, "main")]);
    e.st.outputs_reconcile_now();
    let s = &e.st.list_outputs()[0];
    assert_eq!(s.state, OutputState::Failed);
    assert_eq!(
        s.message.as_deref(),
        Some("Couldn't open the output window")
    );
    e.host.fail.lock().unwrap().clear();
    let before = e.host.count();
    e.st.outputs_reconcile_now();
    assert_eq!(e.host.count(), before + 1, "retried");
    assert_eq!(e.st.list_outputs()[0].state, OutputState::NoProject);
}

#[test]
fn a_page_that_keeps_crashing_is_reported() {
    let e = env();
    let a = display("a", 0, 0, 1920, 1080, 60);
    set_outputs(&e.st, vec![fill("o1", &a, "main")]);
    e.st.outputs_reconcile_now();
    assert_eq!(e.st.list_outputs()[0].state, OutputState::NoProject);
    // The host gave up on the window (outside a reconcile): the next one hands it the
    // unchanged windows again so it can report the failure.
    e.host.crashing.lock().unwrap().insert("output-o1".into());
    let before = e.host.count();
    e.st.outputs_reconcile_now();
    assert_eq!(e.host.count(), before, "unchanged: no window work");
    e.st.outputs_host_changed();
    e.st.outputs_reconcile_now();
    assert_eq!(e.host.count(), before + 1);
    let s = &e.st.list_outputs()[0];
    assert_eq!(s.state, OutputState::Failed);
    assert_eq!(s.message.as_deref(), Some("The output page keeps crashing"));
}

#[test]
fn audio_goes_into_the_screen_url() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let b = display("b", 1920, 0, 1920, 1080, 60);
    let c = display("c", 0, 1080, 1280, 720, 60);
    let url = |audio: Option<OutputAudio>| {
        let mut f = fill("o1", &a, "main");
        f.audio = audio;
        let (w, _) = plan_running(&[f], std::slice::from_ref(&a), &HashMap::new());
        http(&w[0]).to_string()
    };
    let base = "http://127.0.0.1:7770/screen/main?output=o1&bg=000000&mode=fill";
    assert_eq!(url(None), base, "absent: on, system default");
    assert_eq!(
        url(Some(OutputAudio {
            enabled: true,
            device: None
        })),
        base
    );
    assert_eq!(
        url(Some(OutputAudio {
            enabled: false,
            device: Some("Speakers".into())
        })),
        format!("{base}&audio=0")
    );
    assert_eq!(
        url(Some(OutputAudio {
            enabled: true,
            device: Some("Høyttalere (Scarlett 6i6 USB) & co".into())
        })),
        format!("{base}&sink=H%C3%B8yttalere%20%28Scarlett%206i6%20USB%29%20%26%20co")
    );

    // A key output is always silent, even with audio on; a pair in one window follows the fill.
    let mut f = fill("f", &a, "main");
    f.audio = Some(OutputAudio {
        enabled: true,
        device: Some("Speakers".into()),
    });
    let mut k = key("k", &c, "f");
    k.audio = Some(OutputAudio {
        enabled: true,
        device: Some("Other".into()),
    });
    let (w, _) = plan_running(&[f.clone(), k], &[a.clone(), c.clone()], &HashMap::new());
    assert_eq!(
        http(&w[0]),
        "http://127.0.0.1:7770/screen/main?output=f&bg=000000&mode=fill&sink=Speakers"
    );
    assert_eq!(
        http(&w[1]),
        "http://127.0.0.1:7770/screen/main?output=k&bg=000000&mode=key&audio=0"
    );
    let (w, _) = plan_running(
        &[f.clone(), key("k", &b, "f")],
        &[a.clone(), b.clone()],
        &HashMap::new(),
    );
    assert_eq!(w.len(), 1);
    assert_eq!(
        http(&w[0]),
        "http://127.0.0.1:7770/screen/main?output=f&bg=000000&mode=fill&dual=h&sink=Speakers"
    );
    // Test patterns have no media: no audio parameters.
    let (w, _) = plan_running(
        &[f],
        std::slice::from_ref(&a),
        &HashMap::from([("f".to_string(), TestPattern::Bars)]),
    );
    assert!(!http(&w[0]).contains("sink=") && !http(&w[0]).contains("audio="));
}

#[test]
fn audio_is_clamped_and_round_trips() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let with = |device: &str| {
        let mut f = fill("o1", &a, "main");
        f.audio = Some(OutputAudio {
            enabled: true,
            device: Some(device.into()),
        });
        clamp_outputs(vec![f]).remove(0).audio.unwrap()
    };
    assert_eq!(
        with("  Speakers\u{7} (USB)\n ").device.as_deref(),
        Some("Speakers (USB)")
    );
    assert_eq!(with(" \u{1} ").device, None, "empty: the system default");
    assert_eq!(with(&"x".repeat(400)).device.unwrap().chars().count(), 256);

    // Absent stays absent; a partial object fills in.
    let json = serde_json::to_value(fill("o1", &a, "main")).unwrap();
    assert!(json.get("audio").is_none());
    let mut v = json.clone();
    v["audio"] = serde_json::json!({ "enabled": false });
    let c: OutputConfig = serde_json::from_value(v).unwrap();
    assert_eq!(
        c.audio,
        Some(OutputAudio {
            enabled: false,
            device: None
        })
    );
    let mut v = json;
    v["audio"] = serde_json::json!({ "device": "Speakers" });
    let c: OutputConfig = serde_json::from_value(v).unwrap();
    assert_eq!(
        serde_json::to_value(&c).unwrap()["audio"],
        serde_json::json!({ "enabled": true, "device": "Speakers" })
    );
}

#[test]
fn unplug_and_replug_through_the_watcher() {
    let e = env();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    e.st.set_outputs_runtime(rt.handle().clone());
    let b = display("b", 1920, 0, 1920, 1080, 60);
    set_outputs(&e.st, vec![fill("o1", &b, "main")]);
    wait_until("the window", || e.host.last().len() == 1);
    assert!(e.st.displays_rt().watching());

    // Unplugged: several OS notifications, one re-enumeration after the debounce.
    let enumerations = e.displays.enumerations();
    e.displays.set(vec![display("a", 0, 0, 1920, 1080, 60)]);
    let at = Instant::now();
    for _ in 0..4 {
        e.displays.notify();
        std::thread::sleep(Duration::from_millis(10));
    }
    wait_until("closed", || e.host.last().is_empty());
    assert_eq!(e.displays.enumerations(), enumerations + 1, "debounced");
    assert_eq!(e.st.list_outputs()[0].state, OutputState::DisplayMissing);
    let changed = e.rec.named(names::DISPLAYS_CHANGED);
    assert_eq!(changed.last().unwrap().as_array().unwrap().len(), 1);
    assert!(
        e.st.displays_rt().watching(),
        "still watching for the replug"
    );

    // Replugged: restored within a second of the (last) OS notification.
    e.displays.set(vec![
        display("a", 0, 0, 1920, 1080, 60),
        display("b", 1920, 0, 1920, 1080, 60),
    ]);
    let notified = Instant::now();
    e.displays.notify();
    wait_until("restored", || e.host.last().len() == 1);
    let restored = e.host.applied.lock().unwrap().last().unwrap().0;
    let took = restored.duration_since(notified);
    eprintln!("hot-plug → window restored in {took:?} (debounce 50 ms in this test)");
    assert!(took < Duration::from_secs(1));
    assert!(at.elapsed() < Duration::from_secs(5));

    // A notification without a change: nothing announced, no window work.
    let events = e.rec.named(names::DISPLAYS_CHANGED).len();
    let applied = e.host.count();
    e.displays.notify();
    wait_until("handled", || e.st.displays_rt().os_changes() >= 3);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(e.rec.named(names::DISPLAYS_CHANGED).len(), events);
    assert_eq!(e.host.count(), applied);
    drop(rt);
}

#[test]
fn identify_and_shutdown() {
    let e = env();
    e.st.identify_displays().unwrap();
    let shown = e.host.identified.lock().unwrap().clone();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].len(), 2);
    assert!(
        !e.st.displays_rt().watching(),
        "Identify doesn't start the watcher"
    );
    let a = display("a", 0, 0, 1920, 1080, 60);
    set_outputs(&e.st, vec![fill("o1", &a, "main")]);
    e.st.outputs_reconcile_now();
    assert!(e.st.displays_rt().watching());
    e.st.outputs_shutdown();
    assert_eq!(*e.host.closed.lock().unwrap(), 1);
    assert!(!e.st.displays_rt().watching());
    let before = e.host.count();
    e.st.outputs_reconcile_now();
    assert_eq!(e.host.count(), before, "nothing after exit");
}

#[test]
fn watch_displays_is_counted() {
    let e = env();
    let list = e.st.watch_displays(true);
    assert_eq!(list.len(), 2);
    assert!(e.st.displays_rt().watching());
    e.st.watch_displays(true);
    e.st.watch_displays(false);
    assert!(e.st.displays_rt().watching());
    e.st.watch_displays(false);
    assert!(!e.st.displays_rt().watching());
    e.st.watch_displays(false); // unmatched: ignored
    assert_eq!(e.displays.threads(), 0);
}

#[test]
fn a_host_set_later_gets_the_windows_planned_before() {
    let e = env();
    e.st.set_output_host(None);
    let a = display("a", 0, 0, 1920, 1080, 60);
    set_outputs(&e.st, vec![fill("o1", &a, "main")]);
    e.st.outputs_reconcile_now();
    assert_eq!(e.st.outputs_rt().windows().len(), 1);
    let host = Arc::new(Host::default());
    e.st.set_output_host(Some(host.clone()));
    e.st.outputs_reconcile_now();
    assert_eq!(host.last().len(), 1);
}

// ---- NDI outputs -----------------------------------------------------------------------------

fn ndi(id: &str, screen: &str) -> OutputConfig {
    OutputConfig {
        id: id.into(),
        kind: OutputKind::Ndi,
        name: format!("Kinetra {id}"),
        enabled: true,
        display: OutputDisplayRef::default(),
        screen_id: Some(screen.into()),
        mode: OutputMode::Fill,
        background: OutputBackground::Black,
        key_of: None,
        always_on_top: false,
        audio: None,
        ndi: Some(NdiOutputSettings::default()),
    }
}

#[test]
fn ndi_outputs_are_clamped() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let mut n = ndi("n1", "main");
    n.display = OutputDisplayRef {
        id: a.id.clone(),
        ..Default::default()
    };
    n.mode = OutputMode::Key;
    n.key_of = Some("o1".into());
    n.audio = Some(OutputAudio {
        enabled: true,
        device: None,
    });
    n.always_on_top = true;
    n.ndi = Some(NdiOutputSettings {
        groups: Some("  Studio A \u{7}".into()),
        ..Default::default()
    });
    // A key output can't key an NDI output.
    let k = key("k1", &a, "n1");
    // Without settings an NDI output gets the defaults; a display output loses stray ones.
    let bare = OutputConfig {
        ndi: None,
        ..ndi("n2", "main")
    };
    let disp = OutputConfig {
        ndi: Some(NdiOutputSettings::default()),
        ..fill("o1", &a, "main")
    };
    let mut list = vec![n, k, bare, disp];
    list.extend((3..=5).map(|i| ndi(&format!("n{i}"), "main")));
    let out = clamp_outputs(list);
    let n = &out[0];
    assert_eq!(n.display, OutputDisplayRef::default());
    assert_eq!(n.mode, OutputMode::Fill);
    assert_eq!(n.key_of, None);
    assert_eq!(n.audio, None);
    assert!(!n.always_on_top);
    assert_eq!(n.ndi.as_ref().unwrap().groups.as_deref(), Some("Studio A"));
    let k = out.iter().find(|c| c.id == "k1").unwrap();
    assert_eq!((k.key_of.clone(), k.enabled), (None, false));
    let n2 = out.iter().find(|c| c.id == "n2").unwrap();
    assert_eq!(n2.ndi, Some(NdiOutputSettings::default()));
    assert_eq!(out.iter().find(|c| c.id == "o1").unwrap().ndi, None);
    // At most 4 NDI outputs: the 5th is dropped.
    let count = out.iter().filter(|c| c.kind == OutputKind::Ndi).count();
    assert_eq!(count, NDI_OUTPUTS_MAX);
    assert!(!out.iter().any(|c| c.id == "n5"));
}

#[test]
fn ndi_settings_round_trip_with_rate_labels() {
    let json = r#"{"id":"n1abc","kind":"ndi","name":"Main","enabled":true,"screenId":"main",
        "ndi":{"rate":"59.94","alpha":false,"premultiplied":true,"groups":"A","bus":"preview"}}"#;
    let c: OutputConfig = serde_json::from_str(json).unwrap();
    let s = c.ndi.clone().unwrap();
    assert_eq!(s.rate, NdiRate::R59_94);
    assert!(!s.alpha && s.premultiplied);
    assert_eq!(s.bus, NdiBus::Preview);
    let back = serde_json::to_value(&c).unwrap();
    assert_eq!(back["ndi"]["rate"], "59.94");
    assert_eq!(back["kind"], "ndi");
    // A display output never writes `ndi`.
    let a = display("a", 0, 0, 1920, 1080, 60);
    let d = serde_json::to_value(fill("o1", &a, "main")).unwrap();
    assert!(d.get("ndi").is_none());
}

#[test]
fn the_ndi_plan_follows_project_server_and_screen() {
    let mut sizes = screens();
    sizes.insert("odd".into(), (1279, 721));
    let p = |server| {
        Some(PlanProject {
            server,
            screens: &sizes,
        })
    };
    let mut n = ndi("n1", "main");
    // No project / server failed / screen missing / starting: nothing sent.
    let (plans, st) = plan_ndi(&[n.clone()], None);
    assert!(plans.is_empty());
    assert_eq!(st[0].1.state, OutputState::NoProject);
    let failed = ServerState::Failed {
        message: "Port 7770 is in use".into(),
    };
    let (_, st) = plan_ndi(&[n.clone()], p(&failed));
    assert_eq!(st[0].1.state, OutputState::Failed);
    assert_eq!(st[0].1.message.as_deref(), Some("Port 7770 is in use"));
    let (plans, st) = plan_ndi(&[ndi("n1", "gone")], p(&RUNNING));
    assert!(plans.is_empty());
    assert_eq!(st[0].1.state, OutputState::ScreenMissing);
    let (plans, st) = plan_ndi(&[n.clone()], p(&ServerState::Starting));
    assert!(plans.is_empty());
    assert_eq!(st[0].1.state, OutputState::Showing);
    // Running: the capture page, the screen's size, UYVA straight at the chosen rate.
    let (plans, st) = plan_ndi(&[n.clone()], p(&RUNNING));
    assert_eq!(st[0].1.state, OutputState::Showing);
    assert_eq!(st[0].1.ndi, Some(NdiLive::default()));
    let pl = &plans[0];
    assert_eq!(pl.url, "http://127.0.0.1:7770/screen/main?audio=0&cap=1");
    assert_eq!((pl.width, pl.height, pl.rate), (1920, 1080, FrameRate::R50));
    assert_eq!(pl.format, PixelFormat::Uyva);
    assert_eq!(pl.alpha, AlphaMode::Straight);
    assert_eq!(pl.name, "Kinetra n1");
    // Preview bus, alpha off over a color, premultiplied, odd width rounded up to even.
    n.screen_id = Some("odd".into());
    n.background = OutputBackground::Color {
        color: "#102030".into(),
    };
    n.ndi = Some(NdiOutputSettings {
        rate: NdiRate::R29_97,
        alpha: false,
        premultiplied: true,
        groups: Some("A".into()),
        bus: NdiBus::Preview,
    });
    let (plans, _) = plan_ndi(&[n.clone()], p(&RUNNING));
    let pl = &plans[0];
    assert_eq!(
        pl.url,
        "http://127.0.0.1:7770/screen/odd/preview?audio=0&cap=1"
    );
    assert_eq!(
        (pl.width, pl.height, pl.rate),
        (1280, 721, FrameRate::R29_97)
    );
    assert_eq!(pl.format, PixelFormat::Uyvy);
    assert_eq!(pl.alpha, AlphaMode::Premultiplied);
    assert_eq!(pl.background, [0x10, 0x20, 0x30]);
    assert_eq!(pl.groups.as_deref(), Some("A"));
    // Off: no plan, status off (still marked as NDI).
    n.enabled = false;
    let (plans, st) = plan_ndi(&[n], p(&RUNNING));
    assert!(plans.is_empty());
    assert_eq!(st[0].1.state, OutputState::Off);
    assert_eq!(st[0].1.ndi, Some(NdiLive::default()));
}

#[test]
fn ndi_outputs_have_no_window_and_display_outputs_no_ndi_plan() {
    let a = display("a", 0, 0, 1920, 1080, 60);
    let configs = [ndi("n1", "main"), fill("o1", &a, "main")];
    let (w, s) = plan_running(&configs, &[a], &HashMap::new());
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].outputs, ["o1"]);
    assert_eq!(
        s[0].state,
        OutputState::Off,
        "plan() leaves NDI to plan_ndi"
    );
    let sizes = screens();
    let (plans, st) = plan_ndi(
        &configs,
        Some(PlanProject {
            server: &RUNNING,
            screens: &sizes,
        }),
    );
    assert_eq!(plans.len(), 1);
    assert_eq!(st.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0]);
}

#[test]
fn an_ndi_output_is_on_air_only_with_a_receiver() {
    let status = |state, ndi| OutputStatus {
        id: "x".into(),
        state,
        display_id: None,
        frame_locked: None,
        pattern: None,
        message: None,
        ndi,
    };
    let none = Some(NdiLive::default());
    let one = Some(NdiLive {
        receivers: 1,
        ..Default::default()
    });
    assert!(!status_on_air(&status(OutputState::Showing, none)));
    assert!(status_on_air(&status(OutputState::Showing, one)));
    assert!(!status_on_air(&status(OutputState::NoProject, none)));
    assert!(!status_on_air(&status(OutputState::ScreenMissing, none)));
    assert!(!status_on_air(&status(OutputState::NdiMissing, none)));
    // Display outputs keep their rule.
    assert!(status_on_air(&status(OutputState::NoProject, None)));
    assert!(!status_on_air(&status(OutputState::DisplayMissing, None)));
}

#[test]
fn new_ndi_outputs_default_to_50_in_europe_else_59_94() {
    for r in ["NO", "no", "GB", "DE", "SE", "UA"] {
        assert_eq!(default_ndi_rate(Some(r)), NdiRate::R50, "{r}");
    }
    for r in ["US", "CA", "JP", "BR", "AU", ""] {
        assert_eq!(default_ndi_rate(Some(r)), NdiRate::R59_94, "{r}");
    }
    assert_eq!(default_ndi_rate(None), NdiRate::R59_94);
}

/// Records the NDI plans; receivers and failures are set by the test.
#[derive(Default)]
struct FakeNdi {
    applied: Mutex<Vec<Vec<NdiPlan>>>,
    live: Mutex<HashMap<String, NdiLive>>,
    fail: Mutex<HashMap<String, String>>,
    closed: Mutex<usize>,
}

impl NdiHost for FakeNdi {
    fn apply(&self, plans: &[NdiPlan]) -> HashMap<String, String> {
        self.applied.lock().unwrap().push(plans.to_vec());
        let fail = self.fail.lock().unwrap();
        plans
            .iter()
            .filter_map(|p| fail.get(&p.id).map(|e| (p.id.clone(), e.clone())))
            .collect()
    }
    fn live(&self) -> HashMap<String, NdiLive> {
        self.live.lock().unwrap().clone()
    }
    fn close_all(&self) {
        *self.closed.lock().unwrap() += 1;
    }
}

impl FakeNdi {
    fn count(&self) -> usize {
        self.applied.lock().unwrap().len()
    }
    fn last(&self) -> Vec<NdiPlan> {
        self.applied
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }
}

fn running(port: u16) -> ServerStatus {
    ServerStatus::Running {
        port,
        url: format!("http://127.0.0.1:{port}"),
        lan_url: None,
    }
}

#[test]
fn ndi_outputs_run_through_the_host_and_count_on_air_with_receivers() {
    let e = env();
    let fake = Arc::new(FakeNdi::default());
    e.st.set_ndi_host(Some(fake.clone()));
    let screen = open_with_screen(&e);
    e.st.outputs_set_server(&running(7771));
    set_outputs(&e.st, vec![ndi("n1", &screen)]);
    e.st.outputs_reconcile_now();
    let plans = fake.last();
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0].url,
        format!("http://127.0.0.1:7771/screen/{screen}?audio=0&cap=1")
    );
    assert_eq!((plans[0].width, plans[0].height), (1280, 720));
    assert!(e.host.last().is_empty(), "no window");
    assert!(!e.st.displays_rt().watching(), "no display watcher for NDI");
    let s = &e.st.list_outputs()[0];
    assert_eq!(
        (s.state, s.ndi),
        (OutputState::Showing, Some(NdiLive::default()))
    );
    assert!(!e.st.any_output_showing(), "sending to nobody isn't on air");
    assert!(!e.st.cloud_on_air());

    // Unchanged: no host work.
    let n = fake.count();
    e.st.outputs_reconcile_now();
    assert_eq!(fake.count(), n);

    // A receiver connects: one outputs-changed with it, and now it's on air.
    let events = e.rec.named(names::OUTPUTS_CHANGED).len();
    fake.live.lock().unwrap().insert(
        "n1".into(),
        NdiLive {
            receivers: 1,
            program: true,
            preview: false,
            dropping: false,
        },
    );
    e.st.outputs_ndi_changed();
    let ev = e.rec.named(names::OUTPUTS_CHANGED);
    assert_eq!(ev.len(), events + 1);
    assert_eq!(ev.last().unwrap()[0]["ndi"]["receivers"], 1);
    assert_eq!(ev.last().unwrap()[0]["ndi"]["program"], true);
    assert!(e.st.any_output_showing());
    assert!(e.st.cloud_on_air());
    // The same again: nothing sent.
    e.st.outputs_ndi_changed();
    assert_eq!(e.rec.named(names::OUTPUTS_CHANGED).len(), events + 1);

    // Test patterns are for display outputs.
    assert!(e
        .st
        .output_test_pattern("n1", Some(TestPattern::Bars))
        .is_err());

    // A changed setting is a new plan; the runtime missing shows as ndiMissing and is retried.
    fake.fail
        .lock()
        .unwrap()
        .insert("n1".into(), NDI_MISSING.into());
    let mut p = e.st.prefs();
    p.outputs[0].ndi.as_mut().unwrap().rate = NdiRate::R59_94;
    e.st.set_prefs(p);
    e.st.outputs_reconcile_now();
    assert_eq!(fake.last()[0].rate, FrameRate::R59_94);
    assert_eq!(e.st.list_outputs()[0].state, OutputState::NdiMissing);
    assert!(!e.st.any_output_showing());
    let n = fake.count();
    fake.fail.lock().unwrap().clear();
    e.st.outputs_reconcile_now();
    assert_eq!(
        fake.count(),
        n + 1,
        "a failure is retried on the next reconcile"
    );
    assert_eq!(e.st.list_outputs()[0].state, OutputState::Showing);

    // Off: the host stops it.
    e.st.all_outputs_off();
    e.st.outputs_reconcile_now();
    assert!(fake.last().is_empty());
    assert!(!e.st.any_output_showing());

    // Exit stops everything.
    e.st.outputs_shutdown();
    assert_eq!(*fake.closed.lock().unwrap(), 1);
}

#[test]
fn without_an_ndi_host_an_ndi_output_says_so() {
    let e = env();
    let screen = open_with_screen(&e);
    e.st.outputs_set_server(&running(7771));
    set_outputs(&e.st, vec![ndi("n1", &screen)]);
    e.st.outputs_reconcile_now();
    let s = &e.st.list_outputs()[0];
    assert_eq!(s.state, OutputState::Failed);
    assert_eq!(s.message.as_deref(), Some(NDI_UNAVAILABLE));
}
