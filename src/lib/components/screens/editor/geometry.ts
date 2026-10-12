// Pure maths of the screen layout editor: fitting the screen into the canvas, screen <-> canvas
// coordinates, move / resize (aspect lock), snapping, nudging and clamping, plus the two timing
// helpers of a drag (rAF coalescing, throttle). No Svelte, no DOM.
//
// Rects are in SCREEN pixels (integers) everywhere except where a name says "canvas".

import type { ContentSizing, Rect } from "$lib/types";

export interface Size {
  w: number;
  h: number;
}

export interface Stage {
  /** Canvas px. */
  w: number;
  h: number;
  /** Canvas px per screen px. */
  scale: number;
}

/** The largest box with the screen's aspect ratio that fits `avail` (canvas px). */
export function fitStage(availW: number, availH: number, screenW: number, screenH: number): Stage {
  if (!(availW > 0) || !(availH > 0) || !(screenW > 0) || !(screenH > 0)) return { w: 0, h: 0, scale: 0 };
  const scale = Math.min(availW / screenW, availH / screenH);
  return { w: screenW * scale, h: screenH * scale, scale };
}

/** Screen rect -> canvas rect. */
export function toCanvas(r: Rect, scale: number): Rect {
  return { x: r.x * scale, y: r.y * scale, w: r.w * scale, h: r.h * scale };
}

/** A canvas-px distance in screen px (0 while the stage has no size). */
export function toScreen(d: number, scale: number): number {
  return scale > 0 ? d / scale : 0;
}

/** Snap distance in screen px for a given distance in CSS px (default ~6 CSS px). */
export function snapThreshold(scale: number, cssPx = 6): number {
  return scale > 0 ? cssPx / scale : 0;
}

export const fullRect = (screen: Size): Rect => ({ x: 0, y: 0, w: screen.w, h: screen.h });

export function sameRect(a: Rect, b: Rect): boolean {
  return a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}

function roundRect(r: Rect): Rect {
  return { x: Math.round(r.x), y: Math.round(r.y), w: Math.max(1, Math.round(r.w)), h: Math.max(1, Math.round(r.h)) };
}

/**
 * Keeps a rect valid for the backend: integers, w/h >= 1 and at least one pixel overlapping the
 * screen on each axis (a box can be dragged mostly off-screen, never entirely).
 */
export function clampRect(r: Rect, screen: Size): Rect {
  const o = roundRect(r);
  o.x = Math.min(Math.max(o.x, 1 - o.w), screen.w - 1);
  o.y = Math.min(Math.max(o.y, 1 - o.h), screen.h - 1);
  return o;
}

// ---- Move / resize --------------------------------------------------------------------------------

/** `start` moved by (dx, dy) screen px (not rounded: snap first, then clamp). */
export function moveRect(start: Rect, dx: number, dy: number): Rect {
  return { x: start.x + dx, y: start.y + dy, w: start.w, h: start.h };
}

/** The box around `rects` (a selection's bounds); null for none. */
export function boundsOf(rects: readonly Rect[]): Rect | null {
  if (rects.length === 0) return null;
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const r of rects) {
    x0 = Math.min(x0, r.x);
    y0 = Math.min(y0, r.y);
    x1 = Math.max(x1, r.x + r.w);
    y1 = Math.max(y1, r.y + r.h);
  }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/**
 * `starts` all moved by the same whole-pixel (dx, dy), the delta limited so every box keeps at
 * least one pixel on screen (a group moves together and never changes shape).
 */
export function moveGroup(starts: readonly Rect[], dx: number, dy: number, screen: Size): Rect[] {
  let minX = -Infinity;
  let maxX = Infinity;
  let minY = -Infinity;
  let maxY = Infinity;
  for (const r of starts) {
    minX = Math.max(minX, 1 - r.w - r.x);
    maxX = Math.min(maxX, screen.w - 1 - r.x);
    minY = Math.max(minY, 1 - r.h - r.y);
    maxY = Math.min(maxY, screen.h - 1 - r.y);
  }
  const ddx = Math.min(Math.max(Math.round(dx), minX), maxX);
  const ddy = Math.min(Math.max(Math.round(dy), minY), maxY);
  return starts.map((r) => ({ x: r.x + ddx, y: r.y + ddy, w: r.w, h: r.h }));
}

