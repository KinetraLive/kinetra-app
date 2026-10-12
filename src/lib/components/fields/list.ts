// `list` field values: a JSON array of row objects (column key → text), carried as the field's
// string value. Pure: no Svelte, no DOM. Editing (and the server limits) lives in the spreadsheet
// editor (sheet/).

import type { SourceColumn } from "$lib/types";

export type ListRow = Record<string, string>;

/**
 * The rows of a list value. Anything that isn't an array of objects reads as no rows; cells are
 * kept as text (numbers and booleans stringified, other values dropped).
 */
export function parseRows(value: string | undefined): ListRow[] {
  if (!value) return [];
  let raw: unknown;
  try {
    raw = JSON.parse(value);
  } catch {
    return [];
  }
  if (!Array.isArray(raw)) return [];
  const rows: ListRow[] = [];
  for (const r of raw) {
    if (!r || typeof r !== "object" || Array.isArray(r)) continue;
    const row: ListRow = {};
    for (const [k, v] of Object.entries(r as Record<string, unknown>)) {
      if (typeof v === "string") row[k] = v;
      else if (typeof v === "number" || typeof v === "boolean") row[k] = String(v);
    }
    rows.push(row);
  }
  return rows;
}

/** Row count of a list value without keeping the rows (for the collapsed summary). */
export function rowCount(value: string | undefined): number {
  return parseRows(value).length;
}

/** The value for `rows`: only the columns' keys, in column order; empty cells left out. */
export function serializeRows(rows: readonly ListRow[], columns: readonly SourceColumn[]): string {
  return JSON.stringify(
    rows.map((r) => {
      const out: ListRow = {};
      for (const c of columns) if (r[c.key]) out[c.key] = r[c.key];
      return out;
    }),
  );
}
