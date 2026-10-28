// The button panel editor's pure parts: new buttons and steps with sensible targets, the
// pickers' options, what a step reads like, and what keeps a button from saving. The core
// validates again (core/panel.rs); a missing target is a soft issue there, never refused here.

import type {
  FieldDef,
  PanelAction,
  PanelActionKind,
  PanelButton,
  PanelColor,
  PanelLayer,
  PanelTargets,
} from "$lib/types";
import { PANEL_ACTIONS_MAX, PANEL_BUTTONS_MAX, PANEL_LABEL_MAX } from "$lib/types";

export const KIND_LABELS: Record<PanelActionKind, string> = {
  take: "Take layer",
  clear: "Clear layer",
  runCue: "Run cue",
  setFields: "Set field values",
  setVariant: "Switch look",
  timer: "Timer",
  selectRow: "Pick a row",
  studioTake: "Studio take",
};

export const COLOR_LABELS: Record<PanelColor, string> = {
  grey: "Gray",
  red: "Red",
  orange: "Orange",
  yellow: "Yellow",
  green: "Green",
  blue: "Blue",
  purple: "Purple",
};

/** Field types a button can set (text-like; files, lists, clocks and timers are set elsewhere). */
const SETTABLE = new Set(["text", "multiline", "number", "color", "html"]);

/** A new id (10 characters, like the core's nanoid). */
export function panelId(): string {
  const abc = "0123456789abcdefghijklmnopqrstuvwxyz";
  const bytes = new Uint8Array(10);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => abc[b % abc.length]).join("");
}

export interface LayerRef {
  screen: string;
  screenName: string;
  layer: PanelLayer;
}

/** Every layer, top first per screen, optionally filtered. */
export function layers(t: PanelTargets, keep: (l: PanelLayer) => boolean = () => true): LayerRef[] {
  return t.screens.flatMap((s) => s.layers.filter(keep).map((layer) => ({ screen: s.id, screenName: s.name, layer })));
}

export const settableFields = (l: PanelLayer): FieldDef[] =>
  l.fields.filter((f) => SETTABLE.has(f.type) && !l.bound.includes(f.key));
export const timerFields = (l: PanelLayer): FieldDef[] => l.fields.filter((f) => f.type === "timer");

/** The layers a step of `kind` can target. */
export function layersFor(t: PanelTargets, kind: PanelActionKind): LayerRef[] {
  switch (kind) {
    case "take":
    case "clear":
      return layers(t);
    case "setFields":
      return layers(t, (l) => !l.media && settableFields(l).length > 0);
    case "setVariant":
      return layers(t, (l) => !l.media && l.variants.length > 1);
    case "timer":
      return layers(t, (l) => !l.media && timerFields(l).length > 0);
    default:
      return [];
  }
}

/** Whether a step of `kind` has anything to point at in this project. */
export function available(t: PanelTargets, kind: PanelActionKind): boolean {
  switch (kind) {
    case "runCue":
      return t.cues.length > 0;
    case "selectRow":
      return t.sources.some((s) => s.rows.length > 0);
    case "studioTake":
      return t.screens.length > 0;
    default:
      return layersFor(t, kind).length > 0;
  }
}

/** A step of `kind` aimed at the first fitting target; `null` when there is none. */
export function newAction(t: PanelTargets, kind: PanelActionKind): PanelAction | null {
  const first = layersFor(t, kind)[0];
  switch (kind) {
    case "take":
    case "clear":
      return first ? { kind, screen: first.screen, layer: first.layer.id } : null;
    case "setFields":
      return first ? { kind, screen: first.screen, layer: first.layer.id, values: {} } : null;
    case "setVariant":
      return first
        ? { kind, screen: first.screen, layer: first.layer.id, variant: first.layer.variants[0]!.id }
        : null;
    case "timer":
      return first
        ? { kind, screen: first.screen, layer: first.layer.id, field: timerFields(first.layer)[0]!.key, op: "start" }
        : null;
    case "runCue":
      return t.cues[0] ? { kind, flow: t.cues[0].id } : null;
    case "selectRow": {
      const s = t.sources.find((x) => x.rows.length > 0);
      return s ? { kind, source: s.id, row: s.rows[0]!.id } : null;
    }
    case "studioTake":
      return t.screens[0] ? { kind, screen: t.screens[0].id, mode: "take" } : null;
  }
}

/** The same step pointed at another layer (keeping what still fits). */
export function retarget(t: PanelTargets, a: PanelAction, screen: string, layer: string): PanelAction {
  const l = t.screens.find((s) => s.id === screen)?.layers.find((x) => x.id === layer);
  switch (a.kind) {
    case "take":
    case "clear":
      return { ...a, screen, layer };
    case "setFields": {
      const keys = new Set(l ? settableFields(l).map((f) => f.key) : []);
      return { ...a, screen, layer, values: Object.fromEntries(Object.entries(a.values).filter(([k]) => keys.has(k))) };
    }
    case "setVariant":
      return { ...a, screen, layer, variant: l?.variants.find((v) => v.id === a.variant)?.id ?? l?.variants[0]?.id ?? a.variant };
    case "timer":
      return { ...a, screen, layer, field: l && timerFields(l).some((f) => f.key === a.field) ? a.field : (l ? timerFields(l)[0]?.key : undefined) ?? a.field };
    default:
      return a;
  }
}

/** A new button: the next free "Button N", gray, its first step a take if there is a layer. */
export function newButton(t: PanelTargets, existing: PanelButton[]): PanelButton | null {
  if (existing.length >= PANEL_BUTTONS_MAX) return null;
  const kinds: PanelActionKind[] = ["take", "runCue", "studioTake", "selectRow"];
  const first = kinds.map((k) => newAction(t, k)).find((a) => a !== null);
  if (!first) return null;
  const names = new Set(existing.map((b) => b.label));
  let n = existing.length + 1;
  while (names.has(`Button ${n}`)) n++;
  return { id: panelId(), label: `Button ${n}`, color: "grey", confirm: false, actions: [first] };
}

/** Why a button can't be saved yet (the panel waits until every button is fine), or `null`. */
export function problem(b: PanelButton): string | null {
  const n = [...b.label.trim()].length;
  if (n === 0) return "Give this button a label.";
  if (n > PANEL_LABEL_MAX) return `Keep the label to ${PANEL_LABEL_MAX} characters.`;
  if (b.actions.length === 0) return "Add at least one action.";
  if (b.actions.length > PANEL_ACTIONS_MAX) return `A button runs at most ${PANEL_ACTIONS_MAX} actions.`;
  const empty = b.actions.findIndex((a) => a.kind === "setFields" && Object.keys(a.values).length === 0);
  if (empty >= 0) return `Action ${empty + 1}: change at least one field.`;
  return null;
}

export function move<T>(list: T[], from: number, to: number): T[] {
  if (to < 0 || to >= list.length || from === to) return list;
  const out = [...list];
  const [x] = out.splice(from, 1);
  out.splice(to, 0, x!);
  return out;
}
