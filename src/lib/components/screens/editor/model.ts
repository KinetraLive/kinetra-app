// Pure screen-document logic of the layout editor: new layers, z-order, template switching,
// effective variable values and the "why can't this be saved" check. No Svelte, no I/O.
//
// `doc.layers` is the z-order, bottom first. The layers panel lists them reversed (top = front).

import {
  LIST_ROWS_MAX,
  SCREEN_LAYERS_MAX,
  SCREEN_NAME_MAX,
  type ContentSizing,
  type Rect,
  type SafeArea,
  type ScreenDoc,
  type ScreenGuides,
  type ScreenLayer,
  type SourceBinding,
  type TemplateScreenLayer,
  type SourceSummary,
  type TemplateDesign,
  type TemplateDoc,
  type VariableDef,
} from "$lib/types";
import { valueIssue } from "$lib/components/templates/editor/model";
import { moveLayerTo } from "./anchor";

const ID_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

/** nanoid-style id: 10 random chars from [A-Za-z0-9_-]. */
export function newLayerId(
  random: (a: Uint8Array<ArrayBuffer>) => Uint8Array = (a) => crypto.getRandomValues(a),
): string {
  const bytes = random(new Uint8Array(10));
  let id = "";
  for (const b of bytes) id += ID_ALPHABET[b & 63];
  return id;
}

/** Each field's template default, by key. */
export function fieldDefaults(t: Pick<TemplateDoc, "fields">): Record<string, string> {
  return Object.fromEntries(t.fields.map((f) => [f.key, f.default]));
}

/** The reference screen a template's design position is given on. */
export const DESIGN_SCREEN = { w: 1920, h: 1080 } as const;

/**
 * A new layer's box for a template's design box on a `screen`:
 * - size: the design size; when it doesn't fit the screen it shrinks uniformly to fit
 *   (k = min(W / w, H / h), each side rounded, at least 1 px);
 * - position: x and y are on a 1920×1080 screen, scaled by W / 1920 and H / 1080 and rounded;
 *   absent = centered (floor((W - w) / 2));
 * - then clamped so the box lies inside the screen.
 * Rust's layer creation (core) follows the same rule.
 */
export function designRect(design: TemplateDesign, screen: { width: number; height: number }): Rect {
  const W = screen.width;
  const H = screen.height;
  let w = design.w;
  let h = design.h;
  if (w > W || h > H) {
    const k = Math.min(W / w, H / h);
    w = Math.max(1, Math.round(w * k));
    h = Math.max(1, Math.round(h * k));
  }
  const x = design.x == null ? Math.floor((W - w) / 2) : Math.round((design.x * W) / DESIGN_SCREEN.w);
  const y = design.y == null ? Math.floor((H - h) / 2) : Math.round((design.y * H) / DESIGN_SCREEN.h);
  return { x: Math.min(Math.max(x, 0), W - w), y: Math.min(Math.max(y, 0), H - h), w, h };
}

/**
 * A new layer showing `t`: the template's name and field defaults, no overrides. A template with
 * a design box gets that box (`designRect`); others fill the screen. A "fit" design is written as
 * the layer's sizing; a "reflow" one leaves it absent (reflow until the user resizes the layer,
 * which scales it; only a Reflow the user picks is kept on resize). The core does the same.
 */
export function newLayer(t: TemplateDoc, screen: { width: number; height: number }, id = newLayerId()): TemplateScreenLayer {
  const layer: TemplateScreenLayer = {
    id,
    name: t.name,
    kind: "template",
    templateId: t.id,
    rect: t.design ? designRect(t.design, screen) : { x: 0, y: 0, w: screen.width, h: screen.height },
    fields: fieldDefaults(t),
    vars: {},
  };
  if (t.design?.sizing === "fit") layer.sizing = "fit";
  return layer;
}

export const canAddLayer = (doc: ScreenDoc) => doc.layers.length < SCREEN_LAYERS_MAX;

/** Adds `layer` in front of everything (the end of the list). */
export function addLayer(doc: ScreenDoc, layer: ScreenLayer): ScreenDoc {
  if (!canAddLayer(doc)) return doc;
  return { ...doc, layers: [...doc.layers, layer] };
}

export function removeLayer(doc: ScreenDoc, id: string): ScreenDoc {
  return { ...doc, layers: doc.layers.filter((l) => l.id !== id) };
}

export function patchLayer(doc: ScreenDoc, id: string, patch: Partial<ScreenLayer>): ScreenDoc {
  return {
    ...doc,
    layers: doc.layers.map((l) => (l.id === id ? ({ ...l, ...patch } as ScreenLayer) : l)),
  };
}

