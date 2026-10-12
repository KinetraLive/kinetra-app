// The anchor grid's labels and the multi-selection's shared anchor: only the Inspector chunks
// (AnchorPad, MultiInspector) need these, so they stay out of the editor route's initial JS.

import type { LayerAnchor, ScreenLayer } from "$lib/types";
import { normalizeAnchor, type AnchorH, type AnchorV } from "./anchor";

/** The anchor the layers share (null = free for all), or "mixed". */
export function commonAnchor(layers: readonly ScreenLayer[]): LayerAnchor | null | "mixed" {
  const keys = new Set(layers.map((l) => JSON.stringify(normalizeAnchor(l.anchor))));
  if (keys.size !== 1) return "mixed";
  return normalizeAnchor(layers[0]?.anchor);
}

export const ANCHOR_H: readonly AnchorH[] = ["left", "center", "right"];
export const ANCHOR_V: readonly AnchorV[] = ["top", "middle", "bottom"];

const H_NAME = { left: "left", center: "center", right: "right" } as const;
const V_NAME = { top: "Top", middle: "Middle", bottom: "Bottom" } as const;

/** "Top left" … "Middle center" → "Center" … "Bottom right". */
export function anchorLabel(h: AnchorH, v: AnchorV): string {
  if (h === "center" && v === "middle") return "Center";
  return `${V_NAME[v]} ${H_NAME[h]}`;
}