export type Handle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
export const HANDLES: readonly Handle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

const movesLeft = (h: Handle) => h.includes("w");
const movesRight = (h: Handle) => h.includes("e");
const movesTop = (h: Handle) => h.startsWith("n");
const movesBottom = (h: Handle) => h.startsWith("s");

/** How a resize behaves: Shift / the layer's lock keep the ratio, Alt resizes from the centre. */
export interface ResizeOptions {
  keepAspect?: boolean;
  /** The opposite edge mirrors the drag (the centre stays put). */
  fromCenter?: boolean;
  /** Smallest width and height (screen px). A box already smaller keeps its own size as its floor. */
  min?: number;
  /** The width / height `keepAspect` keeps (default: the start box's). */
  ratio?: number;
}

// ---- Resizing scales ------------------------------------------------------------------------------
//
// A resize (a handle drag, a typed W / H) of a layer that isn't explicitly Reflow makes it Scale to
// fit: the graphic gets bigger or smaller with its box. Such a layer keeps the graphic's aspect by
// default (Shift stretches freely); an explicit Reflow layer resizes freely (Shift keeps the
// ratio). The layer's aspect lock, once toggled, wins over the default.

/** True when resizing the layer scales its graphic (it is, or becomes, Scale to fit). */
export const scalesOnResize = (sizing: ContentSizing | null | undefined) => sizing !== "reflow";

/** The layer's aspect lock: the toggled value, else on for layers that scale. */
export const aspectLock = (sizing: ContentSizing | null | undefined, toggled: boolean | undefined) =>
  toggled ?? scalesOnResize(sizing);

/** Whether a handle drag keeps the ratio: for a layer that scales, Shift inverts its lock. */
export const dragKeepsAspect = (scales: boolean, lock: boolean, shift: boolean) => (scales ? lock !== shift : lock || shift);

/**
 * The aspect a scaling layer keeps: its template's design box (the graphic), else the layer's own
 * box (no design: the graphic is the frame the user drew).
 */
export function graphicRatio(rect: Rect, design: { w: number; h: number } | null | undefined): number {
  if (design && design.w > 0 && design.h > 0) return design.w / design.h;
  return rect.w > 0 && rect.h > 0 ? rect.w / rect.h : 1;
}

/**
 * One axis of a resize. `lo`: the low edge moves (the high one is fixed); `hi`: the reverse;
 * `mid`: both edges move around the centre; `fixed`: the size doesn't change.
 */
interface Axis {
  kind: "lo" | "hi" | "mid" | "fixed";
  lo: number;
  hi: number;
  c: number;
}

function axisOf(lo: number, size: number, movesLo: boolean, movesHi: boolean, center: boolean, cross: boolean): Axis {
  const kind = movesLo || movesHi ? (center ? "mid" : movesLo ? "lo" : "hi") : cross ? "mid" : "fixed";
  return { kind, lo, hi: lo + size, c: lo + size / 2 };
}

/** Where an axis of `size` starts. */
function startOf(a: Axis, size: number): number {
  return a.kind === "lo" ? a.hi - size : a.kind === "hi" ? a.lo : a.c - size / 2;
}

/** The nearest snap of an axis's moving edges at `size`: the size that puts that edge on the target. */
function snapAxis(
  a: Axis,
  size: number,
  targets: readonly number[],
  threshold: number,
  grid: number | undefined,
): { size: number; at: number; offset: number } | null {
  if (a.kind === "fixed" || threshold <= 0) return null;
  const s0 = startOf(a, size);
  const edges = a.kind === "lo" ? [s0] : a.kind === "hi" ? [s0 + size] : [s0, s0 + size];
  const n = nearest(edges, targets, threshold, grid);
  if (!n) return null;
  const t = n.at;
  return { size: a.kind === "lo" ? a.hi - t : a.kind === "hi" ? t - a.lo : 2 * Math.abs(t - a.c), at: t, offset: n.offset };
}

