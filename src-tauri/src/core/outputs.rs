//! Display outputs: borderless windows showing a screen (fill) or its alpha matte (key) on a
//! connected display.
//!
//! # Model
//! `prefs.json` `outputs` ([`OutputConfig`], per machine), clamped by [`clamp_outputs`].
//!
//! # Plan (pure)
//! [`plan`] turns the configs, the connected displays, the open project (server port, screens)
//! and the test patterns showing into the windows that should exist ([`WindowPlan`]) and a
//! status per output ([`OutputStatus`]):
//! - display match: by id; else by EDID key (non-empty) + the same size ("Moved to another
//!   port"); else `displayMissing`. A display already used by an earlier output: `failed`.
//! - a key output and its fill on displays of the same size and refresh rate that share a full
//!   edge become ONE window spanning both (`frameLocked: true`): the fill left of the key
//!   `dual=h`, above it `dual=v`, right of it `dual=hr`, below it `dual=vr` (r: the key's half
//!   comes first); otherwise two windows (`frameLocked: false`).
//! - no project → the app's idle page (`output-idle.html`, black or the fill color); a screen
//!   that isn't in the project → idle (`screenMissing`); the server failed → idle (`failed`).
//! - a test pattern replaces the screen of the output it was asked for (turning it on while it
//!   shows) and of its pair partner only when that partner is on.
//!
//! # Runtime
//! Reconciles run on the blocking pool (never on the caller's thread: `set_prefs` may run on the
//! main thread, and creating a window waits for it), coalesced (one pending at a time) and
//! serialized. They are requested by prefs changes (outputs), display changes, project open /
//! close, the screen set or a screen size changing, and server status changes. Each one plans,
//! hands the windows to the [`OutputHost`] (Tauri, `src/outputs.rs`) only when they changed,
//! and emits `outputs-changed` only when a status changed. With no output configured nothing
//! runs: no window, no display watcher.
//!
//! The host recovers crashed output pages itself; when one keeps crashing it closes the window,
//! calls [`AppState::outputs_host_changed`] and reports [`PAGE_CRASHING`] from `apply` until the
//! window's plan changes.
//!
//! # Audio
//! A fill output's screen URL carries its media audio: `&audio=0` (off) or `&sink=<label>` (a
//! device; absent = the system default). Key outputs always get `&audio=0`; a fill + key pair
//! in one window follows the fill.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

use serde::Deserialize;

use crate::core::events::names;
use crate::core::frame_out::{AlphaMode, FrameRate, PixelFormat};
use crate::core::model::{
    DisplayInfo, NdiBus, NdiLive, NdiRate, OutputAudio, OutputBackground, OutputConfig,
    OutputDisplayRef, OutputKind, OutputMode, OutputState, OutputStatus, ServerStatus, TestPattern,
    CHROMA_GREEN, NDI_GROUPS_MAX, NDI_OUTPUTS_MAX, OUTPUTS_MAX, OUTPUT_AUDIO_DEVICE_MAX,
    OUTPUT_NAME_MAX,
};
use crate::core::state::AppState;
use crate::core::templates::is_id;
use crate::error::{AppError, AppResult};

/// How long Identify displays shows each display's name.
pub const IDENTIFY_FOR: Duration = Duration::from_secs(3);
/// Window label prefix of output windows (`output-<fill or output id>`).
pub const OUTPUT_LABEL_PREFIX: &str = "output-";
/// Window label prefix of the temporary Identify windows.
pub const IDENTIFY_LABEL_PREFIX: &str = "identify-";
/// The app page output windows show without a project.
pub const IDLE_PAGE: &str = "output-idle.html";

const MOVED: &str = "Moved to another port";
const DISPLAY_TAKEN: &str = "Display used by another output";
const WINDOW_FAILED: &str = "Couldn't open the output window";
/// The host's failure for an output window whose page crashed more than 3 times in a minute
/// (shown as the status message as is; other host failures show [`WINDOW_FAILED`]).
pub const PAGE_CRASHING: &str = "The output page keeps crashing";

/// Longest stored display id / name / EDID key.
const DISPLAY_ID_MAX: usize = 512;
const DISPLAY_NAME_MAX: usize = 128;
const EDID_KEY_MAX: usize = 64;
const DISPLAY_SIDE_MAX: u32 = 16384;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- Model helpers ----------------------------------------------------------------------------

/// `Prefs.outputs` deserializer: entries that don't parse are dropped (one bad entry never
/// quarantines the whole prefs.json); anything but an array is no outputs.
pub fn lenient_outputs<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<OutputConfig>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    let serde_json::Value::Array(items) = v else {
        return Ok(Vec::new());
    };
    Ok(items
        .into_iter()
        .filter_map(|i| serde_json::from_value(i).ok())
        .collect())
}

/// A `#rrggbb` color, lowercased; `None` otherwise.
pub fn normalize_hex(color: &str) -> Option<String> {
    let hex = color.strip_prefix('#')?;
    (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("#{}", hex.to_ascii_lowercase()))
}

