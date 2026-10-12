// Pure helpers for media files and media layers (no Svelte, no I/O): kinds by extension, time
// text, the playback position (the renderers' rule, for Control's elapsed / remaining), readiness
// text, issue summaries and new media layers. Small: the lazy media chunks import it.

import type {
  AssetInfo,
  MediaEndAction,
  MediaInfo,
  MediaIssue,
  MediaKind,
  MediaLayerConfig,
  MediaPlatform,
  MediaPlayback,
  MediaReadiness,
  MediaScreenLayer,
  Rect,
} from "$lib/types";

/** File extensions the library imports, by kind (the file dialog's filters). */
export const MEDIA_EXTENSIONS: Readonly<Record<MediaKind, readonly string[]>> = {
  video: ["mp4", "m4v", "webm", "mov"],
  audio: ["mp3", "wav", "aac", "m4a"],
  image: ["png", "jpg", "jpeg", "webp", "gif", "svg"],
};

/** The kind of a file name or `/assets/<name>` value by its extension (null when unknown). */
export function kindOfName(name: string): MediaKind | null {
  const ext = name.slice(name.lastIndexOf(".") + 1).toLowerCase();
  for (const k of ["video", "audio", "image"] as const) if (MEDIA_EXTENSIONS[k].includes(ext)) return k;
  return null;
}

/** An asset's kind: the probe's when known, else by extension. */
export const kindOf = (a: Pick<AssetInfo, "name" | "media">): MediaKind | null => a.media?.kind ?? kindOfName(a.name);

/**
 * Time as operators read it: "0:05", "12:34", "1:02:03"; `precise` adds hundredths ("0:05.20").
 * Negative values count as 0.
 */
export function formatTime(ms: number, precise = false): string {
  const t = Math.max(0, ms);
  const whole = precise ? Math.floor(t / 1000) : Math.floor(t / 1000 + 1e-9);
  const h = Math.floor(whole / 3600);
  const m = Math.floor((whole % 3600) / 60);
  const s = whole % 60;
  const base = h > 0 ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}` : `${m}:${String(s).padStart(2, "0")}`;
  if (!precise) return base;
  return `${base}.${String(Math.floor((t % 1000) / 10)).padStart(2, "0")}`;
}

/**
 * A typed time in ms: "1:23.5", "83.5", "1:02:03", "0:05.20". Null when it can't be read.
 */
export function parseTime(text: string): number | null {
  const s = text.trim().replace(",", ".");
  if (!/^\d+(:\d{1,2}){0,2}(\.\d+)?$/.test(s)) return null;
  const parts = s.split(":");
  const sec = Number(parts.pop());
  let total = sec;
  if (parts.length > 0 && sec >= 60) return null;
  let mult = 60;
  for (let i = parts.length - 1; i >= 0; i--) {
    const n = Number(parts[i]);
    if (i > 0 && n >= 60) return null;
    total += n * mult;
    mult *= 60;
  }
  return Math.round(total * 1000);
}

/** The play range's end: the out point, else the file's end (null while unknown). */
export const outPoint = (m: Pick<MediaLayerConfig, "outMs">, durationMs: number | null): number | null =>
  m.outMs ?? durationMs;

/**
 * The media position at `now` (server ms) by the renderers' rule: `posMs + (now - at)` while
 * playing (never before `at`), else `posMs`; then the end action at the out point: loop wraps into
 * the range, hold and clear stop at the out point (`ended`).
 */
export function positionAt(
  playback: MediaPlayback,
  now: number,
  m: Pick<MediaLayerConfig, "inMs" | "outMs" | "endAction">,
  durationMs: number | null,
): { pos: number; ended: boolean } {
  let pos = playback.posMs;
  if (playback.state === "playing") pos += Math.max(0, now - playback.at);
  const end = outPoint(m, durationMs);
  if (end === null || end <= m.inMs) return { pos: Math.max(m.inMs, pos), ended: false };
  if (pos < end) return { pos: Math.max(m.inMs, pos), ended: false };
  if (m.endAction === "loop" && playback.state === "playing") {
    const len = end - m.inMs;
    return { pos: m.inMs + ((pos - m.inMs) % len), ended: false };
  }
  return { pos: end, ended: true };
}

/** Elapsed (from the in point) and remaining (to the out point; null while the end is unknown). */
export function elapsedRemaining(
  playback: MediaPlayback,
  now: number,
  m: Pick<MediaLayerConfig, "inMs" | "outMs" | "endAction">,
  durationMs: number | null,
): { elapsed: number; remaining: number | null } {
  const { pos } = positionAt(playback, now, m, durationMs);
  const end = outPoint(m, durationMs);
  // Remaining counts down like a countdown: whole seconds rounded up ("0:01" until the end).
  return { elapsed: pos - m.inMs, remaining: end === null ? null : Math.ceil(Math.max(0, end - pos) / 1000) * 1000 };
}

/** Control's readiness chip: null when every output has it (or there are none). */
export function readinessText(r: MediaReadiness | null | undefined): { text: string; error: boolean } | null {
  if (!r) return null;
  if (r.error) return { text: r.total > 0 ? `Can't load on ${r.total - r.ready} of ${r.total}` : "Can't load", error: true };
  if (r.total === 0 || r.ready >= r.total) return null;
  return { text: `Loading ${r.ready}/${r.total}`, error: false };
}

