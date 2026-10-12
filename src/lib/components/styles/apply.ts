// Applying a style (lazy chunk; the gallery, Preferences > Style & brand and the palette load it).
//
//   applyToProject(style)            every layer whose template has (or can get) the style's look
//   applyToLayer(style, layer)       one layer
//
// Project scope asks first when a layer that would switch is on air ("N layers on air will
// switch", with what Live edit will do). The result is a toast ("Frame applied · 12 switched ·
// 2 custom templates unchanged") with Undo; Ctrl+Z also undoes while that toast is up (not while
// typing). The server keeps the previous variant of every layer, so Undo is one step.

import { mount, unmount } from "svelte";
import { currentProject, toAppError } from "$lib/api";
import { listScreens } from "$lib/api/live";
import { getScreen } from "$lib/api/screens";
import { applyStyle, getTemplate, listPresets, listTemplates, undoApplyStyle } from "$lib/api/templates";
import { isTyping } from "$lib/commands/keys";
import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
import { project, uiState } from "$lib/stores/project.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import type { ApplyStyleResult, ApplyStyleScope, VariantTransition } from "$lib/types";
import { afterStyle, beforeStyle } from "./styles";

/** How long the result toast (and its Undo, Ctrl+Z included) stays up. */
export const UNDO_MS = 10_000;

export interface StyleRef {
  id: string;
  name: string;
}

