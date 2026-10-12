// Row selection of the sheet: click, Shift range (from the anchor), Ctrl toggle, all. Pure and
// immutable: every operation returns a new selection.

export interface RowSelection {
  rows: ReadonlySet<number>;
  /** Where a Shift range starts (the last plain or Ctrl click). */
  anchor: number | null;
}

export const NONE: RowSelection = { rows: new Set(), anchor: null };

export const selectOne = (i: number): RowSelection => ({ rows: new Set([i]), anchor: i });

/** Ctrl+click: toggles row `i`; the anchor moves to it. */
export function toggle(sel: RowSelection, i: number): RowSelection {
  const rows = new Set(sel.rows);
  if (rows.has(i)) rows.delete(i);
  else rows.add(i);
  return { rows, anchor: i };
}

/** Shift+click: the rows from the anchor to `i` (the anchor stays). No anchor: just `i`. */
export function range(sel: RowSelection, i: number): RowSelection {
  const a = sel.anchor ?? i;
  const rows = new Set<number>();
  for (let r = Math.min(a, i); r <= Math.max(a, i); r++) rows.add(r);
  return { rows, anchor: a };
}

/** Ctrl+A. */
export function all(n: number): RowSelection {
  const rows = new Set<number>();
  for (let r = 0; r < n; r++) rows.add(r);
  return { rows, anchor: n > 0 ? 0 : null };
}

/** The selected rows in ascending order. */
export const sorted = (sel: RowSelection): number[] => [...sel.rows].sort((a, b) => a - b);

/**
 * The selection after the rows changed: `origin[i]` is the old index of new row `i` (-1 = new).
 * Selected rows that still exist stay selected at their new index; so does the anchor.
 */
export function remap(sel: RowSelection, origin: readonly number[]): RowSelection {
  if (sel.rows.size === 0 && sel.anchor === null) return sel;
  const rows = new Set<number>();
  let anchor: number | null = null;
  origin.forEach((o, i) => {
    if (o < 0) return;
    if (sel.rows.has(o)) rows.add(i);
    if (o === sel.anchor) anchor = i;
  });
  return { rows, anchor };
}
