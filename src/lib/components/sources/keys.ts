// Column keys from labels (shared by the Sources editor, the sheet and the template editor). Pure.

const KEY_MAX = 32;

/**
 * A column key from a label: lowercase ASCII, runs of anything outside [a-z0-9_-] become one `_`,
 * starting with a letter (leading digits/symbols dropped; "col" when nothing is left), at most 32
 * characters, and unique against `taken` (`_2`, `_3`, … appended within the 32).
 */
export function deriveKey(label: string, taken: ReadonlySet<string> | readonly string[] = []): string {
  const used = taken instanceof Set ? taken : new Set(taken as readonly string[]);
  let base = label
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "_")
    .replace(/^[^a-z]+/, "")
    .replace(/_+$/, "")
    .slice(0, KEY_MAX)
    .replace(/_+$/, "");
  if (base === "") base = "col";
  if (!used.has(base)) return base;
  for (let n = 2; ; n++) {
    const suffix = `_${n}`;
    const key = `${base.slice(0, KEY_MAX - suffix.length)}${suffix}`;
    if (!used.has(key)) return key;
  }
}
