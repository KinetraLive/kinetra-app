//! MIDI: parsing, the CC crossing, profiles and the LED diff (pure), then the input path, learn,
//! conflicts, in-use / port lifecycle, hot-plug, LED feedback (latency included) and
//! `trigger.midi`, all against the in-memory test backend.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::backend::{unique_names, SentMessage, TestBackend};
use super::feedback::{diff, status, velocity, Led};
use super::parse::{parse, parse_hex, MidiMsg};
use super::*;
use crate::core::events::EventSink;
use crate::core::model::{
    ControlMapping, FlowEdge, FlowNode, MidiFeedbackPref, MidiInputPref, MidiPrefs, NewFlow,
    NewScreen, NodeKind, NodePos, PortRef, Rect, Retrigger, ScreenDoc, ScreenLayer,
    ScreenLayerKind, TemplateSource, ValueTarget, WireKind,
};
use crate::core::protocol::Fields;

// ---- Pure parts ------------------------------------------------------------------------------

#[test]
fn parses_notes_and_ccs_only() {
    assert_eq!(
        parse(&[0x90, 36, 100]),
        Some(MidiMsg::NoteOn {
            channel: 1,
            note: 36,
            velocity: 100
        })
    );
    assert_eq!(
        parse(&[0x9F, 36, 0]),
        Some(MidiMsg::NoteOff {
            channel: 16,
            note: 36
        })
    );
    assert_eq!(
        parse(&[0x83, 5, 64]),
        Some(MidiMsg::NoteOff {
            channel: 4,
            note: 5
        })
    );
    assert_eq!(
        parse(&[0xB1, 7, 127]),
        Some(MidiMsg::Cc {
            channel: 2,
            number: 7,
            value: 127
        })
    );
    // Pitch bend, program change, clock, SysEx, short and malformed messages.
    for bad in [
        &[0xE0, 0, 64][..],
        &[0xC0, 3],
        &[0xF8],
        &[0xF0, 1, 2, 0xF7],
        &[0x90, 36],
        &[0x90, 0x80, 1],
        &[0x24, 1, 2],
        &[],
    ] {
        assert_eq!(parse(bad), None, "{bad:?}");
    }
    assert_eq!(parse_hex("90 24 7f"), Some(vec![0x90, 0x24, 0x7f]));
    assert_eq!(parse_hex("b0077F"), Some(vec![0xb0, 0x07, 0x7f]));
    assert_eq!(parse_hex("9"), None);
    assert_eq!(parse_hex("zz"), None);
    assert_eq!(parse_hex("é9"), None);
    assert_eq!(parse_hex("90 24 7f 00"), None);
}

#[test]
fn cc_crosses_64() {
    assert_eq!(cc_crossing(0, 127), Some(ControlInput::Press));
    assert_eq!(cc_crossing(63, 64), Some(ControlInput::Press));
    assert_eq!(cc_crossing(64, 100), None);
    assert_eq!(cc_crossing(100, 63), Some(ControlInput::Release));
    assert_eq!(cc_crossing(10, 20), None);
}

#[test]
fn profiles_map_states_to_velocities() {
    use MidiFeedbackProfile as P;
    let t = |p| {
        (
            velocity(p, Led::OnAir),
            velocity(p, Led::Ready),
            velocity(p, Led::Off),
        )
    };
    assert_eq!(t(P::Generic), (127, 1, 0));
    assert_eq!(t(P::Apc40), (3, 1, 0));
    assert_eq!(t(P::ApcMini), (3, 1, 0));
    assert_eq!(t(P::Launchpad), (5, 21, 0));
    assert_eq!(t(P::Xtouch), (127, 0, 0));
    assert_eq!(status(MidiKind::Note, None), 0x90);
    assert_eq!(status(MidiKind::Note, Some(10)), 0x99);
    assert_eq!(status(MidiKind::Cc, Some(1)), 0xB0);
}

#[test]
fn led_diff_sends_changes_only() {
    let mut last = HashMap::new();
    let want = HashMap::from([((0x90, 1), 1), ((0x90, 2), 0)]);
    assert_eq!(
        diff(&mut last, &want),
        vec![[0x90, 1, 1], [0x90, 2, 0]],
        "a new output gets everything"
    );
    assert!(diff(&mut last, &want).is_empty(), "nothing changed");
    let want = HashMap::from([((0x90, 1), 127), ((0x90, 2), 0)]);
    assert_eq!(diff(&mut last, &want), vec![[0x90, 1, 127]]);
    let want = HashMap::from([((0x90, 2), 0)]);
    assert_eq!(
        diff(&mut last, &want),
        vec![[0x90, 1, 0]],
        "an LED no longer wanted is switched off"
    );
    assert!(diff(&mut last, &HashMap::new()).is_empty(), "already off");
    assert!(last.is_empty());
}