export interface LayerRef {
  screenId: string;
  layerId: string;
  name: string;
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** "Frame applied · 12 switched · 2 custom templates unchanged · 3 variants added". */
export function summary(r: ApplyStyleResult, style: string, layer?: string): string {
  if (layer) {
    return r.switched > 0
      ? `${style} applied to ${layer}`
      : `${layer} has no ${style} look (custom template)`;
  }
  const parts = [`${style} applied`, `${r.switched} switched`];
  if (r.unchangedTemplates > 0)
    parts.push(`${plural(r.unchangedTemplates, "custom template")} unchanged`);
  if (r.added > 0) parts.push(`${plural(r.added, "variant")} added`);
  return parts.join(" · ");
}

/**
 * How many on-air layers would switch to `style`: their variant differs and their template is
 * from a preset that ships the style, or already has it.
 */
export async function onAirAffected(style: string): Promise<number> {
  const views = await listScreens();
  const onAir = views.flatMap((v) =>
    v.layers.filter((l) => l.onAir && l.variant !== style).map((l) => ({ screenId: v.screen.id, layerId: l.id })),
  );
  if (onAir.length === 0) return 0;
  const screenIds = [...new Set(onAir.map((o) => o.screenId))];
  const [presets, templates, docs] = await Promise.all([
    listPresets(),
    listTemplates(),
    Promise.all(screenIds.map((id) => getScreen(id).then((s) => [id, s.doc] as const))),
  ]);
  const shipping = new Set(presets.filter((p) => p.variants?.some((v) => v.id === style)).map((p) => p.id));
  const byId = new Map(templates.map((t) => [t.id, t]));
  const byScreen = new Map(docs);
  const has = new Map<string, Promise<boolean>>();
  let n = 0;
  for (const o of onAir) {
    const ol = byScreen.get(o.screenId)?.layers.find((l) => l.id === o.layerId);
    const tid = (!!ol && ol.kind !== "media") ? ol.templateId : undefined;
    const t = tid ? byId.get(tid) : undefined;
    if (!t) continue;
    if (t.preset && shipping.has(t.preset.id)) {
      n++;
      continue;
    }
    if (t.variantCount < 2) continue;
    let p = has.get(t.id);
    if (!p) {
      p = getTemplate(t.id).then(
        (d) => d.variants.some((v) => v.id === style),
        () => false,
      );
      has.set(t.id, p);
    }
    if (await p) n++;
  }
  return n;
}

/** The confirm dialog, mounted on <body> while open; resolves true for Apply. */
function confirm(title: string, message: string): Promise<boolean> {
  const back = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  return new Promise((resolve) => {
    const done = (ok: boolean) => {
      resolve(ok);
      queueMicrotask(() => {
        void unmount(dialog);
        if (back?.isConnected) back.focus();
      });
    };
    const dialog = mount(ConfirmDialog, {
      target: document.body,
      props: { title, message, confirmLabel: "Apply", onconfirm: () => done(true), oncancel: () => done(false) },
    });
  });
}

export function onAirMessage(n: number): string {
  const head = `${plural(n, "layer")} on air will switch.`;
  return uiState.value?.control.liveEdit === "onTake"
    ? `${head} They change on the next take.`
    : `${head} They crossfade to the new look now.`;
}

async function refreshProject() {
  try {
    const p = await currentProject();
    if (p) project.replace(p);
  } catch {
    // The next project event brings it.
  }
}

let busy = false;
let disarm = () => {};

/** Ctrl+Z undoes while toast `id` is up (capture phase, so it wins over a page's own undo). */
function armUndo(id: number, undo: () => void) {
  disarm();
  const off = () => {
    window.removeEventListener("keydown", onKey, true);
    clearTimeout(timer);
    if (disarm === off) disarm = () => {};
  };
  const onKey = (e: KeyboardEvent) => {
    if (!toasts.list.some((t) => t.id === id)) return off();
    if (e.key.toLowerCase() !== "z" || !(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey) return;
    if (isTyping(e.target)) return;
    e.preventDefault();
    e.stopPropagation();
    toasts.dismiss(id);
    off();
    undo();
  };
  window.addEventListener("keydown", onKey, true);
  const timer = setTimeout(off, UNDO_MS + 100);
  disarm = off;
}

async function undo(token: string, style: string): Promise<void> {
  try {
    await beforeStyle();
    await undoApplyStyle(token);
    toasts.push({ kind: "info", message: `${style} undone` });
  } catch (e) {
    toasts.push({ kind: "error", message: `Couldn't undo ${style}: ${toAppError(e).message}` });
  } finally {
    afterStyle();
    void refreshProject();
  }
}

async function run(style: StyleRef, scope: ApplyStyleScope, transition: VariantTransition, layer?: string) {
  let r: ApplyStyleResult;
  try {
    await beforeStyle();
    r = await applyStyle(style.id, scope, transition);
  } catch (e) {
    toasts.push({ kind: "error", message: `Couldn't apply ${style.name}: ${toAppError(e).message}` });
    return null;
  } finally {
    afterStyle();
  }
  if (scope.kind === "project") void refreshProject();
  const token = r.undo;
  const again = token ? () => void undo(token, style.name) : null;
  const id = toasts.push({
    kind: layer && r.switched === 0 ? "info" : "success",
    message: summary(r, style.name, layer),
    timeoutMs: UNDO_MS,
    action: again ? { label: "Undo", run: () => (disarm(), again()) } : undefined,
  });
  if (again) armUndo(id, again);
  return r;
}

/**
 * Applies `style` to the whole project (asks first when on-air layers would switch). Resolves
 * with the result, or null when cancelled or failed (the error is toasted).
 */
export async function applyToProject(
  style: StyleRef,
  transition: VariantTransition = "crossfade",
): Promise<ApplyStyleResult | null> {
  if (busy) return null;
  busy = true;
  try {
    const n = await onAirAffected(style.id).catch(() => 0);
    if (n > 0 && !(await confirm(`Apply ${style.name} to the project?`, onAirMessage(n)))) return null;
    return await run(style, { kind: "project" }, transition);
  } finally {
    busy = false;
  }
}

/** Applies `style` to one layer (on air it follows Live edit, like the Variant select). */
export async function applyToLayer(
  style: StyleRef,
  layer: LayerRef,
  transition: VariantTransition = "crossfade",
): Promise<ApplyStyleResult | null> {
  if (busy) return null;
  busy = true;
  try {
    return await run(style, { kind: "layer", screenId: layer.screenId, layerId: layer.layerId }, transition, layer.name);
  } finally {
    busy = false;
  }
}
