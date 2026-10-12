import { describe, expect, it } from "vitest";
import type { FieldDef, VariableDef, VariantDoc } from "$lib/types";
import {
  applyDefs,
  copyName,
  effectiveValues,
  effectiveVariables,
  pruneVars,
  unusedFields,
  usedKeys,
  variantNameIssue,
} from "./variants";

const f = (key: string, type: FieldDef["type"] = "text"): FieldDef => ({ key, label: key, type, default: "" });

describe("field use", () => {
  it("finds {{key}} in text and attributes, with or without spaces", () => {
    expect([...usedKeys('<div class="{{tone}}"><b>{{ name }}</b><img src="{{logo}}"></div>')]).toEqual([
      "tone",
      "name",
      "logo",
    ]);
  });

  it("finds list keys from data-each in any quoting, and layer fields inside rows", () => {
    const html = `<ul><li data-each="rows">{{.title}} {{#}} {{suffix}}</li></ul><ol data-each='more'></ol><p data-each=bare></p>`;
    const used = usedKeys(html);
    expect(used).toEqual(new Set(["suffix", "rows", "more", "bare"]));
    // Row cells and the row number are not fields.
    expect(used.has(".title")).toBe(false);
    expect(used.has("#")).toBe(false);
  });

  it("clock, timer and html fields bind with {{key}} like text", () => {
    const fields = [f("clock", "clock"), f("timer", "timer"), f("body", "html"), f("unused", "image")];
    expect(unusedFields("<div>{{clock}} {{timer}} {{body}}</div>", fields).map((x) => x.key)).toEqual(["unused"]);
  });

  it("lists every field an empty or unrelated template leaves out, in field order", () => {
    expect(unusedFields("<div>static</div>", [f("b"), f("a")]).map((x) => x.key)).toEqual(["b", "a"]);
    expect(unusedFields("<div>{{name}}</div>", [])).toEqual([]);
  });
});

const color = (key: string, d = "#ffffff"): VariableDef => ({ key, label: key, type: "color", default: d });
const range = (key: string, d = "4"): VariableDef => ({
  key,
  label: key,
  type: "range",
  min: 0,
  max: 10,
  step: 1,
  unit: "px",
  default: d,
});
const variant = (id: string, vars: Record<string, string> = {}): VariantDoc => ({ id, name: id, vars, html: "", css: "" });

describe("variable values", () => {
  it("effective values: the variant's own, else the definition default", () => {
    const defs = [color("a", "#000000"), range("r")];
    expect(effectiveValues(defs, { a: "#ff0000", stale: "x" })).toEqual({ a: "#ff0000", r: "4" });
    const shown = effectiveVariables(defs, { a: "#ff0000" });
    expect(shown[0].default).toBe("#ff0000");
    expect(shown[1]).toBe(defs[1]); // untouched entries keep their identity
  });

  it("pruneVars drops keys of removed variables", () => {
    expect(pruneVars([color("a")], { a: "#000000", gone: "1" })).toEqual({ a: "#000000" });
  });
});

describe("applyDefs", () => {
  const defs = [color("a", "#000000"), range("r", "4")];
  const variants = [variant("default", { a: "#111111" }), variant("compact", { a: "#222222", r: "8" })];

  it("keeps definition defaults for untouched and label-only edits", () => {
    const shown = effectiveVariables(defs, variants[0].vars);
    const next = [{ ...shown[0], label: "Accent" }, shown[1]];
    const r = applyDefs(defs, shown, next, variants, "default");
    expect(r.defs[0]).toMatchObject({ label: "Accent", default: "#000000" });
    expect(r.defs[1]).toBe(defs[1]);
    expect(r.changed.size).toBe(0);
    expect(r.variants[1]).toBe(variants[1]);
  });

  it("renames a key in every variant", () => {
    const shown = effectiveVariables(defs, variants[0].vars);
    const r = applyDefs(defs, shown, [{ ...shown[0], key: "accent" }, shown[1]], variants, "default");
    expect(r.variants.map((v) => v.vars)).toEqual([{ accent: "#111111" }, { accent: "#222222", r: "8" }]);
    expect([...r.changed].sort()).toEqual(["compact", "default"]);
  });

  it("a kind change drops the key everywhere; the new value is the definition default", () => {
    const shown = effectiveVariables(defs, variants[1].vars);
    const r = applyDefs(defs, shown, [shown[0], color("r", "#ffffff")], variants, "compact");
    expect(r.defs[1]).toMatchObject({ type: "color", default: "#ffffff" });
    expect(r.variants[1].vars).toEqual({ a: "#222222" });
    expect([...r.changed]).toEqual(["compact"]);
  });

  it("narrowing a range drops values outside it", () => {
    const shown = effectiveVariables(defs, variants[1].vars);
    const r = applyDefs(defs, shown, [shown[0], { ...(shown[1] as VariableDef & { max: number }), max: 6 } as VariableDef], variants, "compact");
    expect(r.variants[1].vars).toEqual({ a: "#222222" });
    expect(r.defs[1].default).toBe("4");
  });

  it("an added variable uses its own default and changes no variant", () => {
    const shown = effectiveVariables(defs, {});
    const r = applyDefs(defs, shown, [...shown, color("c")], variants, "default");
    expect(r.defs).toHaveLength(3);
    expect(r.changed.size).toBe(0);
  });

  it("a removed variable leaves the rest untouched", () => {
    const shown = effectiveVariables(defs, {});
    const r = applyDefs(defs, shown, [shown[1]], variants, "default");
    expect(r.defs).toEqual([defs[1]]);
  });
});

describe("names", () => {
  it("validates variant names", () => {
    expect(variantNameIssue("  ", [])).toBe("Enter a name.");
    expect(variantNameIssue("x".repeat(41), [])).toMatch(/Up to 40/);
    expect(variantNameIssue("compact", ["Compact"])).toBe("Another variant has this name.");
    expect(variantNameIssue("Sponsor", ["Compact"])).toBeNull();
  });

  it("suggests unused copy names within the limit", () => {
    expect(copyName("Compact", ["Default", "Compact"])).toBe("Compact copy");
    expect(copyName("Compact", ["Compact", "Compact copy"])).toBe("Compact copy 2");
    expect(copyName("x".repeat(40), []).length).toBeLessThanOrEqual(40);
  });
});