/**
 * `start` resized by dragging `handle` by (dx, dy) screen px, snapped to `targets` within
 * `threshold` (0 = no snapping).
 * - The opposite edge / corner stays put; with `fromCenter` (Alt) it mirrors the drag instead, so
 *   the centre stays put.
 * - `keepAspect` keeps `ratio` (default start.w / start.h; another one reshapes the box to it on
 *   the first move): corners follow the larger change, edge handles grow the other axis around
 *   the centre. Snapping takes the nearest snap of
 *   either axis's moving edges and derives the other side.
 * - Without the lock only the moving edges snap.
 * - Width and height never go below `min` (or the start size, when that is already smaller); a
 *   snap that would is ignored.
 */
export function resizeSnap(
  start: Rect,
  handle: Handle,
  dx: number,
  dy: number,
  opts: ResizeOptions = {},
  targets: SnapTargets = { x: [], y: [] },
  threshold = 0,
): { rect: Rect; guides: Guide[] } {
  const keep = !!opts.keepAspect && start.w > 0 && start.h > 0;
  const center = !!opts.fromCenter;
  const min = opts.min ?? 1;
  const minW = Math.max(1, Math.min(min, start.w));
  const minH = Math.max(1, Math.min(min, start.h));
  const xMoves = movesLeft(handle) || movesRight(handle);
  const yMoves = movesTop(handle) || movesBottom(handle);
  const ax = axisOf(start.x, start.w, movesLeft(handle), movesRight(handle), center, keep);
  const ay = axisOf(start.y, start.h, movesTop(handle), movesBottom(handle), center, keep);
  // Outward deltas (dragging a low edge towards negative grows the box); from the centre the
  // opposite edge moves too.
  const ox = (movesLeft(handle) ? -dx : dx) * (center ? 2 : 1);
  const oy = (movesTop(handle) ? -dy : dy) * (center ? 2 : 1);
  let w = Math.max(xMoves ? start.w + ox : start.w, minW);
  let h = Math.max(yMoves ? start.h + oy : start.h, minH);
  const guides: Guide[] = [];

  if (!keep) {
    const sx = snapAxis(ax, w, targets.x, threshold, targets.grid);
    if (sx && sx.size >= minW) {
      w = sx.size;
      guides.push({ axis: "x", pos: sx.at });
    }
    const sy = snapAxis(ay, h, targets.y, threshold, targets.grid);
    if (sy && sy.size >= minH) {
      h = sy.size;
      guides.push({ axis: "y", pos: sy.at });
    }
  } else {
    const ratio = opts.ratio && opts.ratio > 0 && Number.isFinite(opts.ratio) ? opts.ratio : start.w / start.h;
    // Corners follow the larger relative change, edge handles their own axis. A corner of a box
    // reshaped to another ratio follows the pointer projected onto that ratio's diagonal.
    const own = start.w / start.h;
    if (xMoves && yMoves && Math.abs(ratio - own) > own * 0.01) {
      h = Math.max((w * ratio + h) / (ratio * ratio + 1), 1);
      w = h * ratio;
    } else if (xMoves && yMoves ? w / start.w >= h / start.h : xMoves) h = w / ratio;
    else w = h * ratio;
    const sx = snapAxis(ax, w, targets.x, threshold, targets.grid);
    const sy = snapAxis(ay, h, targets.y, threshold, targets.grid);
    if (sx && (!sy || Math.abs(sx.offset) <= Math.abs(sy.offset))) {
      w = sx.size;
      h = w / ratio;
      guides.push({ axis: "x", pos: sx.at });
    } else if (sy) {
      h = sy.size;
      w = h * ratio;
      guides.push({ axis: "y", pos: sy.at });
    }
    // Both sides at least the minimum, keeping the ratio (a snap that went below is dropped).
    const grow = Math.max(1, minW / w, minH / h);
    if (grow > 1) {
      w *= grow;
      h *= grow;
      guides.length = 0;
    }
  }
  return { rect: { x: startOf(ax, w), y: startOf(ay, h), w, h }, guides };
}

