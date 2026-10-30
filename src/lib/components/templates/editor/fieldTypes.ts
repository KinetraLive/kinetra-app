// List, clock and timer field definitions in the template editor: type switching, list columns,
// clock formats, timer defaults, preview values and their validation. Pure: no Svelte, no I/O.
// Kept out of model.ts, which the layout editor and Control share (pickerHex, valueIssue).

import { LIST_VALUE_MAX, SOURCE_COLUMNS_MAX, type FieldDef, type FieldType, type SourceColumn } from "$lib/types";
import { keyIssue } from "./model";

// Column editing (key derivation) lives in columns.ts and the timer math in fields/timer.ts: both
// load only with the list / timer settings, so this module stays small.

/** A new timer field's default: paused, counting up from 0 (fields/timer.ts TIMER_DEFAULT). */
export const TIMER_DEFAULT_VALUE = '{"run":false,"at":0,"base":0,"dir":"up","end":"stop"}';
/** Longest timer start (99:59:59), as fields/timer.ts TIMER_MAX_MS. */
const TIMER_MAX_MS = (99 * 3600 + 59 * 60 + 59) * 1000;

export const CLOCK_FORMAT_DEFAULT = "HH:mm:ss";
export const CLOCK_FORMAT_MAX = 32;
/** Tokens HH, mm, ss separated by ":", "." or spaces. */
const CLOCK_FORMAT = /^(HH|mm|ss|[:. ])+$/;

/** Problem with a clock format, or null. */
export function clockFormatIssue(format: string): string | null {
  if (format.length === 0) return "Enter a format, e.g. HH:mm:ss.";
  if (format.length > CLOCK_FORMAT_MAX) return `Up to ${CLOCK_FORMAT_MAX} characters.`;
  if (!CLOCK_FORMAT.test(format) || !/HH|mm|ss/.test(format)) return "Use HH, mm and ss with : . or spaces.";
  return null;
}

/** A list field's first column for a new list. */
export const FIRST_COLUMN: SourceColumn = { key: "name", label: "Name" };

/**
 * The field changed to `type`, keeping key and label. Type-specific settings start fresh: a list
 * gets one column and no rows, a clock the default format, a timer a paused count-up at 0; other
 * types drop `columns` / `format` and the old default when it came from those. A preview `sample`
 * is dropped (it was an example of the old type).
 */
export function withFieldType(f: FieldDef, type: FieldType): FieldDef {
  if (type === f.type) return f;
  const base: FieldDef = { key: f.key, label: f.label, type, default: f.default };
  const special = f.type === "list" || f.type === "clock" || f.type === "timer";
  switch (type) {
    case "list":
      return { ...base, columns: [{ ...FIRST_COLUMN }], default: "[]" };
    case "clock":
      return { ...base, format: CLOCK_FORMAT_DEFAULT, default: CLOCK_FORMAT_DEFAULT };
    case "timer":
      return { ...base, default: TIMER_DEFAULT_VALUE };
    default:
      return special ? { ...base, default: "" } : base;
  }
}

export function hasRows(v: string): boolean {
  try {
    const rows: unknown = JSON.parse(v);
    return Array.isArray(rows) && rows.length > 0;
  } catch {
    return false;
  }
}

/** Words that make a text field number-like (whole words of the key or the label). Mirrors
 *  `NUMBER_WORDS` in src-tauri/src/core/fieldvals.rs. */
export const NUMBER_WORDS: readonly string[] = [
  "score", "scores", "points", "pts", "goals", "goal", "boost", "count", "total", "round",
  "rounds", "set", "sets", "game", "games", "lap", "laps", "rank", "position", "pos", "place",
  "number", "num", "kills", "deaths", "assists", "wins", "losses", "saves", "shots", "fouls",
  "percent", "pct", "%", "speed", "age", "year", "series", "quarter", "period", "inning", "frame",
];

const words = (key: string, label: string) =>
  [...key.split(/[^\p{L}\p{N}%]+/u), ...label.split(/[^\p{L}\p{N}%]+/u)].filter(Boolean).map((w) => w.toLowerCase());

/** Whether a key or label names a number (see `NUMBER_WORDS`). */
export const numberWords = (key: string, label: string) => words(key, label).some((w) => NUMBER_WORDS.includes(w));

