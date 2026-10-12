// The sheet's limits: rows, bytes of the serialised value, bytes per cell. A change that would
// go past one is refused with a message (never cut silently). Pure.

import { SOURCE_COLUMNS_MAX, type SourceColumn } from "$lib/types";
import type { SheetLimits, SheetRow } from "./types";

let encoder: TextEncoder | undefined;

/** UTF-8 bytes of a string (ASCII fast path). */
export function utf8Bytes(s: string): number {
  // eslint-disable-next-line no-control-regex
  if (!/[^\x00-\x7f]/.test(s)) return s.length;
  encoder ??= new TextEncoder();
  return encoder.encode(s).length;
}

/** Bytes of the rows as a list value (only the columns' keys; empty cells left out). */
export function valueBytes(rows: readonly SheetRow[], columns: readonly SourceColumn[]): number {
  const out = rows.map((r) => {
    const o: SheetRow = {};
    for (const c of columns) if (r[c.key]) o[c.key] = r[c.key];
    return o;
  });
  return utf8Bytes(JSON.stringify(out));
}

export const columnsMax = (limits: SheetLimits) => limits.columns ?? SOURCE_COLUMNS_MAX;

const kb = (n: number) => `${Math.round(n / 1024)} KB`;

/** A size as written: "180 B", "2.4 KB" (one decimal under 10 KB), "37 KB". */
export function sizeText(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 10 * 1024) return `${(Math.round((n / 1024) * 10) / 10).toFixed(1)} KB`;
  return `${Math.round(n / 1024)} KB`;
}

const rowsText = (n: number) => `${n} ${n === 1 ? "row" : "rows"}`;

/** Why a cell value is refused, or null. */
export function cellIssue(value: string, limits: SheetLimits): string | null {
  if (limits.cell !== null && value.length * 3 > limits.cell && utf8Bytes(value) > limits.cell)
    return `A cell holds up to ${kb(limits.cell)}.`;
  return null;
}

/**
 * Why `rows` can't be taken, or null. `before` is the current row count: a change that doesn't
 * add rows is never refused for the row count (an over-limit table can still shrink).
 */
export function rowsIssue(
  rows: readonly SheetRow[],
  columns: readonly SourceColumn[],
  limits: SheetLimits,
  before = 0,
): string | null {
  if (rows.length > limits.rows && rows.length > before) {
    const over = rows.length - limits.rows;
    return `Up to ${limits.rows} rows: this would make ${rows.length} (${rowsText(over)} too many).`;
  }
  if (limits.bytes !== null) {
    const bytes = valueBytes(rows, columns);
    if (bytes > limits.bytes) return `Up to ${kb(limits.bytes)} of data: this would make ${sizeText(bytes)}.`;
  }
  if (limits.cell !== null) {
    for (const r of rows) for (const k in r) if (cellIssue(r[k], limits)) return cellIssue(r[k], limits);
  }
  return null;
}

/** "523 / 1000 rows" (and the bytes when limited): the count against the limit. */
export function countText(rows: readonly SheetRow[], columns: readonly SourceColumn[], limits: SheetLimits): string {
  let s = `${rows.length} / ${limits.rows} rows`;
  if (limits.bytes !== null) s += ` · ${sizeText(valueBytes(rows, columns))} / ${kb(limits.bytes)}`;
  return s;
}
