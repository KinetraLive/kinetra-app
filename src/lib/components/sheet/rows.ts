// Row and column edits of the sheet. Pure and immutable. Row edits that move rows return an
// `order`: new row i is old row order[i] (-1 = a new row), which the editor turns into the
// `origin` it reports, the selection remap and the row identities undo keeps.

import { deriveKey } from "$lib/components/sources/keys";
import { TEMPLATE_KEY_PATTERN, type ColumnKind, type SourceColumn } from "$lib/types";
import type { SheetRow } from "./types";

export const identity = (n: number): number[] => Array.from({ length: n }, (_, i) => i);

/** `items` in `order` (order[i] = old index; -1 takes `fill()`). */
export function permute<T>(items: readonly T[], order: readonly number[], fill: () => T): T[] {
  return order.map((o) => (o < 0 ? fill() : items[o]));
}

/**
 * The order that moves the `picked` rows (any order, unique) as one block so it lands before old
 * row `before` (0..n; n = the end). Rows keep their relative order.
 */
export function moveOrder(n: number, picked: readonly number[], before: number): number[] {
  const set = new Set(picked.filter((i) => i >= 0 && i < n));
  const block = [...set].sort((a, b) => a - b);
  const rest: number[] = [];
  for (let i = 0; i < n; i++) if (!set.has(i)) rest.push(i);
  let at = 0;
  while (at < rest.length && rest[at] < before) at++;
  return [...rest.slice(0, at), ...block, ...rest.slice(at)];
}

/** Alt+↑ / Alt+↓ on the picked rows: one step (null when they are already at that end). */
export function nudgeOrder(n: number, picked: readonly number[], dir: -1 | 1): number[] | null {
  if (picked.length === 0) return null;
  const lo = Math.min(...picked);
  const hi = Math.max(...picked);
  if (dir < 0 && lo === 0) return null;
  if (dir > 0 && hi === n - 1) return null;
  return moveOrder(n, picked, dir < 0 ? lo - 1 : hi + 2);
}

export const isIdentity = (order: readonly number[]) => order.every((o, i) => o === i);

/** The order that removes `picked` rows. */
export function removeOrder(n: number, picked: readonly number[]): number[] {
  const set = new Set(picked);
  return identity(n).filter((i) => !set.has(i));
}

/** The order that inserts `count` new rows at `at`. */
export function insertOrder(n: number, at: number, count = 1): number[] {
  const o = identity(n);
  o.splice(Math.min(Math.max(0, at), n), 0, ...Array<number>(count).fill(-1));
  return o;
}

export function setCell(rows: readonly SheetRow[], i: number, key: string, value: string): SheetRow[] {
  return rows.map((r, j) => (j === i ? { ...r, [key]: value } : r));
}

/** Sets several cells: `cells` as [row, key][] all to "" (Delete on cells). */
export function clearCells(rows: readonly SheetRow[], cells: ReadonlyArray<readonly [number, string]>): SheetRow[] {
  const byRow = new Map<number, string[]>();
  for (const [r, k] of cells) byRow.set(r, [...(byRow.get(r) ?? []), k]);
  return rows.map((r, i) => {
    const keys = byRow.get(i);
    if (!keys || keys.every((k) => !r[k])) return r;
    const next = { ...r };
    for (const k of keys) next[k] = "";
    return next;
  });
}

// ---- Columns ------------------------------------------------------------------------------------

export interface Table {
  rows: SheetRow[];
  columns: SourceColumn[];
}

/** Adds "Column N" at the end. */
export function addColumn(t: Table, label = `Column ${t.columns.length + 1}`): Table {
  const key = deriveKey(label, t.columns.map((c) => c.key));
  return { rows: t.rows, columns: [...t.columns, { key, label }] };
}

/** Renames cells of key `from` to `to` in every row. */
function rekeyRows(rows: readonly SheetRow[], from: string, to: string): SheetRow[] {
  return rows.map((r) => {
    if (!Object.hasOwn(r, from)) return r;
    const { [from]: v, ...rest } = r;
    return { ...rest, [to]: v };
  });
}

/** Sets column `i`'s label; with `rekey` its key follows the label (cells move along). */
export function setLabel(t: Table, i: number, label: string, rekey: boolean): Table {
  const col = t.columns[i];
  if (!col) return t;
  if (!rekey) return { rows: t.rows, columns: t.columns.map((c, j) => (j === i ? { ...c, label } : c)) };
  const key = deriveKey(label, t.columns.filter((_, j) => j !== i).map((c) => c.key));
  const columns = t.columns.map((c, j) => (j === i ? { ...c, key, label } : c));
  return { columns, rows: key === col.key ? t.rows : rekeyRows(t.rows, col.key, key) };
}

/** Why `key` can't be column `i`'s key, or null. */
export function keyIssue(t: Table, i: number, key: string): string | null {
  if (!TEMPLATE_KEY_PATTERN.test(key)) return "A key starts with a letter: a–z, 0–9, _ or -, up to 32.";
  if (t.columns.some((c, j) => j !== i && c.key === key)) return "Another column has this key.";
  return null;
}

/** Sets column `i`'s key (cells move along); unchanged when the key is not valid. */
export function setKey(t: Table, i: number, key: string): Table {
  const col = t.columns[i];
  if (!col || col.key === key || keyIssue(t, i, key)) return t;
  return {
    columns: t.columns.map((c, j) => (j === i ? { ...c, key } : c)),
    rows: rekeyRows(t.rows, col.key, key),
  };
}

/** Sets column `i`'s kind (text is stored as absent). */
export function setKind(t: Table, i: number, kind: ColumnKind): Table {
  const col = t.columns[i];
  if (!col || (col.kind ?? "text") === kind) return t;
  const columns = t.columns.map((c, j) => {
    if (j !== i) return c;
    if (kind === "image") return { ...c, kind };
    const { kind: _drop, ...rest } = c;
    return rest;
  });
  return { rows: t.rows, columns };
}

/** Removes column `i` and its cells. */
export function removeColumn(t: Table, i: number): Table {
  const col = t.columns[i];
  if (!col) return t;
  return {
    columns: t.columns.filter((_, j) => j !== i),
    rows: t.rows.map((r) => {
      if (!Object.hasOwn(r, col.key)) return r;
      const { [col.key]: _drop, ...rest } = r;
      return rest;
    }),
  };
}

/** Moves column `i` one place left (-1) or right (+1). */
export function shiftColumn(t: Table, i: number, dir: 1 | -1): Table {
  const j = i + dir;
  if (i < 0 || j < 0 || i >= t.columns.length || j >= t.columns.length) return t;
  const columns = t.columns.slice();
  [columns[i], columns[j]] = [columns[j], columns[i]];
  return { rows: t.rows, columns };
}

/** True when two tables hold the same data (cells compared over the columns). */
export function sameTable(a: Table, b: Table): boolean {
  if (a.rows.length !== b.rows.length || a.columns.length !== b.columns.length) return false;
  for (let i = 0; i < a.columns.length; i++) {
    const x = a.columns[i];
    const y = b.columns[i];
    if (x.key !== y.key || x.label !== y.label || (x.kind ?? "text") !== (y.kind ?? "text")) return false;
  }
  for (let i = 0; i < a.rows.length; i++) {
    const r = a.rows[i];
    const s = b.rows[i];
    if (r === s) continue;
    for (const c of a.columns) if ((r[c.key] ?? "") !== (s[c.key] ?? "")) return false;
  }
  return true;
}