/** The Take question for a layer that isn't loaded everywhere (null: take at once). */
export function notReadyQuestion(r: MediaReadiness | null | undefined): string | null {
  if (!r || r.total === 0 || r.ready >= r.total) return null;
  const n = r.total - r.ready;
  return `Not loaded on ${n} output${n === 1 ? "" : "s"}. Take anyway?`;
}

export const PLATFORM_LABEL: Readonly<Record<MediaPlatform, string>> = {
  windows: "Windows",
  macos: "Mac",
  obs: "OBS",
  vmix: "vMix",
};

/** The worst severity among the issues, or null. */
export function worstIssue(issues: readonly MediaIssue[] | undefined): "error" | "warning" | null {
  if (!issues || issues.length === 0) return null;
  return issues.some((i) => i.severity === "error") ? "error" : "warning";
}

/** Errors first, then warnings (stable). */
export const sortIssues = (issues: readonly MediaIssue[]): MediaIssue[] =>
  [...issues].sort((a, b) => Number(a.severity !== "error") - Number(b.severity !== "error"));

/** "1920×1080" or null. */
export const resolutionText = (m: MediaInfo | undefined): string | null =>
  m?.width && m.height ? `${m.width}×${m.height}` : null;

/** "29.97 fps" / "25 fps" or null. */
export const fpsText = (m: MediaInfo | undefined): string | null =>
  m?.fps ? `${Math.round(m.fps * 100) / 100} fps` : null;

/** Video and audio codecs, "h264 High 4.1 · aac", or null. */
export const codecText = (m: MediaInfo | undefined): string | null =>
  [m?.videoCodec, m?.audioCodec].filter(Boolean).join(" · ") || null;

/** The asset's `/assets/<name>` value. */
export const assetValue = (name: string) => `/assets/${encodeURIComponent(name)}`;

/** The layer's default name for a file: the name without its extension. */
export function mediaLayerName(fileName: string): string {
  const dot = fileName.lastIndexOf(".");
  return (dot > 0 ? fileName.slice(0, dot) : fileName).slice(0, 40) || "Media";
}

export const DEFAULT_MEDIA: Omit<MediaLayerConfig, "asset"> = {
  fit: "fit",
  inMs: 0,
  outMs: null,
  volume: 100,
  muted: false,
  endAction: "hold",
  autoplay: true,
};

/**
 * The box a new media layer gets: a picture smaller than the screen at its own size, centered;
 * larger ones (and sound, which has no picture) fill the screen.
 */
export function mediaRect(info: MediaInfo | undefined, screen: { width: number; height: number }): Rect {
  const W = screen.width;
  const H = screen.height;
  const w = info?.kind !== "audio" ? info?.width : null;
  const h = info?.kind !== "audio" ? info?.height : null;
  if (!w || !h || w >= W || h >= H) return { x: 0, y: 0, w: W, h: H };
  const rw = Math.max(16, Math.round(w));
  const rh = Math.max(16, Math.round(h));
  return { x: Math.floor((W - rw) / 2), y: Math.floor((H - rh) / 2), w: rw, h: rh };
}

/** A new media layer for an asset (`id` from the editor's id generator). */
export function newMediaLayer(
  asset: Pick<AssetInfo, "name" | "url" | "media">,
  screen: { width: number; height: number },
  id: string,
): MediaScreenLayer {
  return {
    id,
    name: mediaLayerName(asset.name),
    kind: "media",
    rect: mediaRect(asset.media, screen),
    media: { asset: asset.url, ...DEFAULT_MEDIA, endAction: (asset.media?.kind === "image" ? "hold" : DEFAULT_MEDIA.endAction) as MediaEndAction },
  };
}

/** Clamps a play range to the file: 0 ≤ in < out ≤ duration (out null = the end). */
export function clampRange(
  inMs: number,
  outMs: number | null,
  durationMs: number | null,
): { inMs: number; outMs: number | null } {
  const end = durationMs ?? Number.POSITIVE_INFINITY;
  let i = Math.max(0, Math.round(inMs));
  let o = outMs === null ? null : Math.round(outMs);
  if (Number.isFinite(end)) i = Math.min(i, Math.max(0, end - 1));
  if (o !== null) {
    if (Number.isFinite(end) && o >= end) o = null;
    else if (o <= i) o = i + 1;
  }
  return { inMs: i, outMs: o };
}