/// Up to `max` characters of `s` without control characters.
fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// Clamps outputs as stored (see `types.ts`):
/// - ≤ [`OUTPUTS_MAX`]; entries with an invalid or repeated id are dropped;
/// - names trimmed to 1–40 characters (empty → "Output N"), unique ignoring case (a repeat gets
///   " 2", " 3", …);
/// - background colors `#rrggbb` lowercase (invalid → black); screen ids must be ids;
/// - a fill output has no `keyOf`; a key output whose `keyOf` doesn't name another fill output,
///   or names a fill an earlier key output already keys, loses it (`keyOf: null`) and is
///   **turned off** (a key output never silently becomes a fill output: a keyer would cut a hole
///   with full-color video);
/// - no two enabled outputs on the same display: the later one is turned off;
/// - audio device labels without control characters, ≤ 256 characters (empty → the system
///   default).
pub fn clamp_outputs(list: Vec<OutputConfig>) -> Vec<OutputConfig> {
    let mut out: Vec<OutputConfig> = Vec::new();
    let mut ids = HashSet::new();
    for mut c in list {
        if out.len() >= OUTPUTS_MAX {
            break;
        }
        if !is_id(&c.id) || !ids.insert(c.id.clone()) {
            continue;
        }
        c.display.id = clean(&c.display.id, DISPLAY_ID_MAX);
        c.display.name = clean(c.display.name.trim(), DISPLAY_NAME_MAX);
        c.display.edid_key = clean(&c.display.edid_key, EDID_KEY_MAX);
        c.display.width = c.display.width.min(DISPLAY_SIDE_MAX);
        c.display.height = c.display.height.min(DISPLAY_SIDE_MAX);
        c.screen_id = c.screen_id.filter(|s| is_id(s));
        c.audio = c.audio.map(|a| OutputAudio {
            enabled: a.enabled,
            device: a
                .device
                .map(|d| clean(d.trim(), OUTPUT_AUDIO_DEVICE_MAX).trim().to_string())
                .filter(|d| !d.is_empty()),
        });
        if let OutputBackground::Color { color } = &c.background {
            c.background = match normalize_hex(color) {
                Some(color) => OutputBackground::Color { color },
                None => OutputBackground::Black,
            };
        }
        match c.kind {
            OutputKind::Display => c.ndi = None,
            OutputKind::Ndi => {
                if out.iter().filter(|o| o.kind == OutputKind::Ndi).count() >= NDI_OUTPUTS_MAX {
                    continue;
                }
                // An NDI source is a fill with alpha: no display, no key pair, no audio (v1).
                c.display = OutputDisplayRef::default();
                c.mode = OutputMode::Fill;
                c.key_of = None;
                c.audio = None;
                c.always_on_top = false;
                let mut ndi = c.ndi.take().unwrap_or_default();
                ndi.groups = ndi
                    .groups
                    .map(|g| clean(g.trim(), NDI_GROUPS_MAX).trim().to_string())
                    .filter(|g| !g.is_empty());
                c.ndi = Some(ndi);
            }
        }
        out.push(c);
    }
    // Names: unique ignoring case.
    let mut taken: HashSet<String> = HashSet::new();
    for (i, c) in out.iter_mut().enumerate() {
        let mut base: String = clean(c.name.trim(), OUTPUT_NAME_MAX).trim().to_string();
        if base.is_empty() {
            base = format!("Output {}", i + 1);
        }
        let mut name = base.clone();
        let mut n = 2;
        while taken.contains(&name.to_lowercase()) {
            let suffix = format!(" {n}");
            let keep = OUTPUT_NAME_MAX - suffix.chars().count();
            name = format!(
                "{}{suffix}",
                base.chars().take(keep).collect::<String>().trim_end()
            );
            n += 1;
        }
        taken.insert(name.to_lowercase());
        c.name = name;
    }
    // Key pairs.
    let fills: HashSet<String> = out
        .iter()
        .filter(|c| c.mode == OutputMode::Fill && c.kind == OutputKind::Display)
        .map(|c| c.id.clone())
        .collect();
    let mut keyed = HashSet::new();
    for c in out.iter_mut() {
        match c.mode {
            OutputMode::Fill => c.key_of = None,
            OutputMode::Key => {
                let ok = c
                    .key_of
                    .as_ref()
                    .is_some_and(|f| *f != c.id && fills.contains(f) && keyed.insert(f.clone()));
                if !ok {
                    c.key_of = None;
                    c.enabled = false;
                }
            }
        }
    }
    // One enabled output per display.
    let mut used = HashSet::new();
    for c in out.iter_mut().filter(|c| c.enabled) {
        if !c.display.id.is_empty() && !used.insert(c.display.id.clone()) {
            c.enabled = false;
        }
    }
    out
}

/// Percent-encodes a query value (RFC 3986 unreserved characters stay).
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A fill output's audio as screen URL parameters: `&audio=0` when off, `&sink=<label>`
/// (percent-encoded) for a chosen device, nothing for the system default. Key outputs always get
/// `&audio=0` (see [`plan`]); a fill + key pair in one window follows the fill.
pub fn audio_query(audio: Option<&OutputAudio>) -> String {
    match audio {
        Some(a) if !a.enabled => "&audio=0".into(),
        Some(OutputAudio {
            device: Some(d), ..
        }) => format!("&sink={}", enc(d)),
        _ => String::new(),
    }
}

/// The page background of a fill output, as the URL's `bg` (`rrggbb`).
pub fn bg_hex(bg: &OutputBackground) -> String {
    let hex = match bg {
        OutputBackground::Black => "#000000",
        OutputBackground::Green => CHROMA_GREEN,
        OutputBackground::Color { color } => color.as_str(),
    };
    normalize_hex(hex)
        .map(|h| h[1..].to_string())
        .unwrap_or_else(|| "000000".into())
}

// ---- Plan -------------------------------------------------------------------------------------

/// A window rectangle in physical desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PlanRect {
    fn of(d: &DisplayInfo) -> Self {
        Self {
            x: d.x,
            y: d.y,
            width: d.width,
            height: d.height,
        }
    }
}

/// What a window loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowUrl {
    /// The output server (`http://127.0.0.1:<port>/…`).
    Http(String),
    /// An app page with its query (`output-idle.html?bg=000000`).
    App(String),
}

