//! MIDI on macOS against the real CoreMIDI, with virtual endpoints (no device needed; GitHub's
//! macOS runners have CoreMIDI): the service thread's wake-ups, hot-plug through CoreMIDI's
//! notifications, the system backend's port list and input, and the whole path from a virtual
//! controller to a layer on air.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::backend::{MidiBackend, SystemBackend};
use super::service::{Event, Service};
use super::*;

fn wait_for(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A virtual controller other apps (and Kinetra) see as a MIDI input.
struct VirtualPad {
    name: String,
    source: coremidi::VirtualSource,
    _client: coremidi::Client,
}

impl VirtualPad {
    fn new(tag: &str) -> Self {
        let client = coremidi::Client::new("Kinetra test controller").expect("CoreMIDI client");
        let name = format!("Kinetra {tag} {}", std::process::id());
        let source = client.virtual_source(&name).expect("virtual source");
        Self {
            name,
            source,
            _client: client,
        }
    }

    fn send(&self, bytes: &[u8]) {
        self.source
            .received(&coremidi::PacketBuffer::new(0, bytes))
            .expect("send");
    }
}

#[test]
fn service_thread_wakes_for_input_and_stops() {
    let q = Arc::new(InputQueue::default());
    let (tx, rx) = mpsc::channel();
    let s = Service::start(q.clone(), move |ev| tx.send(ev).is_ok()).unwrap();
    let next = || rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        next(),
        Event::Reconcile,
        "the first reconcile opens the ports"
    );
    assert_eq!(next(), Event::Input);
    assert!(q.wake.read().unwrap().is_some());
    let t0 = Instant::now();
    q.push(&Arc::from("Pad"), &[0x90, 1, 1]);
    assert_eq!(next(), Event::Input);
    let woke = t0.elapsed();
    assert!(woke < Duration::from_millis(20), "wake-up took {woke:?}");
    assert_eq!(q.pop().unwrap().1, [0x90, 1, 1]);
    s.reconcile();
    assert_eq!(next(), Event::Reconcile);
    drop(s);
    assert!(q.wake.read().unwrap().is_none(), "stopped and joined");
}

/// Plugging and unplugging a device reaches the service as ONE debounced reconcile each.
#[test]
fn hot_plug_wakes_the_service_after_the_debounce() {
    let q = Arc::new(InputQueue::default());
    let (tx, rx) = mpsc::channel();
    let s = Service::start(q, move |ev| tx.send(ev).is_ok()).unwrap();
    let next = |secs| rx.recv_timeout(Duration::from_secs(secs));
    assert_eq!(next(2).unwrap(), Event::Reconcile);
    assert_eq!(next(2).unwrap(), Event::Input);

    let t0 = Instant::now();
    let pad = VirtualPad::new("Hot-plug");
    assert_eq!(next(5).unwrap(), Event::Reconcile, "plugged in");
    let took = t0.elapsed();
    assert!(took >= Duration::from_millis(250), "debounced: {took:?}");
    println!("plug-in → reconcile: {took:?}");
    assert!(
        next(1).is_err(),
        "one reconcile for the burst of notifications"
    );

    drop(pad);
    assert_eq!(next(5).unwrap(), Event::Reconcile, "unplugged");
    drop(s);
}

#[test]
fn the_system_backend_lists_and_reads_a_virtual_controller() {
    let pad = VirtualPad::new("Backend");
    wait_for("the virtual source in the list", 5, || {
        SystemBackend.ports().inputs.contains(&pad.name)
    });
    let got: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    let g = got.clone();
    let conn = SystemBackend
        .open_input(
            &pad.name,
            Box::new(move |bytes: &[u8]| g.lock().unwrap().push(bytes.to_vec())),
        )
        .unwrap();
    // Two messages in one packet arrive as two messages (midir splits them).
    pad.send(&[0x90, 36, 100, 0xB0, 7, 64]);
    wait_for("the input", 5, || got.lock().unwrap().len() >= 2);
    assert_eq!(
        *got.lock().unwrap(),
        [vec![0x90, 36, 100], vec![0xB0, 7, 64]]
    );
    drop(conn);
}

/// The whole path on the real system: a mapping for a controller that isn't there yet, the
/// controller plugged in (hot-plug opens it), a note → the layer on air; the service thread
/// exists only while MIDI is in use.
#[test]
fn a_virtual_controller_takes_a_layer_on_air() {
    use crate::core::model::{
        ControlMapping, NewScreen, Rect, ScreenDoc, ScreenLayer, ScreenLayerKind, TemplateSource,
    };
    use crate::core::protocol::Fields;

    struct Quiet;
    impl crate::core::events::EventSink for Quiet {
        fn emit(&self, _: &str, _: serde_json::Value) {}
    }

    let tmp = tempfile::tempdir().unwrap();
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        Arc::new(Quiet),
        Duration::from_secs(60),
    )
    .unwrap();
    st.set_content_flush_delay(Duration::from_secs(60));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    st.use_system_midi();
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
    doc.meta.layers.push(ScreenLayer {
        id: "l1".into(),
        name: "Lower third".into(),
        rect: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        kind: ScreenLayerKind::Template {
            template_id: bar,
            fields: Fields::new(),
            vars: Fields::new(),
            bindings: Default::default(),
            variant: None,
            sizing: None,
            opacity: None,
        },
        anchor: None,
    });
    let screen = st.save_screen(doc).unwrap().doc.id;
    std::thread::sleep(Duration::from_millis(100));
    assert!(!st.midi_rt().service_running(), "unused: no thread");

    let name = format!("Kinetra Path {}", std::process::id());
    st.save_controls(ControlsDoc {
        mappings: vec![ControlMapping {
            id: "m1".into(),
            source: MidiSource {
                device: Some(name.clone()),
                channel: None,
                kind: MidiKind::Note,
                number: 36,
            },
            target: ControlTarget::Command {
                command_id: format!("live.take:{screen}/l1"),
            },
            feedback: false,
        }],
    })
    .unwrap();
    wait_for("the service thread", 5, || st.midi_rt().service_running());

    // Plugged in after the mapping: hot-plug opens it.
    let client = coremidi::Client::new("Kinetra test controller").unwrap();
    let source = client.virtual_source(&name).unwrap();
    wait_for("the input to open", 5, || {
        st.midi_devices().iter().any(|d| d.name == name && d.active)
    });
    let t0 = Instant::now();
    source
        .received(&coremidi::PacketBuffer::new(0, &[0x90, 36, 100]))
        .unwrap();
    wait_for("the layer on air", 5, || {
        st.live().layer_view(&screen, "l1").unwrap().on_air
    });
    println!("virtual note → on air: {:?}", t0.elapsed());

    st.save_controls(ControlsDoc { mappings: vec![] }).unwrap();
    wait_for("the service thread to end", 5, || {
        !st.midi_rt().service_running()
    });
    st.midi_close_all();
}
