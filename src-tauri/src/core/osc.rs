//! OSC control, the protocol-free part: the address scheme, the accept rule, the `trigger.osc`
//! address patterns and the on-air feedback diff. The UDP listener and packet decoding live in
//! `server::osc` (network types stay in `server/`).
//!
//! # Addresses
//! Layers are addressed by screen AND layer id (layer ids are unique per screen only).
//!
//! | address | argument | does |
//! |---|---|---|
//! | `/kinetra/layer/<s>/<l>/take` | – | `live.take:<s>/<l>` |
//! | `/kinetra/layer/<s>/<l>/clear` | – | `live.clear:<s>/<l>` |
//! | `/kinetra/layer/<s>/<l>/toggle` | – | `live.toggle:<s>/<l>` |
//! | `/kinetra/layer/<s>/<l>/update` | – | `live.update:<s>/<l>` |
//! | `/kinetra/layer/<s>/<l>/opacity` | level 0..1 | layer opacity 0–100 % |
//! | `/kinetra/layer/<s>/<l>/var/<key>` | level 0..1 | a `range` variable, min..max |
//! | `/kinetra/layer/<s>/<l>/play` / `pause` / `stop` | – | `media.play|pause|stop:<s>/<l>` |
//! | `/kinetra/layer/<s>/<l>/seek` | seconds | media layer: seek to that position |
//! | `/kinetra/flow/<flowId>/go` | – | `cue.go:<flowId>` |
//! | `/kinetra/source/<id>/next` / `prev` | – | `source.next|prev:<id>` |
//! | `/kinetra/screen/<s>/clear` | – | `live.clearScreen:<s>` |
//! | `/kinetra/clear-all` | – | `live.clearAll` |
//! | `/kinetra/panic` | – | `live.panic` |
//!
//! **Triggers** (every address without a level) run on a message whose first argument is not a
//! numeric 0 or `false`: TouchOSC-style buttons send 1 on press and 0 on release, and the release
//! does nothing. **Levels** (`opacity`, `var/<key>`) take the first argument as a level 0..=1 (an
//! int, float, double, `true`/`false` or a numeric string; like a MIDI fader's CC/127); a level
//! outside 0..=1 is rejected (`Invalid`, logged at debug), a message without arguments sets 1.
//! Everything goes through `AppState::run_control`, so the live rules apply as for the operator.
//!
//! # Accept rule
//! Packets from loopback are always accepted. From other addresses only when the project's
//! `lanAccess` is on AND the sender IP is in `OscSettings.allow` (exact match; IPv4-mapped IPv6
//! addresses are compared as IPv4). Everything else is dropped.
//!
//! # Flows
//! Every accepted message (also non-`/kinetra/` addresses) is offered to `trigger.osc`: a
//! pattern matches when it has the same number of `/` segments and each segment is equal or `*`.
//! ● value: the first argument as text (`""` without arguments).

use std::collections::HashMap;
use std::net::IpAddr;

use crate::core::control_actions::ControlInput;
use crate::core::model::{ControlTarget, MediaAction, ValueTarget};
use crate::core::state::AppState;

/// Longest `trigger.osc` address pattern.
pub const OSC_ADDRESS_MAX: usize = 256;

/// One OSC argument, as far as Kinetra reads it.
#[derive(Debug, Clone, PartialEq)]
pub enum OscArg {
    Int(i64),
    Float(f32),
    Double(f64),
    Text(String),
    Bool(bool),
    /// Nil, Inf, blobs, time tags, colours, MIDI, arrays.
    Other,
}

impl OscArg {
    /// The argument as a number (numeric strings included).
    pub fn number(&self) -> Option<f64> {
        match self {
            OscArg::Int(i) => Some(*i as f64),
            OscArg::Float(f) => Some(f64::from(*f)),
            OscArg::Double(d) => Some(*d),
            OscArg::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            OscArg::Text(t) => t.trim().parse::<f64>().ok(),
            OscArg::Other => None,
        }
        .filter(|n| n.is_finite())
    }

    /// The argument as text (a flow's ● value).
    pub fn text(&self) -> String {
        match self {
            OscArg::Int(i) => i.to_string(),
            // f32's own shortest form: 0.1f32 is "0.1", not "0.10000000149011612".
            OscArg::Float(f) => f.to_string(),
            OscArg::Double(d) => d.to_string(),
            OscArg::Text(t) => t.clone(),
            OscArg::Bool(b) => b.to_string(),
            OscArg::Other => String::new(),
        }
    }
}

