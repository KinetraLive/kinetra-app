//! Matching a device name saved in a project (a mapping's or a `trigger.midi`'s `device`) with
//! the ports present on this computer. Windows (WinMM) and macOS (CoreMIDI) name the same
//! controller differently: WinMM cuts names at 31 characters and may wrap them
//! ("MIDIIN2 (APC mini mk2)"); CoreMIDI adds the port's name ("APC mini mk2 Control"). A project
//! made on one system keeps working on the other (owner's decision, macOS parity plan):
//!
//! 1. the exact name always matches;
//! 2. otherwise, while no present port has exactly the saved name, the one present port whose
//!    [`normalize`]d name equals the saved name's matches. Two or more such ports: none (the
//!    operator learns the mapping again rather than Kinetra guessing).

/// Port-name endings that only say what kind of port it is.
const KIND_SUFFIXES: [&str; 2] = [" control", " midi"];
/// WinMM's limit on device names.
const WINMM_MAX: usize = 31;

/// The comparable form of a port name: lowercase, whitespace collapsed, a WinMM wrapper
/// (`MIDIIN2 (…)`, `MIDIOUT2 (…)`) removed, a trailing port-kind word or port number ("Control",
/// "MIDI", "Port 1", "MIDI 1") removed, cut to WinMM's 31 characters. Kinetra's own `#2` suffix
/// for a second identical device is kept, so it only matches the second device.
pub fn normalize(name: &str) -> String {
    let mut s = name.split_whitespace().collect::<Vec<_>>().join(" ");
    s = s.to_lowercase();
    // Kinetra's suffix for repeated names (`backend::unique_names`).
    let (base, nth) = match s.rsplit_once(" #") {
        Some((b, n)) if !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()) => {
            (b.to_string(), format!(" #{n}"))
        }
        _ => (s.clone(), String::new()),
    };
    let mut base = unwrap_winmm(&base).to_string();
    loop {
        let before = base.len();
        base = strip_port_number(&base).to_string();
        for suffix in KIND_SUFFIXES {
            if let Some(b) = base.strip_suffix(suffix) {
                if !b.is_empty() {
                    base = b.to_string();
                }
            }
        }
        if base.len() == before {
            break;
        }
    }
    let base: String = base.chars().take(WINMM_MAX).collect();
    format!("{}{nth}", base.trim_end())
}

/// `midiin2 (name)` / `midiout (name)` → `name`.
fn unwrap_winmm(s: &str) -> &str {
    for prefix in ["midiin", "midiout"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
            if let Some(inner) = rest.strip_prefix(" (").and_then(|r| r.strip_suffix(')')) {
                if !inner.is_empty() {
                    return inner;
                }
            }
        }
    }
    s
}

/// `name port 2` / `name 2` after a kind word (`name midi 2`) → without the number.
fn strip_port_number(s: &str) -> &str {
    let Some((head, n)) = s.rsplit_once(' ') else {
        return s;
    };
    if n.is_empty() || !n.bytes().all(|c| c.is_ascii_digit()) {
        return s;
    }
    if let Some(h) = head.strip_suffix(" port") {
        return h;
    }
    if KIND_SUFFIXES.iter().any(|k| head.ends_with(k)) {
        return head;
    }
    s
}

/// Whether the saved device name `saved` means the present port `port` (see the module docs).
/// `present` is every input port present now.
pub fn device_is(saved: &str, port: &str, present: &[String]) -> bool {
    if saved == port {
        return true;
    }
    if present.iter().any(|p| p == saved) {
        return false;
    }
    let want = normalize(saved);
    if want.is_empty() || normalize(port) != want {
        return false;
    }
    present.iter().filter(|p| normalize(p) == want).count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn normalizes_the_ways_systems_name_one_controller() {
        assert_eq!(normalize("APC mini mk2 Control"), "apc mini mk2");
        assert_eq!(normalize("MIDIIN2 (APC mini mk2)"), "apc mini mk2");
        assert_eq!(normalize("MIDIOUT2 (APC mini mk2)"), "apc mini mk2");
        assert_eq!(normalize("  Akai   APC40 "), "akai apc40");
        assert_eq!(normalize("nanoKONTROL2 MIDI 1"), "nanokontrol2");
        assert_eq!(normalize("Launch Control XL Port 2"), "launch control xl");
        assert_eq!(normalize("X-Touch #2"), "x-touch #2");
        assert_eq!(normalize("X-Touch MIDI #2"), "x-touch #2");
        // A product name ending in a kind word keeps the rest.
        assert_eq!(normalize("Control"), "control");
        assert_eq!(normalize("MIDI"), "midi");
        // WinMM's 31 characters.
        assert_eq!(
            normalize("Arturia BeatStep Pro Arturia BeatStepPro"),
            normalize("Arturia BeatStep Pro Arturia Be")
        );
    }

    #[test]
    fn a_mapping_from_windows_finds_the_mac_port() {
        let mac = v(&["APC mini mk2 Control", "APC mini mk2 Notes"]);
        assert!(device_is("APC mini mk2", "APC mini mk2 Control", &mac));
        assert!(!device_is("APC mini mk2", "APC mini mk2 Notes", &mac));
        let win = v(&["MIDIIN2 (APC mini mk2)", "APC mini mk2"]);
        // On Windows the exact port wins over the wrapped one.
        assert!(device_is("APC mini mk2", "APC mini mk2", &win));
        assert!(!device_is("APC mini mk2", "MIDIIN2 (APC mini mk2)", &win));
        // A mapping saved on a Mac finds the Windows port.
        let win = v(&["Arturia BeatStep Pro Arturia Be"]);
        assert!(device_is(
            "Arturia BeatStep Pro Arturia BeatStepPro",
            "Arturia BeatStep Pro Arturia Be",
            &win
        ));
    }

    #[test]
    fn exact_names_win_and_ambiguous_names_match_nothing() {
        let present = v(&["X-Touch", "X-Touch MIDI"]);
        assert!(device_is("X-Touch", "X-Touch", &present));
        assert!(!device_is("X-Touch", "X-Touch MIDI", &present));
        // Two ports normalize alike and neither is the saved name: no guess.
        let present = v(&["X-Touch Control", "X-Touch MIDI"]);
        assert!(!device_is("X-Touch", "X-Touch Control", &present));
        assert!(!device_is("X-Touch", "X-Touch MIDI", &present));
        // The second identical device stays the second.
        let present = v(&["X-Touch Control", "X-Touch Control #2"]);
        assert!(device_is("X-Touch #2", "X-Touch Control #2", &present));
        assert!(!device_is("X-Touch #2", "X-Touch Control", &present));
        // Unrelated names never match.
        assert!(!device_is("Launchpad X", "LPX MIDI", &v(&["LPX MIDI"])));
        assert!(!device_is("", "LPX MIDI", &v(&["LPX MIDI"])));
    }
}