#[test]
fn repeated_port_names_are_told_apart() {
    assert_eq!(
        unique_names(["A".to_string(), "B".into(), "A".into(), "A".into()]),
        ["A", "B", "A #2", "A #3"]
    );
}

#[test]
fn lookup_filters_by_device_and_channel() {
    let doc = ControlsDoc {
        mappings: vec![
            mapping("a", Some("Pad"), Some(1), MidiKind::Note, 36, "live.panic"),
            mapping("b", None, Some(2), MidiKind::Note, 36, "live.clearAll"),
            mapping("c", None, None, MidiKind::Cc, 36, "live.clearAll"),
        ],
    };
    let l = Lookup::build(&doc);
    let ids = |d, ch, k, n| -> Vec<String> {
        l.matching(d, ch, k, n)
            .iter()
            .map(|e| e.id.clone())
            .collect()
    };
    assert_eq!(ids("Pad", 1, MidiKind::Note, 36), ["a"]);
    assert!(ids("Other", 1, MidiKind::Note, 36).is_empty());
    assert_eq!(ids("Other", 2, MidiKind::Note, 36), ["b"]);
    assert_eq!(ids("Other", 9, MidiKind::Cc, 36), ["c"]);
    assert!(ids("Pad", 1, MidiKind::Note, 37).is_empty());
}

// ---- Environment -----------------------------------------------------------------------------

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

const PAD: &str = "Test Pad";