/** `resizeSnap` without snapping. */
export function resizeRect(
  start: Rect,
  handle: Handle,
  dx: number,
  dy: number,
  keepAspect = false,
  min = 1,
  fromCenter = false,
): Rect {
  return resizeSnap(start, handle, dx, dy, { keepAspect, min, fromCenter }).rect;
}

/**
 * The box with W or H typed in the inspector: at least `min` per side (or the current size when
 * that is already smaller); with the aspect lock the other side follows at `ratio` (default the
 * current one), and both stay at or above the minimum.
 */
export function sizeRect(cur: Rect, patch: Partial<Rect>, locked: boolean, min = 1, ratio?: number): Rect {
  const minW = Math.max(1, Math.min(min, cur.w));
  const minH = Math.max(1, Math.min(min, cur.h));
  const out = { ...cur, ...patch };
  if (patch.w !== undefined) out.w = Math.max(minW, Math.round(patch.w));
  if (patch.h !== undefined) out.h = Math.max(minH, Math.round(patch.h));
  if (locked && cur.w > 0 && cur.h > 0 && (patch.w !== undefined) !== (patch.h !== undefined)) {
    const r = ratio && ratio > 0 && Number.isFinite(ratio) ? ratio : null;
    if (patch.w !== undefined) out.h = Math.round(r ? out.w / r : (out.w * cur.h) / cur.w);
    else out.w = Math.round(r ? out.h * r : (out.h * cur.w) / cur.h);
    if (out.w < minW || out.h < minH) {
      const grow = Math.max(minW / out.w, minH / out.h);
      out.w = Math.round(out.w * grow);
      out.h = Math.round(out.h * grow);
      // Rounding may leave the derived side a pixel short.
      out.w = Math.max(out.w, minW);
      out.h = Math.max(out.h, minH);
    }
  }
  return out;
}

// ---- Safe areas and crops ------------------------------------------------------------------------

/** SMPTE ST 2046-1: action safe is the central 93 % of each side, title safe the central 90 %. */
export const ACTION_SAFE = 0.93;
export const TITLE_SAFE = 0.9;

/** The central `fraction` of the screen on each side (not rounded). */
export function safeRect(screen: Size, fraction: number): Rect {
  const w = screen.w * fraction;
  const h = screen.h * fraction;
  return { x: (screen.w - w) / 2, y: (screen.h - h) / 2, w, h };
}

/** Width / height of each aspect crop overlay. */
export const CROP_RATIOS: Readonly<Record<string, number>> = { "9:16": 9 / 16, "4:5": 4 / 5, "1:1": 1, "4:3": 4 / 3 };

/** The largest centred box with the crop's aspect ratio (null for "none" or an unknown crop). */
export function cropRect(screen: Size, crop: string): Rect | null {
  const ratio = CROP_RATIOS[crop];
  if (!ratio || !(screen.w > 0) || !(screen.h > 0)) return null;
  const w = Math.min(screen.w, screen.h * ratio);
  const h = w / ratio;
  return { x: (screen.w - w) / 2, y: (screen.h - h) / 2, w, h };
}

// ---- Snapping -------------------------------------------------------------------------------------

/** A guide line in screen px: axis "x" is a vertical line at x = pos, "y" a horizontal one. */
export interface Guide {
  axis: "x" | "y";
  pos: number;
}

/** Snap lines per axis, sorted and unique (screen px), plus an optional grid spacing. */
export interface SnapTargets {
  x: number[];
  y: number[];
  /** Grid spacing (screen px): every multiple is a target too. */
  grid?: number;
}

export interface ExtraTargets {
  /** Lines (ruler guides). */
  x?: readonly number[];
  y?: readonly number[];
  /** Boxes whose edges and centre are targets (custom safe areas, the crop overlay). */
  rects?: readonly Rect[];
  grid?: number;
}

const sortedUnique = (v: number[]) => [...new Set(v.map((n) => Math.round(n)))].sort((a, b) => a - b);

/**
 * Where boxes snap: the screen edges and centre, the action / title safe areas (93 % / 90 %), the
 * edges and centres of the other layers, and `extra` (ruler guides, visible custom safe areas and
 * crop, the grid). Computed once per drag; each lookup is a binary search.
 */
