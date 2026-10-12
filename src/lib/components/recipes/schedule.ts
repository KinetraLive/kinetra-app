// Schedule panel model: plain functions (badges, time formatting, drift, shortcuts, the actions an
// item offers), no Svelte, so they are unit-testable.
//
// Times: the server computes statuses from its clock and sends epoch ms (`startMs`, `computedAt`).
// The panel corrects its own clock by the drift seen when a view arrives (`computedAt - Date.now()`)
// so countdowns match the server even if the clocks differ. Item times are local date-times
// (`YYYY-MM-DDTHH:MM`); what the operator sees is their HH:MM part.

import type { ItemStatus, ScheduleAction, ScheduleItem, ScheduleItemView, ScheduleView } from "$lib/types";

export type Tally = "program" | "preview" | "warning" | "idle";

/** The badge of a status: text (never colour alone) and its dot. `upcoming` has none. */
export const STATUS_BADGE: Readonly<Record<ItemStatus, { text: string; tally: Tally } | null>> = {
  now: { text: "NOW", tally: "program" },
  next: { text: "UP NEXT", tally: "preview" },
  delayed: { text: "DELAYED", tally: "warning" },
  previous: { text: "PREVIOUS", tally: "idle" },
  cancelled: { text: "CANCELLED", tally: "idle" },
  upcoming: null,
};

const pad = (n: number) => String(n).padStart(2, "0");

/** `2026-10-05T14:30` → `14:30`. */
export function hhmm(local: string): string {
  return local.slice(11, 16);
}

/** `2026-10-05T14:30` → `2026-10-05`. */
export function datePart(local: string): string {
  return local.slice(0, 10);
}

