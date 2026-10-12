//! `controls.json`: validation, conflicts, the store (save, events, listeners, flush), the watcher,
//! backups and the settings export; plus the MIDI prefs and OSC settings checks.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::*;
use crate::core::events::EventSink;
use crate::core::model::{
    BackupReason, MidiFeedbackPref, MidiFeedbackProfile, MidiInputPref, MidiPrefs, OscFeedback,
    OscSettings, Prefs, ProjectSettings, SettingsSection,
};
use crate::core::settings_io;
use crate::core::state::{clamp_prefs, validate_project_settings};

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
    tmp: tempfile::TempDir,
    st: AppState,
    rec: Arc<Recorder>,
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
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    Env { tmp, st, rec, dir }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn src(device: Option<&str>, channel: Option<u8>, kind: MidiKind, number: u8) -> MidiSource {
    MidiSource {
        device: device.map(Into::into),
        channel,
        kind,
        number,
    }
}

fn mapping(id: &str, source: MidiSource, command: &str) -> ControlMapping {
    ControlMapping {
        id: id.into(),
        source,
        target: ControlTarget::Command {
            command_id: command.into(),
        },
        feedback: false,
    }
}

fn doc(mappings: Vec<ControlMapping>) -> ControlsDoc {
    ControlsDoc { mappings }
}

fn note(n: u8) -> MidiSource {
    src(Some("APC40"), Some(1), MidiKind::Note, n)
}

#[test]
fn the_wire_shape_matches_the_ts_contract() {
    let d = doc(vec![
        mapping("m1", note(53), "live.toggle:s1/l1"),
        ControlMapping {
            id: "m2".into(),
            source: src(None, None, MidiKind::Cc, 7),
            target: ControlTarget::Value {
                screen_id: "s1".into(),
                layer_id: "l1".into(),
                target: ValueTarget::Var {
                    key: "scale".into(),
                },
            },
            feedback: true,
        },
    ]);
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(
        v,
        json!({"mappings": [
            {"id": "m1", "source": {"device": "APC40", "channel": 1, "kind": "note", "number": 53},
             "target": {"kind": "command", "commandId": "live.toggle:s1/l1"}, "feedback": false},
            {"id": "m2", "source": {"device": null, "channel": null, "kind": "cc", "number": 7},
             "target": {"kind": "value", "screenId": "s1", "layerId": "l1",
                        "target": {"kind": "var", "key": "scale"}}, "feedback": true}
        ]})
    );
    let back: ControlsDoc = serde_json::from_value(v).unwrap();
    assert_eq!(back, d);
    let opacity: ValueTarget = serde_json::from_value(json!({"kind": "opacity"})).unwrap();
    assert_eq!(opacity, ValueTarget::Opacity);
}

#[test]
fn validation_rejects_bad_mappings_and_allows_conflicts() {
    let ok = doc(vec![
        mapping("m1", note(1), "live.panic"),
        mapping("m2", note(1), "live.clearAll"),
    ]);
    validate(&ok).unwrap();
    let bad = [
        mapping("bad id!", note(1), "live.panic"),
        mapping("m1", src(Some(""), None, MidiKind::Note, 1), "live.panic"),
        mapping(
            "m1",
            src(Some(&"x".repeat(129)), None, MidiKind::Note, 1),
            "live.panic",
        ),
        mapping("m1", src(None, Some(0), MidiKind::Note, 1), "live.panic"),
        mapping("m1", src(None, Some(17), MidiKind::Note, 1), "live.panic"),
        mapping("m1", src(None, None, MidiKind::Cc, 128), "live.panic"),
        mapping("m1", note(1), "nav.control"),
        mapping("m1", note(1), "live.toggle:s1"),
        ControlMapping {
            target: ControlTarget::Value {
                screen_id: "s1".into(),
                layer_id: "l 1".into(),
                target: ValueTarget::Opacity,
            },
            ..mapping("m1", note(1), "live.panic")
        },
        ControlMapping {
            target: ControlTarget::Value {
                screen_id: "s1".into(),
                layer_id: "l1".into(),
                target: ValueTarget::Field { key: "Bad".into() },
            },
            ..mapping("m1", note(1), "live.panic")
        },
    ];
    for m in bad {
        assert!(
            matches!(validate(&doc(vec![m.clone()])), Err(AppError::Invalid(_))),
            "{m:?}"
        );
    }
    let dup = doc(vec![
        mapping("m1", note(1), "live.panic"),
        mapping("m1", note(2), "live.panic"),
    ]);
    assert!(validate(&dup).is_err());
    let many = doc((0..=CONTROL_MAPPINGS_CAP)
        .map(|i| mapping(&format!("m{i}"), note((i % 128) as u8), "live.panic"))
        .collect());
    assert!(validate(&many).is_err());
}