export function snapTargets(screen: Size, others: readonly Rect[], extra: ExtraTargets = {}): SnapTargets {
  const boxes = [safeRect(screen, ACTION_SAFE), safeRect(screen, TITLE_SAFE), ...others, ...(extra.rects ?? [])];
  const x = [0, screen.w / 2, screen.w, ...(extra.x ?? [])];
  const y = [0, screen.h / 2, screen.h, ...(extra.y ?? [])];
  for (const r of boxes) {
    x.push(r.x, r.x + r.w / 2, r.x + r.w);
    y.push(r.y, r.y + r.h / 2, r.y + r.h);
  }
  const out: SnapTargets = { x: sortedUnique(x), y: sortedUnique(y) };
  if (extra.grid && extra.grid > 0) out.grid = extra.grid;
  return out;
}

/** The target nearest to `c` in a sorted list (binary search), or null for an empty list. */
function closest(sorted: readonly number[], c: number): number | null {
  let lo = 0;
  let hi = sorted.length - 1;
  if (hi < 0) return null;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (sorted[mid] < c) lo = mid + 1;
    else hi = mid;
  }
  const a = sorted[lo];
  const b = lo > 0 ? sorted[lo - 1] : a;
  return Math.abs(b - c) <= Math.abs(a - c) ? b : a;
}

/** The nearest (target - candidate) within `threshold`, or null. Lines win ties over the grid. */
function nearest(
  candidates: readonly number[],
  targets: readonly number[],
  threshold: number,
  grid?: number,
): { offset: number; at: number } | null {
  let best: { offset: number; at: number } | null = null;
  const consider = (c: number, t: number | null) => {
    if (t === null) return;
    const d = t - c;
    if (Math.abs(d) <= threshold && (best === null || Math.abs(d) < Math.abs(best.offset))) best = { offset: d, at: t };
  };
  for (const c of candidates) consider(c, closest(targets, c));
  if (grid) for (const c of candidates) consider(c, Math.round(c / grid) * grid);
  return best;
}

/** Snaps a moving box: its left/centre/right and top/middle/bottom to the nearest targets. */
export function snapMove(r: Rect, targets: SnapTargets, threshold: number): { rect: Rect; guides: Guide[] } {
  const guides: Guide[] = [];
  const out = { ...r };
  if (threshold > 0) {
    const sx = nearest([r.x, r.x + r.w / 2, r.x + r.w], targets.x, threshold, targets.grid);
    if (sx) {
      out.x += sx.offset;
      guides.push({ axis: "x", pos: sx.at });
    }
    const sy = nearest([r.y, r.y + r.h / 2, r.y + r.h], targets.y, threshold, targets.grid);
    if (sy) {
      out.y += sy.offset;
      guides.push({ axis: "y", pos: sy.at });
    }
  }
  return { rect: out, guides };
}

// ---- Keyboard -------------------------------------------------------------------------------------

/** Arrow-key nudge: 1 px, 10 px with Shift. Null for any other key. */
export function nudge(r: Rect, key: string, shift: boolean): Rect | null {
  const step = shift ? 10 : 1;
  switch (key) {
    case "ArrowLeft":
      return { ...r, x: r.x - step };
    case "ArrowRight":
      return { ...r, x: r.x + step };
    case "ArrowUp":
      return { ...r, y: r.y - step };
    case "ArrowDown":
      return { ...r, y: r.y + step };
    default:
      return null;
  }
}

// ---- Drag readout ---------------------------------------------------------------------------------

/** What the readout shows: the position while moving, the size while resizing. */
export type ReadoutMode = "move" | "size";

/** `X 120 Y 840` (move) or `W 640 H 180` (resize), whole screen px. */
export function readoutText(mode: ReadoutMode, r: Rect): string {
  return mode === "move"
    ? `X ${Math.round(r.x)} Y ${Math.round(r.y)}`
    : `W ${Math.round(r.w)} H ${Math.round(r.h)}`;
}

/** Room the readout needs, in canvas px (a generous estimate: placing it reads no layout). */
export const READOUT_W = 112;
export const READOUT_H = 18;
const READOUT_GAP = 4;

