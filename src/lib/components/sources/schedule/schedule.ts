// Pure schedule-source logic of the Sources editor: local date-time arithmetic, item edits that
// keep the list sorted by start, "Shift all", paste / CSV rows, validation against the backend's
// limits and display helpers. No Svelte, no I/O.
//
// Times are local date-times `YYYY-MM-DDTHH:MM` without a time zone. Arithmetic treats them as
// UTC so it never meets a daylight-saving gap.

import {
  SCHEDULE_COLUMNS,
  SCHEDULE_ITEMS_MAX,
  SOURCE_CELL_MAX,
  SOURCE_NAME_MAX,
  type ItemStatus,
  type ScheduleDoc,
  type ScheduleItem,
  type ScheduleLabels,
  type ScheduleView,
  type SourceSummary,
} from "$lib/types";
import { byteLength } from "../model";

export const LABEL_MAX = 32;
/** Default length of a new item when there is nothing to copy, in minutes. */
export const DEFAULT_MINUTES = 30;
/** Longest item, in minutes (7 days). */
export const DURATION_MAX = 7 * 24 * 60;

export const LABEL_KEYS: ReadonlyArray<{ key: keyof ScheduleLabels; label: string }> = [
  { key: "now", label: "Now" },
  { key: "next", label: "Up next" },
  { key: "delayed", label: "Delayed" },
  { key: "previous", label: "Previous" },
  { key: "cancelled", label: "Canceled" },
  { key: "upcoming", label: "Upcoming" },
];

// ---- Date-times -----------------------------------------------------------------------------------

const DT = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/;
const pad = (n: number) => String(n).padStart(2, "0");

/** Minutes since 1970 of a local date-time, or NaN when it isn't `YYYY-MM-DDTHH:MM`. */
export function toMinutes(dt: string): number {
  const m = DT.exec(dt);
  if (!m) return NaN;
  const [y, mo, d, h, mi] = m.slice(1).map(Number);
  if (mo < 1 || mo > 12 || d < 1 || d > 31 || h > 23 || mi > 59) return NaN;
  const ms = Date.UTC(y, mo - 1, d, h, mi);
  if (new Date(ms).getUTCDate() !== d) return NaN; // 31 Feb
  return ms / 60000;
}