/// A window that should exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowPlan {
    /// `output-<id>` (a fill + key pair in one window: the fill's id).
    pub label: String,
    pub rect: PlanRect,
    pub url: WindowUrl,
    pub always_on_top: bool,
    /// The outputs it shows (two for a spanning pair: fill, key).
    pub outputs: Vec<String>,
}

/// The output server as the plan sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ServerState {
    #[default]
    Stopped,
    Starting,
    Running {
        port: u16,
    },
    Failed {
        message: String,
    },
}

impl From<&ServerStatus> for ServerState {
    fn from(s: &ServerStatus) -> Self {
        match s {
            ServerStatus::Stopped => ServerState::Stopped,
            ServerStatus::Starting { .. } => ServerState::Starting,
            ServerStatus::Running { port, .. } => ServerState::Running { port: *port },
            ServerStatus::Error { message, .. } => ServerState::Failed {
                message: message.clone(),
            },
        }
    }
}

/// The open project as the plan sees it.
#[derive(Debug, Clone, Copy)]
pub struct PlanProject<'a> {
    pub server: &'a ServerState,
    /// Screen id → (width, height).
    pub screens: &'a HashMap<String, (u32, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dual {
    /// Fill left, key right.
    H,
    /// Fill above, key below.
    V,
    /// Key left, fill right.
    Hr,
    /// Key above, fill below.
    Vr,
}

impl Dual {
    fn as_str(self) -> &'static str {
        match self {
            Dual::H => "h",
            Dual::V => "v",
            Dual::Hr => "hr",
            Dual::Vr => "vr",
        }
    }

    /// Side by side (else stacked).
    fn horizontal(self) -> bool {
        matches!(self, Dual::H | Dual::Hr)
    }
}

/// One window for a fill and its key: same size and refresh rate, sharing a full edge (the fill
/// on either side; the renderer draws each in its half).
fn spanning(fill: &DisplayInfo, key: &DisplayInfo) -> Option<Dual> {
    if fill.width != key.width || fill.height != key.height || fill.refresh_hz != key.refresh_hz {
        return None;
    }
    let (fx, fy, kx, ky) = (
        i64::from(fill.x),
        i64::from(fill.y),
        i64::from(key.x),
        i64::from(key.y),
    );
    let (w, h) = (i64::from(fill.width), i64::from(fill.height));
    if fy == ky && fx + w == kx {
        Some(Dual::H)
    } else if fy == ky && kx + w == fx {
        Some(Dual::Hr)
    } else if fx == kx && fy + h == ky {
        Some(Dual::V)
    } else if fx == kx && ky + h == fy {
        Some(Dual::Vr)
    } else {
        None
    }
}

/// The window content of one output.
struct Content {
    state: OutputState,
    message: Option<String>,
    url: WindowUrl,
}

