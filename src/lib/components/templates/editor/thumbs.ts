// Variant thumbnails: each variant rendered statically by the RENDERER's own `compile` (the same
// code OBS runs, not a second rendering path) into a small inert shadow root, scaled down with a
// CSS transform, animations frozen at their end state. No iframe, no timers, no network: fonts
// fall back to system fonts and the output server's /assets are not loaded.
//
// Lazy chunk: loaded by VariantTabs with a dynamic import, so neither this nor the renderer code
// is in the editor route's initial bundle.

import { compile } from "../../../../../renderer/src/layer";
import type { FieldType as WireFieldType } from "../../../../../renderer/src/protocol";
import { TEMPLATE_FONTS, type FieldDef, type VariableDef } from "$lib/types";
import { previewDefault } from "./fieldTypes";
import { effectiveValues } from "./variants";

/** The frame a template lays out in (the preview screen's size). */
export const STAGE_W = 1920;
export const STAGE_H = 1080;

/**
 * Freezes every animation at its end and drops transitions, so the "in" look shows at once. The
 * selector's two id-level `:not()`s outrank the template's own `!important` rules (e.g. a
 * reduced-motion block that resets animation-delay), which would otherwise freeze it at the start.
 */
const ANY = ":not(#_):not(#_)";
const FREEZE = `${ANY},${ANY}::before,${ANY}::after{animation-delay:-99s!important;animation-play-state:paused!important;transition:none!important}`;

const HEX = /^#(?:[0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
const NUM = /^-?(?:\d+\.?\d*|\.\d+)$/;
const OPTION = /^[\w-]{1,64}$/;
const UNIT = /^[a-z%]{0,8}$/;

/**
 * A variable's CSS value, by the same strict grammar the server applies (core/vars.rs), or null
 * when the value is not valid (the thumbnail then leaves `--key` unset).
 */
export function cssValue(def: VariableDef, value: string): string | null {
  switch (def.type) {
    case "color":
      return HEX.test(value) ? value : value.toLowerCase() === "transparent" ? "transparent" : null;
    case "font":
      return TEMPLATE_FONTS.find((f) => f.name === value)?.stack ?? null;
    case "range": {
      const v = value.trim();
      const n = Number(v);
      if (!NUM.test(v) || !Number.isFinite(n) || n < def.min || n > def.max || !UNIT.test(def.unit)) return null;
      return `${n}${def.unit}`;
    }
    case "select":
      return OPTION.test(value) && def.options.some((o) => o.value === value) ? value : null;
  }
}

export interface ThumbInput {
  html: string;
  css: string;
  vars: Record<string, string>;
}

/** One thumbnail: an outer shadow root (the scaled stage) holding the layer host's shadow root. */
interface Slot {
  stage: HTMLElement;
  layer: HTMLElement;
  shadow: ShadowRoot;
}

const slots = new WeakMap<HTMLElement, Slot>();

function slot(el: HTMLElement): Slot {
  let s = slots.get(el);
  if (s) return s;
  const outer = el.shadowRoot ?? el.attachShadow({ mode: "open" });
  const style = document.createElement("style");
  style.textContent =
    `:host{display:block;overflow:hidden;contain:strict}` +
    `.stage{position:relative;width:${STAGE_W}px;height:${STAGE_H}px;transform-origin:0 0;overflow:hidden}` +
    `.layer{position:absolute;inset:0}`;
  const stage = document.createElement("div");
  stage.className = "stage";
  const layer = document.createElement("div");
  layer.className = "layer";
  stage.append(layer);
  outer.replaceChildren(style, stage);
  s = { stage, layer, shadow: layer.attachShadow({ mode: "open" }) };
  slots.set(el, s);
  el.setAttribute("inert", "");
  return s;
}

/**
 * The value a thumbnail shows: the field's default; text without one shows its label (as the
 * preview does); media without one stays empty (no request for a label as a URL).
 */
export function thumbValue(f: FieldDef): string {
  switch (f.type) {
    case "list":
    case "clock":
    case "timer":
      return previewDefault(f);
    case "image":
    case "video":
      return f.default;
    default:
      return f.default || previewDefault(f);
  }
}