#[test]
fn sanitize_drops_only_the_bad_ones() {
    let d = doc(vec![
        mapping("m1", note(1), "live.panic"),
        mapping("m2", note(2), "bogus"),
        mapping("m1", note(3), "live.panic"),
        mapping("m3", note(4), "live.clearAll"),
    ]);
    let ids: Vec<String> = sanitize(d).mappings.into_iter().map(|m| m.id).collect();
    assert_eq!(ids, ["m1", "m3"]);
}

#[test]
fn conflicts_group_overlapping_sources() {
    let d = doc(vec![
        mapping("a", note(10), "live.panic"),
        // Any device, channel 1, note 10: overlaps a.
        mapping("b", src(None, Some(1), MidiKind::Note, 10), "live.clearAll"),
        // Another device, any channel: overlaps b (device any) but not a.
        mapping(
            "c",
            src(Some("Launchpad"), None, MidiKind::Note, 10),
            "live.clearAll",
        ),
        // A CC with the same number: no conflict.
        mapping(
            "d",
            src(Some("APC40"), Some(1), MidiKind::Cc, 10),
            "live.panic",
        ),
        // Another channel on the same device: no conflict with a.
        mapping(
            "e",
            src(Some("APC40"), Some(2), MidiKind::Note, 10),
            "live.panic",
        ),
        mapping("f", note(11), "live.panic"),
        mapping("g", note(11), "live.panic"),
    ]);
    let c = conflicts(&d);
    assert_eq!(c.len(), 2, "{c:?}");
    assert_eq!(c[0].0, note(10));
    assert_eq!(c[0].1, ["a", "b", "c"]);
    assert_eq!(c[1].1, ["f", "g"]);
    assert!(conflicts(&doc(vec![mapping("x", note(1), "live.panic")])).is_empty());
    // At runtime: a message matching two mappings.
    let m = matching(&d, "APC40", 1, MidiKind::Note, 10);
    assert_eq!(m.len(), 2);
    let m = matching(&d, "APC40", 2, MidiKind::Note, 10);
    assert_eq!(
        m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["e"],
        "b is channel 1 only"
    );
    assert!(matching(&d, "APC40", 1, MidiKind::Note, 99).is_empty());
}

#[test]
fn save_stores_announces_and_writes_coalesced() {
    let e = env();
    let seen = Arc::new(AtomicUsize::new(usize::MAX));
    let s2 = seen.clone();
    e.st.on_controls_changed(Arc::new(move |d| {
        s2.store(d.mappings.len(), Ordering::SeqCst)
    }));
    assert_eq!(seen.load(Ordering::SeqCst), 0, "called at once");
    assert!(e.st.get_controls().unwrap().mappings.is_empty());
    let d = doc(vec![mapping("m1", note(1), "live.panic")]);
    assert_eq!(e.st.save_controls(d.clone()).unwrap(), d);
    assert_eq!(e.st.get_controls().unwrap(), d);
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    let ev = e.rec.named(names::CONTROLS_CHANGED);
    assert_eq!(ev.last().unwrap()["mappings"][0]["id"], "m1");
    // The same doc again: no event.
    let n = e.rec.named(names::CONTROLS_CHANGED).len();
    e.st.save_controls(d.clone()).unwrap();
    assert_eq!(e.rec.named(names::CONTROLS_CHANGED).len(), n);
    // Invalid: rejected, nothing changes.
    assert!(e
        .st
        .save_controls(doc(vec![mapping("m1", note(1), "x")]))
        .is_err());
    assert_eq!(e.st.get_controls().unwrap(), d);
    // Written by the store flusher (versioned).
    let path = e.dir.join(CONTROLS_FILE);
    wait_until("controls.json", || path.exists());
    let v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(v["schemaVersion"], 1);
    assert_eq!(v["mappings"][0]["id"], "m1");
    // Reopening loads it; closing announces an empty doc.
    e.st.close_project();
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    assert!(matches!(e.st.get_controls(), Err(AppError::NoProject)));
    e.st.open_project(&e.dir).unwrap();
    assert_eq!(e.st.get_controls().unwrap(), d);
    assert_eq!(seen.load(Ordering::SeqCst), 1);
}

