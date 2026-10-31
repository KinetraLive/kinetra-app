// Media playback: a lazy chunk, loaded by App on the first media layer or video/image field
// (`MEDIA` in app.ts). From then on every layer of the page is a MediaLayer: plain template layers
// pay one regex test per build.
//
// - Media layers (`WireLayer.media`): a <video>/<audio>/<img> in the inner box (object-fit per
//   `fit`), fading in/out over 200 ms on its root. The position comes from the server clock
//   (`playback`: `posMs + (now - at)` while playing), so every renderer of a screen shows the
//   same frame; a play `at` in the future waits for it. While playing, drift is checked on
//   requestVideoFrameCallback (audio: `timeupdate`), at most 4×/s: within DEAD_MS nothing,
//   up to SEEK_MS `playbackRate` ±≤ 2 % on muted elements only (rate changes stretch audible
//   sound), beyond it a seek, at most one per SEEK_GAP_MS. One timer only while playing toward an out
//   point (hold pauses there, loop wraps; the core clears for `clear`) or a scheduled start.
//   Paused, held, stopped or off air: nothing scheduled.
// - Preload: off air the element is loaded (`preload="auto"`, ≤ PRELOAD_CAP videos per page,
//   more get `metadata`) and seeked to the in point; once that frame is decoded (or the image
//   decoded) the renderer sends `{t: "ready"}` (again on a new socket: after a reconnect).
// - Template video/image fields: every `video[src]` / `img[src]` of the layer is loaded and
//   decoded off air, then `ready`. Videos keep their markup's muted/loop; `autoplay` ones play
//   while the layer is on air and rewind off air.
// - Audio: `?mute=1` (app previews), `?audio=0` and key outputs are always muted; `?output` with
//   `sink=<label>` plays to that output device (`setSinkId`; missing → the default). A blocked
//   unmuted play (autoplay policy) retries muted.
import { now } from "./clock";
import { live } from "./connection";
import { Layer, type Swap } from "./layer";
import type { ClientMessage, WireLayer, WireMedia, WirePlayback } from "./protocol";

const q = new URLSearchParams(location.search);
const silent = q.get("mute") == "1" || q.get("audio") == "0" || q.get("mode") == "key";
const sink = q.has("output") && q.get("sink");

export const PRELOAD_CAP = 4;
/** Drift (ms) beyond which a playing element seeks. A seek interrupts sound and stalls the
 *  picture, so it is the last resort: far enough out that frame timing (33 ms at 30 fps) and
 *  short decode hiccups never trigger it. */
export const SEEK_MS = 250;
/** Shortest time between two corrective seeks of one element (a busy machine must not seek
 *  over and over while it catches up). */
export const SEEK_GAP_MS = 3000;
/** Drift (ms) left alone: about 1.5 frames at 30 fps (the measurement itself is frame-quantized). */
export const DEAD_MS = 45;
/** Videos on this page loading with `preload="auto"`. */
let full = 0;

const FIT: Record<string, string> = { fill: "cover", fit: "contain", stretch: "fill" };
const CSS =
  ".m{position:absolute;inset:0}.m>*{display:block;width:100%;height:100%}[data-state=in]{animation:k .2s}[data-state=out]{animation:k .2s reverse forwards}@keyframes k{from{opacity:0}}";
const dbg = (...a: unknown[]) => console.debug("renderer: media", ...a);

let sid: Promise<string> | undefined;
/** The output device's id for `sink` (a label), "" when not found (the default plays). */
const sinkId = () =>
  (sid ||= navigator.mediaDevices
    .enumerateDevices()
    .then((ds) => ds.find((d) => d.kind == "audiooutput" && d.label == sink)?.deviceId || (dbg("no device", sink), ""))
    .catch(() => ""));

/**
 * The media position (ms) at server time `t` for a file `d` ms long (Infinity: unknown): the out
 * point (`outMs`, else the end) applied per end action (loop wraps into in..out, otherwise held
 * there); stopped = the in point.
 */
export function position(m: WireMedia, p: WirePlayback, t: number, d: number): number {
  const a = m.inMs;
  const b = Math.min(m.outMs ?? d, d);
  const x = p.state == "stopped" ? a : p.posMs + (p.state == "playing" ? Math.max(0, t - p.at) : 0);
  return m.endAction == "loop" && b > a && b < 1 / 0 && x >= b ? a + ((x - a) % (b - a)) : Math.min(x, b);
}

/** Playback rate for a drift of `d` ms (element ahead > 0): 0 = seek beyond SEEK_MS; 1 within
 *  DEAD_MS. In between a muted element is nudged ±≤ 2 % (proportional); an audible one keeps
 *  rate 1, because rate changes stretch the sound (audible warble and dropouts). */
export const rate = (d: number, audible = false) =>
  Math.abs(d) > SEEK_MS ? 0 : audible || Math.abs(d) < DEAD_MS ? 1 : 1 - Math.max(-0.02, Math.min(0.02, d / 1000));

type El = HTMLVideoElement | HTMLAudioElement | HTMLImageElement;