/// Plans windows and statuses (see the module docs). Statuses come in config order, one per
/// config.
pub fn plan(
    configs: &[OutputConfig],
    displays: &[DisplayInfo],
    project: Option<PlanProject<'_>>,
    patterns: &HashMap<String, TestPattern>,
) -> (Vec<WindowPlan>, Vec<OutputStatus>) {
    let n = configs.len();
    let index: HashMap<&str, usize> = configs
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str(), i))
        .collect();
    // The fill a key output keys for (a valid pair only).
    let fill_of = |i: usize| -> Option<usize> {
        let c = &configs[i];
        if c.mode != OutputMode::Key {
            return None;
        }
        let f = *index.get(c.key_of.as_deref()?)?;
        (configs[f].mode == OutputMode::Fill && configs[f].kind == OutputKind::Display && f != i)
            .then_some(f)
    };
    let key_of_fill: HashMap<usize, usize> =
        (0..n).filter_map(|k| fill_of(k).map(|f| (f, k))).collect();
    // A pattern shows on the output it was asked for, and on its pair partner only when that
    // partner is on (an output that is off stays off).
    let pats: Vec<Option<TestPattern>> = (0..n)
        .map(|i| {
            let own = patterns.get(&configs[i].id).copied();
            let partner = fill_of(i).or_else(|| key_of_fill.get(&i).copied());
            own.or_else(|| {
                configs[i]
                    .enabled
                    .then_some(partner)
                    .flatten()
                    .and_then(|p| patterns.get(&configs[p].id).copied())
            })
        })
        .collect();
    // NDI outputs have no window (see `plan_ndi`).
    let active: Vec<bool> = (0..n)
        .map(|i| {
            configs[i].kind == OutputKind::Display && (configs[i].enabled || pats[i].is_some())
        })
        .collect();

    // Displays: exact ids first, then the fallback match for the rest.
    let mut assigned: Vec<Option<(&DisplayInfo, bool)>> = vec![None; n];
    let mut clash = vec![false; n];
    let mut taken: HashSet<&str> = HashSet::new();
    for i in (0..n).filter(|&i| active[i]) {
        if let Some(d) = displays.iter().find(|d| d.id == configs[i].display.id) {
            if taken.insert(d.id.as_str()) {
                assigned[i] = Some((d, false));
            } else {
                clash[i] = true;
            }
        }
    }
    for i in 0..n {
        if !active[i] || assigned[i].is_some() || clash[i] {
            continue;
        }
        let r = &configs[i].display;
        if r.edid_key.is_empty() {
            continue;
        }
        if let Some(d) = displays.iter().find(|d| {
            !taken.contains(d.id.as_str())
                && d.edid_key == r.edid_key
                && d.width == r.width
                && d.height == r.height
        }) {
            taken.insert(d.id.as_str());
            assigned[i] = Some((d, true));
        }
    }

    // Pairs.
    let mut frame_locked: Vec<Option<bool>> = vec![None; n];
    let mut span: HashMap<usize, (usize, Dual)> = HashMap::new();
    let mut spanned_keys: HashSet<usize> = HashSet::new();
    for (&f, &k) in &key_of_fill {
        if !active[f] || !active[k] {
            continue;
        }
        let (Some((fd, _)), Some((kd, _))) = (assigned[f], assigned[k]) else {
            continue;
        };
        let dual = spanning(fd, kd);
        frame_locked[f] = Some(dual.is_some());
        frame_locked[k] = Some(dual.is_some());
        if let Some(dual) = dual {
            span.insert(f, (k, dual));
            spanned_keys.insert(k);
        }
    }

    let content = |i: usize, d: &DisplayInfo, dual: Option<Dual>| -> Content {
        let c = &configs[i];
        let key = c.mode == OutputMode::Key;
        let mode = if key { "key" } else { "fill" };
        let bg = if key {
            "000000".to_string()
        } else {
            bg_hex(&c.background)
        };
        let screen_id = match fill_of(i) {
            Some(f) => configs[f].screen_id.as_deref(),
            None if key => None,
            None => c.screen_id.as_deref(),
        };
        let dual_q = dual
            .map(|d| format!("&dual={}", d.as_str()))
            .unwrap_or_default();
        let res = format!("{}x{}", d.width, d.height);
        let idle = |with_name: bool| {
            let mut q = if dual.is_some() {
                "bg=000000".to_string()
            } else {
                format!("bg={bg}")
            };
            if key && dual.is_none() {
                q.push_str("&mode=key");
            }
            if with_name {
                q.push_str(&format!(
                    "&name={}&res={res}&hz={}",
                    enc(&c.name),
                    d.refresh_hz
                ));
            }
            WindowUrl::App(format!("{IDLE_PAGE}?{q}"))
        };
        let Some(p) = project else {
            return Content {
                state: OutputState::NoProject,
                message: None,
                url: idle(pats[i].is_some()),
            };
        };
        let screen = screen_id.and_then(|s| p.screens.get(s).map(|wh| (s, *wh)));
        match p.server {
            ServerState::Failed { message } => Content {
                state: OutputState::Failed,
                message: Some(message.clone()),
                url: idle(pats[i].is_some()),
            },
            ServerState::Running { port } => {
                if let Some(pat) = pats[i] {
                    let (w, h) = screen.map(|(_, wh)| wh).unwrap_or((d.width, d.height));
                    return Content {
                        state: OutputState::Showing,
                        message: None,
                        url: WindowUrl::Http(format!(
                            "http://127.0.0.1:{port}/pattern?kind={}&w={w}&h={h}&name={}&display={}&res={res}&hz={}&mode={mode}{dual_q}",
                            pat.as_str(),
                            enc(&c.name),
                            enc(&d.name),
                            d.refresh_hz,
                        )),
                    };
                }
                match screen {
                    Some((sid, _)) => Content {
                        state: OutputState::Showing,
                        message: None,
                        url: WindowUrl::Http(format!(
                            "http://127.0.0.1:{port}/screen/{}?output={}&bg={bg}&mode={mode}{dual_q}{}",
                            enc(sid),
                            enc(&c.id),
                            if key {
                                "&audio=0".to_string()
                            } else {
                                audio_query(c.audio.as_ref())
                            },
                        )),
                    },
                    None => Content {
                        state: OutputState::ScreenMissing,
                        message: None,
                        url: idle(false),
                    },
                }
            }
            // Starting (or about to): black until the server listens, a moment later.
            ServerState::Starting | ServerState::Stopped => Content {
                state: if pats[i].is_some() || screen.is_some() {
                    OutputState::Showing
                } else {
                    OutputState::ScreenMissing
                },
                message: None,
                url: idle(false),
            },
        }
    };

    let mut windows = Vec::new();
    let mut statuses = Vec::with_capacity(n);
    for i in 0..n {
        let c = &configs[i];
        let mut st = OutputStatus {
            id: c.id.clone(),
            state: OutputState::Off,
            display_id: None,
            frame_locked: None,
            pattern: None,
            message: None,
            ndi: None,
        };
        if !active[i] {
            statuses.push(st);
            continue;
        }
        st.pattern = pats[i];
        if clash[i] {
            st.state = OutputState::Failed;
            st.message = Some(DISPLAY_TAKEN.into());
            statuses.push(st);
            continue;
        }
        let Some((d, moved)) = assigned[i] else {
            st.state = OutputState::DisplayMissing;
            statuses.push(st);
            continue;
        };
        // The pair's window (the key's display is assigned: checked when the span was made).
        let pair = span
            .get(&i)
            .and_then(|&(k, dual)| assigned[k].map(|(kd, _)| (k, kd, dual)));
        let ct = content(i, d, pair.map(|(_, _, dual)| dual));
        st.state = ct.state;
        st.message = ct.message.or_else(|| moved.then(|| MOVED.to_string()));
        st.display_id = Some(d.id.clone());
        st.frame_locked = frame_locked[i];
        statuses.push(st);
        if spanned_keys.contains(&i) {
            continue; // shown by its fill's window
        }
        let (rect, aot, outputs) = match pair {
            Some((k, kd, dual)) => {
                // From the top-left display of the two.
                let r = PlanRect {
                    x: d.x.min(kd.x),
                    y: d.y.min(kd.y),
                    width: if dual.horizontal() {
                        d.width + kd.width
                    } else {
                        d.width
                    },
                    height: if dual.horizontal() {
                        d.height
                    } else {
                        d.height + kd.height
                    },
                };
                (
                    r,
                    c.always_on_top || configs[k].always_on_top,
                    vec![c.id.clone(), configs[k].id.clone()],
                )
            }
            None => (PlanRect::of(d), c.always_on_top, vec![c.id.clone()]),
        };
        windows.push(WindowPlan {
            label: format!("{OUTPUT_LABEL_PREFIX}{}", c.id),
            rect,
            url: ct.url,
            always_on_top: aot,
            outputs,
        });
    }
    (windows, statuses)
}

