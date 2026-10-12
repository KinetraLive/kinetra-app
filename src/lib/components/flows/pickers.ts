// Picker options and value mappings shared by the linear editor (Control) and the node editor's
// inspector: screens, layers, timer fields, sources, flows, and the small enumerations (on/off air,
// pause mode, timer action, schedule op, source step, schedule item). Plain functions, no Svelte.

import type { FieldDef, ItemPick, ItemStatus, ScheduleOp, SourceStep, TimerAction } from "$lib/types";
import type { Lookups } from "./linear";

export type Opt = { value: string; label: string; disabled?: boolean };

/** "Take for N s": 1 s to 1 h. */
export const TAKE_FOR_MS = { min: 1_000, max: 3_600_000 } as const;
/** "N minutes before the next item": 1 min to a day. */
export const BEFORE_MINUTES = { min: 1, max: 1440 } as const;
/** Schedule "Delay by N min". */
export const DELAY_MINUTES = { min: -1440, max: 1440 } as const;

export const STATUS_OPTIONS: { value: ItemStatus; label: string }[] = [
  { value: "now", label: "NOW" },
  { value: "next", label: "UP NEXT" },
  { value: "delayed", label: "DELAYED" },
  { value: "previous", label: "PREVIOUS" },
  { value: "cancelled", label: "CANCELLED" },
];

/** Picker options with the current value kept (marked missing) when it no longer exists. */
export function withCurrent(options: Opt[], current: string, placeholder: string): Opt[] {
  if (current === "") return [{ value: "", label: placeholder, disabled: true }, ...options];
  if (options.some((o) => o.value === current)) return options;
  return [...options, { value: current, label: `Missing (${current})` }];
}

export const screenOptions = (k: Lookups): Opt[] =>
  [...k.screens.values()].map((v) => ({ value: v.screen.id, label: v.screen.name }));

export const scheduleOptions = (k: Lookups): Opt[] =>
  [...k.sources.values()].filter((s) => s.kind === "schedule").map((s) => ({ value: s.id, label: s.name }));

export const textOptions = (k: Lookups): Opt[] =>
  [...k.sources.values()].filter((s) => s.kind !== "schedule").map((s) => ({ value: s.id, label: s.name }));

export const apiOptions = (k: Lookups): Opt[] =>
  [...k.sources.values()].filter((s) => s.kind === "api").map((s) => ({ value: s.id, label: s.name }));

/** API value changed: any change (whole rows) or one column of the current row. */
export function apiColumnOptions(k: Lookups, sourceId: string, current: string | null): Opt[] {
  const cols = k.sources.get(sourceId)?.columns ?? [];
  const opts: Opt[] = [{ value: "", label: "Any row changes" }, ...cols.map((c) => ({ value: c.key, label: c.label || c.key }))];
  return current && !cols.some((c) => c.key === current) ? [...opts, { value: current, label: `Missing (${current})` }] : opts;
}

export const flowOptions = (k: Lookups, selfId: string | null): Opt[] =>
  [...k.flows.entries()].filter(([id]) => id !== selfId).map(([id, f]) => ({ value: id, label: f.name }));

/** The Run flow picker's placeholder when there is no other flow to run. */
export const NO_OTHER_FLOWS = "No other flows — use the Repeat node to loop this flow.";

/** Run flow: every other flow (a flow can't run itself; Repeat loops it). */
export function runFlowPick(k: Lookups, selfId: string | null, current: string): Opt[] {
  const opts = flowOptions(k, selfId);
  return withCurrent(opts, current, opts.length > 0 ? "Choose a flow" : NO_OTHER_FLOWS);
}

/** Stop flow: this flow first ("This flow", once it exists), then the others. */
export function stopFlowPick(k: Lookups, selfId: string | null, current: string): Opt[] {
  const self = selfId ? [{ value: selfId, label: "This flow" }] : [];
  return withCurrent([...self, ...flowOptions(k, selfId)], current, "Choose a flow");
}

/** A screen's layers, top first. */
/** Media transport choices (Flows' Media action). */
export const MEDIA_ACTIONS: { value: "play" | "pause" | "stop" | "restart" | "seek"; label: string }[] = [
  { value: "play", label: "Play" },
  { value: "pause", label: "Pause" },
  { value: "stop", label: "Stop" },
  { value: "restart", label: "Restart" },
  { value: "seek", label: "Go to time" },
];

export function layersOf(k: Lookups, screenId: string): Opt[] {
  return [...(k.screens.get(screenId)?.layers ?? [])].reverse().map((l) => ({ value: l.id, label: l.name }));
}

