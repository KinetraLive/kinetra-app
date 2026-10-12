// Linking a `list` field to a data table, and unlinking it. Linking is a binding
// `{ sourceId, column: "" }` (optional sort / limit): the list shows every row of the table,
// columns matched by key, and several layers can share one table. Unlinking keeps a copy of what
// the list showed (the table's rows, sorted and limited as the binding said, mapped onto the
// field's columns by key) as the layer's own value. Pure; `unlinkedValue` loads the table through
// the loader it is given.

import {
  LIST_ROWS_MAX,
  LIST_VALUE_MAX,
  type SourceBinding,
  type SourceColumn,
  type SourceDoc,
  type SourceSummary,
} from "$lib/types";

type Row = Record<string, string>;

let encoder: TextEncoder | undefined;
const bytes = (s: string) => (encoder ??= new TextEncoder()).encode(s).length;

/** Rows ordered like the server orders a bound list (stable; numbers first when numeric). */
export function sortRows(rows: readonly Row[], sort: SourceBinding["sort"]): Row[] {
  const out = rows.slice();
  if (!sort) return out;
  const dir = sort.desc ? -1 : 1;
  if (sort.numeric) {
    const num = (r: Row) => {
      const v = r[sort.column]?.trim();
      const n = v === undefined || v === "" ? NaN : Number(v);
      return Number.isFinite(n) ? n : null;
    };
    out.sort((a, b) => {
      const x = num(a);
      const y = num(b);
      if (x !== null && y !== null) return (x - y) * dir;
      if (x !== null) return -1;
      if (y !== null) return 1;
      return 0;
    });
  } else {
    const text = (r: Row) => (r[sort.column] ?? "").toLowerCase();
    out.sort((a, b) => {
      const x = text(a);
      const y = text(b);
      return (x < y ? -1 : x > y ? 1 : 0) * dir;
    });
  }
  return out;
}

export interface UnlinkCopy {
  /** The list value (JSON) to keep. */
  value: string;
  /** Rows the copy couldn't keep (over LIST_ROWS_MAX or 64 KB), said out loud; null = none. */
  note: string | null;
}

/** The copy a list keeps when it is unlinked from a data table with these rows. */
export function unlinkCopy(rows: readonly Row[], binding: SourceBinding, columns: readonly SourceColumn[]): UnlinkCopy {
  const sourceKeys = new Set(rows.flatMap((r) => Object.keys(r)));
  const sort = binding.sort && sourceKeys.has(binding.sort.column) ? binding.sort : undefined;
  let order = sortRows(rows, sort);
  const cap = Math.min(binding.limit ?? LIST_ROWS_MAX, LIST_ROWS_MAX);
  let note: string | null = null;
  if (order.length > cap) {
    if (binding.limit === undefined || binding.limit > LIST_ROWS_MAX)
      note = `A list keeps up to ${LIST_ROWS_MAX} rows: the first ${LIST_ROWS_MAX} of ${order.length} were kept.`;
    order = order.slice(0, cap);
  }
  const mapped = order.map((r) => {
    const o: Row = {};
    for (const c of columns) if (r[c.key]) o[c.key] = r[c.key];
    return o;
  });
  let value = JSON.stringify(mapped);
  if (bytes(value) > LIST_VALUE_MAX) {
    const all = mapped.length;
    while (mapped.length > 0 && bytes(value) > LIST_VALUE_MAX) {
      mapped.pop();
      value = JSON.stringify(mapped);
    }
    note = `A list holds up to 64 KB: the first ${mapped.length} of ${all} rows were kept.`;
  }
  return { value, note };
}

/**
 * The copy for unlinking `binding` (null when there is nothing to copy: not a whole-source
 * binding, or the source is not a data table). `load` is getSource from $lib/api/sources.
 */
export async function unlinkedValue(
  binding: SourceBinding | undefined,
  columns: readonly SourceColumn[],
  sources: readonly SourceSummary[] | null,
  load: (id: string) => Promise<SourceDoc>,
): Promise<UnlinkCopy | null> {
  if (!binding || binding.column !== "") return null;
  const summary = sources?.find((s) => s.id === binding.sourceId);
  if (summary && summary.kind !== "text") return null;
  const doc = await load(binding.sourceId);
  if (doc.kind !== "text") return null;
  return unlinkCopy(doc.rows, binding, columns);
}
