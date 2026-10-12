// Pure template-editor logic: validation, new rows, value parsing. No Svelte, no I/O.

import {
  TEMPLATE_CSS_MAX,
  TEMPLATE_FIELDS_MAX,
  TEMPLATE_FONTS,
  TEMPLATE_HTML_MAX,
  TEMPLATE_KEY_PATTERN,
  TEMPLATE_VARIABLES_MAX,
  type FieldDef,
  type FieldType,
  type VariableDef,
  type VariableKind,
} from "$lib/types";
import { isColor } from "$lib/components/variables/color";

export const NAME_MAX = 100;
export const AUTO_OUT_DEFAULT_S = 5;
export const AUTO_OUT_MAX_S = 999;

export const FIELD_TYPES: { value: FieldType; label: string }[] = [
  { value: "text", label: "Text" },
  { value: "multiline", label: "Multiline" },
  { value: "image", label: "Image" },
  { value: "video", label: "Video" },
  { value: "color", label: "Color" },
  { value: "number", label: "Number" },
  { value: "html", label: "HTML" },
  { value: "list", label: "List" },
  { value: "clock", label: "Clock" },
  { value: "timer", label: "Timer" },
];

export const VARIABLE_KINDS: { value: VariableKind["type"]; label: string }[] = [
  { value: "color", label: "Color" },
  { value: "font", label: "Font" },
  { value: "range", label: "Range" },
  { value: "select", label: "Select" },
];

export const FONT_OPTIONS = TEMPLATE_FONTS.map((f) => ({ value: f.name, label: f.name }));

export type IssueScope = "field" | "variable" | "html" | "css";

export interface Issue {
  scope: IssueScope;
  /** Row index for field/variable issues, 0 otherwise. */
  index: number;
  /** "key" | "label" | "value" | "range" | "options" | "size" | ... */
  prop: string;
  message: string;
}

export interface ValidationInput {
  fields: readonly FieldDef[];
  variables: readonly VariableDef[];
  htmlLength: number;
  cssLength: number;
  /** Type-specific problems of a field (list columns, clock format, timer default): fieldTypes.ts. */
  fieldIssues?: (f: FieldDef) => { prop: string; message: string }[];
}

export function keyIssue(key: string): string | null {
  if (key === "") return "Enter a key.";
  if (!TEMPLATE_KEY_PATTERN.test(key))
    return "Use a–z, 0–9, - or _, starting with a letter (max 32).";
  return null;
}

export { isHex, pickerHex } from "$lib/components/variables/color";

/** Problem with a variable's current value, or null when it's valid. */
export function valueIssue(v: VariableDef): string | null {
  switch (v.type) {
    case "color":
      return isColor(v.default) ? null : "Enter a color like #1a1a1a.";
    case "font":
      return TEMPLATE_FONTS.some((f) => f.name === v.default) ? null : "Choose a font.";
    case "range": {
      const n = v.default.trim() === "" ? NaN : Number(v.default);
      if (!Number.isFinite(n)) return "Enter a number.";
      if (n < v.min || n > v.max) return `Between ${v.min} and ${v.max}.`;
      return null;
    }
    case "select":
      return v.options.some((o) => o.value === v.default) ? null : "Choose an option.";
  }
}