#[test]
fn a_hand_edited_file_with_a_bad_mapping_loads_the_rest() {
    let e = env();
    e.st.close_project();
    fs::write(
        e.dir.join(CONTROLS_FILE),
        r#"{"schemaVersion":1,"mappings":[
          {"id":"m1","source":{"device":null,"channel":null,"kind":"note","number":1},"target":{"kind":"command","commandId":"live.panic"}},
          {"id":"m2","source":{"device":null,"channel":99,"kind":"note","number":2},"target":{"kind":"command","commandId":"live.panic"}}
        ]}"#,
    )
    .unwrap();
    e.st.open_project(&e.dir).unwrap();
    let d = e.st.get_controls().unwrap();
    assert_eq!(d.mappings.len(), 1);
    assert_eq!(d.mappings[0].id, "m1");
    assert!(!d.mappings[0].feedback, "absent = false");
}

fn files_changed(e: &Env) -> Vec<Value> {
    e.rec.named(names::PROJECT_FILES_CHANGED)
}

#[test]
fn the_watcher_reloads_external_edits_and_keeps_the_last_good_state() {
    let e = env();
    assert!(e.st.watcher_running());
    let d = doc(vec![mapping("m1", note(1), "live.panic")]);
    e.st.save_controls(d.clone()).unwrap();
    let path = e.dir.join(CONTROLS_FILE);
    wait_until("own write", || path.exists());
    e.st.flush_all();
    wait_until("the watcher's batch", || e.st.watcher_batches() > 0);
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        files_changed(&e).is_empty(),
        "own writes: {:?}",
        files_changed(&e)
    );

    // Valid external edit: reloaded and announced.
    let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    v["mappings"][0]["source"]["number"] = json!(9);
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    wait_until("changed", || !files_changed(&e).is_empty());
    assert_eq!(
        files_changed(&e)[0],
        json!({"kind": "controls", "id": "controls.json", "change": "changed", "message": null})
    );
    assert_eq!(e.st.get_controls().unwrap().mappings[0].source.number, 9);
    assert_eq!(
        e.rec.named(names::CONTROLS_CHANGED).last().unwrap()["mappings"][0]["source"]["number"],
        9
    );

    // Invalid (a bad mapping, then broken JSON): last good state kept, file left alone.
    v["mappings"][0]["source"]["channel"] = json!(40);
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    wait_until("invalid", || files_changed(&e).len() == 2);
    assert_eq!(files_changed(&e)[1]["change"], "invalid");
    fs::write(&path, "{ half").unwrap();
    wait_until("invalid 2", || files_changed(&e).len() == 3);
    assert_eq!(files_changed(&e)[2]["change"], "invalid");
    assert_eq!(e.st.get_controls().unwrap().mappings[0].source.number, 9);
    assert_eq!(fs::read_to_string(&path).unwrap(), "{ half");

    // Removed: no mappings.
    fs::remove_file(&path).unwrap();
    wait_until("removed", || files_changed(&e).len() == 4);
    assert_eq!(files_changed(&e)[3]["change"], "removed");
    assert!(e.st.get_controls().unwrap().mappings.is_empty());
}

#[test]
fn backups_carry_controls_json() {
    let e = env();
    let d = doc(vec![mapping("m1", note(1), "live.panic")]);
    e.st.save_controls(d.clone()).unwrap();
    let info = e.st.create_snapshot(BackupReason::Manual).unwrap();
    e.st.save_controls(ControlsDoc::default()).unwrap();
    e.st.restore_backup(&info.id).unwrap();
    assert_eq!(e.st.get_controls().unwrap(), d);
    assert_eq!(
        e.rec.named(names::CONTROLS_CHANGED).last().unwrap()["mappings"][0]["id"],
        "m1"
    );
}

#[test]
fn settings_export_and_import_carry_the_mappings() {
    let e = env();
    let d = doc(vec![mapping("m1", note(1), "live.panic")]);
    e.st.save_controls(d.clone()).unwrap();
    let file = e.tmp.path().join("settings.json");
    settings_io::export(&e.st, &file).unwrap();
    let v: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(v["controls"]["mappings"][0]["id"], "m1");

    e.st.save_controls(ControlsDoc::default()).unwrap();
    let parsed = settings_io::read_file(&file).unwrap();
    let preview = settings_io::preview(&e.st, &parsed).unwrap();
    let section = preview
        .sections
        .iter()
        .find(|s| s.section == SettingsSection::Controls)
        .unwrap();
    assert!(!section.changes.is_empty());
    settings_io::apply(&e.st, &parsed, &[SettingsSection::Controls]).unwrap();
    assert_eq!(e.st.get_controls().unwrap(), d);
}