export class MediaLayer extends Layer {
  // `declare`: no field initializers (Layer's constructor calls build() before they would run).
  declare el: El | undefined;
  declare m: WireMedia | undefined;
  declare pb: WirePlayback | undefined;
  /** Out point / scheduled start timer (only while playing). */
  declare t: ReturnType<typeof setTimeout> | undefined;
  /** Pending requestVideoFrameCallback; the last drift check (performance.now()). */
  declare f: number;
  declare c: number;
  /** When the last corrective seek happened (performance.now()). */
  declare s: number;
  /** This element counts toward PRELOAD_CAP. */
  declare full: boolean;
  /** The last readiness, and the socket it went out on. */
  declare ok: ClientMessage | undefined;
  declare sent: unknown;
  /** Template media: their srcs as last loaded; autoplay videos. */
  declare k: string | undefined;
  declare tv: HTMLVideoElement[] | undefined;

  protected build(def: WireLayer) {
    this.free();
    super.build(def);
    this.k = this.ok = undefined;
    this.tv = [];
    const m = (this.m = def.media);
    if (!m) return this.scan();
    this.pb = m.playback;
    const el = (this.el = document.createElement(m.kind == "image" ? "img" : m.kind));
    const w = document.createElement("div");
    const st = document.createElement("style");
    st.textContent = CSS;
    w.className = "m";
    w.append(el);
    this.inner.append(st, w);
    this.root = w;
    if (el instanceof HTMLImageElement) {
      el.src = m.url;
      el.decode().then(
        () => this.el == el && this.ready(true),
        () => this.el == el && this.ready(false, "Can't show this image"),
      );
      return this.cfg();
    }
    const v = el;
    const cap = m.kind == "video" && full >= PRELOAD_CAP;
    if (m.kind == "video" && !cap) (this.full = true), full++;
    v.preload = cap ? "metadata" : "auto";
    (v as HTMLVideoElement).playsInline = true;
    if (sink) sinkId().then((id) => {
        if (id && this.el == v) v.setSinkId?.(id).catch(dbg);
      });
    const done = () => !this.ok && v.readyState > 1 && !v.seeking && this.ready(true);
    v.onloadedmetadata = () => {
      if (cap) dbg("preload capped: metadata only", def.id), this.ready(true);
      this.drive();
    };
    v.onseeked = v.onloadeddata = done;
    v.onerror = () => this.ready(false, v.error?.code == 4 ? "Format not supported" : "Can't load the file");
    v.ontimeupdate = () => !v.paused && el instanceof HTMLAudioElement && this.drift(v.currentTime * 1000, now());
    v.onended = () => this.drive();
    v.src = m.url;
    this.cfg();
  }

  apply(def: WireLayer, swap?: Swap) {
    super.apply(def, swap);
    const m = def.media;
    if (m) (this.m = m), (this.pb = m.playback);
    this.cfg();
    this.tell();
  }

  patch(f: Record<string, string>, replace?: boolean) {
    super.patch(f, replace);
    this.scan();
  }

  /** The core's transport (`{t: "media"}`). */
  play(p: WirePlayback) {
    this.pb = p;
    this.drive();
  }

  protected show() {
    super.show();
    this.drive();
  }

  /** Off the DOM: stopped at the in point (the core sends no `media` for clear/cut), preloaded. */
  detach() {
    super.detach();
    const m = this.m;
    if (m) this.pb = { state: "stopped", at: 0, posMs: m.inMs };
    this.drive();
  }

  dispose() {
    super.dispose();
    this.free();
  }

  /** Fit, volume and mute from the config, then the transport. */
  private cfg() {
    const v = this.el;
    const m = this.m;
    if (v && m) {
      v.style.objectFit = FIT[m.fit] || "contain";
      if (!(v instanceof HTMLImageElement)) {
        v.volume = Math.min(1, Math.max(0, m.volume / 100)) || 0;
        // A key (alpha matte) stage never plays sound: its box has the key filter.
        v.muted = silent || m.muted || !!(this.parent as HTMLElement).style?.filter;
      }
    }
    this.drive();
  }

  /** Puts the element where the playback says now; schedules only while playing. */
  private drive() {
    clearTimeout(this.t);
    this.t = undefined;
    const on = !!this.state;
    for (const v of this.tv || []) if (on) v.play().catch(dbg);
    else if (!v.paused) v.pause(), (v.currentTime = 0);
    const v = this.el;
    const m = this.m;
    const p = this.pb;
    if (!m || !p || !v || v instanceof HTMLImageElement) return;
    const n = now();
    const d = m.durationMs ?? (v.duration * 1000 || 1 / 0);
    const b = Math.min(m.outMs ?? d, d);
    v.loop = m.endAction == "loop" && !m.inMs && m.outMs == null;
    const x = on ? position(m, p, n, d) : m.inMs;
    const go = on && p.state == "playing" && (m.endAction == "loop" || x < b);
    if (!go || p.at > n) {
      v.pause();
      this.unwatch();
      this.seek(x, 1);
      if (go) this.t = setTimeout(() => this.drive(), p.at - n);
      return;
    }
    // A start, resume or jump lands within a few frames (one seek, not a correction loop).
    this.seek(x, 80);
    v.play().catch((e) => {
      dbg(e);
      // Autoplay policy: a page without a gesture may only play muted.
      if (!v.muted && this.el == v) (v.muted = true), v.play().catch(dbg);
    });
    if (!v.loop && b < 1 / 0) this.t = setTimeout(() => this.drive(), b - x + 1);
    this.watch();
  }