/// One received message (bundles are flattened, in order).
#[derive(Debug, Clone, PartialEq)]
pub struct OscMessage {
    pub address: String,
    pub args: Vec<OscArg>,
}

/// What a `/kinetra/…` address asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OscAction {
    /// A registry command id (`live.take:<s>/<l>`, `cue.go:<id>`, …): a trigger.
    Command(String),
    /// A layer value set from a level.
    Value {
        screen_id: String,
        layer_id: String,
        target: ValueTarget,
    },
    /// A media layer seek: the first argument in seconds.
    Seek { screen_id: String, layer_id: String },
}

impl OscAction {
    /// The control target (a seek has none: it is run with [`AppState::osc_message`]).
    pub fn target(&self) -> ControlTarget {
        match self {
            OscAction::Command(id) => ControlTarget::Command {
                command_id: id.clone(),
            },
            OscAction::Seek {
                screen_id,
                layer_id,
            } => ControlTarget::Command {
                command_id: format!("media.seek:{screen_id}/{layer_id}"),
            },
            OscAction::Value {
                screen_id,
                layer_id,
                target,
            } => ControlTarget::Value {
                screen_id: screen_id.clone(),
                layer_id: layer_id.clone(),
                target: target.clone(),
            },
        }
    }
}

/// The action of a `/kinetra/…` address (see the module docs); `None` for anything else.
pub fn parse_address(address: &str) -> Option<OscAction> {
    let rest = address.strip_prefix("/kinetra/")?;
    let seg: Vec<&str> = rest.split('/').collect();
    if seg.iter().any(|s| s.is_empty()) {
        return None;
    }
    let cmd = |id: String| Some(OscAction::Command(id));
    match seg.as_slice() {
        ["layer", s, l, verb @ ("take" | "clear" | "toggle" | "update")] => {
            cmd(format!("live.{verb}:{s}/{l}"))
        }
        ["layer", s, l, verb @ ("play" | "pause" | "stop")] => cmd(format!("media.{verb}:{s}/{l}")),
        ["layer", s, l, "seek"] => Some(OscAction::Seek {
            screen_id: s.to_string(),
            layer_id: l.to_string(),
        }),
        ["layer", s, l, "opacity"] => Some(OscAction::Value {
            screen_id: s.to_string(),
            layer_id: l.to_string(),
            target: ValueTarget::Opacity,
        }),
        ["layer", s, l, "var", key] => Some(OscAction::Value {
            screen_id: s.to_string(),
            layer_id: l.to_string(),
            target: ValueTarget::Var {
                key: key.to_string(),
            },
        }),
        ["flow", f, "go"] => cmd(format!("cue.go:{f}")),
        ["source", id, step @ ("next" | "prev")] => cmd(format!("source.{step}:{id}")),
        ["screen", s, "clear"] => cmd(format!("live.clearScreen:{s}")),
        ["clear-all"] => cmd("live.clearAll".into()),
        ["panic"] => cmd("live.panic".into()),
        _ => None,
    }
}

/// The control input a message gives an action; `None` = nothing to do (a trigger's release, a
/// level that isn't a number).
pub fn input_for(action: &OscAction, args: &[OscArg]) -> Option<ControlInput> {
    let first = args.first();
    match action {
        OscAction::Command(_) => {
            let release = first.and_then(OscArg::number).is_some_and(|n| n == 0.0);
            (!release).then_some(ControlInput::Press)
        }
        OscAction::Value { .. } => match first {
            None => Some(ControlInput::Press),
            Some(a) => a.number().map(|n| ControlInput::Level(n as f32)),
        },
        // Seconds (not a level): see `AppState::osc_message`.
        OscAction::Seek { .. } => first
            .and_then(OscArg::number)
            .map(|n| ControlInput::Level(n as f32)),
    }
}

// ---- Accept rule ------------------------------------------------------------------------------

/// IPv4-mapped IPv6 (`::ffff:a.b.c.d`) as IPv4; everything else unchanged.
pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    ip.to_canonical()
}

