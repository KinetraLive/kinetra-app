// Pure template-variant logic: which fields a variant's HTML uses, effective variable values, and
// keeping every variant's `vars` in step with edits to the shared variable definitions.
// No Svelte, no I/O.

import { VARIANT_NAME_MAX, type FieldDef, type VariableDef, type VariantDoc } from "$lib/types";
import { valueIssue } from "./model";

/** `{{ key }}` as the renderer binds it (text and attributes); `.col` / `#` are row cells. */
const BIND = /\{\{\s*([\w.-]+|#)\s*\}\}/g;
/** `data-each="key"`: a list row template. */
const EACH = /\bdata-each\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))/gi;

/**
 * Field keys `html` references: `{{key}}` in text or an attribute (also inside list rows, which
 * the renderer forwards to every row) and `data-each="key"`. Clock, timer, image and html fields
 * bind with `{{key}}` too, so this covers every field type.
 */
export function usedKeys(html: string): Set<string> {
  const out = new Set<string>();
  for (const m of html.matchAll(BIND)) if (m[1] !== "#" && m[1][0] !== ".") out.add(m[1]);
  for (const m of html.matchAll(EACH)) out.add((m[1] ?? m[2] ?? m[3]).trim());
  return out;
}

/** Fields the variant's HTML never references, in field order. */
export function unusedFields(html: string, fields: readonly FieldDef[]): FieldDef[] {
  const used = usedKeys(html);
  return fields.filter((f) => !used.has(f.key));
}

/** Variable definitions with `default` replaced by the variant's own value where it has one. */
export function effectiveVariables(defs: readonly VariableDef[], vars: Record<string, string>): VariableDef[] {
  return defs.map((d) => (d.key in vars && vars[d.key] !== d.default ? ({ ...d, default: vars[d.key] } as VariableDef) : d));
}

/** The variant's values as `{ key: value }` for every defined variable (preview drafts). */
export function effectiveValues(defs: readonly VariableDef[], vars: Record<string, string>): Record<string, string> {
  return Object.fromEntries(defs.map((d) => [d.key, vars[d.key] ?? d.default]));
}

/** `vars` restricted to defined keys (stale keys of removed variables are not saved). */
export function pruneVars(defs: readonly VariableDef[], vars: Record<string, string>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const d of defs) if (d.key in vars) out[d.key] = vars[d.key];
  return out;
}

export interface DefsResult {
  /** The shared definitions to keep (their `default` stays the definition default). */
  defs: VariableDef[];
  /** Every variant, with `vars` renamed/dropped/updated as needed. */
  variants: VariantDoc[];
  /** Ids of variants whose `vars` changed (they need saving). */
  changed: Set<string>;
}

/**
 * Applies a definitions edit from the Variables tab. The form works on `shown` (the definitions
 * with the active variant's values as `default`) and hands back `next`: unchanged entries keep
 * their identity, an edited one is replaced in place, an added one is appended.
 *
 * - A renamed key is renamed in every variant's `vars`.
 * - A changed kind (its value reset) drops the key from every variant; the new value becomes the
 *   definition default.
 * - Other edits (label, range bounds, options) keep the definition default; the shown value
 *   stays the active variant's.
 * - A new variable's value is its definition default.
 */
export function applyDefs(
  defs: readonly VariableDef[],
  shown: readonly VariableDef[],
  next: readonly VariableDef[],
  variants: readonly VariantDoc[],
  active: string,
): DefsResult {
  let list = variants.map((v) => ({ ...v, vars: { ...v.vars } }));
  const changed = new Set<string>();
  const set = (id: string, key: string, value: string | undefined) => {
    const v = list.find((x) => x.id === id);
    if (!v || v.vars[key] === value) return;
    if (value === undefined) delete v.vars[key];
    else v.vars[key] = value;
    changed.add(id);
  };
  const sameLength = shown.length === next.length;
  const out = next.map((n, i): VariableDef => {
    const at = shown.indexOf(n);
    if (at >= 0) return defs[at]; // untouched (moved or not)
    if (!sameLength || i >= shown.length) return n; // added
    const o = shown[i];
    const d = defs[i];
    if (o.key !== n.key) {
      for (const v of list)
        if (o.key in v.vars) {
          const value = v.vars[o.key];
          delete v.vars[o.key];
          v.vars[n.key] = value;
          changed.add(v.id);
        }
    }
    if (o.type !== n.type) {
      for (const v of list) set(v.id, n.key, undefined);
      return n;
    }
    // Same kind: the definition keeps its default unless the edit made it invalid (an option
    // removed); the shown value is the active variant's; values the edit invalidated are dropped.
    const def = { ...n, default: d.default } as VariableDef;
    if (valueIssue(def) !== null) def.default = n.default;
    if (n.default !== o.default) set(active, n.key, n.default);
    for (const v of list)
      if (n.key in v.vars && valueIssue({ ...def, default: v.vars[n.key] } as VariableDef) !== null)
        set(v.id, n.key, undefined);
    return def;
  });
  list = list.map((v) => (changed.has(v.id) ? v : (variants.find((x) => x.id === v.id) ?? v)));
  return { defs: out, variants: list, changed };
}

/** Problem with a variant name, or null. `others`: the other variants' names. */
export function variantNameIssue(name: string, others: readonly string[]): string | null {
  const n = name.trim();
  if (n === "") return "Enter a name.";
  if (n.length > VARIANT_NAME_MAX) return `Up to ${VARIANT_NAME_MAX} characters.`;
  if (others.some((o) => o.trim().toLowerCase() === n.toLowerCase())) return "Another variant has this name.";
  return null;
}

/** "Compact copy", "Compact copy 2", ... not used by `names`, within the length limit. */
export function copyName(base: string, names: readonly string[]): string {
  const taken = new Set(names.map((n) => n.toLowerCase()));
  for (let i = 1; ; i++) {
    const suffix = i === 1 ? " copy" : ` copy ${i}`;
    const name = base.slice(0, VARIANT_NAME_MAX - suffix.length) + suffix;
    if (!taken.has(name.toLowerCase())) return name;
  }
}