/// The temporary Identify windows: one per display, its name, resolution and refresh rate on
/// the idle page (no server needed), on top.
pub fn identify_plan(displays: &[DisplayInfo]) -> Vec<WindowPlan> {
    displays
        .iter()
        .enumerate()
        .map(|(i, d)| WindowPlan {
            label: format!("{IDENTIFY_LABEL_PREFIX}{}", i + 1),
            rect: PlanRect::of(d),
            url: WindowUrl::App(format!(
                "{IDLE_PAGE}?bg=000000&name={}&res={}x{}&hz={}",
                enc(&d.name),
                d.width,
                d.height,
                d.refresh_hz
            )),
            always_on_top: true,
            outputs: Vec::new(),
        })
        .collect()
}

// ---- NDI plan (pure) --------------------------------------------------------------------------

/// The NDI host's failure for "the NDI Runtime isn't installed" (status `ndiMissing`; other
/// failures are shown as `failed` with the message).
pub const NDI_MISSING: &str = "ndiMissing";
/// Shown where no NDI host exists (macOS for now).
pub const NDI_UNAVAILABLE: &str = "NDI output isn't available on this computer yet";

/// An NDI source that should be sending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NdiPlan {
    /// The output's id.
    pub id: String,
    /// The NDI source name (the output's name; receivers show `MACHINE (name)`).
    pub name: String,
    /// The renderer page the source captures (silent, counted for readiness, `cap=1`).
    pub url: String,
    /// Capture size: the screen's, the width rounded up to even (4:2:2).
    pub width: u32,
    pub height: u32,
    pub rate: FrameRate,
    pub format: PixelFormat,
    pub alpha: AlphaMode,
    /// Behind the graphics when alpha is off.
    pub background: [u8; 3],
    pub groups: Option<String>,
}

/// European countries and territories (ISO 3166-1 alpha-2): 50 Hz video.
const EUROPE: &[&str] = &[
    "AD", "AL", "AM", "AT", "AX", "AZ", "BA", "BE", "BG", "BY", "CH", "CY", "CZ", "DE", "DK", "EE",
    "ES", "FI", "FO", "FR", "GB", "GE", "GG", "GI", "GR", "HR", "HU", "IE", "IM", "IS", "IT", "JE",
    "LI", "LT", "LU", "LV", "MC", "MD", "ME", "MK", "MT", "NL", "NO", "PL", "PT", "RO", "RS", "RU",
    "SE", "SI", "SJ", "SK", "SM", "TR", "UA", "VA", "XK",
];

/// A new NDI output's frame rate: 50 in Europe, 59.94 elsewhere (owner decision 2026-10-21),
/// from the system's region (`None`: unknown → 59.94).
pub fn default_ndi_rate(region: Option<&str>) -> NdiRate {
    match region {
        Some(r) if EUROPE.contains(&r.trim().to_ascii_uppercase().as_str()) => NdiRate::R50,
        _ => NdiRate::R59_94,
    }
}

pub fn frame_rate(r: NdiRate) -> FrameRate {
    match r {
        NdiRate::R25 => FrameRate::R25,
        NdiRate::R29_97 => FrameRate::R29_97,
        NdiRate::R30 => FrameRate::R30,
        NdiRate::R50 => FrameRate::R50,
        NdiRate::R59_94 => FrameRate::R59_94,
        NdiRate::R60 => FrameRate::R60,
    }
}

fn rgb(bg: &OutputBackground) -> [u8; 3] {
    let hex = bg_hex(bg);
    let b = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    [b(0), b(2), b(4)]
}

/// The NDI sources to run and the statuses of the NDI outputs (index into `configs`, status).
/// An enabled NDI output sends only with a project whose server runs and whose screen exists;
/// otherwise `noProject` / `screenMissing` / `failed` (server) with nothing sent. A source that
/// is planned reports `showing`; the host's failures and receivers are filled in afterwards.
pub fn plan_ndi(
    configs: &[OutputConfig],
    project: Option<PlanProject<'_>>,
) -> (Vec<NdiPlan>, Vec<(usize, OutputStatus)>) {
    let mut plans = Vec::new();
    let mut statuses = Vec::new();
    for (i, c) in configs.iter().enumerate() {
        if c.kind != OutputKind::Ndi {
            continue;
        }
        // Always `Some` for NDI outputs: that is how `status_on_air` tells them apart.
        let mut st = OutputStatus {
            id: c.id.clone(),
            state: OutputState::Off,
            display_id: None,
            frame_locked: None,
            pattern: None,
            message: None,
            ndi: Some(NdiLive::default()),
        };
        if c.enabled {
            let settings = c.ndi.clone().unwrap_or_default();
            match project {
                None => st.state = OutputState::NoProject,
                Some(p) => {
                    let screen = c
                        .screen_id
                        .as_deref()
                        .and_then(|s| p.screens.get(s).map(|wh| (s, *wh)));
                    match (p.server, screen) {
                        (_, None) => st.state = OutputState::ScreenMissing,
                        (ServerState::Failed { message }, _) => {
                            st.state = OutputState::Failed;
                            st.message = Some(message.clone());
                        }
                        // Starting: sends once the server listens, a moment later.
                        (ServerState::Starting | ServerState::Stopped, Some(_)) => {
                            st.state = OutputState::Showing;
                        }
                        (ServerState::Running { port }, Some((sid, (w, h)))) => {
                            let bus = match settings.bus {
                                NdiBus::Program => "",
                                NdiBus::Preview => "/preview",
                            };
                            plans.push(NdiPlan {
                                id: c.id.clone(),
                                name: c.name.clone(),
                                url: format!(
                                    "http://127.0.0.1:{port}/screen/{}{bus}?audio=0&cap=1",
                                    enc(sid)
                                ),
                                width: w + w % 2,
                                height: h,
                                rate: frame_rate(settings.rate),
                                format: if settings.alpha {
                                    PixelFormat::Uyva
                                } else {
                                    PixelFormat::Uyvy
                                },
                                alpha: if settings.premultiplied {
                                    AlphaMode::Premultiplied
                                } else {
                                    AlphaMode::Straight
                                },
                                background: rgb(&c.background),
                                groups: settings.groups.clone(),
                            });
                            st.state = OutputState::Showing;
                        }
                    }
                }
            }
        }
        statuses.push((i, st));
    }
    (plans, statuses)
}