export function fromMinutes(min: number): string {
  const d = new Date(min * 60000);
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}T${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}`;
}

export const validDateTime = (dt: string) => !Number.isNaN(toMinutes(dt));
export const datePart = (dt: string) => dt.slice(0, 10);
export const timePart = (dt: string) => dt.slice(11, 16);
export const addMinutes = (dt: string, n: number) => fromMinutes(toMinutes(dt) + n);

/** A `Date`'s local wall time as `YYYY-MM-DDTHH:MM`. */
export function localDateTime(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Minutes from start to end (NaN when either is invalid). */
export function duration(item: Pick<ScheduleItem, "start" | "end">): number {
  return toMinutes(item.end) - toMinutes(item.start);
}

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "Sat 5 Oct 2026" for a `YYYY-MM-DD` day. */
export function dayLabel(day: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!m) return day;
  const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])));
  return `${WEEKDAYS[d.getUTCDay()]} ${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]} ${d.getUTCFullYear()}`;
}

// ---- Ids --------------------------------------------------------------------------------------------

const ID_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

/** nanoid-style id: 10 random chars from [A-Za-z0-9_-]. */
export function newItemId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(10));
  let id = "";
  for (const b of bytes) id += ID_ALPHABET[b & 63];
  return id;
}

// ---- Edits ------------------------------------------------------------------------------------------

/** Items sorted by start (invalid starts last), then by end; stable otherwise. */
export function sortItems(items: readonly ScheduleItem[]): ScheduleItem[] {
  const key = (dt: string) => {
    const s = toMinutes(dt);
    return Number.isNaN(s) ? Infinity : s;
  };
  return items
    .map((it, i) => ({ it, i }))
    .sort((a, b) => {
      const ka = key(a.it.start);
      const kb = key(b.it.start);
      if (ka !== kb) return ka < kb ? -1 : 1;
      const ea = key(a.it.end);
      const eb = key(b.it.end);
      if (ea !== eb) return ea < eb ? -1 : 1;
      return a.i - b.i;
    })
    .map((x) => x.it);
}

const withItems = (doc: ScheduleDoc, items: ScheduleItem[]): ScheduleDoc => ({ ...doc, items: sortItems(items) });

export const canAddItem = (doc: ScheduleDoc) => doc.items.length < SCHEDULE_ITEMS_MAX;

export function blankItem(start: string, end: string, id = newItemId()): ScheduleItem {
  return { id, title: "", subtitle: "", location: "", start, end, delayMin: 0, state: "auto" };
}

/** The last item by start among those with valid times. */
function lastTimed(items: readonly ScheduleItem[]): ScheduleItem | null {
  const timed = sortItems(items).filter((it) => validDateTime(it.start) && validDateTime(it.end));
  return timed.at(-1) ?? null;
}

/**
 * Adds an item after the last one, as long as it (at least 1 minute); with no items it starts at
 * the next quarter hour from `now` and lasts DEFAULT_MINUTES. Returns the doc and the new id.
 */
export function addItem(
  doc: ScheduleDoc,
  now: Date = new Date(),
  id = newItemId(),
): { doc: ScheduleDoc; id: string } {
  if (!canAddItem(doc)) return { doc, id: "" };
  const last = lastTimed(doc.items);
  let start: string;
  let len: number;
  if (last) {
    start = last.end;
    len = Math.max(1, duration(last));
  } else {
    const n = toMinutes(localDateTime(now));
    start = fromMinutes(Math.ceil(n / 15) * 15);
    len = DEFAULT_MINUTES;
  }
  return { doc: withItems(doc, [...doc.items, blankItem(start, addMinutes(start, len), id)]), id };
}

/** A copy of the item starting when it ends (same length and texts, state automatic). */
export function duplicateItem(
  doc: ScheduleDoc,
  itemId: string,
  id = newItemId(),
): { doc: ScheduleDoc; id: string } {
  const it = doc.items.find((x) => x.id === itemId);
  if (!it || !canAddItem(doc) || !validDateTime(it.start) || !validDateTime(it.end)) return { doc, id: "" };
  const len = Math.max(1, duration(it));
  const copy: ScheduleItem = {
    ...blankItem(it.end, addMinutes(it.end, len), id),
    title: it.title,
    subtitle: it.subtitle,
    location: it.location,
  };
  return { doc: withItems(doc, [...doc.items, copy]), id };
}

export function deleteItem(doc: ScheduleDoc, itemId: string): ScheduleDoc {
  return { ...doc, items: doc.items.filter((x) => x.id !== itemId) };
}

/** Sets a text of an item (title, subtitle, location); the order doesn't change. */
export function setText(
  doc: ScheduleDoc,
  itemId: string,
  key: "title" | "subtitle" | "location",
  value: string,
): ScheduleDoc {
  return { ...doc, items: doc.items.map((x) => (x.id === itemId ? { ...x, [key]: value } : x)) };
}

/** Moves the item to a new start, keeping its length (DEFAULT_MINUTES when its end was invalid). */
export function setStart(doc: ScheduleDoc, itemId: string, start: string): ScheduleDoc {
  return withItems(
    doc,
    doc.items.map((x) => {
      if (x.id !== itemId) return x;
      if (!validDateTime(start)) return { ...x, start };
      const len = duration(x);
      const keep = Number.isFinite(len) && len > 0 ? len : DEFAULT_MINUTES;
      return { ...x, start, end: addMinutes(start, keep) };
    }),
  );
}

/** Sets the end by time of day: on the start's day, or the next day when that is not after the start. */
export function setEndTime(doc: ScheduleDoc, itemId: string, time: string): ScheduleDoc {
  return {
    ...doc,
    items: doc.items.map((x) => {
      if (x.id !== itemId || !validDateTime(x.start)) return x;
      let end = `${datePart(x.start)}T${time}`;
      if (!validDateTime(end)) return { ...x, end };
      if (toMinutes(end) <= toMinutes(x.start)) end = addMinutes(end, 24 * 60);
      return { ...x, end };
    }),
  };
}

/** Sets the length in minutes (the end moves). */
export function setDuration(doc: ScheduleDoc, itemId: string, minutes: number): ScheduleDoc {
  const len = Math.min(DURATION_MAX, Math.max(1, Math.round(minutes)));
  return {
    ...doc,
    items: doc.items.map((x) =>
      x.id === itemId && validDateTime(x.start) ? { ...x, end: addMinutes(x.start, len) } : x,
    ),
  };
}

/** Moves every item (start and end) by `minutes` (negative = earlier). */
export function shiftAll(doc: ScheduleDoc, minutes: number): ScheduleDoc {
  if (!Number.isFinite(minutes) || minutes === 0) return doc;
  const move = (dt: string) => (validDateTime(dt) ? addMinutes(dt, minutes) : dt);
  return { ...doc, items: doc.items.map((x) => ({ ...x, start: move(x.start), end: move(x.end) })) };
}

export interface DayGroup {
  /** `YYYY-MM-DD`, or "" for items without a valid start. */
  day: string;
  items: ScheduleItem[];
}

/** Items (already sorted) grouped by the day they start. */
export function groupByDay(items: readonly ScheduleItem[]): DayGroup[] {
  const out: DayGroup[] = [];
  for (const it of items) {
    const day = validDateTime(it.start) ? datePart(it.start) : "";
    const last = out.at(-1);
    if (last && last.day === day) last.items.push(it);
    else out.push({ day, items: [it] });
  }
  return out;
}

// ---- Paste / CSV -------------------------------------------------------------------------------------

type PasteKey = "title" | "subtitle" | "location" | "start" | "end";

const HEADER_NAMES: Record<string, PasteKey> = {
  title: "title",
  name: "title",
  event: "title",
  item: "title",
  subtitle: "subtitle",
  description: "subtitle",
  location: "location",
  room: "location",
  stage: "location",
  where: "location",
  start: "start",
  begin: "start",
  from: "start",
  time: "start",
  end: "end",
  to: "end",
  until: "end",
  finish: "end",
};
const DEFAULT_ORDER: PasteKey[] = ["title", "start", "end", "location"];

/** The column of each cell when the first line is a header (it names a title or start), else null. */
export function scheduleHeader(first: readonly string[]): (PasteKey | null)[] | null {
  const seen = new Set<PasteKey>();
  const keys = first.map((c) => {
    const k = HEADER_NAMES[c.trim().toLowerCase()] ?? null;
    if (!k || seen.has(k)) return null;
    seen.add(k);
    return k;
  });
  return seen.has("title") || seen.has("start") ? keys : null;
}

const checked = (dt: string) => (validDateTime(dt) ? dt : null);
const hasDate = (text: string) => /\d{4}/.test(text);

/**
 * A pasted time as a local date-time: "14:30" / "9.05" (on `day`), "2026-10-05 14:30",
 * "2026-10-05T14:30(:00)", "5.10.2026 14:30" or "5/10/2026 14:30" (day first). Null when unreadable.
 */
export function parseWhen(text: string, day: string): string | null {
  const t = text.trim();
  let m = /^(\d{1,2})[:.](\d{2})$/.exec(t);
  if (m) return checked(`${day}T${pad(Number(m[1]))}:${m[2]}`);
  m = /^(\d{4})-(\d{1,2})-(\d{1,2})[T ](\d{1,2}):(\d{2})(?::\d{2}(?:\.\d+)?)?$/.exec(t);
  if (m) return checked(`${m[1]}-${pad(Number(m[2]))}-${pad(Number(m[3]))}T${pad(Number(m[4]))}:${m[5]}`);
  m = /^(\d{1,2})[./](\d{1,2})[./](\d{4})[ T,]+(\d{1,2})[:.](\d{2})$/.exec(t);
  if (m) return checked(`${m[3]}-${pad(Number(m[2]))}-${pad(Number(m[1]))}T${pad(Number(m[4]))}:${m[5]}`);
  return null;
}

export interface SchedulePasteResult {
  doc: ScheduleDoc;
  header: boolean;
  added: number;
  /** Lines without a title or a readable start. */
  skipped: number;
  /** Lines over SCHEDULE_ITEMS_MAX. */
  dropped: number;
}

/**
 * Adds pasted lines as items. A header line names the columns (title, start, end, location,
 * subtitle and a few synonyms); otherwise the cells are title, start, end, location. A time
 * without a date is on the day of the line before (or of the last item, or `today`), rolling to
 * the next day when it is earlier than the line before. A missing end keeps the length of the line
 * before (DEFAULT_MINUTES at first); an end time that is not after the start is on the next day.
 */
export function applySchedulePaste(
  doc: ScheduleDoc,
  lines: readonly (readonly string[])[],
  today: string,
  makeId: () => string = newItemId,
): SchedulePasteResult {
  const head = lines.length > 0 ? scheduleHeader(lines[0]) : null;
  const body = head ? lines.slice(1) : lines;
  const keys = head ?? DEFAULT_ORDER;
  const last = lastTimed(doc.items);
  let day = last ? datePart(last.start) : today;
  let prevStart = -Infinity;
  let len = last ? Math.max(1, duration(last)) : DEFAULT_MINUTES;
  const room = Math.max(0, SCHEDULE_ITEMS_MAX - doc.items.length);
  const added: ScheduleItem[] = [];
  let skipped = 0;
  let dropped = 0;

  for (const line of body) {
    const cell: Partial<Record<PasteKey, string>> = {};
    line.forEach((v, i) => {
      const k = keys[i];
      if (k && cell[k] === undefined) cell[k] = v.trim();
    });
    const title = cell.title ?? "";
    let start = cell.start ? parseWhen(cell.start, day) : null;
    if (title === "" || !start) {
      skipped++;
      continue;
    }
    if (added.length >= room) {
      dropped++;
      continue;
    }
    if (!hasDate(cell.start ?? "") && toMinutes(start) < prevStart) start = addMinutes(start, 24 * 60);
    let end = cell.end ? parseWhen(cell.end, datePart(start)) : null;
    if (end && !hasDate(cell.end ?? "") && toMinutes(end) <= toMinutes(start)) end = addMinutes(end, 24 * 60);
    if (!end || toMinutes(end) <= toMinutes(start)) end = addMinutes(start, len);
    len = Math.max(1, toMinutes(end) - toMinutes(start));
    day = datePart(start);
    prevStart = toMinutes(start);
    added.push({
      ...blankItem(start, end, makeId()),
      title,
      subtitle: cell.subtitle ?? "",
      location: cell.location ?? "",
    });
  }
  return {
    doc: withItems(doc, [...doc.items, ...added]),
    header: head !== null,
    added: added.length,
    skipped,
    dropped,
  };
}

// ---- Validation ---------------------------------------------------------------------------------------

export interface ItemIssues {
  title: string | null;
  start: string | null;
  end: string | null;
  /** A text over the byte limit. */
  text: string | null;
}

export interface ScheduleIssues {
  name: string | null;
  /** Only items with a problem. */
  items: Map<string, ItemIssues>;
  labels: Partial<Record<keyof ScheduleLabels, string>>;
  table: string | null;
}

export function scheduleIssues(doc: ScheduleDoc): ScheduleIssues {
  const trimmed = doc.name.trim();
  const name =
    trimmed === ""
      ? "Enter a name."
      : [...trimmed].length > SOURCE_NAME_MAX
        ? `Up to ${SOURCE_NAME_MAX} characters.`
        : null;
  const items = new Map<string, ItemIssues>();
  for (const it of doc.items) {
    const title = it.title.trim() === "" ? "Enter a title." : null;
    const start = validDateTime(it.start) ? null : "Enter a date and time.";
    let end: string | null = null;
    if (!validDateTime(it.end)) end = "Enter an end time.";
    else if (!start && toMinutes(it.end) <= toMinutes(it.start)) end = "The end must be after the start.";
    else if (!start && duration(it) > DURATION_MAX) end = "Up to 7 days long.";
    const text = [it.title, it.subtitle, it.location].some((v) => byteLength(v) > SOURCE_CELL_MAX)
      ? `Up to ${SOURCE_CELL_MAX / 1024} KB of text.`
      : null;
    if (title || start || end || text) items.set(it.id, { title, start, end, text });
  }
  const labels: ScheduleIssues["labels"] = {};
  for (const { key } of LABEL_KEYS) {
    // An empty label is allowed: that status then shows no badge (upcoming has none by default).
    const v = doc.labels[key]?.trim() ?? "";
    if ([...v].length > LABEL_MAX) labels[key] = `Up to ${LABEL_MAX} characters.`;
  }
  const table = doc.items.length > SCHEDULE_ITEMS_MAX ? `Up to ${SCHEDULE_ITEMS_MAX} items.` : null;
  return { name, items, labels, table };
}

/** Why the working copy can't be saved, or null. */
export function scheduleBlocked(doc: ScheduleDoc, issues: ScheduleIssues = scheduleIssues(doc)): string | null {
  if (issues.name) return `Name: ${issues.name}`;
  if (issues.table) return issues.table;
  for (const it of doc.items) {
    const e = issues.items.get(it.id);
    if (!e) continue;
    return `${it.title.trim() || "An item"}: ${e.title ?? e.start ?? e.end ?? e.text}`;
  }
  const label = LABEL_KEYS.find(({ key }) => issues.labels[key]);
  if (label) return `${label.label} label: ${issues.labels[label.key]}`;
  return null;
}

// ---- Display ---------------------------------------------------------------------------------------------

export type Tally = "program" | "preview" | "warning" | "idle";

/** How a status shows: the tally dot, the label key and whether it is struck through. */
export const STATUS_LOOK: Record<ItemStatus, { tally: Tally; label: keyof ScheduleLabels; strike?: boolean }> = {
  now: { tally: "program", label: "now" },
  next: { tally: "preview", label: "next" },
  delayed: { tally: "warning", label: "delayed" },
  previous: { tally: "idle", label: "previous" },
  cancelled: { tally: "idle", label: "cancelled", strike: true },
  upcoming: { tally: "idle", label: "upcoming" },
};

export { itemsLabel } from "../model";

/** The list entry as it is after saving `doc` (NOW / NEXT from the last view). */
export function scheduleSummary(
  doc: ScheduleDoc,
  usedBy: readonly string[],
  view: Pick<ScheduleView, "nowId" | "nextId"> | null,
): SourceSummary {
  return {
    id: doc.id,
    name: doc.name,
    kind: "schedule",
    rowCount: doc.items.length,
    index: 0,
    current: {},
    columns: SCHEDULE_COLUMNS.map((c) => ({ ...c })),
    usedBy: [...usedBy],
    updatedAt: doc.updatedAt,
    auto: null,
    nextAt: null,
    nowId: view?.nowId ?? null,
    nextId: view?.nextId ?? null,
  };
}
