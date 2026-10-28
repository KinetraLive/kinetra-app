// Pure data-table logic of the Sources editor: validation against the backend's limits and display
// helpers (row and column edits are the sheet's: $lib/components/sheet). No Svelte, no I/O.

import {
  SOURCE_CELL_MAX,
  SOURCE_COLUMNS_MAX,
  SOURCE_NAME_MAX,
  SOURCE_ROWS_MAX,
  TEMPLATE_KEY_PATTERN,
  type SourceDoc,
  type SourceSummary,
} from "$lib/types";

export { deriveKey } from "./keys";

const clampIndex = (index: number, rows: number) => (rows === 0 ? 0 : Math.min(Math.max(0, index), rows - 1));

// ---- Validation ---------------------------------------------------------------------------------

let encoder: TextEncoder | undefined;
/** UTF-8 byte length (the backend's cell limit is in bytes). */
export function byteLength(s: string): number {
  // Fast path: ASCII-only strings shorter than the limit never need encoding.
  if (s.length * 3 <= SOURCE_CELL_MAX) return s.length;
  encoder ??= new TextEncoder();
  return encoder.encode(s).length;
}

export interface SourceIssues {
  name: string | null;
  /** Per column: what's wrong with its label/key, or null. */
  columns: (string | null)[];
  /** Too many columns or rows. */
  table: string | null;
  /** Cells over the byte limit, as "row:key" (row index, 0-based). */
  cells: Set<string>;
}

export function sourceIssues(doc: SourceDoc): SourceIssues {
  const trimmed = doc.name.trim();
  const name =
    trimmed === ""
      ? "Enter a name."
      : [...trimmed].length > SOURCE_NAME_MAX
        ? `Up to ${SOURCE_NAME_MAX} characters.`
        : null;

  const counts = new Map<string, number>();
  for (const c of doc.columns) counts.set(c.key, (counts.get(c.key) ?? 0) + 1);
  const columns = doc.columns.map((c) => {
    if (c.label.trim() === "") return "Enter a label.";
    if (!TEMPLATE_KEY_PATTERN.test(c.key)) return "The key must start with a letter: a–z, 0–9, _ or -.";
    if ((counts.get(c.key) ?? 0) > 1) return "Another column has this key.";
    return null;
  });

  let table: string | null = null;
  if (doc.columns.length === 0) table = "Add at least one column.";
  else if (doc.columns.length > SOURCE_COLUMNS_MAX) table = `Up to ${SOURCE_COLUMNS_MAX} columns.`;
  else if (doc.rows.length > SOURCE_ROWS_MAX) table = `Up to ${SOURCE_ROWS_MAX} rows.`;

  const cells = new Set<string>();
  doc.rows.forEach((r, i) => {
    for (const [k, v] of Object.entries(r)) if (byteLength(v) > SOURCE_CELL_MAX) cells.add(`${i}:${k}`);
  });
  return { name, columns, table, cells };
}

/** Why the working copy can't be saved, or null. */
export function blockedReason(doc: SourceDoc, issues: SourceIssues = sourceIssues(doc)): string | null {
  if (issues.name) return `Name: ${issues.name}`;
  const ci = issues.columns.findIndex((e) => e !== null);
  if (ci >= 0) return `${doc.columns[ci].label.trim() || `Column ${ci + 1}`}: ${issues.columns[ci]}`;
  if (issues.table) return issues.table;
  if (issues.cells.size > 0) {
    const first = [...issues.cells][0];
    const row = Number(first.split(":")[0]) + 1;
    return `Row ${row}: a cell is over ${SOURCE_CELL_MAX / 1024} KB.`;
  }
  return null;
}

// ---- Display ------------------------------------------------------------------------------------

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export const rowsLabel = (n: number) => plural(n, "row", "rows");
export const itemsLabel = (n: number) => plural(n, "item", "items");

export function usedByLabel(n: number): string {
  return n === 0 ? "Not used" : `Used by ${plural(n, "screen", "screens")}`;
}

const KIND_LABEL = { text: "Data table", schedule: "Schedule", api: "API", poll: "Poll", socket: "Live data" } as const;

/** What a source is called in lists: an API source fetching CSV is a "CSV link" (as in New source). */
export function kindLabel(s: Pick<SourceSummary, "kind" | "format">): string {
  if (s.kind === "api" && s.format === "csv") return "CSV link";
  return KIND_LABEL[s.kind] ?? s.kind;
}

/** Sources sorted by name (case-insensitive), ties by id. */
export function sortSources(list: readonly SourceSummary[]): SourceSummary[] {
  return [...list].sort(
    (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }) || a.id.localeCompare(b.id),
  );
}

/** A valid source name: 1–SOURCE_NAME_MAX characters after trimming. */
export function validSourceName(name: string): boolean {
  const n = [...name.trim()].length;
  return n >= 1 && n <= SOURCE_NAME_MAX;
}

/** "Source N" with the smallest N ≥ count + 1 that no existing source uses. */
export function defaultSourceName(names: readonly string[]): string {
  const taken = new Set(names.map((n) => n.trim().toLowerCase()));
  let n = names.length + 1;
  while (taken.has(`source ${n}`)) n++;
  return `Source ${n}`;
}

/** The summary as it is after saving `doc` (the list updates without a refetch). */
export function summaryOf(doc: SourceDoc, usedBy: readonly string[]): SourceSummary {
  const index = clampIndex(doc.index, doc.rows.length);
  return {
    id: doc.id,
    name: doc.name,
    kind: doc.kind,
    rowCount: doc.rows.length,
    index,
    current: doc.rows[index] ?? {},
    columns: doc.columns,
    usedBy: [...usedBy],
    updatedAt: doc.updatedAt,
    auto: doc.auto ?? null,
    // Only the server knows when an armed rotation steps next.
    nextAt: null,
    nowId: null,
    nextId: null,
    format: doc.kind === "api" ? doc.format : null,
  };
}
