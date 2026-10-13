//! WebSocket protocol v1 between the server and the renderer (OBS/vMix browser sources and the
//! app's preview iframes). Mirrored in `renderer/src/protocol.ts`; change both together.
//!
//! The server talks; renderers send only status (`ClientMsg::Ready`, media preload), never
//! control (outside clients are read-only).
//! A renderer connects to `/ws?screen=<id>` and receives `hello`, then `snapshot`, then diffs.
//! Messages are serialized once per change and shared by every connected renderer.
//!
//! # Buses (Studio mode)
//! Every screen has two outputs:
//! - **Program** — page `/screen/<id>`, WebSocket `/ws?screen=<id>` (or `&bus=program`): what is
//!   on air. Takes arrive as `take` (or `prepare` / `tbar` / `commit` / `abort` for the T-bar).
//! - **Preview** — page `/screen/<id>/preview` (the same `index.html`), WebSocket
//!   `/ws?screen=<id>&bus=preview`: Program with the Studio diff applied. The renderer picks the
//!   bus from its page path (`/screen/<id>/preview` → `bus=preview`). It receives the same
//!   message types as Program (`snapshot`, `layer`, `state`, `fields`, `vars`, `rect`,
//!   `remove`, `clear`, `error`) and never `take`/`prepare`/`tbar`/`commit`/`abort`/`cut`. Its
//!   layers are shown settled: a preview renderer applies `state in` without the in-animation
//!   (and `state out` may simply hide). Its `templateRev`s are its own (independent of Program's).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

/// Field values by key. Always plain strings; the renderer escapes everything except `html`.
pub type Fields = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldType {
    #[default]
    Text,
    Multiline,
    Image,
    Video,
    Color,
    Number,
    /// Inserted as markup. Every other type is inserted as text.
    Html,
    /// Repeated rows: the value is a JSON array of objects (column key → text), at most
    /// `LIST_ROWS_MAX` rows. Rendered by cloning the template's `data-each="<key>"` element per row;
    /// `{{.col}}` binds a column, `{{#}}` the row number. Cells are inserted as text.
    List,
    /// Time of day, rendered by the renderer from its clock. The value sent to renderers is the
    /// display format (`FieldDef.format`, default `HH:mm:ss`; tokens `HH`, `mm`, `ss`).
    Clock,
    /// Count up or down: the value is a `TimerState` as JSON; the renderer computes the display.
    Timer,
}

/// Most rows a `list` value may hold.
pub const LIST_ROWS_MAX: usize = 100;
/// Most bytes a `list` value (the JSON) may hold.
pub const LIST_VALUE_MAX: usize = 64 * 1024;

/// A `timer` field's value (JSON in the field). Shown time = `base` (+ or −, by `dir`) the time
/// since `at` while `run` is true; `base` alone while paused. Countdowns stop at zero; with
/// `end: hide` the server takes the layer off air at zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimerState {
    pub run: bool,
    /// Server epoch ms when it (re)started; meaningful while `run`.
    #[serde(default)]
    pub at: u64,
    /// Elapsed (up) or remaining (down) ms at `at` or while paused.
    #[serde(default)]
    pub base: u64,
    #[serde(default)]
    pub dir: TimerDir,
    #[serde(default)]
    pub end: TimerEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimerDir {
    #[default]
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimerEnd {
    /// A countdown holds at zero.
    #[default]
    Stop,
    /// A countdown takes its layer off air at zero (server-side).
    Hide,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenInfo {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// `"transparent"` or a CSS color.
    pub background: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayerState {
    In,
    Out,
}

/// A layer's box in screen pixels. The template lays out inside it (`reflow`: it is the template's
/// frame), or renders at its design size scaled uniformly into it (`fit`, see
/// [`WireLayer::sizing`]). `x`/`y` may be negative; the box must intersect the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// How a layer's content fills its box (`ScreenLayer.sizing`, `WireLayer.sizing`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentSizing {
    /// The template renders at its design size and scales uniformly to fit the box, centered.
    Fit,
    /// The template fills the box and lays itself out (what every layer did before sizing).
    #[default]
    Reflow,
}

/// A template's design size in px (`WireLayer.design`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignSize {
    pub w: u32,
    pub h: u32,
}

/// How an on-air layer changes looks when its template variant changes (`WireLayer::swap`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VariantTransition {
    /// The old and the new look overlap and cross over in opacity (~300 ms).
    #[default]
    Crossfade,
    /// The old look plays its out-animation, then the new one plays in.
    OutIn,
}

/// How a Studio take changes Program (`ServerMsg::Take`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TakeTransitionKind {
    /// Incoming layers appear settled, outgoing ones vanish at once (no animation).
    Cut,
    /// Host opacity over `ms`.
    #[default]
    Crossfade,
    /// The layers' normal out / in animations.
    OutIn,
}

