//! LED feedback: what a mapped pad shows, the colour tables per controller, and the diff that
//! sends only changed LEDs.
//!
//! A pad's LED is driven by the message the pad sends: a note mapping gets note-on
//! (`0x90 | channel-1`, note, velocity), a CC mapping gets a CC (`0xB0 | channel-1`, number,
//! value), on the mapping's channel (channel 1 when the mapping listens to any channel). The
//! velocity selects the colour:
//!
//! | profile | on air | ready | off | source |
//! |---|---|---|---|---|
//! | Generic | 127 | 1 | 0 | most controllers light on any velocity > 0; 1 dims where supported |
//! | APC40 (mk1/mkII clip launch) | 3 (red) | 1 (green) | 0 | Akai APC40 Communications Protocol, "Clip Launch LED" values: 0 off, 1 green, 2 green blink, 3 red, 4 red blink, 5 yellow |
//! | APC mini (mk1) | 3 (red) | 1 (green) | 0 | Akai APC mini Communication Protocol: 0 off, 1 green, 3 red, 5 yellow (+1 blink) |
//! | Launchpad (Mk2 / X / Mini mk3, programmer or session layout) | 5 (red) | 21 (green) | 0 | Novation Launchpad Programmer's Reference: velocity = palette index, 5 = red, 21 = green |
//! | X-Touch (Mackie Control buttons) | 127 (lit) | 0 | 0 | Behringer X-Touch MC mode: button LEDs 0 off, 1 flash, 127 on; no colours, so ready = off |

use std::collections::HashMap;

use crate::core::model::{MidiFeedbackProfile, MidiKind};

/// What a pad shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Led {
    Off,
    /// Off air but loaded (a layer that exists; an armed cue): green.
    Ready,
    /// On air (a layer on Program; a running cue): red.
    OnAir,
}

/// The velocity (colour) of `led` in a profile (see the module table).
pub fn velocity(profile: MidiFeedbackProfile, led: Led) -> u8 {
    use MidiFeedbackProfile as P;
    match (profile, led) {
        (_, Led::Off) => 0,
        (P::Generic, Led::OnAir) => 127,
        (P::Generic, Led::Ready) => 1,
        (P::Apc40 | P::ApcMini, Led::OnAir) => 3,
        (P::Apc40 | P::ApcMini, Led::Ready) => 1,
        (P::Launchpad, Led::OnAir) => 5,
        (P::Launchpad, Led::Ready) => 21,
        (P::Xtouch, Led::OnAir) => 127,
        (P::Xtouch, Led::Ready) => 0,
    }
}

/// The status byte that lights a pad of `kind` on `channel` (1–16; `None` = 1).
pub fn status(kind: MidiKind, channel: Option<u8>) -> u8 {
    let ch = channel.unwrap_or(1).clamp(1, 16) - 1;
    match kind {
        MidiKind::Note => 0x90 | ch,
        MidiKind::Cc => 0xB0 | ch,
    }
}

/// One LED on an output: (status byte, note / CC number).
pub type LedKey = (u8, u8);

/// The messages that bring an output from `last` to `want` (changed LEDs only; LEDs no longer
/// wanted are switched off and forgotten), sorted. Updates `last`.
pub fn diff(last: &mut HashMap<LedKey, u8>, want: &HashMap<LedKey, u8>) -> Vec<[u8; 3]> {
    let mut out = Vec::new();
    for (&(s, n), &v) in want {
        if last.get(&(s, n)) != Some(&v) {
            last.insert((s, n), v);
            out.push([s, n, v]);
        }
    }
    last.retain(|k, v| {
        if want.contains_key(k) {
            true
        } else {
            if *v != 0 {
                out.push([k.0, k.1, 0]);
            }
            false
        }
    });
    out.sort_unstable();
    out
}