/// Whether an output counts as "on air" for Exit, updates, license changes and cloud backup:
/// a display output while its window shows anything; an NDI output only while a receiver is
/// connected (owner decision 2026-10-21).
pub fn status_on_air(s: &OutputStatus) -> bool {
    match &s.ndi {
        Some(live) => s.state == OutputState::Showing && live.receivers > 0,
        None => matches!(
            s.state,
            OutputState::Showing | OutputState::NoProject | OutputState::ScreenMissing
        ),
    }
}

// ---- Runtime ----------------------------------------------------------------------------------

/// Creates, moves, navigates and closes the output windows (Tauri, `src/outputs.rs`). Called on
/// the blocking pool, one call at a time.
pub trait OutputHost: Send + Sync {
    /// Makes the output windows exactly `windows` (labels `output-*`). Returns the labels that
    /// failed, with the error.
    fn apply(&self, windows: &[WindowPlan]) -> HashMap<String, String>;
    /// Shows the Identify windows for `duration`, then closes them (replacing any shown).
    fn identify(&self, windows: Vec<WindowPlan>, duration: Duration);
    /// Closes every output and Identify window now (exit). Must not wait for a reconcile.
    fn close_all(&self);
}

/// Runs the NDI sources (Windows: `src/ndi/host.rs`). Called on the blocking pool, one call at a
/// time.
pub trait NdiHost: Send + Sync {
    /// Makes the running sources exactly `plans` (a changed plan restarts its source). Returns
    /// the outputs that failed, with the error ([`NDI_MISSING`] when the runtime isn't
    /// installed).
    fn apply(&self, plans: &[NdiPlan]) -> HashMap<String, String>;
    /// Receivers and tally per running source. Never waits for `apply`.
    fn live(&self) -> HashMap<String, NdiLive>;
    /// Stops every source now (exit).
    fn close_all(&self);
}

/// What the last reconcile applied.
#[derive(Default)]
struct Applied {
    windows: Vec<WindowPlan>,
    failures: HashMap<String, String>,
    ndi: Vec<NdiPlan>,
    ndi_failures: HashMap<String, String>,
}

/// Output runtime (one per `AppState`).
#[derive(Default)]
pub struct Outputs {
    host: RwLock<Option<Arc<dyn OutputHost>>>,
    ndi_host: RwLock<Option<Arc<dyn NdiHost>>>,
    /// An NDI live update is queued on the blocking pool.
    ndi_pending: AtomicBool,
    rt: Mutex<Option<tokio::runtime::Handle>>,
    /// A reconcile is queued on the blocking pool.
    pending: AtomicBool,
    /// Requested while no runtime is set (tests): the next `list_outputs` reconciles.
    stale: AtomicBool,
    /// Serializes reconciles.
    run: Mutex<Applied>,
    statuses: Mutex<Vec<OutputStatus>>,
    patterns: Mutex<HashMap<String, TestPattern>>,
    server: Mutex<ServerState>,
    /// The last reconcile had outputs or windows (requests are skipped otherwise).
    active: AtomicBool,
    /// The host was replaced since the last reconcile.
    new_host: AtomicBool,
    /// Exiting: no more windows.
    closed: AtomicBool,
    reconciles: AtomicU64,
}