/** Every validation problem, in display order. Cheap: runs over the (small) field/variable lists. */
export function validate(input: ValidationInput): Issue[] {
  const issues: Issue[] = [];
  const seen = new Set<string>();

  if (input.htmlLength > TEMPLATE_HTML_MAX)
    issues.push({ scope: "html", index: 0, prop: "size", message: "HTML is over 256 KB." });
  if (input.cssLength > TEMPLATE_CSS_MAX)
    issues.push({ scope: "css", index: 0, prop: "size", message: "CSS is over 256 KB." });

  if (input.fields.length > TEMPLATE_FIELDS_MAX)
    issues.push({
      scope: "field",
      index: TEMPLATE_FIELDS_MAX,
      prop: "count",
      message: `Up to ${TEMPLATE_FIELDS_MAX} fields.`,
    });
  input.fields.forEach((f, index) => {
    const k = keyIssue(f.key);
    if (k) issues.push({ scope: "field", index, prop: "key", message: k });
    else if (seen.has(f.key))
      issues.push({ scope: "field", index, prop: "key", message: `Key “${f.key}” is already used.` });
    seen.add(f.key);
    if (f.label.trim() === "")
      issues.push({ scope: "field", index, prop: "label", message: "Enter a label." });
    for (const issue of input.fieldIssues?.(f) ?? []) issues.push({ scope: "field", index, ...issue });
  });

  if (input.variables.length > TEMPLATE_VARIABLES_MAX)
    issues.push({
      scope: "variable",
      index: TEMPLATE_VARIABLES_MAX,
      prop: "count",
      message: `Up to ${TEMPLATE_VARIABLES_MAX} variables.`,
    });
  input.variables.forEach((v, index) => {
    const k = keyIssue(v.key);
    if (k) issues.push({ scope: "variable", index, prop: "key", message: k });
    else if (seen.has(v.key))
      issues.push({
        scope: "variable",
        index,
        prop: "key",
        message: `Key “${v.key}” is already used.`,
      });
    seen.add(v.key);
    if (v.label.trim() === "")
      issues.push({ scope: "variable", index, prop: "label", message: "Enter a label." });
    if (v.type === "range") {
      if (!(v.max > v.min))
        issues.push({ scope: "variable", index, prop: "range", message: "Max must be above min." });
      else if (!(v.step > 0))
        issues.push({ scope: "variable", index, prop: "range", message: "Step must be above 0." });
    }
    if (v.type === "select") {
      const values = v.options.map((o) => o.value);
      if (values.length === 0)
        issues.push({ scope: "variable", index, prop: "options", message: "Add an option." });
      else if (values.some((x) => x.trim() === ""))
        issues.push({ scope: "variable", index, prop: "options", message: "Every option needs a value." });
      else if (new Set(values).size !== values.length)
        issues.push({ scope: "variable", index, prop: "options", message: "Option values must be unique." });
    }
    const val = valueIssue(v);
    if (val) issues.push({ scope: "variable", index, prop: "value", message: val });
  });

  return issues;
}

/** The first issue of field `index` whose prop starts with `prefix` (e.g. any column issue). */
export function issueWith(issues: readonly Issue[], index: number, prefix: string): string | null {
  return issues.find((i) => i.scope === "field" && i.index === index && i.prop.startsWith(prefix))?.message ?? null;
}

export function issueFor(
  issues: readonly Issue[],
  scope: IssueScope,
  index: number,
  prop: string,
): string | null {
  return issues.find((i) => i.scope === scope && i.index === index && i.prop === prop)?.message ?? null;
}

/** "Fields: Key “name” is already used." — the one-line reason shown next to "Not saved". */
export function describeIssue(i: Issue): string {
  const where = { field: "Fields", variable: "Variables", html: "HTML", css: "CSS" }[i.scope];
  return `${where}: ${i.message}`;
}

/** Smallest `<prefix><n>` not used by any field or variable. */
export function nextKey(prefix: string, used: Iterable<string>): string {
  const taken = new Set(used);
  let n = 1;
  while (taken.has(`${prefix}${n}`)) n++;
  return `${prefix}${n}`;
}

export function newField(used: Iterable<string>): FieldDef {
  const key = nextKey("field", used);
  return { key, label: `Field ${key.slice(5)}`, type: "text", default: "" };
}

/** A variable of `type` keeping key/label; settings reset to the kind's defaults. */
export function withKind(
  base: { key: string; label: string },
  type: VariableKind["type"],
): VariableDef {
  switch (type) {
    case "color":
      return { key: base.key, label: base.label, type, default: "#ffffff" };
    case "font":
      return { key: base.key, label: base.label, type, default: TEMPLATE_FONTS[0].name };
    case "range":
      return { key: base.key, label: base.label, type, min: 0, max: 100, step: 1, unit: "px", default: "0" };
    case "select":
      return {
        key: base.key,
        label: base.label,
        type,
        options: [{ value: "option-1", label: "Option 1" }],
        default: "option-1",
      };
  }
}

export function newVariable(used: Iterable<string>): VariableDef {
  const key = nextKey("var", used);
  return withKind({ key, label: `Variable ${key.slice(3)}` }, "color");
}

/** Returns a copy of `list` with items `i` and `j` swapped (no-op when out of range). */
export function swap<T>(list: readonly T[], i: number, j: number): T[] {
  const next = list.slice();
  if (i < 0 || j < 0 || i >= list.length || j >= list.length) return next;
  [next[i], next[j]] = [next[j], next[i]];
  return next;
}

export function replaceAt<T>(list: readonly T[], i: number, item: T): T[] {
  const next = list.slice();
  next[i] = item;
  return next;
}

export function removeAt<T>(list: readonly T[], i: number): T[] {
  return list.filter((_, j) => j !== i);
}

