// Smart guides of the layout editor (lazy: part of the overlays chunk): equal-spacing snapping and
// what to draw while dragging (distances to the nearest neighbours or screen edges, equal-gap
// markers). Pure maths over rects in screen px; no Svelte, no DOM, no layout reads.
//
// Cost per drag frame is O(n) over the other layers (n <= 64): the context sorts them once per
// drag, each frame filters that sorted list to the dragged box's row / column.

import type { Rect } from "$lib/types";
import type { Guide, Size } from "./geometry";

type Axis = "x" | "y";

const lo = (r: Rect, ax: Axis) => (ax === "x" ? r.x : r.y);
const len = (r: Rect, ax: Axis) => (ax === "x" ? r.w : r.h);
const hi = (r: Rect, ax: Axis) => lo(r, ax) + len(r, ax);
const cross = (ax: Axis): Axis => (ax === "x" ? "y" : "x");
/** a and b overlap along `ax` (touching doesn't count). */
const overlaps = (a: Rect, b: Rect, ax: Axis) => lo(a, ax) < hi(b, ax) && lo(b, ax) < hi(a, ax);

/** Gaps that differ by less than this (screen px) are equal. */
const EQUAL_EPS = 0.5;

export interface SmartContext {
  screen: Size;
  /** The other layers sorted by x, and by y. */
  byX: Rect[];
  byY: Rect[];
}

/** Built once per drag (the boxes that don't move). */
export function smartContext(screen: Size, others: readonly Rect[]): SmartContext {
  return {
    screen,
    byX: [...others].sort((a, b) => a.x - b.x),
    byY: [...others].sort((a, b) => a.y - b.y),
  };
}

/**
 * A measured gap. Axis "x": a horizontal segment from x = a to x = b at y = at; axis "y": a
 * vertical one from y = a to y = b at x = at. Screen px.
 */
export interface Gap {
  axis: Axis;
  a: number;
  b: number;
  at: number;
}

export interface Smart {
  /** The distance to the nearest neighbour (or the screen edge) on each side with room. */
  measures: Gap[];
  /** Equal-spacing markers: the dragged box's gaps that equal another gap in its row / column, and those gaps. */
  spacing: Gap[];
}

/** The other boxes in r's row (axis x: overlapping vertically) or column (axis y), sorted along `ax`. */
function lane(r: Rect, ctx: SmartContext, ax: Axis): Rect[] {
  const c = cross(ax);
  return (ax === "x" ? ctx.byX : ctx.byY).filter((o) => overlaps(o, r, c));
}

/** The nearest box entirely before r and entirely after r along `ax` (members sorted). */
function neighbours(r: Rect, members: readonly Rect[], ax: Axis): { before: Rect | null; after: Rect | null } {
  let before: Rect | null = null;
  let after: Rect | null = null;
  for (const m of members) {
    if (hi(m, ax) <= lo(r, ax) + EQUAL_EPS && (!before || hi(m, ax) > hi(before, ax))) before = m;
    if (lo(m, ax) >= hi(r, ax) - EQUAL_EPS && (!after || lo(m, ax) < lo(after, ax))) after = m;
  }
  return { before, after };
}

/** Gaps between consecutive, non-overlapping members (sorted along `ax`). */
function laneGaps(members: readonly Rect[], ax: Axis): { a: Rect; b: Rect; size: number }[] {
  const out: { a: Rect; b: Rect; size: number }[] = [];
  for (let i = 1; i < members.length; i++) {
    const a = members[i - 1];
    const b = members[i];
    const size = lo(b, ax) - hi(a, ax);
    if (size > 0) out.push({ a, b, size });
  }
  return out;
}

/** The middle of where a and b overlap across `ax` (else between their centres). */
function across(a: Rect, b: Rect, ax: Axis): number {
  const c = cross(ax);
  const top = Math.max(lo(a, c), lo(b, c));
  const bottom = Math.min(hi(a, c), hi(b, c));
  return top < bottom ? (top + bottom) / 2 : (lo(a, c) + len(a, c) / 2 + lo(b, c) + len(b, c) / 2) / 2;
}

