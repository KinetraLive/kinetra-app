// WebSocket protocol v1. Mirrors src-tauri/src/core/protocol.rs; change both together.
//
// The server only talks; the renderer never sends anything in v1. Connect to /ws?screen=<id>;
// expect `hello`, then `snapshot`, then diffs.
//
// Buses: the page /screen/<id> is Program and connects to /ws?screen=<id>. The page
// /screen/<id>/preview (Studio mode's Preview) connects to /ws?screen=<id>&bus=preview and gets
// the same message types for Program with the Preview diff applied; it shows every layer settled
// (no in/out animation, no swap). Any other `bus` value, or none, is Program.
//
// Studio messages (take, prepare, tbar, commit, abort) are handled by a lazily loaded renderer
// chunk; the renderer queues every message while it loads, so order is kept.

export const PROTOCOL_VERSION = 1;

/** Field values by key. Always plain strings; everything except `html` is inserted as text. */
export type Fields = Record<string, string>;

export type FieldType =
  | "text"
  | "multiline"
  | "image"
  | "video"
  | "color"
  | "number"
  | "html"
  /** Value: JSON array of row objects; rendered by cloning `data-each="<key>"` per row. */
  | "list"
  /** Time of day from the renderer clock; the field def's format comes as the value. */
  | "clock"
  /** Value: TimerState JSON ({run, at, base, dir, end}); display computed by the renderer. */
  | "timer";

export interface ScreenInfo {
  id: string;
  name: string;
  width: number;
  height: number;
  /** "transparent" or a CSS color. */
  background: string;
}

export type LayerState = "in" | "out";

/** A layer's box in screen pixels; the template lays out inside it (`reflow`) or is scaled
 *  uniformly into it from its design size (`fit`, see WireLayer.sizing). */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/*
 * Template markup the renderer understands beyond {{key}} / data-each (style library):
 * - `data-fit` (on the bound element or any ancestor of the binding, rows included): before the
 *   graphic is shown, on every change of a value bound inside it, on a rect change and when vars
 *   change, its inline font size is set so that it has no horizontal overflow and its height is
 *   at most `data-fit-lines` (default 1) × line-height (+ half a line), never below
 *   `data-fit-min` (default 0.6) × its CSS font size; fitting text keeps the CSS size (no inline
 *   style). Single-line fits need `white-space:nowrap` (otherwise text wraps and the lines limit
 *   applies). Measured with the root in its pre-"in" state: don't hide a fitted element with
 *   `display:none` before "in". Same task as the text change: no visible reflow.
 * - `data-empty` (boolean attribute): set on the element whose text holds a binding, or that has a
 *   binding in a URL attribute (src, href, srcset, poster), when every key it shows is empty
 *   after trimming; removed when one is not. Other attributes (class, style) don't count. Inside
 *   rows too (`{{.col}}`); the parent of a `data-each` element gets it while there are no rows.
 *   CSS: `[data-empty]{display:none}`.
 * - Fonts: before play-in (and before a crossfade swap starts) the renderer loads every curated
 *   family named in the CSS or the vars × every weight the CSS names (`font-weight`/`font`
 *   numbers, `bold`) plus 400 (document.fonts.load, capped 1.5 s); a fallback font never
 *   flashes on air. Fonts already loaded: no delay.
 * - `data-count` / `--n`: the parent element of a `data-each` block's rows gets
 *   `data-count="<n>"` and the custom property `--n: <n>`, n = the rows on screen (rows animating
 *   out not counted), written only when n changes (balanced grids: `[data-count="7"]`, `calc()`).
 * - Row marks used by library templates: `_winner` (poll, server-set when closed), `_leader`
 *   (poll source: the leading answer while votes come in), `_highlight` (chat), next to
 *   `_status`, `_current`, `_duration`.
 */
/** A media layer's file and playback (`WireLayer.media`). The layer's html/css are empty. */
export interface WireMedia {
  /** `/assets/<name>`. */
  url: string;
  kind: "video" | "audio" | "image";
  fit: "fill" | "fit" | "stretch";
  inMs: number;
  /** Absent = the end of the file. */
  outMs?: number;
  durationMs?: number;
  /** 0–100. */
  volume: number;
  muted: boolean;
  endAction: "hold" | "clear" | "loop";
  playback: WirePlayback;
}

/** Position at server time t: `posMs + (t - at)` while playing, else `posMs` (then out point /
 * end action). `at` may be ~120 ms in the future (scheduled start). */
export interface WirePlayback {
  state: "playing" | "paused" | "stopped";
  at: number;
  posMs: number;
}

