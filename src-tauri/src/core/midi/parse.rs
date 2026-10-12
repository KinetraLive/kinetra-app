//! MIDI channel messages Kinetra reacts to: note-on, note-off and control change. Everything
//! else (clock, active sensing, SysEx, pitch bend, aftertouch, program change) is ignored.

/// A parsed message. Channels are 1–16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMsg {
    /// Note-on with velocity > 0.
    NoteOn { channel: u8, note: u8, velocity: u8 },
    /// Note-off, or note-on with velocity 0 (running-status style).
    NoteOff { channel: u8, note: u8 },
    /// Control change.
    Cc { channel: u8, number: u8, value: u8 },
}

/// Parses one complete message (as midir delivers them: status byte first). `None` for anything
/// that isn't a note-on / note-off / CC with valid data bytes.
pub fn parse(bytes: &[u8]) -> Option<MidiMsg> {
    let (&status, data) = bytes.split_first()?;
    if status & 0x80 == 0 || data.len() < 2 || data[0] > 127 || data[1] > 127 {
        return None;
    }
    let channel = (status & 0x0F) + 1;
    let (a, b) = (data[0], data[1]);
    match status & 0xF0 {
        0x80 => Some(MidiMsg::NoteOff { channel, note: a }),
        0x90 if b == 0 => Some(MidiMsg::NoteOff { channel, note: a }),
        0x90 => Some(MidiMsg::NoteOn {
            channel,
            note: a,
            velocity: b,
        }),
        0xB0 => Some(MidiMsg::Cc {
            channel,
            number: a,
            value: b,
        }),
        _ => None,
    }
}

/// Parses hex bytes like `90 24 7f` or `90247f` (the fixture's `midiin`).
pub fn parse_hex(s: &str) -> Option<Vec<u8>> {
    let digits: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !digits.is_ascii()
        || digits.is_empty()
        || !digits.len().is_multiple_of(2)
        || digits.len() > 6
    {
        return None;
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).ok())
        .collect()
}