/** Content sizing of the layers `ids`, written explicitly (absent means reflow). */
export function setLayerSizing<D extends ScreenDoc>(doc: D, ids: readonly string[], sizing: ContentSizing): D {
  const set = new Set(ids);
  return { ...doc, layers: doc.layers.map((l) => (set.has(l.id) && l.kind !== "media" ? { ...l, sizing } : l)) } as D;
}

/**
 * Layers resized by hand become Scale to fit (the graphic follows its box), unless the user chose
 * Reflow for them. Unchanged when there is nothing to switch.
 */
export function resizedToFit<D extends ScreenDoc>(doc: D, ids: readonly string[]): D {
  const set = new Set(ids);
  const hit = (l: ScreenLayer): l is TemplateScreenLayer => set.has(l.id) && l.kind !== "media";
  if (!doc.layers.some((l) => hit(l) && l.sizing !== "fit" && l.sizing !== "reflow")) return doc;
  return { ...doc, layers: doc.layers.map((l) => (hit(l) && l.sizing !== "reflow" ? { ...l, sizing: "fit" } : l)) } as D;
}

/** A layer moved or resized by hand; an anchored one follows the core's rule (`moveLayerTo`). */
export function setRect(doc: ScreenDoc, id: string, rect: Rect): ScreenDoc {
  return setRects(doc, [{ id, rect }]);
}

/**
 * Several layers' boxes at once (one edit, one save). Anchored layers: a move releases the moved
 * axis, a resize stays anchored (placed), as the core does on save.
 */
export function setRects(doc: ScreenDoc, changes: readonly { id: string; rect: Rect }[]): ScreenDoc {
  const by = new Map(changes.map((c) => [c.id, c.rect]));
  const screen = { width: doc.width, height: doc.height };
  return { ...doc, layers: doc.layers.map((l) => (by.has(l.id) ? moveLayerTo(l, by.get(l.id) as Rect, screen) : l)) };
}

/** The screen's ruler guides (sorted, unique); none = no `guides` key, as the file omits it. */
export function setGuides(doc: ScreenDoc, g: ScreenGuides): ScreenDoc {
  const norm = (v: readonly number[]) => [...new Set(v.map((n) => Math.round(n)))].sort((a, b) => a - b);
  const next: ScreenDoc = { ...doc, guides: { x: norm(g.x), y: norm(g.y) } };
  if (next.guides!.x.length === 0 && next.guides!.y.length === 0) delete next.guides;
  return next;
}

/** The screen's custom safe areas; none = no `safeAreas` key. */
export function setSafeAreas(doc: ScreenDoc, areas: readonly SafeArea[]): ScreenDoc {
  const next: ScreenDoc = { ...doc, safeAreas: [...areas] };
  if (areas.length === 0) delete next.safeAreas;
  return next;
}

/**
 * Moves a layer one step in z: `+1` towards the front (up in the panel), `-1` towards the back.
 * Unchanged at either end.
 */
export function shiftLayer(doc: ScreenDoc, id: string, dir: 1 | -1): ScreenDoc {
  const i = doc.layers.findIndex((l) => l.id === id);
  const j = i + dir;
  if (i < 0 || j < 0 || j >= doc.layers.length) return doc;
  const layers = doc.layers.slice();
  [layers[i], layers[j]] = [layers[j], layers[i]];
  return { ...doc, layers };
}

/** Moves the layer at doc index `from` to doc index `to` (drag reorder). */
export function moveLayer(doc: ScreenDoc, from: number, to: number): ScreenDoc {
  const n = doc.layers.length;
  if (from === to || from < 0 || to < 0 || from >= n || to >= n) return doc;
  const layers = doc.layers.slice();
  const [l] = layers.splice(from, 1);
  layers.splice(to, 0, l);
  return { ...doc, layers };
}

/** Panel row index (top = front) <-> doc index (bottom first). */
export const panelToDoc = (count: number, row: number) => count - 1 - row;

/**
 * The layer switched to template `next`: field values kept for keys `next` also has (others get
 * `next`'s defaults, keys it lacks are dropped), variable overrides kept only for its keys. A name
 * that was the old template's name follows the new template.
 */
export function switchTemplate(
  layer: TemplateScreenLayer,
  prev: { name: string } | null,
  next: TemplateDoc,
): TemplateScreenLayer {
  const fields: Record<string, string> = {};
  for (const f of next.fields) fields[f.key] = Object.hasOwn(layer.fields, f.key) ? layer.fields[f.key] : f.default;
  const keys = new Set(next.variables.map((v) => v.key));
  const vars = Object.fromEntries(Object.entries(layer.vars).filter(([k]) => keys.has(k)));
  const name = prev && layer.name === prev.name ? next.name : layer.name;
  const out: TemplateScreenLayer = { ...layer, templateId: next.id, name, fields, vars };
  // The new template shows its default variant.
  delete out.variant;
  // Bindings survive only for field keys the new template also has.
  delete out.bindings;
  if (layer.bindings) {
    const fieldKeys = new Set(next.fields.map((f) => f.key));
    const kept = Object.entries(layer.bindings).filter(([k]) => fieldKeys.has(k));
    if (kept.length > 0) out.bindings = Object.fromEntries(kept);
  }
  return out;
}