/** Epoch ms → local `HH:MM:SS`. */
export function clockText(ms: number): string {
  const d = new Date(ms);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** Epoch ms → local `YYYY-MM-DD`. */
export function localDate(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Remaining ms → `mm:ss`, or `h:mm:ss` past an hour (rounded up, never negative). */
export function countdownText(ms: number): string {
  const s = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h > 0 ? `${h}:${pad(m)}:${pad(s % 60)}` : `${pad(m)}:${pad(s % 60)}`;
}

/** Below this the NEXT card counts down; above it shows the start time. */
export const COUNTDOWN_WITHIN_MS = 60 * 60_000;

/** "starts in 04:59", "starting", or "at 15:10" (the day too when it is not `today`). */
export function startsText(item: ScheduleItemView, serverNow: number, today: string): string {
  const left = item.startMs - serverNow;
  if (left <= 0) return "starting";
  if (left < COUNTDOWN_WITHIN_MS) return `starts in ${countdownText(left)}`;
  const day = datePart(item.start);
  return day === today ? `at ${hhmm(item.start)}` : `at ${dayLabel(day)} ${hhmm(item.start)}`;
}

/** `2026-10-05` → "Mon 5 Oct" (the user's locale). */
export function dayLabel(date: string): string {
  const d = new Date(`${date}T00:00`);
  if (Number.isNaN(d.getTime())) return date;
  return d.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
}

/** Local date-time → epoch ms (NaN when malformed). */
export function localMs(local: string): number {
  return /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(local) ? new Date(local).getTime() : NaN;
}

/** Minutes the effective start is later than planned (0 when on time or unknown). */
export function lateMinutes(view: ScheduleItemView, item: ScheduleItem | undefined): number {
  if (!item) return 0;
  const shift = (localMs(view.start) - localMs(item.start)) / 60_000;
  return Number.isFinite(shift) && shift > 0 ? Math.round(shift) : 0;
}

/** Server-minus-local clock offset carried by a view. */
export function driftOf(view: ScheduleView, localNow = Date.now()): number {
  return view.computedAt - localNow;
}

/** `HH:MM` (24 h). */
export function validTime(t: string): boolean {
  return /^([01]\d|2[0-3]):[0-5]\d$/.test(t);
}

/** The item's day with a new time of day: `YYYY-MM-DDTHH:MM`. */
export function atTime(local: string, time: string): string {
  return `${datePart(local)}T${time}`;
}

/** The item the countdown panel targets: UP NEXT, else the first not yet over, else the first. */
export function countdownTarget(view: ScheduleView): ScheduleItemView | null {
  return (
    view.items.find((i) => i.id === view.nextId) ??
    view.items.find((i) => i.status !== "previous" && i.status !== "cancelled") ??
    view.items[0] ??
    null
  );
}

/** Rows that start a new day get its label (only when the schedule spans several days). */
export function dayBreaks(items: readonly ScheduleItemView[]): Map<string, string> {
  const out = new Map<string, string>();
  const days = new Set(items.map((i) => datePart(i.start)));
  if (days.size < 2) return out;
  let last = "";
  for (const i of items) {
    const d = datePart(i.start);
    if (d !== last) out.set(i.id, dayLabel(d));
    last = d;
  }
  return out;
}

export type ScheduleKey = "startNext" | "delayNext" | "undo";

/**
 * The panel's shortcuts (only while focus is inside it): N = start UP NEXT, D = delay UP NEXT +5,
 * Ctrl+Z = undo. Never while typing, never on key repeat.
 */
export function scheduleKey(e: KeyboardEvent, typing: boolean): ScheduleKey | null {
  if (typing || e.repeat || e.altKey || e.metaKey) return null;
  const k = e.key.toLowerCase();
  if (e.ctrlKey) return k === "z" && !e.shiftKey ? "undo" : null;
  if (e.shiftKey) return null;
  if (k === "n") return "startNext";
  if (k === "d") return "delayNext";
  return null;
}

/** Whether "Back to automatic" changes anything for this item. */
export function hasOverride(
  view: ScheduleItemView,
  item: ScheduleItem | undefined,
  nextOverride: string | null,
): boolean {
  return (
    nextOverride === view.id ||
    view.moved ||
    (item !== undefined && (item.state !== "auto" || item.delayMin !== 0 || !!item.startedAt || !!item.endedAt))
  );
}

/** Quick delays offered per item (minutes). */
export const DELAYS = [5, 10, 15] as const;
/** Custom delays: whole minutes in this range. */
export const DELAY_RANGE = { min: 1, max: 24 * 60 } as const;

export type ItemAction = { id: string; label: string; action: ScheduleAction | null; form?: FormKind };
export type FormKind = "next" | "delay" | "start";

/** Every action the item's menu offers, in order (a `form` opens an inline input instead). */
export function itemActions(
  view: ScheduleItemView,
  item: ScheduleItem | undefined,
  nextOverride: string | null,
): ItemAction[] {
  const itemId = view.id;
  const out: ItemAction[] = [];
  const cancelled = view.status === "cancelled";
  if (!cancelled) {
    if (view.status !== "now") out.push({ id: "start", label: "Start now", action: { do: "startNow", itemId } });
    if (view.status === "now") out.push({ id: "end", label: "End", action: { do: "end", itemId } });
    if (view.status !== "now" && view.status !== "previous")
      out.push({ id: "next", label: "Set as next…", action: null, form: "next" });
    for (const m of DELAYS) out.push({ id: `delay${m}`, label: `Delay +${m} min`, action: { do: "delay", itemId, minutes: m } });
    out.push({ id: "delay", label: "Delay by…", action: null, form: "delay" });
    out.push({ id: "setStart", label: "Set start time…", action: null, form: "start" });
    out.push({ id: "cancel", label: "Cancel", action: { do: "cancel", itemId } });
  } else {
    out.push({ id: "restore", label: "Restore", action: { do: "restore", itemId } });
  }
  if (hasOverride(view, item, nextOverride))
    out.push({ id: "auto", label: "Back to automatic", action: { do: "auto", itemId } });
  return out;
}
