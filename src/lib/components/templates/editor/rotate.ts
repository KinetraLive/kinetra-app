// The template editor's preview of a slideshow-like template. On air, a source's auto-advance moves
// the `_current` row mark (`data-current` on the row) and the template's CSS animates the change.
// The preview has no source, so without this nothing ever moves and the slide transition can't be
// seen: while the preview is on air, the editor moves the mark through the preview rows itself, at
// the template's own pace (its `interval` variable, in seconds).

import type { FieldDef, VariableDef } from "$lib/types";

/** Default seconds per row without an `interval` variable. */
export const ROTATE_DEFAULT_S = 5;

/** The template styles the current row (`[data-current…]` in its CSS). */
export const usesCurrent = (css: string): boolean => /\[\s*data-current\b/.test(css);

/** The rows of a list value, or [] when it isn't a JSON array of objects. */
function rows(value: string): Record<string, unknown>[] {
  try {
    const a: unknown = JSON.parse(value);
    return Array.isArray(a) ? a.filter((r): r is Record<string, unknown> => !!r && typeof r === "object") : [];
  } catch {
    return [];
  }
}

/** The list keys among `fields` whose preview value has at least two rows. */
export function rotatingLists(fields: readonly FieldDef[], values: Readonly<Record<string, string>>): string[] {
  return fields.filter((f) => f.type === "list" && rows(values[f.key] ?? "").length >= 2).map((f) => f.key);
}

/** `value` with only row `i` (mod the row count) marked `_current: "true"`. */
export function markRow(value: string, i: number): string {
  const r = rows(value);
  if (!r.length) return value;
  const at = ((i % r.length) + r.length) % r.length;
  return JSON.stringify(
    r.map((row, j) => {
      const { _current: _, ...rest } = row;
      return j === at ? { ...rest, _current: "true" } : rest;
    }),
  );
}

/** Milliseconds per row: the `interval` variable's value (seconds, 1–120), else the default. */
export function rotateMs(variables: readonly VariableDef[]): number {
  const v = variables.find((x) => x.key === "interval");
  const s = Number(v?.default);
  return Math.round(Number.isFinite(s) && s > 0 ? Math.min(Math.max(s, 1), 120) * 1000 : ROTATE_DEFAULT_S * 1000);
}