impl Outputs {
    fn host(&self) -> Option<Arc<dyn OutputHost>> {
        self.host
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn ndi_host(&self) -> Option<Arc<dyn NdiHost>> {
        self.ndi_host
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Reconciles run so far (tests, diagnostics).
    pub fn reconciles(&self) -> u64 {
        self.reconciles.load(Ordering::SeqCst)
    }

    /// The windows the last reconcile planned.
    pub fn windows(&self) -> Vec<WindowPlan> {
        lock(&self.run).windows.clone()
    }
}

impl AppState {
    /// Wires the screen store to the outputs (called once by `load`).
    pub(crate) fn outputs_init(&self) {
        let weak = self.downgrade();
        self.screens().set_docs_listener(Some(Arc::new(move || {
            if let Some(st) = weak.upgrade() {
                st.outputs_request();
            }
        })));
    }

    /// Sets the window host (the app: Tauri windows) and reconciles.
    pub fn set_output_host(&self, host: Option<Arc<dyn OutputHost>>) {
        *self
            .outputs_rt()
            .host
            .write()
            .unwrap_or_else(PoisonError::into_inner) = host;
        // The new host gets every window, even when the plan is unchanged.
        self.outputs_rt().new_host.store(true, Ordering::SeqCst);
        self.outputs_request();
    }

    /// Sets the NDI host (the app on Windows) and reconciles.
    pub fn set_ndi_host(&self, host: Option<Arc<dyn NdiHost>>) {
        *self
            .outputs_rt()
            .ndi_host
            .write()
            .unwrap_or_else(PoisonError::into_inner) = host;
        self.outputs_rt().new_host.store(true, Ordering::SeqCst);
        self.outputs_request();
    }

    /// An NDI source's receivers or tally changed (called by the host from its sender threads;
    /// never blocks): the statuses are patched on the blocking pool, coalesced, without a whole
    /// reconcile.
    pub fn outputs_ndi_changed(&self) {
        let o = self.outputs_rt();
        let rt = lock(&o.rt).clone();
        let Some(rt) = rt else {
            self.outputs_ndi_refresh();
            return;
        };
        if o.ndi_pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = self.downgrade();
        rt.spawn_blocking(move || {
            if let Some(st) = weak.upgrade() {
                st.outputs_rt().ndi_pending.store(false, Ordering::SeqCst);
                st.outputs_ndi_refresh();
            }
        });
    }

    /// Copies the host's receivers and tally into the statuses of sending NDI outputs; emits
    /// `outputs-changed` and re-checks what waits for "nothing on air" when they changed.
    fn outputs_ndi_refresh(&self) {
        let o = self.outputs_rt();
        let Some(host) = o.ndi_host() else { return };
        let live = host.live();
        let (changed, statuses, on_air_changed) = {
            let mut g = lock(&o.statuses);
            let before = g.iter().any(status_on_air);
            let mut changed = false;
            for s in g.iter_mut() {
                if s.state != OutputState::Showing || s.ndi.is_none() {
                    continue;
                }
                let next = live.get(&s.id).copied().unwrap_or_default();
                if s.ndi != Some(next) {
                    s.ndi = Some(next);
                    changed = true;
                }
            }
            (changed, g.clone(), before != g.iter().any(status_on_air))
        };
        if !changed {
            return;
        }
        match serde_json::to_value(&statuses) {
            Ok(v) => self.events().emit(names::OUTPUTS_CHANGED, v),
            Err(e) => tracing::warn!("outputs-changed not sent: {e}"),
        }
        if on_air_changed {
            self.license_settle();
            self.cloud_outputs_changed();
        }
    }

    /// The NDI Runtime is installed (Preferences found its file): outputs that couldn't start
    /// without it start now, so installing it needs no restart.
    pub fn outputs_ndi_runtime_found(&self) {
        let missing = lock(&self.outputs_rt().statuses)
            .iter()
            .any(|s| s.state == OutputState::NdiMissing);
        if missing {
            self.outputs_request();
        }
    }

    /// The host's windows changed outside a reconcile (an output page crashed too often and the
    /// host gave up on it): the next reconcile hands it every window again, so it can report
    /// the failure.
    pub fn outputs_host_changed(&self) {
        self.outputs_rt().new_host.store(true, Ordering::SeqCst);
        self.outputs_request();
    }

    /// The runtime reconciles run on (Tauri's). Without one, requests only mark the statuses
    /// stale (tests call [`AppState::outputs_reconcile_now`]).
    pub fn set_outputs_runtime(&self, rt: tokio::runtime::Handle) {
        *lock(&self.outputs_rt().rt) = Some(rt);
    }

    /// The output server's status changed.
    pub fn outputs_set_server(&self, status: &ServerStatus) {
        let s = ServerState::from(status);
        let changed = {
            let mut g = lock(&self.outputs_rt().server);
            let changed = *g != s;
            *g = s;
            changed
        };
        if changed {
            self.outputs_request();
        }
    }

    /// Asks for a reconcile (coalesced, on the blocking pool). Never blocks. Skipped when no
    /// output is configured and none was.
    pub fn outputs_request(&self) {
        let o = self.outputs_rt();
        if o.closed.load(Ordering::SeqCst)
            || !(self.has_outputs() || o.active.load(Ordering::SeqCst))
        {
            return;
        }
        let rt = lock(&o.rt).clone();
        let Some(rt) = rt else {
            o.stale.store(true, Ordering::SeqCst);
            return;
        };
        if o.pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = self.downgrade();
        rt.spawn_blocking(move || {
            if let Some(st) = weak.upgrade() {
                st.outputs_rt().pending.store(false, Ordering::SeqCst);
                st.outputs_reconcile_now();
            }
        });
    }

    /// Plans and applies now, on this thread (see the module docs).
    pub fn outputs_reconcile_now(&self) {
        let o = self.outputs_rt();
        o.stale.store(false, Ordering::SeqCst);
        let mut configs = self.prefs().outputs;
        if o.closed.load(Ordering::SeqCst) {
            configs.clear();
        }
        let patterns = {
            let mut p = lock(&o.patterns);
            p.retain(|id, _| configs.iter().any(|c| c.id == *id));
            p.clone()
        };
        // Only display outputs need the display watcher.
        let needed = configs
            .iter()
            .any(|c| c.enabled && c.kind == OutputKind::Display)
            || !patterns.is_empty();
        // Starts or stops the display watcher outside every outputs lock (stopping joins it).
        self.displays_set_needed(needed);

        let mut run = lock(&o.run);
        o.reconciles.fetch_add(1, Ordering::SeqCst);
        let displays = if needed {
            self.displays_current()
        } else {
            self.displays_cached()
        };
        let server = lock(&o.server).clone();
        let screens = self.screens().sizes();
        let project = self.project_dir().ok().map(|_| PlanProject {
            server: &server,
            screens: &screens,
        });
        let (windows, mut statuses) = plan(&configs, &displays, project, &patterns);
        let (ndi_plans, ndi_statuses) = plan_ndi(&configs, project);
        for (i, s) in ndi_statuses {
            statuses[i] = s;
        }
        let new_host = o.new_host.swap(false, Ordering::SeqCst);
        if (new_host || ndi_plans != run.ndi || !run.ndi_failures.is_empty())
            && !o.closed.load(Ordering::SeqCst)
        {
            run.ndi_failures = match o.ndi_host() {
                Some(h) if !(ndi_plans.is_empty() && run.ndi.is_empty() && !new_host) => {
                    h.apply(&ndi_plans)
                }
                Some(_) => HashMap::new(),
                None => ndi_plans
                    .iter()
                    .map(|p| (p.id.clone(), NDI_UNAVAILABLE.to_string()))
                    .collect(),
            };
            for (id, e) in &run.ndi_failures {
                if e != NDI_MISSING {
                    tracing::warn!("NDI output {id}: {e}");
                }
            }
            run.ndi = ndi_plans;
        }
        let live = o.ndi_host().map(|h| h.live()).unwrap_or_default();
        for s in statuses
            .iter_mut()
            .filter(|s| s.ndi.is_some() && s.state == OutputState::Showing)
        {
            if let Some(e) = run.ndi_failures.get(&s.id) {
                if e == NDI_MISSING {
                    s.state = OutputState::NdiMissing;
                } else {
                    s.state = OutputState::Failed;
                    s.message = Some(e.clone());
                }
            } else if let Some(l) = live.get(&s.id) {
                s.ndi = Some(*l);
            }
        }
        if ((new_host && !windows.is_empty()) || windows != run.windows || !run.failures.is_empty())
            && !o.closed.load(Ordering::SeqCst)
        {
            run.failures = match o.host() {
                Some(h) => h.apply(&windows),
                None => HashMap::new(),
            };
            for (label, e) in &run.failures {
                tracing::warn!("output window {label}: {e}");
            }
            run.windows = windows;
        }
        for w in &run.windows {
            if let Some(e) = run.failures.get(&w.label) {
                let message = if e == PAGE_CRASHING {
                    PAGE_CRASHING
                } else {
                    WINDOW_FAILED
                };
                for s in statuses.iter_mut().filter(|s| w.outputs.contains(&s.id)) {
                    s.state = OutputState::Failed;
                    s.message = Some(message.into());
                }
            }
        }
        o.active.store(
            !configs.is_empty() || !run.windows.is_empty(),
            Ordering::SeqCst,
        );
        // Still serialized by `run`: a slower, older reconcile never overwrites a newer status.
        let changed = {
            let mut g = lock(&o.statuses);
            let changed = *g != statuses;
            *g = statuses.clone();
            changed
        };
        if changed {
            match serde_json::to_value(&statuses) {
                Ok(v) => self.events().emit(names::OUTPUTS_CHANGED, v),
                Err(e) => tracing::warn!("outputs-changed not sent: {e}"),
            }
        }
        drop(run);
        if changed {
            // An output going dark may end the show: a pending license change can apply now.
            self.license_settle();
            self.cloud_outputs_changed();
        }
    }

    /// Every configured output's status.
    pub fn list_outputs(&self) -> Vec<OutputStatus> {
        let o = self.outputs_rt();
        let stale = o.stale.load(Ordering::SeqCst) || {
            let st = lock(&o.statuses);
            let configs = self.prefs().outputs;
            st.len() != configs.len() || st.iter().zip(&configs).any(|(s, c)| s.id != c.id)
        };
        if stale {
            self.outputs_reconcile_now();
        }
        lock(&o.statuses).clone()
    }

    /// Shows a test pattern on an output and its pair (`None`: the screen again).
    pub fn output_test_pattern(&self, id: &str, pattern: Option<TestPattern>) -> AppResult<()> {
        match self.prefs().outputs.iter().find(|c| c.id == id) {
            None => return Err(AppError::NotFound(format!("output {id}"))),
            Some(c) if c.kind == OutputKind::Ndi => {
                return Err(AppError::Invalid(
                    "Test patterns are for display outputs".into(),
                ))
            }
            Some(_) => {}
        }
        {
            let mut p = lock(&self.outputs_rt().patterns);
            match pattern {
                Some(pat) => p.insert(id.to_string(), pat),
                None => p.remove(id),
            };
        }
        self.outputs_rt().active.store(true, Ordering::SeqCst);
        self.outputs_request();
        Ok(())
    }

    /// Shows each display's name full-screen for [`IDENTIFY_FOR`].
    pub fn identify_displays(&self) -> AppResult<()> {
        let o = self.outputs_rt();
        if o.closed.load(Ordering::SeqCst) {
            return Ok(());
        }
        let windows = identify_plan(&self.list_displays());
        if let Some(h) = o.host() {
            h.identify(windows, IDENTIFY_FOR);
        }
        Ok(())
    }

    /// Turns every output off and ends every test pattern.
    pub fn all_outputs_off(&self) {
        lock(&self.outputs_rt().patterns).clear();
        let mut prefs = self.prefs();
        prefs.outputs.iter_mut().for_each(|c| c.enabled = false);
        self.set_prefs(prefs);
        self.outputs_request();
    }

    /// Whether any output window shows something (a screen, black without a project, a
    /// pattern): exit asks first, updates wait.
    pub fn any_output_showing(&self) -> bool {
        lock(&self.outputs_rt().statuses).iter().any(status_on_air)
    }

    /// Closes every output window and stops the display watcher (exit).
    pub fn outputs_shutdown(&self) {
        let o = self.outputs_rt();
        o.closed.store(true, Ordering::SeqCst);
        if let Some(h) = o.host() {
            h.close_all();
        }
        if let Some(h) = o.ndi_host() {
            h.close_all();
        }
        self.displays_shutdown();
    }
}

#[cfg(test)]
mod tests;
