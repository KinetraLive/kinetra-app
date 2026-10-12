// Editing a list field's columns in the template editor (loaded with ListColumns, a lazy chunk).
// Pure: no Svelte, no I/O.

import type { FieldDef, SourceColumn } from "$lib/types";
import { deriveKey } from "$lib/components/sources/model";
import { parseRows } from "$lib/components/fields/list";
import { replaceAt } from "./model";

/** A new column for a list field, its key derived from the label and unique among `columns`. */
export function newColumn(columns: readonly SourceColumn[]): SourceColumn {
  const label = `Column ${columns.length + 1}`;
  return { key: deriveKey(label, columns.map((c) => c.key)), label };
}

/**
 * Column `i` relabelled. Its key follows the label while it still is the key derived from the old
 * label (so typing a label names the key); a key edited by hand stays.
 */
export function relabelColumn(columns: readonly SourceColumn[], i: number, label: string): SourceColumn[] {
  const c = columns[i];
  const others = columns.filter((_, j) => j !== i).map((x) => x.key);
  const follows = c.key === deriveKey(c.label, others);
  return replaceAt(columns, i, { key: follows ? deriveKey(label, others) : c.key, label });
}

/** The field's default with list rows reduced to its columns (keeps the default valid). */
export function listDefault(f: FieldDef, columns: readonly SourceColumn[]): string {
  const keys = new Set(columns.map((c) => c.key));
  const rows = parseRows(f.default).map((r) => Object.fromEntries(Object.entries(r).filter(([k]) => keys.has(k))));
  return JSON.stringify(rows);
}