export interface ThumbOptions {
  /** Field values to show instead of the defaults (e.g. a preset's sample data). */
  values?: Readonly<Record<string, string>>;
  /**
   * Play the in-animation (the gallery's hover preview): framed on the end state, then rebuilt
   * without the freeze, its start state styled, then `data-state="in"` set, so the template's own
   * animations and transitions run once.
   */
  play?: boolean;
}

/** Renders `v` into `el` (re-renders in place on later calls). */
export function renderThumb(
  el: HTMLElement,
  v: ThumbInput,
  fields: readonly FieldDef[],
  variables: readonly VariableDef[],
  opts: ThumbOptions = {},
): void {
  const { stage, layer, shadow } = slot(el);
  const types: Record<string, WireFieldType> = {};
  const values: Record<string, string> = {};
  for (const f of fields) {
    types[f.key] = f.type;
    values[f.key] = opts.values?.[f.key] ?? thumbValue(f);
  }
  const build = () => compile(v.html, types, (k) => values[k] ?? "");
  const c = build();
  c.root?.setAttribute("data-state", "in");
  const freeze = document.createElement("style");
  freeze.textContent = FREEZE;
  const style = document.createElement("style");
  style.textContent = v.css;
  shadow.replaceChildren(style, freeze, c.frag);
  layer.removeAttribute("style");
  const vals = effectiveValues(variables, v.vars);
  for (const d of variables) {
    const css = cssValue(d, vals[d.key]);
    if (css !== null) layer.style.setProperty(`--${d.key}`, css);
  }
  fit(el, stage, shadow);
  if (!opts.play) return;
  const p = build();
  shadow.replaceChildren(style, p.frag);
  // Style the start state first (a forced layout), so transitions from it run too.
  p.root?.getBoundingClientRect();
  p.root?.setAttribute("data-state", "in");
}

/**
 * Frames the graphic: scales and centres the stage so the box around the template's visible leaf
 * elements (panels, text, images) fills the thumbnail; a lower third would otherwise be a speck
 * in a 1920×1080 frame. One layout read per render. Nothing visible: the whole frame.
 */
function fit(el: HTMLElement, stage: HTMLElement, shadow: ShadowRoot) {
  const tw = el.clientWidth;
  const th = el.clientHeight;
  if (!tw || !th) return;
  stage.style.transform = "none";
  const o = stage.getBoundingClientRect();
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const e of Array.from(shadow.querySelectorAll("*"))) {
    if (e.firstElementChild || e.localName === "style") continue;
    const r = e.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) continue;
    x0 = Math.min(x0, r.left - o.left);
    y0 = Math.min(y0, r.top - o.top);
    x1 = Math.max(x1, r.right - o.left);
    y1 = Math.max(y1, r.bottom - o.top);
  }
  // Clamp to the frame, with a little air around the graphic.
  x0 = Math.max(0, x0);
  y0 = Math.max(0, y0);
  x1 = Math.min(STAGE_W, x1);
  y1 = Math.min(STAGE_H, y1);
  if (!(x1 > x0 && y1 > y0)) {
    x0 = y0 = 0;
    x1 = STAGE_W;
    y1 = STAGE_H;
  }
  const pad = Math.max(x1 - x0, y1 - y0) * 0.06;
  x0 = Math.max(0, x0 - pad);
  y0 = Math.max(0, y0 - pad);
  x1 = Math.min(STAGE_W, x1 + pad);
  y1 = Math.min(STAGE_H, y1 + pad);
  const k = Math.min(tw / (x1 - x0), th / (y1 - y0));
  const tx = (tw - (x1 - x0) * k) / 2 - x0 * k;
  const ty = (th - (y1 - y0) * k) / 2 - y0 * k;
  stage.style.transform = `translate(${tx}px,${ty}px) scale(${k})`;
}

/** Frees a thumbnail's content (the editor closed or the variant was deleted). */
export function releaseThumb(el: HTMLElement): void {
  const s = slots.get(el);
  if (!s) return;
  s.shadow.replaceChildren();
  slots.delete(el);
}