// ---- Source bindings ----------------------------------------------------------------------------

/**
 * Binds field `key` of layer `layerId` to a source column (replacing any binding it had). A list
 * binding (column "") keeps its sort and limit (limit clamped to 1–LIST_ROWS_MAX); a single field
 * keeps a NOW / NEXT row selector ("current", the default, is stored as absent).
 */
export function bindField<D extends ScreenDoc>(doc: D, layerId: string, key: string, binding: SourceBinding): D {
  const b: SourceBinding = { sourceId: binding.sourceId, column: binding.column };
  if (binding.column === "") {
    if (binding.sort) b.sort = { column: binding.sort.column, numeric: binding.sort.numeric, desc: binding.sort.desc };
    if (binding.limit !== undefined && Number.isFinite(binding.limit))
      b.limit = Math.min(LIST_ROWS_MAX, Math.max(1, Math.round(binding.limit)));
  } else if (binding.row === "now" || binding.row === "next") {
    b.row = binding.row;
  }
  return {
    ...doc,
    layers: doc.layers.map((l) => (l.id === layerId && l.kind !== "media" ? { ...l, bindings: { ...l.bindings, [key]: b } } : l)),
  } as D;
}

/** Unbinds field `key`; it shows the layer's own value again. No bindings left = no `bindings`. */
export function unbindField<D extends ScreenDoc>(doc: D, layerId: string, key: string): D {
  const layer = doc.layers.find((l) => l.id === layerId);
  if ((!layer || layer.kind === "media") || !layer.bindings || !Object.hasOwn(layer.bindings, key)) return doc;
  const rest = { ...layer.bindings };
  delete rest[key];
  return {
    ...doc,
    layers: doc.layers.map((l) => {
      if (l.id !== layerId || l.kind === "media") return l;
      const next: TemplateScreenLayer = { ...l, bindings: rest };
      if (Object.keys(rest).length === 0) delete next.bindings;
      return next;
    }),
  } as D;
}

/** True when any layer of the screen has a binding. */
export const hasBindings = (doc: ScreenDoc) =>
  doc.layers.some((l) => l.kind !== "media" && l.bindings !== undefined && Object.keys(l.bindings).length > 0);

/** Template defaults with the layer's overrides on top. */
export function effectiveVars(defs: readonly VariableDef[], overrides: Readonly<Record<string, string>>): Record<string, string> {
  return Object.fromEntries(defs.map((v) => [v.key, Object.hasOwn(overrides, v.key) ? overrides[v.key] : v.default]));
}

/** Error per variable index for the overridden values (null when fine or not overridden). */
export function overrideErrors(defs: readonly VariableDef[], overrides: Readonly<Record<string, string>>): (string | null)[] {
  return defs.map((v) =>
    Object.hasOwn(overrides, v.key) ? valueIssue({ ...v, default: overrides[v.key] } as VariableDef) : null,
  );
}

const HEX = /^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$/;
export const isBackground = (v: string) => v === "transparent" || HEX.test(v);

/** Why the working copy can't be saved, or null. `templates` = the loaded docs by id. */
export function blockedReason(doc: ScreenDoc, templates: ReadonlyMap<string, TemplateDoc>): string | null {
  if (!isBackground(doc.background)) return "Background: enter a color like #1a1a1a.";
  if (doc.safeAreas?.some((a) => a.name.trim() === "")) return "Safe areas: every safe area needs a name.";
  for (const l of doc.layers) {
    if (l.name.trim() === "") return "Layers: every layer needs a name.";
    // Media layers: the Inspector keeps the range valid (clampRange); the core checks it again.
    if (l.kind === "media") continue;
    const t = templates.get(l.templateId);
    if (!t) continue;
    const errs = overrideErrors(t.variables, l.vars);
    const i = errs.findIndex((e) => e !== null);
    if (i >= 0) return `${l.name}: ${t.variables[i].label || t.variables[i].key}: ${errs[i]}`;
  }
  return null;
}

export const LAYER_NAME_MAX = SCREEN_NAME_MAX;

/** Why a (trimmed) screen name can't be used, or null: the name field and "Rename screen…". */
export function screenNameError(name: string): string | null {
  const n = name.trim();
  if (n === "") return "Enter a name.";
  if (n.length > SCREEN_NAME_MAX) return `Up to ${SCREEN_NAME_MAX} characters.`;
  return null;
}

// Binding labels live in a dependency-free module (Control uses them too); re-exported here.
export { bindingLabel, bindingText, listBindingLabel } from "$lib/components/fields/binding-label";