export function fieldDefsOf(k: Lookups, screenId: string, layerId: string): FieldDef[] {
  return k.screens.get(screenId)?.layers.find((l) => l.id === layerId)?.fieldDefs ?? [];
}

export function timerKeys(k: Lookups, screenId: string, layerId: string): Opt[] {
  return fieldDefsOf(k, screenId, layerId)
    .filter((f) => f.type === "timer")
    .map((f) => ({ value: f.key, label: f.label || f.key }));
}

export function fieldKeys(k: Lookups, screenId: string, layerId: string): Opt[] {
  return fieldDefsOf(k, screenId, layerId).map((f) => ({ value: f.key, label: f.label || f.key }));
}

/** A config aimed at a screen's layer (and maybe its fields or a timer key). */
export type Layered = { screenId: string; layerId: string; fields?: Record<string, string>; key?: string; variant?: string };

/** A new screen: its top layer, no fields, its first timer. Mutates `x`. */
export function setScreen(k: Lookups, x: Layered, screenId: string): void {
  x.screenId = screenId;
  x.layerId = layersOf(k, screenId)[0]?.value ?? "";
  if (x.fields) x.fields = {};
  if (x.key !== undefined) x.key = timerKeys(k, screenId, x.layerId)[0]?.value ?? "";
  if (x.variant !== undefined) x.variant = "";
}

/** A new layer: no fields, its first timer. Mutates `x`. */
export function setLayer(k: Lookups, x: Layered, layerId: string): void {
  x.layerId = layerId;
  if (x.fields) x.fields = {};
  if (x.key !== undefined) x.key = timerKeys(k, x.screenId, layerId)[0]?.value ?? "";
  if (x.variant !== undefined) x.variant = "";
}

export const ON_AIR: { value: "on" | "off"; label: string }[] = [
  { value: "on", label: "goes on air" },
  { value: "off", label: "goes off air" },
];

export const PAUSE: { value: "pause" | "resume" | "toggle"; label: string }[] = [
  { value: "pause", label: "Pause" },
  { value: "resume", label: "Resume" },
  { value: "toggle", label: "Pause or resume" },
];

export type TimerKind = TimerAction["action"];
export const TIMER_ACTIONS: { value: TimerKind; label: string }[] = [
  { value: "start", label: "Start" },
  { value: "pause", label: "Pause" },
  { value: "reset", label: "Reset" },
  { value: "set", label: "Set to" },
];

export type OpKind = ScheduleOp["op"];
export const OPS: { value: OpKind; label: string }[] = [
  { value: "startNow", label: "Start now" },
  { value: "end", label: "End" },
  { value: "setNext", label: "Make next" },
  { value: "delay", label: "Delay by" },
  { value: "setStart", label: "Move start to" },
  { value: "cancel", label: "Cancel" },
  { value: "restore", label: "Restore" },
  { value: "auto", label: "Back to automatic" },
];

export type StepMode = "next" | "prev" | "index";
export const STEP_MODES: { value: StepMode; label: string }[] = [
  { value: "next", label: "Next row" },
  { value: "prev", label: "Previous row" },
  { value: "index", label: "Go to row" },
];
export const modeOf = (st: SourceStep): StepMode => (st.to === "index" ? "index" : st.delta < 0 ? "prev" : "next");
export const stepFor = (mode: StepMode): SourceStep =>
  mode === "index" ? { to: "index", index: 0 } : { to: "by", delta: mode === "prev" ? -1 : 1 };

export const timerFor = (kind: TimerKind, from: TimerAction): TimerAction =>
  kind === "set" ? { action: "set", ms: from.action === "set" ? from.ms : 60_000 } : ({ action: kind } as TimerAction);

export function opFor(kind: OpKind, from: ScheduleOp): ScheduleOp {
  if (kind === "delay") return { op: "delay", minutes: from.op === "delay" ? from.minutes : 5 };
  if (kind === "setNext") return { op: "setNext", start: from.op === "setNext" ? (from.start ?? null) : null };
  // A new start is required; empty until the operator enters one (an error until then).
  if (kind === "setStart") return { op: "setStart", start: from.op === "setStart" ? from.start : "" };
  return { op: kind } as ScheduleOp;
}

export const pickValue = (p: ItemPick): string => (p.pick === "item" ? `item:${p.itemId}` : p.pick);
export const pickFor = (v: string): ItemPick =>
  v.startsWith("item:") ? { pick: "item", itemId: v.slice(5) } : ({ pick: v } as ItemPick);

/** 0 = Monday … 6 = Sunday: toggles a day, kept sorted. */
export function toggleDay(days: number[], d: number): number[] {
  return days.includes(d) ? days.filter((x) => x !== d) : [...days, d].sort((a, b) => a - b);
}
