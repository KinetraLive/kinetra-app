// Align and distribute for the layout editor (lazy: loaded by the align buttons and the palette
// commands). Pure maths over rects in screen px; results are whole pixels.

import type { Rect } from "$lib/types";
import { boundsOf, type Size } from "./shared";

export type AlignKind = "left" | "center" | "right" | "top" | "middle" | "bottom";

export const ALIGN_KINDS: readonly AlignKind[] = ["left", "center", "right", "top", "middle", "bottom"];

/**
 * `rects` aligned to their bounds (two or more), or one rect to the screen. Order is kept; only
 * x (left / center / right) or y (top / middle / bottom) changes.
 */
export function align(rects: readonly Rect[], kind: AlignKind, screen: Size): Rect[] {
  const to = rects.length === 1 ? { x: 0, y: 0, w: screen.w, h: screen.h } : boundsOf(rects);
  if (!to) return [];
  return rects.map((r) => {
    switch (kind) {
      case "left":
        return { ...r, x: to.x };
      case "center":
        return { ...r, x: Math.round(to.x + (to.w - r.w) / 2) };
      case "right":
        return { ...r, x: to.x + to.w - r.w };
      case "top":
        return { ...r, y: to.y };
      case "middle":
        return { ...r, y: Math.round(to.y + (to.h - r.h) / 2) };
      case "bottom":
        return { ...r, y: to.y + to.h - r.h };
    }
  });
}

/**
 * Three or more rects with equal gaps along `axis`: the first and last (by their start edge) stay,
 * the ones between are spaced evenly (gaps rounded so the total is exact). Fewer than three: as is.
 */
export function distribute(rects: readonly Rect[], axis: "x" | "y"): Rect[] {
  if (rects.length < 3) return rects.map((r) => ({ ...r }));
  const pos = (r: Rect) => (axis === "x" ? r.x : r.y);
  const size = (r: Rect) => (axis === "x" ? r.w : r.h);
  const order = rects.map((r, i) => i).sort((a, b) => pos(rects[a]) - pos(rects[b]) || a - b);
  const first = rects[order[0]];
  const last = rects[order[order.length - 1]];
  const span = pos(last) + size(last) - pos(first);
  const total = order.reduce((s, i) => s + size(rects[i]), 0);
  const gap = (span - total) / (order.length - 1);
  const out = rects.map((r) => ({ ...r }));
  let used = 0;
  order.forEach((i, k) => {
    if (k > 0 && k < order.length - 1) {
      const v = Math.round(pos(first) + used + gap * k);
      if (axis === "x") out[i].x = v;
      else out[i].y = v;
    }
    used += size(rects[i]);
  });
  return out;
}
