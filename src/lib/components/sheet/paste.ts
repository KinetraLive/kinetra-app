// Paste into the sheet: tab-separated text (Excel, Google Sheets) or CSV, through a column
// mapping. Pure: no Svelte, no DOM.
//
//   planPaste   parses the text and proposes a mapping: when the first line names columns (by
//               label or key) it is a header and its cells map to those columns; otherwise the
//               pasted columns map to the sheet's columns in order. Unmatched pasted columns are
//               Ignore (or New column when columns are editable and there is room).
//   applyPaste  the rows after the paste, appended or written over the rows from `at` down (rows
//               past the end are added), or the reason it can't be applied (a limit): nothing is
//               ever cut silently.

import { deriveKey } from "$lib/components/sources/keys";
import { headerMatch, parseDelimited } from "$lib/components/sources/paste";
import type { SourceColumn } from "$lib/types";
import { columnsMax, rowsIssue } from "./limits";
import type { SheetLimits, SheetRow } from "./types";

/** A pasted column's target: a column index, IGNORE, or NEW_COLUMN. */
export const IGNORE = -1;
export const NEW_COLUMN = -2;

export interface PastePlan {
  lines: string[][];
  /** Widest line. */
  width: number;
  /** The first line was detected as a header. */
  header: boolean;
  /** Per pasted column: its name (the header cell, else "Column N"). */
  names: string[];
  /** Per pasted column: the proposed target. */
  map: number[];
}

/** Null when the text holds no cells. */
export function planPaste(text: string, columns: readonly SourceColumn[], newColumns = false, maxColumns = 16): PastePlan | null {
  if (text.trim() === "") return null;
  const lines = parseDelimited(text);
  if (lines.length === 0) return null;
  let width = 0;
  for (const l of lines) if (l.length > width) width = l.length;
  const match = headerMatch(lines[0], columns);
  const header = match !== null;
  const names = Array.from({ length: width }, (_, i) => (header ? (lines[0][i] ?? "").trim() : "") || `Column ${i + 1}`);
  let room = newColumns ? Math.max(0, maxColumns - columns.length) : 0;
  const map = names.map((_, i) => {
    if (match) {
      const m = match[i] ?? -1;
      if (m >= 0) return m;
      if ((lines[0][i] ?? "").trim() !== "" && room > 0) {
        room--;
        return NEW_COLUMN;
      }
      return IGNORE;
    }
    if (i < columns.length) return i;
    if (room > 0) {
      room--;
      return NEW_COLUMN;
    }
    return IGNORE;
  });
  return { lines, width, header, names, map };
}

export interface PasteOptions {
  /** The first line is a header (not pasted as a row). */
  header: boolean;
  /** Per pasted column: a column index, IGNORE or NEW_COLUMN. */
  map: readonly number[];
  mode: "append" | "replace";
  /** replace: the first row written over. */
  at: number;
}

export type PasteOutcome =
  | {
      ok: true;
      rows: SheetRow[];
      columns: SourceColumn[];
      /** Rows added at the end / rows written over. */
      added: number;
      replaced: number;
      /** Order for the editor: new row i is old row order[i] (-1 = new). */
      order: number[];
    }
  | { ok: false; reason: string };

/** The pasted body rows (without the header line). */
export const bodyLines = (plan: PastePlan, header: boolean) => (header ? plan.lines.slice(1) : plan.lines);

export function applyPaste(
  rows: readonly SheetRow[],
  columns: readonly SourceColumn[],
  plan: PastePlan,
  opts: PasteOptions,
  limits: SheetLimits,
): PasteOutcome {
  const cols = columns.slice();
  const taken = new Set(cols.map((c) => c.key));
  const target: (string | null)[] = [];
  for (let i = 0; i < plan.width; i++) {
    const t = opts.map[i] ?? IGNORE;
    if (t === NEW_COLUMN) {
      if (cols.length >= columnsMax(limits)) return { ok: false, reason: `Up to ${columnsMax(limits)} columns.` };
      const label = opts.header ? plan.names[i] : `Column ${cols.length + 1}`;
      const key = deriveKey(label, taken);
      taken.add(key);
      cols.push({ key, label });
      target.push(key);
    } else target.push(t >= 0 && cols[t] ? cols[t].key : null);
  }
  if (target.every((t) => t === null)) return { ok: false, reason: "Map at least one column." };

  const body = bodyLines(plan, opts.header);
  if (body.length === 0) return { ok: false, reason: "Only a header line: there are no rows to paste." };
  const fill = (base: SheetRow, line: readonly string[]): SheetRow => {
    const r = { ...base };
    for (let i = 0; i < target.length; i++) {
      const k = target[i];
      if (k !== null) r[k] = line[i] ?? "";
    }
    return r;
  };

  const start = opts.mode === "append" ? rows.length : Math.min(Math.max(0, opts.at), rows.length);
  const next = rows.slice();
  const order = rows.map((_, i) => i);
  let replaced = 0;
  let added = 0;
  body.forEach((line, k) => {
    const i = start + k;
    if (i < next.length) {
      next[i] = fill(next[i], line);
      replaced++;
    } else {
      next.push(fill({}, line));
      order.push(-1);
      added++;
    }
  });
  const issue = rowsIssue(next, cols, limits, rows.length);
  if (issue) return { ok: false, reason: issue };
  return { ok: true, rows: next, columns: cols, added, replaced, order };
}
