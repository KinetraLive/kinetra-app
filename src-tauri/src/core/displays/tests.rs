//! EDID parsing, the registry key, and the watcher lifecycle / debounce / diff with the test
//! backend. (Hot-plug → reconciled outputs is in `core::outputs` tests; the Windows smoke test
//! is in `windows`.)

use std::sync::Mutex;
use std::time::Instant;

use serde_json::Value;

use super::*;
use crate::core::events::EventSink;

/// A minimal EDID base block: header, manufacturer "DEL", product 0xA0B1, serial, and the given
/// descriptors (tag, text) in the four descriptor slots.
fn edid(serial: u32, descriptors: &[(u8, &str)]) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
    // D=4, E=5, L=12 → 0b0_00100_00101_01100
    let mfg: u16 = (4 << 10) | (5 << 5) | 12;
    b[8..10].copy_from_slice(&mfg.to_be_bytes());
    b[10..12].copy_from_slice(&0xA0B1u16.to_le_bytes());
    b[12..16].copy_from_slice(&serial.to_le_bytes());
    // Slot 0: a detailed timing descriptor (non-zero pixel clock).
    b[54] = 0x02;
    b[55] = 0x3A;
    for (slot, (tag, text)) in descriptors.iter().enumerate() {
        let at = 72 + slot * 18;
        b[at + 3] = *tag;
        let mut t = text.as_bytes().to_vec();
        t.push(0x0A);
        t.resize(13, 0x20);
        b[at + 5..at + 18].copy_from_slice(&t[..13]);
    }
    b
}

#[test]
fn parses_the_edid_name_and_key() {
    let e = parse_edid(&edid(0x1234_5678, &[(0xFC, "DELL U2720Q")])).unwrap();
    assert_eq!(e.name.as_deref(), Some("DELL U2720Q"));
    assert_eq!(e.key, "DEL-A0B1-12345678");

    // A zero serial number: the serial string descriptor.
    let e = parse_edid(&edid(0, &[(0xFF, "ABC123"), (0xFC, "DELL P2419H")])).unwrap();
    assert_eq!(e.name.as_deref(), Some("DELL P2419H"));
    assert_eq!(e.key, "DEL-A0B1-ABC123");

    // No name descriptor; a 13-character name (no terminator); extension blocks are ignored.
    let e = parse_edid(&edid(7, &[])).unwrap();
    assert_eq!(e.name, None);
    assert_eq!(e.key, "DEL-A0B1-00000007");
    let mut long = edid(7, &[(0xFC, "ABCDEFGHIJKLM")]);
    long.extend(vec![0x02; 128]);
    assert_eq!(
        parse_edid(&long).unwrap().name.as_deref(),
        Some("ABCDEFGHIJKLM")
    );

    // Not an EDID.
    assert_eq!(parse_edid(&[0u8; 128]), None);
    assert_eq!(parse_edid(&edid(1, &[])[..100]), None);
}

#[test]
fn edid_registry_keys() {
    assert_eq!(
        edid_registry_key(
            r"\\?\DISPLAY#DEL4123#5&2a3b4c5&0&UID4353#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"
        )
        .as_deref(),
        Some(
            r"SYSTEM\CurrentControlSet\Enum\DISPLAY\DEL4123\5&2a3b4c5&0&UID4353\Device Parameters"
        )
    );
    assert_eq!(edid_registry_key(r"\\.\DISPLAY1"), None);
    assert_eq!(edid_registry_key(r"\\?\USB#VID_1#x#{g}"), None);
    assert_eq!(edid_registry_key(r"\\?\DISPLAY#..\..#x#{g}"), None);
    assert_eq!(edid_registry_key(r"\\?\DISPLAY##x#{g}"), None);
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Value)>>);

impl EventSink for Recorder {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

fn info(id: &str, x: i32) -> DisplayInfo {
    DisplayInfo {
        id: id.into(),
        name: id.into(),
        edid_key: String::new(),
        x,
        y: 0,
        width: 1920,
        height: 1080,
        refresh_hz: 60,
        primary: x == 0,
        has_main_window: false,
        scaled: false,
    }
}

#[test]
fn the_watcher_runs_only_while_watched_and_debounces() {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        rec.clone(),
        Duration::from_millis(20),
    )
    .unwrap();
    let backend = Arc::new(TestBackend::new(
        vec![info("a", 0)],
        // Well above the 10 ms gaps of the burst below, even on a slow CI runner.
        Duration::from_millis(150),
    ));
    st.set_display_backend(Some(backend.clone()));
    assert_eq!(backend.threads(), 0);
    assert!(st.list_displays().len() == 1);
    let changed = || {
        rec.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == names::DISPLAYS_CHANGED)
            .count()
    };
    assert_eq!(changed(), 1, "the first list is announced");
    backend.notify(); // not watching: ignored
    assert_eq!(st.displays_rt().os_changes(), 0);

    st.watch_displays(true);
    assert_eq!(backend.threads(), 1);
    assert_eq!(changed(), 1, "same list: nothing announced");

    // A burst of notifications: one enumeration after the quiet period, one event.
    backend.set(vec![info("a", 0), info("b", 1920)]);
    let before = backend.enumerations();
    let t = Instant::now();
    for _ in 0..5 {
        backend.notify();
        std::thread::sleep(Duration::from_millis(10));
    }
    // The change is counted before the event goes out: wait for both.
    let deadline = Instant::now() + Duration::from_secs(15);
    while st.displays_rt().os_changes() < 1 || changed() < 2 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(t.elapsed() >= Duration::from_millis(150));
    assert_eq!(backend.enumerations(), before + 1);
    assert_eq!(changed(), 2);
    assert_eq!(st.displays_cached().len(), 2);

    st.watch_displays(false);
    assert_eq!(backend.threads(), 0, "stopped and joined");
    assert!(!st.displays_rt().watching());
}