export interface WireLayer {
  id: string;
  z: number;
  /**
   * Bumped whenever html/css/field types change. Recompile only on a new rev; a reconnect
   * snapshot with an unchanged rev must not replay an on-air layer's animation.
   */
  templateRev: number;
  html: string;
  css: string;
  fields: Fields;
  fieldTypes: Record<string, FieldType>;
  state: LayerState;
  /**
   * Template variables as CSS values, applied as `--<key>` custom properties on the layer host.
   * Validated server-side. Absent from older servers.
   */
  vars?: Record<string, string>;
  /** The layer's box. Absent = the full screen. */
  rect?: Rect;
  /**
   * Content sizing (absent = `reflow`). `fit` with `design`: the template's root renders in an
   * inner box of design.w × design.h, scaled uniformly (transform: scale, centered) to fit the
   * host's box; `fit` without `design` uses the screen's size. `reflow`: the content fills the
   * host. A change arrives as a same-rev `layer` message (in place, no recompile, no replay);
   * a `rect` message re-fits. The scale is computed from the numbers (no layout reads).
   */
  sizing?: "fit" | "reflow";
  /** The template's design size, for `fit`. Absent = none. */
  design?: { w: number; h: number };
  /**
   * Layer opacity in percent, 0–99 (absent = 100, never sent). Applied to the inner sizing box,
   * never to the host: crossfades and studio takes / T-bars animate the host's opacity, so the two
   * multiply instead of fighting. A change is a same-rev `layer` message (in place, no replay).
   */
  opacity?: number;
  /** Media layers only: the file and playback (html/css are empty, fields {}). */
  media?: WireMedia;
  /**
   * Only on a live `layer` message replacing an ON-AIR layer's template/variant (new templateRev):
   * how to change looks. `crossfade`: old and new overlap and cross over in opacity (~300 ms);
   * `outIn`: the old plays its out-animation, then the new plays in. Absent (and always in
   * snapshots): no swap animation.
   */
  swap?: "crossfade" | "outIn";
  /**
   * The template's auto-out (ms), when it has one: set as `--auto-out-ms` (e.g. "5000ms") on the
   * layer host, absent otherwise (a countdown bar in CSS). The server does the auto-out itself.
   */
  autoOutMs?: number;
}

export type WireErrorCode = "unknownScreen";

export type ServerMsg =
  /** `now`: server epoch ms, to correct clock/timer display for skew (absent from older servers). */
  | { t: "hello"; protocol: number; now?: number }
  | { t: "snapshot"; screen: ScreenInfo; layers: WireLayer[] }
  | { t: "layer"; layer: WireLayer }
  | { t: "state"; layerId: string; state: LayerState }
  /** Only the keys that changed. */
  | { t: "fields"; layerId: string; fields: Fields }
  /** Only the variables that changed; patch custom properties, never recompile. */
  | { t: "vars"; layerId: string; vars: Record<string, string> }
  /** Box moved/resized: update the host's position and size only (no recompile, no replay). */
  | { t: "rect"; layerId: string; rect: Rect }
  | { t: "remove"; layerId: string }
  /** Every layer off air (animated out). */
  | { t: "clear" }
  /** PANIC: every layer off air at once, no out-animation; layers stay compiled. */
  | { t: "cut" }
  /**
   * Studio take (Program bus): apply these layer changes together with `transition`:
   * `cut` = instantly (settled, no animation), `crossfade` = host opacity over `ms`, `outIn` =
   * normal out/in animations. `layers` are full WireLayers (new or changed content), `hide` the
   * layer ids going off air.
   */
  | { t: "take"; transition: { kind: "cut" | "crossfade" | "outIn"; ms: number }; layers: WireLayer[]; hide: string[] }
  /** T-bar: mount incoming/changed hosts at opacity 0, settled (outgoing stay at 1). */
  | { t: "prepare"; layers: WireLayer[]; hide: string[] }
  /** T-bar position 0..1: outgoing opacity 1-p, incoming p. */
  | { t: "tbar"; p: number }
  /** T-bar released at the end (= take done) or back at the start (abort: Program unchanged). */
  | { t: "commit" }
  | { t: "abort" }
  | { t: "error"; code: WireErrorCode }
  /** A media layer's playback changed (play, pause, stop, seek, loop restart by the core). */
  | { t: "media"; layerId: string; playback: WirePlayback };

/**
 * Renderer → server. The only messages a renderer sends; status only, never control. Sent on the
 * same WebSocket. `ready`: a layer's media (a media layer's file, or a template's video/image
 * fields) is loaded and its first frame at the in point decoded (`ok`), or failed (`error`, short
 * text). Sent once per load (again after a new file / reconnect). `gone`: a layer's out-animation
 * finished and it left the page (Program; a cut or a re-take sends none).
 */
export type ClientMessage = { t: "ready"; layerId: string; ok: boolean; error?: string } | { t: "gone"; layerId: string };