/// The allowlist as addresses (normalized; entries that don't parse are skipped, validation
/// already rejected them on save).
pub fn parse_allow(list: &[String]) -> Vec<IpAddr> {
    list.iter()
        .filter_map(|s| s.trim().parse::<IpAddr>().ok())
        .map(normalize_ip)
        .collect()
}

/// Whether a packet from `src` is accepted: loopback always; other senders only with LAN access
/// on and their IP on the allowlist (`allow` normalized by [`parse_allow`]).
pub fn accepts(src: IpAddr, lan: bool, allow: &[IpAddr]) -> bool {
    let ip = normalize_ip(src);
    ip.is_loopback() || (lan && allow.contains(&ip))
}

// ---- trigger.osc patterns ---------------------------------------------------------------------

/// Whether a `trigger.osc` address is valid: starts with `/`, ≤ [`OSC_ADDRESS_MAX`] chars, no
/// whitespace or control characters, no empty segment.
pub fn is_osc_pattern(p: &str) -> bool {
    p.starts_with('/')
        && p.len() > 1
        && p.chars().count() <= OSC_ADDRESS_MAX
        && !p.chars().any(|c| c.is_whitespace() || c.is_control())
        && p[1..].split('/').all(|s| !s.is_empty())
}

/// Whether a pattern has a `*` segment (indexed in the wildcard list, not by exact address).
pub fn has_wildcard(pattern: &str) -> bool {
    pattern.split('/').any(|s| s == "*")
}

/// Whether `address` matches `pattern`: the same number of segments, each equal or `*`.
pub fn address_matches(pattern: &str, address: &str) -> bool {
    let mut p = pattern.split('/');
    let mut a = address.split('/');
    loop {
        match (p.next(), a.next()) {
            (None, None) => return true,
            (Some(ps), Some(as_)) if ps == "*" || ps == as_ => {}
            _ => return false,
        }
    }
}

// ---- Feedback ---------------------------------------------------------------------------------

/// On-air state last sent per layer; [`OnAirDiff::update`] returns what changed. After
/// [`OnAirDiff::reset`] the next update returns every layer (the full state).
#[derive(Debug, Default)]
pub struct OnAirDiff {
    last: HashMap<(String, String), bool>,
    primed: bool,
}

impl OnAirDiff {
    pub fn reset(&mut self) {
        self.last.clear();
        self.primed = false;
    }

    /// `now`: every user layer with its on-air state. Returns the layers to send: all of them
    /// after a reset, else those whose state changed, plus layers that were on air and are gone
    /// (sent as off).
    pub fn update(&mut self, now: Vec<(String, String, bool)>) -> Vec<(String, String, bool)> {
        let full = !self.primed;
        self.primed = true;
        let mut out = Vec::new();
        let mut next = HashMap::with_capacity(now.len());
        for (s, l, on) in now {
            let prev = self.last.remove(&(s.clone(), l.clone()));
            if full || prev != Some(on) {
                out.push((s.clone(), l.clone(), on));
            }
            next.insert((s, l), on);
        }
        for ((s, l), on) in self.last.drain() {
            if on {
                out.push((s, l, false));
            }
        }
        self.last = next;
        out
    }
}

// ---- Dispatch ---------------------------------------------------------------------------------

impl AppState {
    /// Runs one received OSC message: its `/kinetra/…` action (through `run_control`), then the
    /// `trigger.osc` flows that match its address. May block briefly (call it off the async
    /// recv loop). Errors are logged, never returned.
    pub fn osc_message(&self, m: &OscMessage) {
        if let Some(action) = parse_address(&m.address) {
            let done = match (&action, input_for(&action, &m.args)) {
                (_, None) => Ok(()),
                (
                    OscAction::Seek {
                        screen_id,
                        layer_id,
                    },
                    Some(_),
                ) => {
                    let secs = m.args.first().and_then(OscArg::number).unwrap_or(0.0);
                    let ms = (secs.max(0.0) * 1000.0).round() as u64;
                    self.live()
                        .media_control(screen_id, layer_id, MediaAction::Seek, Some(ms))
                        .map(drop)
                        .map_err(Into::into)
                }
                (_, Some(input)) => self.run_control(&action.target(), input),
            };
            if let Err(e) = done {
                tracing::debug!("OSC {}: {e}", m.address);
            }
        }
        self.flow_osc(&m.address, || {
            m.args.first().map(OscArg::text).unwrap_or_default()
        });
    }
}

#[cfg(test)]
mod tests;
