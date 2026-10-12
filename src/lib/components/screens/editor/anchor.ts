// Layer position anchors: the TS mirror of src-tauri/src/core/anchors.rs (the core is the
// authority; this keeps the editor's canvas instant). Pure, no Svelte.
//
// - Margin: 5 % of the screen side (round(W / 20)) inside title safe, else 0.
// - h: left → x = margin; center → x = round((W − w) / 2) (halves up); right → x = W − margin − w.
//   v likewise with H. A free axis keeps its coordinate.
// - A move with the size unchanged releases the moved axis (both released = no anchor); a resize
//   keeps the anchor; a changed anchor wins over the rect's position.

import type { LayerAnchor, Rect, ScreenDoc, ScreenLayer } from "$lib/types";

export type AnchorH = NonNullable<LayerAnchor["h"]>;
export type AnchorV = NonNullable<LayerAnchor["v"]>;
type Size = { width: number; height: number };

export const FREE: LayerAnchor = { h: null, v: null, safe: false };

export const anchorMargin = (size: number, safe: boolean) => (safe ? Math.floor((size + 10) / 20) : 0);

function coord(at: 0 | 1 | 2, len: number, size: number, safe: boolean): number {
  const m = anchorMargin(size, safe);
  if (at === 0) return m;
  if (at === 1) return Math.floor((size - len + 1) / 2);
  return size - m - len;
}

const H_AT = { left: 0, center: 1, right: 2 } as const;
const V_AT = { top: 0, middle: 1, bottom: 2 } as const;

/** `rect` placed by `anchor` on the screen (free axes unchanged). */
export function placeAnchor(rect: Rect, anchor: LayerAnchor, screen: Size): Rect {
  const r = { ...rect };
  if (anchor.h) r.x = coord(H_AT[anchor.h], r.w, screen.width, anchor.safe);
  if (anchor.v) r.y = coord(V_AT[anchor.v], r.h, screen.height, anchor.safe);
  return r;
}

/** Both axes free and `safe` off is no anchor. */
export function normalizeAnchor(a: LayerAnchor | null | undefined): LayerAnchor | null {
  return a && (a.h || a.v || a.safe) ? { h: a.h ?? null, v: a.v ?? null, safe: !!a.safe } : null;
}

/** A layer's box moved from `prev` to `rect`: the box and anchor afterwards (see the header). */
export function movedAnchor(
  prev: Rect,
  anchor: LayerAnchor | null | undefined,
  rect: Rect,
  screen: Size,
): { rect: Rect; anchor: LayerAnchor | null } {
  const a = normalizeAnchor(anchor);
  if (!a) return { rect, anchor: null };
  if (prev.w === rect.w && prev.h === rect.h) {
    const had = a.h !== null || a.v !== null;
    if (rect.x !== prev.x) a.h = null;
    if (rect.y !== prev.y) a.v = null;
    if (had && a.h === null && a.v === null) return { rect, anchor: null };
  }
  return { rect: placeAnchor(rect, a, screen), anchor: a };
}

/** `layer` with its anchor (absent when free) and its box placed by it. */
export function withAnchor(layer: ScreenLayer, anchor: LayerAnchor | null, screen: Size): ScreenLayer {
  const a = normalizeAnchor(anchor);
  const next: ScreenLayer = { ...layer };
  if (a) {
    next.anchor = a;
    next.rect = placeAnchor(layer.rect, a, screen);
  } else delete next.anchor;
  return next;
}

/** `layer` moved/resized to `rect` by hand (the release rule). */
export function moveLayerTo(layer: ScreenLayer, rect: Rect, screen: Size): ScreenLayer {
  if (!layer.anchor) return { ...layer, rect };
  const m = movedAnchor(layer.rect, layer.anchor, rect, screen);
  const next: ScreenLayer = { ...layer, rect: m.rect };
  if (m.anchor) next.anchor = m.anchor;
  else delete next.anchor;
  return next;
}

/**
 * The layers `ids` anchored by `patch` on top of each one's own anchor (the grid sets both axes,
 * the centre buttons one, the toggle `safe`), each placed on its own.
 */
export function setAnchors(doc: ScreenDoc, ids: readonly string[], patch: Partial<LayerAnchor>): ScreenDoc {
  const set = new Set(ids);
  const screen = { width: doc.width, height: doc.height };
  return {
    ...doc,
    layers: doc.layers.map((l) => (set.has(l.id) ? withAnchor(l, { ...FREE, ...l.anchor, ...patch }, screen) : l)),
  };
}