#[test]
fn midi_prefs_are_clamped_and_round_trip() {
    let mut midi = MidiPrefs::default();
    midi.inputs
        .insert("APC40 mkII".into(), MidiInputPref { enabled: false });
    midi.inputs
        .insert(String::new(), MidiInputPref { enabled: true });
    midi.inputs
        .insert("x".repeat(200), MidiInputPref { enabled: true });
    for i in 0..80 {
        midi.inputs
            .insert(format!("dev {i:02}"), MidiInputPref { enabled: true });
    }
    midi.feedback.insert(
        "APC40 mkII".into(),
        MidiFeedbackPref {
            output: None,
            profile: MidiFeedbackProfile::Apc40,
        },
    );
    midi.feedback.insert(
        "Bad".into(),
        MidiFeedbackPref {
            output: Some(String::new()),
            profile: MidiFeedbackProfile::Generic,
        },
    );
    let p = clamp_prefs(Prefs {
        midi,
        ..Default::default()
    });
    assert_eq!(p.midi.inputs.len(), crate::core::model::MIDI_DEVICES_CAP);
    assert!(!p.midi.inputs.contains_key(""));
    assert!(p.midi.inputs.contains_key("APC40 mkII"));
    assert!(!p.midi.input_enabled("APC40 mkII"));
    assert!(p.midi.input_enabled("never seen"));
    assert_eq!(p.midi.feedback.len(), 1);
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["midi"]["feedback"]["APC40 mkII"]["profile"], "apc40");
    assert_eq!(v["midi"]["feedback"]["APC40 mkII"]["output"], Value::Null);
    // Empty: omitted from prefs.json; old files load.
    let v = serde_json::to_value(Prefs::default()).unwrap();
    assert!(v.get("midi").is_none());
    let old: Prefs = serde_json::from_value(json!({"theme": "dark"})).unwrap();
    assert!(old.midi.is_empty());
    // An unknown profile is not a valid prefs value.
    assert!(serde_json::from_value::<MidiFeedbackPref>(json!({"profile": "nope"})).is_err());
}

#[test]
fn osc_settings_are_validated() {
    let with = |osc: OscSettings| ProjectSettings {
        osc: Some(osc),
        ..Default::default()
    };
    validate_project_settings(&ProjectSettings::default()).unwrap();
    let good = OscSettings {
        enabled: true,
        port: 9000,
        allow: vec!["192.168.1.20".into(), "::1".into(), " 10.0.0.2 ".into()],
        feedback: Some(OscFeedback {
            host: "192.168.1.50".into(),
            port: 9001,
        }),
    };
    validate_project_settings(&with(good.clone())).unwrap();
    let bad = [
        OscSettings {
            port: 80,
            ..good.clone()
        },
        OscSettings {
            allow: vec!["example.com".into()],
            ..good.clone()
        },
        OscSettings {
            allow: (0..33).map(|i| format!("10.0.0.{i}")).collect(),
            ..good.clone()
        },
        OscSettings {
            feedback: Some(OscFeedback {
                host: " ".into(),
                port: 9001,
            }),
            ..good.clone()
        },
        OscSettings {
            feedback: Some(OscFeedback {
                host: "mixer.local".into(),
                port: 0,
            }),
            ..good.clone()
        },
    ];
    for b in bad {
        assert!(
            matches!(
                validate_project_settings(&with(b.clone())),
                Err(AppError::Invalid(_))
            ),
            "{b:?}"
        );
    }
    // Absent in project.json = off; the default port is 9000.
    let s: ProjectSettings = serde_json::from_value(json!({"port": 7770})).unwrap();
    assert!(s.osc.is_none());
    assert!(serde_json::to_value(&s).unwrap().get("osc").is_none());
    assert_eq!(
        OscSettings::default().port,
        crate::core::model::OSC_DEFAULT_PORT
    );
    let o: OscSettings = serde_json::from_value(json!({"enabled": true})).unwrap();
    assert_eq!(o.port, 9000);
}

#[test]
fn osc_settings_are_stored_trimmed() {
    let e = env();
    let mut s = e.st.current_project().unwrap().settings;
    s.osc = Some(OscSettings {
        enabled: true,
        port: 9000,
        allow: vec![" 10.0.0.2 ".into()],
        feedback: Some(OscFeedback {
            host: " mixer.local ".into(),
            port: 9001,
        }),
    });
    let p = e.st.set_project_settings(s).unwrap();
    let osc = p.settings.osc.unwrap();
    assert_eq!(osc.allow, ["10.0.0.2"]);
    assert_eq!(osc.feedback.unwrap().host, "mixer.local");
}
