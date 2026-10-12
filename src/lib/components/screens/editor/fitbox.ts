// "Fit box to graphic" (the Inspector and the MultiInspector, both lazy chunks): a layer's box set
// to its template's design box, so the graphic shows 1:1 with Scale to fit. Plus the Inspector's
// scale readout ("Scale 64 %") and its "100 %" reset.

import type { Rect, TemplateDesign } from "$lib/types";
import { fullRect, sameRect } from "./geometry";
import { designRect } from "./model";

/**
 * The box that shows `design` at 1:1 (shrunk uniformly when it is larger than the screen). It
 * keeps the layer's position, moved inside the screen; a layer that fills the screen gets the
 * template's default position instead (`designRect`, as a new layer would).
 */
export function fitBox(rect: Rect, design: TemplateDesign, screen: { width: number; height: number }): Rect {
  const d = designRect(design, screen);
  if (sameRect(rect, fullRect({ w: screen.width, h: screen.height }))) return d;
  return {
    x: Math.min(Math.max(rect.x, 0), screen.width - d.w),
    y: Math.min(Math.max(rect.y, 0), screen.height - d.h),
    w: d.w,
    h: d.h,
  };
}

type DesignSize = { w: number; h: number } | null | undefined;

/**
 * The graphic's scale in a Scale to fit box: 1 = its design size (the screen's without a design,
 * as the renderer does). The smaller of the two axes, as fit scales.
 */
export function fitScale(rect: Rect, design: DesignSize, screen: { width: number; height: number }): number {
  const w = design?.w || screen.width;
  const h = design?.h || screen.height;
  return w > 0 && h > 0 ? Math.min(rect.w / w, rect.h / h) : 1;
}

/** The scale readout's number: "64 %". */
export const scaleText = (s: number) => `${Math.round(s * 100)} %`;

/**
 * "100 %": the box at the design size (the screen's without a design; shrunk uniformly when larger
 * than the screen), keeping the box's centre, moved inside the screen.
 */
export function actualSize(rect: Rect, design: DesignSize, screen: { width: number; height: number }): Rect {
  let w = design?.w || screen.width;
  let h = design?.h || screen.height;
  if (w > screen.width || h > screen.height) {
    const k = Math.min(screen.width / w, screen.height / h);
    w = Math.max(1, Math.round(w * k));
    h = Math.max(1, Math.round(h * k));
  }
  const x = Math.round(rect.x + rect.w / 2 - w / 2);
  const y = Math.round(rect.y + rect.h / 2 - h / 2);
  return { x: Math.min(Math.max(x, 0), screen.width - w), y: Math.min(Math.max(y, 0), screen.height - h), w, h };
}