struct Env {
    _tmp: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
    st: AppState,
    events: Arc<Recorder>,
    backend: Arc<TestBackend>,
    #[allow(dead_code)]
    dir: PathBuf,
    screen: String,
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
        .worker_threads(2)
        .enable_time()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    let backend = TestBackend::new(&[PAD, "Keys"], &[PAD]);
    st.set_midi_backend(Some(backend.clone()));
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    let bar = st
        .templates()
        .create(
            &dir,
            "classic-bar",
            &TemplateSource::Preset {
                preset_id: "classic-bar".into(),
            },
        )
        .unwrap()
        .id;
    let mut doc: ScreenDoc = st
        .create_screen(&NewScreen {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap()
        .doc;
    doc.meta.layers.push(layer("l1", &bar));
    doc.meta.layers.push(layer("l2", &bar));
    let screen = st.save_screen(doc).unwrap().doc.id;
    events.0.lock().unwrap().clear();
    Env {
        _tmp: tmp,
        _rt: rt,
        st,
        events,
        backend,
        dir,
        screen,
    }
}

fn layer(id: &str, template_id: &str) -> ScreenLayer {
    ScreenLayer {
        id: id.into(),
        name: format!("Layer {id}"),
        rect: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        kind: ScreenLayerKind::Template {
            template_id: template_id.into(),
            fields: Fields::new(),
            vars: Fields::new(),
            bindings: Default::default(),
            variant: None,
            sizing: None,
            opacity: None,
        },
        anchor: None,
    }
}

fn mapping(
    id: &str,
    device: Option<&str>,
    channel: Option<u8>,
    kind: MidiKind,
    number: u8,
    command: &str,
) -> ControlMapping {
    ControlMapping {
        id: id.into(),
        source: MidiSource {
            device: device.map(str::to_string),
            channel,
            kind,
            number,
        },
        target: ControlTarget::Command {
            command_id: command.into(),
        },
        feedback: false,
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

impl Env {
    fn toggle(&self, l: &str) -> String {
        format!("live.toggle:{}/{l}", self.screen)
    }

    fn save(&self, mappings: Vec<ControlMapping>) {
        self.st.save_controls(ControlsDoc { mappings }).unwrap();
    }

    fn on_air(&self, l: &str) -> bool {
        self.st.live().layer_view(&self.screen, l).unwrap().on_air
    }

    fn feedback(&self, device: &str, profile: MidiFeedbackProfile) {
        let mut p = self.st.prefs();
        p.midi.feedback.insert(
            device.into(),
            MidiFeedbackPref {
                output: None,
                profile,
            },
        );
        self.st.set_prefs(p);
    }

    fn sent(&self) -> Vec<[u8; 3]> {
        self.backend
            .take_sent()
            .into_iter()
            .map(|SentMessage { bytes, .. }| [bytes[0], bytes[1], bytes[2]])
            .collect()
    }
}

// ---- Lifecycle -------------------------------------------------------------------------------

#[test]
fn ports_open_only_while_midi_is_in_use() {
    let e = env();
    assert!(
        e.backend.open_inputs().is_empty(),
        "no mapping: nothing open"
    );
    assert!(!e.st.midi_rt().service_running());
    e.save(vec![mapping(
        "m1",
        Some(PAD),
        None,
        MidiKind::Note,
        36,
        &e.toggle("l1"),
    )]);
    assert_eq!(e.backend.open_inputs(), ["Keys", PAD]);
    // A disabled input closes at once.
    let mut p = e.st.prefs();
    p.midi
        .inputs
        .insert("Keys".into(), MidiInputPref { enabled: false });
    e.st.set_prefs(p);
    assert_eq!(e.backend.open_inputs(), [PAD]);
    // Closing the project closes everything.
    e.st.close_project();
    assert!(e.backend.open_inputs().is_empty());
    assert!(e.backend.open_outputs().is_empty());
}

#[test]
fn learn_and_watch_keep_ports_open() {
    let e = env();
    e.st.midi_learn(true);
    assert_eq!(e.backend.open_inputs(), ["Keys", PAD]);
    e.st.midi_learn(false);
    assert!(e.backend.open_inputs().is_empty());
    e.st.watch_midi_devices(true);
    assert_eq!(e.backend.open_inputs(), ["Keys", PAD]);
    e.st.watch_midi_devices(false);
    // An unmatched close never goes below zero.
    e.st.watch_midi_devices(false);
    assert!(e.backend.open_inputs().is_empty());
    e.st.watch_midi_devices(true);
    assert_eq!(e.backend.open_inputs().len(), 2);
}

#[test]
fn hot_plug_opens_new_devices_and_reports_the_list() {
    let e = env();
    e.st.watch_midi_devices(true);
    let first = e.events.take(names::MIDI_DEVICES);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0][0]["name"], PAD);
    assert_eq!(first[0][0]["output"], true);
    e.backend.set_ports(
        vec![PAD.into(), "Keys".into(), "APC40 mkII".into()],
        vec![PAD.into()],
    );
    e.backend.set_busy("APC40 mkII", true);
    e.st.midi_devices_changed();
    let list = e.events.take(names::MIDI_DEVICES);
    assert_eq!(list.len(), 1);
    let apc = list[0]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "APC40 mkII")
        .unwrap();
    assert_eq!(apc["active"], false, "in use by another app");
    // Nothing changed: no event.
    e.st.midi_devices_changed();
    assert!(e.events.take(names::MIDI_DEVICES).is_empty());
    // Unplugged.
    e.backend.set_ports(vec![PAD.into()], vec![PAD.into()]);
    e.st.midi_devices_changed();
    assert_eq!(e.backend.open_inputs(), [PAD]);
    assert_eq!(
        e.events.take(names::MIDI_DEVICES)[0]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(e.st.midi_devices().len(), 1);
}

#[test]
fn without_a_backend_midi_does_nothing() {
    let e = env();
    e.st.set_midi_backend(None);
    e.save(vec![mapping(
        "m1",
        None,
        None,
        MidiKind::Note,
        36,
        &e.toggle("l1"),
    )]);
    assert!(e.backend.open_inputs().is_empty());
    assert!(e.st.midi_devices().is_empty());
}

// ---- Input -----------------------------------------------------------------------------------

#[test]
fn a_note_toggles_a_layer_within_10_ms() {
    let e = env();
    e.save(vec![mapping(
        "m1",
        Some(PAD),
        Some(1),
        MidiKind::Note,
        36,
        &e.toggle("l1"),
    )]);
    let mut rx = e.st.live().subscribe();
    // Through the open port, like the driver delivers it.
    let t0 = Instant::now();
    assert!(e.backend.input(PAD, &[0x90, 36, 127]));
    let msg = rx.try_recv().expect("a renderer message at once");
    let took = t0.elapsed();
    assert_eq!(&*msg.screen_id, e.screen.as_str());
    println!("MIDI note → renderer message: {took:?}");
    assert!(
        took < Duration::from_millis(10),
        "MIDI → renderer took {took:?}"
    );
    assert!(e.on_air("l1"));
    // Note-off does nothing; another device, channel or note does nothing.
    e.st.inject_midi(PAD, &[0x80, 36, 0]);
    e.st.inject_midi("Keys", &[0x90, 36, 127]);
    e.st.inject_midi(PAD, &[0x91, 36, 127]);
    e.st.inject_midi(PAD, &[0x90, 37, 127]);
    assert!(e.on_air("l1"));
    e.st.inject_midi(PAD, &[0x90, 36, 1]);
    assert!(!e.on_air("l1"));
}

#[test]
fn a_cc_presses_when_it_crosses_64() {
    let e = env();
    e.save(vec![mapping(
        "m1",
        None,
        None,
        MidiKind::Cc,
        20,
        &e.toggle("l1"),
    )]);
    e.st.inject_midi(PAD, &[0xB0, 20, 100]);
    assert!(e.on_air("l1"), "first value above 64: a press");
    e.st.inject_midi(PAD, &[0xB0, 20, 120]);
    assert!(e.on_air("l1"), "still above: nothing");
    e.st.inject_midi(PAD, &[0xB0, 20, 10]);
    assert!(e.on_air("l1"), "release does nothing");
    e.st.inject_midi(PAD, &[0xB0, 20, 64]);
    assert!(!e.on_air("l1"), "crossing up again toggles");
}

#[test]
fn a_cc_drives_opacity() {
    let e = env();
    e.save(vec![ControlMapping {
        id: "f1".into(),
        source: MidiSource {
            device: None,
            channel: None,
            kind: MidiKind::Cc,
            number: 7,
        },
        target: ControlTarget::Value {
            screen_id: e.screen.clone(),
            layer_id: "l1".into(),
            target: ValueTarget::Opacity,
        },
        feedback: false,
    }]);
    e.st.inject_midi(PAD, &[0xB0, 7, 64]);
    let o =
        e.st.live()
            .with_layer(&e.screen, "l1", |x| x.opacity)
            .unwrap();
    assert_eq!(o, 50);
}

#[test]
fn conflicting_mappings_run_nothing_and_report_once() {
    let e = env();
    e.save(vec![
        mapping("a", Some(PAD), None, MidiKind::Note, 36, &e.toggle("l1")),
        mapping("b", None, Some(1), MidiKind::Note, 36, &e.toggle("l2")),
    ]);
    e.st.inject_midi(PAD, &[0x90, 36, 127]);
    e.st.inject_midi(PAD, &[0x90, 36, 127]);
    assert!(!e.on_air("l1") && !e.on_air("l2"));
    let c = e.events.take(names::CONTROL_CONFLICT);
    assert_eq!(c.len(), 1, "at most one per 2 s per source");
    assert_eq!(c[0]["mappingIds"], serde_json::json!(["a", "b"]));
    assert_eq!(c[0]["source"]["device"], PAD);
    assert_eq!(c[0]["source"]["number"], 36);
}

#[test]
fn learn_reports_and_runs_nothing() {
    let e = env();
    e.save(vec![mapping(
        "m1",
        None,
        None,
        MidiKind::Note,
        36,
        &e.toggle("l1"),
    )]);
    e.st.midi_learn(true);
    e.st.inject_midi(PAD, &[0x93, 36, 90]);
    assert!(!e.on_air("l1"));
    let l = e.events.take(names::MIDI_LEARN);
    assert_eq!(l.len(), 1);
    assert_eq!(
        l[0],
        serde_json::json!({"device": PAD, "channel": 4, "kind": "note", "number": 36, "value": 90})
    );
    // Note-offs aren't learned.
    std::thread::sleep(LEARN_EVERY);
    e.st.inject_midi(PAD, &[0x83, 36, 0]);
    assert!(e.events.take(names::MIDI_LEARN).is_empty());
    e.st.midi_learn(false);
    e.st.inject_midi(PAD, &[0x90, 36, 90]);
    assert!(e.on_air("l1"));
    assert!(e.events.take(names::MIDI_LEARN).is_empty());
}

// ---- Feedback --------------------------------------------------------------------------------

#[test]
fn feedback_follows_the_layer_within_10_ms() {
    let e = env();
    e.feedback(PAD, MidiFeedbackProfile::Generic);
    let mut m = mapping(
        "m1",
        Some(PAD),
        Some(2),
        MidiKind::Note,
        40,
        &e.toggle("l1"),
    );
    m.feedback = true;
    let mut other = mapping("m2", None, None, MidiKind::Note, 41, &e.toggle("l2"));
    other.feedback = false;
    e.save(vec![m, other]);
    assert_eq!(e.backend.open_outputs(), [PAD]);
    assert_eq!(
        e.sent(),
        [[0x91, 40, 1]],
        "full state at open: ready (green)"
    );
    let t0 = Instant::now();
    e.st.live().show(&e.screen, "l1").unwrap();
    let sent = e.sent();
    let took = t0.elapsed();
    assert_eq!(sent, [[0x91, 40, 127]], "on air (red)");
    println!("on-air change → LED message: {took:?}");
    assert!(
        took < Duration::from_millis(10),
        "LED feedback took {took:?}"
    );
    // No change, nothing sent; a layer without feedback sends nothing.
    e.st.live().show(&e.screen, "l2").unwrap();
    assert!(e.sent().is_empty());
    e.st.live().hide(&e.screen, "l1").unwrap();
    assert_eq!(e.sent(), [[0x91, 40, 1]]);
    // Closing the project switches the LEDs off and closes the output.
    e.st.close_project();
    assert_eq!(e.sent(), [[0x91, 40, 0]]);
    assert!(e.backend.open_outputs().is_empty());
}

#[test]
fn feedback_uses_the_profile_and_removes_unmapped_leds() {
    let e = env();
    e.feedback(PAD, MidiFeedbackProfile::Launchpad);
    let mut m = mapping("m1", None, None, MidiKind::Note, 11, &e.toggle("l1"));
    m.feedback = true;
    e.save(vec![m.clone()]);
    assert_eq!(e.sent(), [[0x90, 11, 21]]);
    e.st.live().show(&e.screen, "l1").unwrap();
    assert_eq!(e.sent(), [[0x90, 11, 5]]);
    // The mapping moves to another pad: the old LED goes off, the new one shows the state.
    m.source.number = 12;
    e.save(vec![m]);
    assert_eq!(e.sent(), [[0x90, 11, 0], [0x90, 12, 5]]);
    // Feedback switched off in prefs: LEDs off, output closed.
    let mut p = e.st.prefs();
    p.midi = MidiPrefs::default();
    e.st.set_prefs(p);
    assert_eq!(e.sent(), [[0x90, 12, 0]]);
    assert!(e.backend.open_outputs().is_empty());
}

#[test]
fn cue_leds_show_armed_and_running() {
    let e = env();
    e.feedback(PAD, MidiFeedbackProfile::Apc40);
    let n = |id: &str, kind| FlowNode {
        id: id.into(),
        position: NodePos::default(),
        kind,
    };
    let edge = |id: &str, from: &str, fp: &str, to: &str| FlowEdge {
        id: id.into(),
        from: PortRef {
            node: from.into(),
            port: fp.into(),
        },
        to: PortRef {
            node: to.into(),
            port: "in".into(),
        },
        kind: WireKind::Event,
    };
    let flow =
        e.st.create_flow(NewFlow {
            name: "Go".into(),
            armed: true,
            retrigger: Retrigger::Restart,
            origin: None,
            nodes: vec![
                n("c", NodeKind::Cue { label: None }),
                n("d", NodeKind::Delay { ms: 300 }),
            ],
            edges: vec![edge("e1", "c", "fired", "d")],
        })
        .unwrap()
        .flow
        .id;
    let mut m = mapping(
        "m1",
        Some(PAD),
        None,
        MidiKind::Note,
        50,
        &format!("cue.go:{flow}"),
    );
    m.feedback = true;
    e.save(vec![m]);
    assert_eq!(e.sent(), [[0x90, 50, 1]], "armed: green");
    e.st.inject_midi(PAD, &[0x90, 50, 127]);
    let mut seen = Vec::new();
    wait_until("running, then idle again", || {
        seen.extend(e.sent());
        seen.ends_with(&[[0x90, 50, 1]]) && seen.contains(&[0x90, 50, 3])
    });
    assert_eq!(seen, [[0x90, 50, 3], [0x90, 50, 1]], "red while running");
    e.st.set_flow_armed(&flow, false).unwrap();
    assert_eq!(e.sent(), [[0x90, 50, 0]], "disarmed: off");
}

#[test]
fn test_lights_show_red_green_then_the_state() {
    let e = env();
    let none = e.st.test_midi_feedback(PAD);
    assert!(matches!(none, Err(AppError::Invalid(_))), "no feedback set");
    e.feedback(PAD, MidiFeedbackProfile::Generic);
    let mut m = mapping("m1", Some(PAD), None, MidiKind::Note, 60, &e.toggle("l1"));
    m.feedback = true;
    let plain = mapping("m2", None, None, MidiKind::Cc, 61, &e.toggle("l2"));
    e.save(vec![m, plain]);
    e.sent();
    e.st.test_midi_feedback(PAD).unwrap();
    let mut seen = Vec::new();
    wait_until("the pattern", || {
        seen.extend(e.sent());
        seen.len() >= 6
    });
    assert_eq!(
        seen,
        [
            [0x90, 60, 127],
            [0xB0, 61, 127],
            [0x90, 60, 1],
            [0xB0, 61, 1],
            [0x90, 60, 1],
            [0xB0, 61, 0]
        ]
    );
}

// ---- Flows -----------------------------------------------------------------------------------

#[test]
fn trigger_midi_fires_a_flow() {
    let e = env();
    let n = |id: &str, kind| FlowNode {
        id: id.into(),
        position: NodePos::default(),
        kind,
    };
    e.st.create_flow(NewFlow {
        name: "Pad".into(),
        armed: true,
        retrigger: Retrigger::Restart,
        origin: None,
        nodes: vec![
            n(
                "m",
                NodeKind::Midi {
                    device: Some(PAD.into()),
                    channel: None,
                    kind: MidiKind::Cc,
                    number: 1,
                },
            ),
            n(
                "t",
                NodeKind::Take {
                    screen_id: e.screen.clone(),
                    layer_id: "l1".into(),
                    for_ms: None,
                },
            ),
        ],
        edges: vec![FlowEdge {
            id: "e1".into(),
            from: PortRef {
                node: "m".into(),
                port: "fired".into(),
            },
            to: PortRef {
                node: "t".into(),
                port: "in".into(),
            },
            kind: WireKind::Event,
        }],
    })
    .unwrap();
    assert_eq!(
        e.backend.open_inputs(),
        ["Keys", PAD],
        "an armed trigger opens the inputs"
    );
    e.st.inject_midi("Keys", &[0xB0, 1, 5]);
    e.st.inject_midi(PAD, &[0xB0, 2, 5]);
    std::thread::sleep(Duration::from_millis(100));
    assert!(!e.on_air("l1"), "another device or number");
    e.st.inject_midi(PAD, &[0xB0, 1, 5]);
    wait_until("the flow's take", || e.on_air("l1"));
}

// ---- The Windows service thread ----------------------------------------------------------------

#[cfg(windows)]
#[test]
fn service_thread_wakes_for_input_and_stops() {
    use super::service::{Event, Service};
    let q = Arc::new(InputQueue::default());
    let (tx, rx) = std::sync::mpsc::channel();
    let s = Service::start(q.clone(), move |ev| tx.send(ev).is_ok()).unwrap();
    let next = || rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        next(),
        Event::Reconcile,
        "the first reconcile opens the ports"
    );
    assert_eq!(next(), Event::Input);
    assert_ne!(q.hwnd.load(Ordering::Acquire), 0);
    let t0 = Instant::now();
    q.push(&Arc::from("Pad"), &[0x90, 1, 1]);
    assert_eq!(next(), Event::Input);
    let woke = t0.elapsed();
    assert!(woke < Duration::from_millis(10), "wake-up took {woke:?}");
    assert_eq!(q.pop().unwrap().1, [0x90, 1, 1]);
    s.reconcile();
    assert_eq!(next(), Event::Reconcile);
    drop(s);
    assert_eq!(q.hwnd.load(Ordering::Acquire), 0, "stopped and joined");
}

/// The real WinMM backend on this machine (opens its MIDI inputs briefly): the service thread
/// runs only while MIDI is in use. `cargo test system_midi -- --ignored --nocapture`.
#[cfg(windows)]
#[test]
#[ignore]
fn system_midi_runs_a_thread_only_while_in_use() {
    let e = env();
    e.st.use_system_midi();
    std::thread::sleep(Duration::from_millis(200));
    assert!(!e.st.midi_rt().service_running(), "unused: no thread");
    println!("devices: {:?}", e.st.midi_devices());
    e.save(vec![mapping(
        "m1",
        None,
        None,
        MidiKind::Note,
        36,
        &e.toggle("l1"),
    )]);
    wait_until("the service thread", || e.st.midi_rt().service_running());
    e.save(Vec::new());
    wait_until("the service thread to end", || {
        !e.st.midi_rt().service_running()
    });
    e.st.midi_close_all();
}