/** Where the readout goes: a canvas-px anchor plus a CSS percentage shift of the label itself. */
export interface ReadoutPlace {
  x: number;
  y: number;
  /** translate(tx%, ty%) after the anchor: -100 puts the label left of / above it. */
  tx: 0 | -100;
  ty: 0 | -100;
}

/**
 * Next to `r` (screen px) on a stage of `screen` × `scale`: under its bottom-left corner, else
 * over its top-left corner, else inside its bottom-left corner (a box filling the screen's
 * height). Kept on the stage horizontally: right-aligned to the box when the left edge has no room.
 */
export function readoutPlace(r: Rect, screen: Size, scale: number): ReadoutPlace {
  const stageW = screen.w * scale;
  const stageH = screen.h * scale;
  const left = r.x * scale;
  const right = (r.x + r.w) * scale;
  const top = r.y * scale;
  const bottom = (r.y + r.h) * scale;
  let x: number;
  let tx: 0 | -100 = 0;
  if (left + READOUT_W <= stageW) x = Math.max(0, left);
  else {
    x = Math.max(Math.min(right, stageW), READOUT_W);
    tx = -100;
  }
  if (bottom + READOUT_GAP + READOUT_H <= stageH) return { x, y: Math.max(0, bottom) + READOUT_GAP, tx, ty: 0 };
  if (top - READOUT_GAP - READOUT_H >= 0) return { x, y: Math.min(top, stageH) - READOUT_GAP, tx, ty: -100 };
  return { x, y: Math.min(bottom, stageH) - READOUT_GAP, tx, ty: -100 };
}

// ---- Timing ---------------------------------------------------------------------------------------

/**
 * Coalesces values into one callback per animation frame (the latest wins). A frame is requested
 * only while a value is pending, so nothing runs at rest.
 */
export class FrameQueue<T> {
  #id: number | null = null;
  #latest: { v: T } | null = null;

  constructor(private readonly fn: (v: T) => void) {}

  push(v: T): void {
    this.#latest = { v };
    if (this.#id === null) {
      this.#id = requestAnimationFrame(() => {
        this.#id = null;
        this.#run();
      });
    }
  }

  /** Runs a pending value now. */
  flush(): void {
    this.#cancelFrame();
    this.#run();
  }

  cancel(): void {
    this.#cancelFrame();
    this.#latest = null;
  }

  #cancelFrame() {
    if (this.#id !== null) cancelAnimationFrame(this.#id);
    this.#id = null;
  }

  #run() {
    const l = this.#latest;
    this.#latest = null;
    if (l) this.fn(l.v);
  }
}

/**
 * At most one call per `intervalMs`: the first call runs at once, later ones within the interval
 * collapse into one trailing call with the latest value (always delivered unless cancelled). A
 * timer exists only while a trailing call is pending.
 */
export class Throttle<T> {
  #last = -Infinity;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #pending: { v: T } | null = null;

  /** May change between calls (the editor scales it with the number of layers per call). */
  intervalMs: number;

  constructor(
    intervalMs: number,
    private readonly fn: (v: T) => void,
  ) {
    this.intervalMs = intervalMs;
  }

  call(v: T): void {
    const wait = this.#last + this.intervalMs - Date.now();
    if (wait <= 0 && this.#timer === undefined) {
      this.#fire(v);
      return;
    }
    this.#pending = { v };
    if (this.#timer === undefined) {
      this.#timer = setTimeout(() => {
        this.#timer = undefined;
        const p = this.#pending;
        this.#pending = null;
        if (p) this.#fire(p.v);
      }, Math.max(0, wait));
    }
  }

  /** Delivers a pending trailing call now. */
  flush(): void {
    this.#clear();
    const p = this.#pending;
    this.#pending = null;
    if (p) this.#fire(p.v);
  }

  cancel(): void {
    this.#clear();
    this.#pending = null;
  }

  #fire(v: T) {
    this.#last = Date.now();
    this.fn(v);
  }

  #clear() {
    if (this.#timer !== undefined) clearTimeout(this.#timer);
    this.#timer = undefined;
  }
}