/**
 * Where r's start along `ax` would make one of its gaps equal another gap in its lane, or centre
 * it between its neighbours: the candidate nearest r within `threshold`, or null.
 */
function spacingTarget(r: Rect, ctx: SmartContext, ax: Axis, threshold: number): number | null {
  const members = lane(r, ctx, ax);
  if (members.length === 0) return null;
  const { before, after } = neighbours(r, members, ax);
  const gaps = laneGaps(members, ax);
  const start = lo(r, ax);
  let best: number | null = null;
  const consider = (v: number) => {
    if (Math.abs(v - start) <= threshold && (best === null || Math.abs(v - start) < Math.abs(best - start))) best = v;
  };
  for (const g of gaps) {
    if (before) consider(hi(before, ax) + g.size);
    if (after) consider(lo(after, ax) - g.size - len(r, ax));
  }
  if (before && after) {
    const mid = (hi(before, ax) + lo(after, ax) - len(r, ax)) / 2;
    if (mid > hi(before, ax)) consider(mid);
  }
  return best;
}

/**
 * Equal-spacing snap on top of `snapped` (snapMove's result for `raw`): per axis, a spacing
 * target nearer than the line snap (or with no line snap) wins and drops that axis's line guide.
 */
export function snapSpacing(
  raw: Rect,
  snapped: { rect: Rect; guides: Guide[] },
  ctx: SmartContext,
  threshold: number,
): { rect: Rect; guides: Guide[] } {
  if (threshold <= 0) return snapped;
  let { rect, guides } = snapped;
  for (const ax of ["x", "y"] as const) {
    const t = spacingTarget(raw, ctx, ax, threshold);
    if (t === null) continue;
    const lined = guides.some((g) => g.axis === ax);
    const lineOffset = lined ? Math.abs(lo(rect, ax) - lo(raw, ax)) : Infinity;
    if (Math.abs(t - lo(raw, ax)) < lineOffset) {
      rect = ax === "x" ? { ...rect, x: t } : { ...rect, y: t };
      guides = guides.filter((g) => g.axis !== ax);
    }
  }
  return { rect, guides };
}

/** What to draw for the dragged box `r` (its final rect this frame). */
export function smartGuides(r: Rect, ctx: SmartContext): Smart {
  const measures: Gap[] = [];
  const spacing: Gap[] = [];
  for (const ax of ["x", "y"] as const) {
    const c = cross(ax);
    const mid = lo(r, c) + len(r, c) / 2;
    const size = ax === "x" ? ctx.screen.w : ctx.screen.h;
    const members = lane(r, ctx, ax);
    const { before, after } = neighbours(r, members, ax);
    const own: Gap[] = [];
    const b: Gap = before
      ? { axis: ax, a: hi(before, ax), b: lo(r, ax), at: across(before, r, ax) }
      : { axis: ax, a: 0, b: lo(r, ax), at: mid };
    const a: Gap = after
      ? { axis: ax, a: hi(r, ax), b: lo(after, ax), at: across(r, after, ax) }
      : { axis: ax, a: hi(r, ax), b: size, at: mid };
    for (const g of [b, a]) if (g.b - g.a >= 1) measures.push(g);
    if (before) own.push(b);
    if (after) own.push(a);

    // Equal spacing: our gaps to neighbours vs the lane's gaps (and each other).
    const gaps = laneGaps(members, ax);
    for (const g of own) {
      const size0 = g.b - g.a;
      if (size0 < 1) continue;
      const same = gaps.filter((x) => Math.abs(x.size - size0) < EQUAL_EPS);
      const twin = own.find((o) => o !== g && Math.abs(o.b - o.a - size0) < EQUAL_EPS);
      if (same.length === 0 && !twin) continue;
      if (!spacing.includes(g)) spacing.push(g);
      for (const s of same) {
        const gap: Gap = { axis: ax, a: hi(s.a, ax), b: lo(s.b, ax), at: across(s.a, s.b, ax) };
        if (!spacing.some((x) => x.axis === ax && x.a === gap.a && x.b === gap.b)) spacing.push(gap);
      }
      if (twin && !spacing.includes(twin)) spacing.push(twin);
    }
  }
  return { measures, spacing };
}
