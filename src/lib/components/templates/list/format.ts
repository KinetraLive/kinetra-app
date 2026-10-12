// Display helpers for the templates list. Plain TypeScript so they are unit-testable.

import type { PresetInfo, PresetRef, TemplateSummary } from "$lib/types";

export const NAME_MAX = 100;

/** "From Classic bar", "Blank", or the preset id when the preset list isn't known. */
export function originLabel(preset: PresetRef | null, presets: readonly PresetInfo[]): string {
  if (!preset) return "Blank";
  const p = presets.find((x) => x.id === preset.id);
  return `From ${p?.name ?? preset.id}`;
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "3 fields · 1 variable". */
export function countsLabel(t: Pick<TemplateSummary, "fieldCount" | "variableCount">): string {
  return `${plural(t.fieldCount, "field", "fields")} · ${plural(t.variableCount, "variable", "variables")}`;
}

let abs: Intl.DateTimeFormat | undefined;

/** Absolute local date/time for a `title` attribute; "" for an unparsable timestamp. */
export function absoluteTime(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  abs ??= new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });
  return abs.format(t);
}

/** Templates sorted by name (case-insensitive), so grid positions stay stable while editing. */
export function sortTemplates(list: readonly TemplateSummary[]): TemplateSummary[] {
  return [...list].sort(
    (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }) || a.id.localeCompare(b.id),
  );
}

/** A valid template name: 1–100 characters after trimming. */
export function validName(name: string): boolean {
  const n = name.trim().length;
  return n >= 1 && n <= NAME_MAX;
}

// Grid keyboard helpers moved to $lib/components/grid/keys; re-exported for existing importers.
export { gridColumns, moveIndex } from "$lib/components/grid/keys";