/// Crossfade duration limits (ms).
pub const TAKE_MS_RANGE: std::ops::RangeInclusive<u32> = 100..=5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TakeTransition {
    pub kind: TakeTransitionKind,
    /// Crossfade duration (ms, 100..=5000); ignored by `cut` and `outIn`.
    pub ms: u32,
}

impl Default for TakeTransition {
    fn default() -> Self {
        Self {
            kind: TakeTransitionKind::Crossfade,
            ms: 500,
        }
    }
}

impl TakeTransition {
    /// `ms` clamped into [`TAKE_MS_RANGE`].
    pub fn clamped(self) -> Self {
        Self {
            kind: self.kind,
            ms: self.ms.clamp(*TAKE_MS_RANGE.start(), *TAKE_MS_RANGE.end()),
        }
    }
}

/// What a media file is (`WireMedia.kind`, `MediaInfo.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

/// How a media layer's picture fills its box: fill = cover (crop), fit = contain (letterbox,
/// transparent), stretch = fill the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaFit {
    Fill,
    #[default]
    Fit,
    Stretch,
}

/// What a media layer does at its out point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaEndAction {
    /// Hold the last frame (the core pauses at the out point).
    #[default]
    Hold,
    /// Take the layer off air at the out point (the core does it).
    Clear,
    /// Loop to the in point (renderers wrap the position).
    Loop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackState {
    Playing,
    Paused,
    #[default]
    Stopped,
}

/// A media layer's playback (Program). Position at server time `t`: `pos_ms + (t - at)` while
/// playing (never below `pos_ms` before `at`), else `pos_ms`; then clamped to the out point
/// (hold, clear) or wrapped into in..out (loop). `at` may be ~120 ms in the future: a play is
/// scheduled ahead so every renderer starts on the same server instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPlayback {
    pub state: PlaybackState,
    /// Server epoch ms.
    pub at: u64,
    pub pos_ms: u64,
}

/// A media layer's file and playback (`WireLayer.media`); the layer's html/css are empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireMedia {
    /// `/assets/<name>`.
    pub url: String,
    pub kind: MediaKind,
    pub fit: MediaFit,
    pub in_ms: u64,
    /// Absent = the end of the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// 0–100.
    pub volume: u8,
    pub muted: bool,
    pub end_action: MediaEndAction,
    pub playback: MediaPlayback,
}

/// Renderer → server: status only, never control.
/// `ready`: a layer's media is loaded and its first frame at the in point decoded (`ok`), or
/// it failed (`error`, short text). Sent once per load (again after a new file / reconnect).
/// `gone`: a layer's out-animation finished and it left the page (Program only; a cut or a
/// re-take sends none). Remote control's exact `on_air` waits for it (`LiveHub::renderer_gone`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ClientMsg {
    Ready {
        layer_id: String,
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
    Gone {
        layer_id: String,
    },
}