  private seek(x: number, tol: number) {
    const v = this.el as HTMLMediaElement;
    if (Math.abs(v.currentTime * 1000 - x) > tol) v.currentTime = x / 1000;
  }

  /** Drift checks on presented frames, ≤ 4/s, only while playing. */
  private watch() {
    const v = this.el as HTMLVideoElement;
    if (this.f || !v.requestVideoFrameCallback) return;
    const cb = (t: number, md: VideoFrameCallbackMetadata) => {
      this.f = 0;
      if (v.paused || this.el != v) return;
      if (!(t - this.c < 250)) (this.c = t), this.drift(md.mediaTime * 1000, now() + md.expectedDisplayTime - performance.now());
      this.f = v.requestVideoFrameCallback(cb);
    };
    this.f = v.requestVideoFrameCallback(cb);
  }

  private unwatch() {
    if (this.f) (this.el as HTMLVideoElement).cancelVideoFrameCallback(this.f);
    this.f = 0;
  }

  /** The element shows `y` ms at server time `t`: correct toward the playback's position. */
  private drift(y: number, t: number) {
    const v = this.el as HTMLMediaElement;
    const m = this.m!;
    const dur = m.durationMs ?? (v.duration * 1000 || 1 / 0);
    const x = position(m, this.pb!, t, dur);
    const len = Math.min(m.outMs ?? dur, dur) - m.inMs;
    let e = y - x;
    // Looping: the shortest way around the loop (just wrapped on one side, not yet on the other).
    if (m.endAction == "loop" && len > 0 && len < 1 / 0) e = ((((e + len / 2) % len) + len) % len) - len / 2;
    const r = rate(e, !v.muted && v.volume > 0);
    if (r) {
      if (Math.abs(v.playbackRate - r) > 0.001) v.playbackRate = r;
    } else if (!(performance.now() - this.s < SEEK_GAP_MS)) {
      dbg("seek", Math.round(e));
      this.s = performance.now();
      v.currentTime = x / 1000;
      v.playbackRate = 1;
    }
  }

  /** Template media: loads every `video[src]` / `img[src]` (once per set of srcs), then `ready`. */
  private scan() {
    if (!/"(video|image)"/.test(JSON.stringify(this.def.fieldTypes || 0))) return;
    const els = Array.from(this.inner.querySelectorAll<HTMLVideoElement | HTMLImageElement>("video[src],img[src]")).filter((e) =>
      e.getAttribute("src"),
    );
    const k = els.map((e) => e.getAttribute("src")).join("\n");
    if (k === this.k) return;
    this.k = k;
    this.ok = undefined;
    Promise.all(
      els.map((e) => {
        // A web image that couldn't be downloaded says so (its copy comes through /web-image).
        if (e instanceof HTMLImageElement)
          return e.decode().catch(() => {
            throw e.getAttribute("src")!.startsWith("/web-image") ? "Couldn't download the image" : 0;
          });
        if (e.autoplay) (e.autoplay = false), this.tv!.push(e);
        if (silent) e.muted = true;
        e.preload = "auto";
        return new Promise((res, rej) => {
          if (e.readyState > 1) res(0);
          e.addEventListener("loadeddata", res);
          e.addEventListener("error", rej);
          if (!e.networkState) e.load();
        });
      }),
    ).then(
      () => this.k == k && this.ready(true),
      (why) => this.k == k && this.ready(false, typeof why == "string" ? why : "Can't load the file"),
    );
    this.drive();
  }

  private ready(ok: boolean, error?: string) {
    this.ok = { t: "ready", layerId: this.def.id, ok, ...(error ? { error } : {}) };
    this.sent = undefined;
    this.tell();
  }

  /** Sends the readiness once per load and socket (a reconnect's snapshot sends it again). */
  private tell() {
    const s = live;
    if (this.ok && s && s != this.sent)
      try {
        s.send?.(JSON.stringify(this.ok));
        this.sent = s;
      } catch (e) {
        dbg(e);
      }
  }

  /** Releases the element: no decoder, no network, no timer. */
  private free() {
    const v = this.el;
    clearTimeout(this.t);
    this.el = undefined;
    if (v instanceof HTMLMediaElement) {
      if (this.f) (v as HTMLVideoElement).cancelVideoFrameCallback(this.f);
      this.f = 0;
      v.pause();
      v.removeAttribute("src");
      v.load();
    }
    if (this.full) full--;
    this.full = false;
  }
}
