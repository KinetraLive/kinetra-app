// How a field binding reads in the UI ("Guests · Name", "Guests · all rows, sorted by Score ↓, top 10",
// "Schedule · Up next · Title", "Schedule · all items").
// Pure and dependency-free: shared by the layout editor and Control.

import { SCHEDULE_COLUMNS, type RowSelector, type SourceBinding, type SourceColumn, type SourceSummary } from "$lib/types";

/** What a label needs from a source; `kind` absent reads as a text source. */
export type LabelSource = Pick<SourceSummary, "name" | "columns"> & { kind?: SourceSummary["kind"] };

/** Row selector names as the UI shows them. */
export const ROW_LABEL: Record<RowSelector, string> = { current: "Current", now: "Now", next: "Up next" };

/** The columns a source offers to bindings: a schedule's fixed columns, else its own. */
export function bindableColumns(source: LabelSource | null): readonly SourceColumn[] {
  return source?.kind === "schedule" ? SCHEDULE_COLUMNS : (source?.columns ?? []);
}

/**
 * "Guests · all rows, sorted by Score ↓, top 10" for a list binding (the whole source). Sort is
 * shown with ↑ / ↓; a missing column shows its key. A schedule reads "Schedule · all items".
 */
export function listBindingLabel(source: LabelSource | null, b: SourceBinding): string {
  const parts = [`${source?.name ?? "Missing source"} · ${source?.kind === "schedule" ? "all items" : "all rows"}`];
  if (b.sort) {
    const col = bindableColumns(source).find((c) => c.key === b.sort!.column)?.label ?? `${b.sort.column} (missing)`;
    parts.push(`sorted by ${col} ${b.sort.desc ? "↓" : "↑"}`);
  }
  if (b.limit !== undefined) parts.push(`top ${b.limit}`);
  return parts.join(", ");
}

/** The label of any binding: a list binding (column "") or "Source · Column" ("Source · Row · Column"). */
export function bindingText(source: LabelSource | null, b: SourceBinding): string {
  if (b.column === "") return listBindingLabel(source, b);
  return bindingLabel(source, b.column, b.row);
}

/**
 * "Guests · Name" for a binding; a schedule's NOW / NEXT row adds its name ("Schedule · Up next ·
 * Title"). Null parts fall back to what is known.
 */
export function bindingLabel(source: LabelSource | null, column: string, row?: RowSelector): string {
  const rowPart = row && row !== "current" ? `${ROW_LABEL[row]} · ` : "";
  if (!source) return `Missing source · ${rowPart}${column}`;
  const col = bindableColumns(source).find((c) => c.key === column);
  return `${source.name} · ${rowPart}${col?.label ?? `${column} (missing)`}`;
}