/// Everything a renderer needs to draw one layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireLayer {
    pub id: String,
    pub z: i32,
    /// Bumped whenever `html`/`css`/field types change. The renderer recompiles only on a new rev,
    /// and a reconnect snapshot with an unchanged rev never replays an on-air layer's animation.
    pub template_rev: u32,
    pub html: String,
    pub css: String,
    pub fields: Fields,
    pub field_types: BTreeMap<String, FieldType>,
    pub state: LayerState,
    /// Template variables as CSS values, applied as `--<key>` custom properties on the layer's
    /// host. Always validated server-side (see `core::templates`); never raw user text.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// The layer's box. Absent = the full screen (the test and preview screens, older servers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    /// Content sizing; absent = `reflow` (only `fit` is ever sent). `fit`: the template renders
    /// in an inner box of `design` (or the screen's size without one), scaled uniformly and
    /// centered in the host's box. A change is a same-rev `layer` message (no recompile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sizing: Option<ContentSizing>,
    /// Layer opacity in percent, 0–99 (absent = 100, never sent). The renderer applies it to the
    /// inner sizing box, never to the host (crossfades and T-bars animate the host's opacity). A
    /// change is a same-rev `layer` message (no recompile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<u8>,
    /// The template's design size, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design: Option<DesignSize>,
    /// Only on a live `layer` message replacing an ON-AIR layer's variant (new `templateRev`):
    /// how to change looks. Absent (and always in snapshots): no swap animation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap: Option<VariantTransition>,
    /// The template's auto-out (ms), when it has one: the renderer exposes it to CSS as
    /// `--auto-out-ms` on the layer host (e.g. a countdown bar). The server still does the
    /// auto-out itself. A change without a recompile is sent as a same-rev `layer` message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_out_ms: Option<u32>,
    /// Media layers only: the file and its playback (html/css are empty, fields `{}`; the
    /// `templateRev` is derived from the file name). Config changes are same-rev `layer`
    /// messages; transport is `media`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<WireMedia>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ServerMsg {
    Hello {
        protocol: u32,
        /// Server epoch ms, so renderers on other machines correct clock/timer display for skew.
        #[serde(default)]
        now: u64,
    },
    /// Full state of the screen. Sent on connect, after the client lagged, and when the project
    /// changes. The renderer reconciles it against what it already shows.
    Snapshot {
        screen: ScreenInfo,
        layers: Vec<WireLayer>,
    },
    /// A layer was added or its template changed.
    Layer {
        layer: WireLayer,
    },
    State {
        layer_id: String,
        state: LayerState,
    },
    /// Only the keys that changed.
    Fields {
        layer_id: String,
        fields: Fields,
    },
    /// Only the variables that changed. Patched as custom properties: no recompile, no replayed
    /// animation.
    Vars {
        layer_id: String,
        vars: Fields,
    },
    /// The layer's box moved or resized. Only the host's position and size change: no recompile,
    /// no replayed animation, `data-state` untouched.
    Rect {
        layer_id: String,
        rect: Rect,
    },
    Remove {
        layer_id: String,
    },
    /// Every layer off air (animated out).
    Clear {},
    /// PANIC: every layer off air at once, with no out-animation. Layers stay compiled, so the
    /// next `state in` animates in normally.
    Cut {},
    /// Studio take (Program bus only): apply these changes together with `transition`.
    /// `layers`: full layers whose look changed or that go in (all `state: in`; a same-rev layer
    /// updates in place, a new rev recompiles); `hide`: ids going off air. Content changes of
    /// layers that stay off air are sent as ordinary messages BEFORE the `take`; content changes
    /// of layers going off air follow AFTER it. One `take` per take.
    Take {
        transition: TakeTransition,
        layers: Vec<WireLayer>,
        hide: Vec<String>,
    },
    /// T-bar begins (Program bus only): mount `layers` (incoming and changed hosts) at opacity 0,
    /// settled; `hide` (outgoing) stay at 1. Same shape and ordering rules as `take`.
    Prepare {
        layers: Vec<WireLayer>,
        hide: Vec<String>,
    },
    /// T-bar position 0..=1 (outgoing opacity 1-p, incoming p). At most ~30 per second per
    /// screen; the latest position wins.
    Tbar {
        p: f64,
    },
    /// T-bar released at the end: the prepared take is done (no further animation). If Program
    /// changed underneath meanwhile, a `snapshot` follows.
    Commit {},
    /// T-bar released at the start (or interrupted): Program is unchanged; drop the prepared hosts.
    Abort {},
    Error {
        code: WireErrorCode,
    },
    /// A media layer's playback changed (play, pause, stop, seek, the core's hold at the out
    /// point). Never sent for `state out` / `cut`: renderers stop a media layer once its
    /// out-animation is done (the core's playback is then `stopped`).
    Media {
        layer_id: String,
        playback: MediaPlayback,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireErrorCode {
    UnknownScreen,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_is_camel_case_and_tagged() {
        let msg = ServerMsg::State {
            layer_id: "lower-third".into(),
            state: LayerState::In,
        };
        assert_eq!(
            serde_json::to_string(&msg).unwrap(),
            r#"{"t":"state","layerId":"lower-third","state":"in"}"#
        );
        assert_eq!(
            serde_json::to_string(&ServerMsg::Clear {}).unwrap(),
            r#"{"t":"clear"}"#
        );
        let err = ServerMsg::Error {
            code: WireErrorCode::UnknownScreen,
        };
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"t":"error","code":"unknownScreen"}"#
        );
    }

    #[test]
    fn wire_layer_auto_out_is_optional() {
        let mut l = WireLayer {
            id: "a".into(),
            z: 0,
            template_rev: 1,
            html: String::new(),
            css: String::new(),
            fields: Fields::new(),
            field_types: BTreeMap::new(),
            state: LayerState::Out,
            vars: BTreeMap::new(),
            rect: None,
            sizing: None,
            opacity: None,
            design: None,
            swap: None,
            auto_out_ms: None,
            media: None,
        };
        let j = serde_json::to_string(&l).unwrap();
        assert!(!j.contains("autoOutMs"), "absent when unset: {j}");
        assert!(!j.contains("sizing") && !j.contains("design"), "{j}");
        assert!(!j.contains("opacity"), "{j}");
        l.opacity = Some(50);
        assert!(serde_json::to_string(&l)
            .unwrap()
            .contains(r#""opacity":50"#));
        l.opacity = None;
        l.auto_out_ms = Some(5000);
        l.sizing = Some(ContentSizing::Fit);
        l.design = Some(DesignSize { w: 640, h: 360 });
        let j = serde_json::to_string(&l).unwrap();
        assert!(j.contains(r#""autoOutMs":5000"#), "{j}");
        assert!(j.contains(r#""sizing":"fit""#), "{j}");
        assert!(j.contains(r#""design":{"w":640,"h":360}"#), "{j}");
        let back: WireLayer = serde_json::from_str(&j).unwrap();
        assert_eq!(back, l);
    }
}