/** A realistic preview number (mirrors `sample_number` in fieldvals.rs): a place 1, a percentage
 *  45, a round 3, a year 2026, an age 28, a speed 88, else 2. Never the label: a long label in a
 *  number's slot overflowed the graphic. */
export function sampleNumber(key: string, label: string): string {
  for (const w of words(key, label)) {
    if (["rank", "position", "pos", "place"].includes(w)) return "1";
    if (["boost", "percent", "pct", "%"].includes(w)) return "45";
    if (["round", "rounds", "set", "sets", "game", "games", "lap", "laps", "quarter", "period", "inning", "frame", "series"].includes(w)) return "3";
    if (w === "year") return "2026";
    if (w === "age") return "28";
    if (w === "speed") return "88";
  }
  return "2";
}

const isNumeric = (s: string) => s.trim() !== "" && Number.isFinite(Number(s.trim()));

/**
 * The template editor's preview value for a field nobody typed into (mirrors `sample_value` in
 * fieldvals.rs): the field's preview `sample` when it has one (a number for a number field); a
 * number-like field (type number, or a text field named like a number) its
 * numeric default, else a realistic number; other fields their default when they have one; else a
 * placeholder ("[Name]") for text fields, the label for other text-like fields; a list's default
 * rows, else one row of its column labels; a clock's format; a timer's default state.
 */
export function previewDefault(f: FieldDef): string {
  const sample = f.sample?.trim() ? f.sample : undefined;
  if (sample !== undefined && (f.type === "text" || f.type === "multiline" || f.type === "html")) return sample;
  if (sample !== undefined && f.type === "number" && isNumeric(sample)) return sample.trim();
  switch (f.type) {
    case "list":
      return hasRows(f.default)
        ? f.default
        : JSON.stringify([Object.fromEntries((f.columns ?? []).map((c) => [c.key, c.label]))]);
    case "clock":
      return f.format ?? CLOCK_FORMAT_DEFAULT;
    case "timer":
      return f.default;
    case "number":
      return isNumeric(f.default) ? f.default.trim() : sampleNumber(f.key, f.label);
    case "text":
    case "multiline":
      if (numberWords(f.key, f.label)) return isNumeric(f.default) ? f.default.trim() : sampleNumber(f.key, f.label);
      return f.default.trim() ? f.default : `[${f.label}]`;
    default:
      return f.default.trim() ? f.default : f.label;
  }
}

/** Problems with a list / clock / timer field's own settings (mirrors the server). */
export function typeIssues(f: FieldDef): { prop: string; message: string }[] {
  const out: { prop: string; message: string }[] = [];
  if (f.type === "list") {
    const cols = f.columns ?? [];
    if (cols.length === 0) out.push({ prop: "columns", message: "Add a column." });
    else if (cols.length > SOURCE_COLUMNS_MAX)
      out.push({ prop: "columns", message: `Up to ${SOURCE_COLUMNS_MAX} columns.` });
    const seen = new Set<string>();
    cols.forEach((c, j) => {
      const k = keyIssue(c.key);
      if (k) out.push({ prop: `column${j}`, message: `Column ${j + 1}: ${k}` });
      else if (seen.has(c.key))
        out.push({ prop: `column${j}`, message: `Column ${j + 1}: key “${c.key}” is already used.` });
      else if (c.label.trim() === "") out.push({ prop: `column${j}`, message: `Column ${j + 1}: enter a label.` });
      seen.add(c.key);
    });
    if (f.default.length > LIST_VALUE_MAX) out.push({ prop: "default", message: "The default rows are over 64 KB." });
  } else if (f.type === "clock") {
    const m = clockFormatIssue(f.format ?? "");
    if (m) out.push({ prop: "format", message: m });
  } else if (f.type === "timer") {
    let ok = false;
    try {
      const v = JSON.parse(f.default) as Record<string, unknown>;
      ok =
        typeof v === "object" &&
        v !== null &&
        typeof v.base === "number" &&
        v.base >= 0 &&
        v.base <= TIMER_MAX_MS &&
        (v.dir === "up" || v.dir === "down") &&
        (v.end === "stop" || v.end === "hide");
    } catch {
      ok = false;
    }
    if (!ok) out.push({ prop: "default", message: "Set a start time up to 99:59:59." });
  }
  return out;
}

