/// <reference types="node" />
// Built-in template presets (presets/templates/<id>/, every look: variants/<id>/ in schema 2): file
// shape, bindings, variables, sample data, the style library index and the animation contract.
// Runs in jsdom so each look also goes through the real compile().
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { italicsOf, weightsOf } from "../scripts/fonts.mjs";
import { FONTS, familiesInCss } from "./fonts";
import { timerText } from "./clock";
import { compile } from "./layer";
import type { FieldType } from "./protocol";

/**
 * The size budget of a look (HTML + CSS, in characters): 12 KB, and 14 KB for every built-in Poll
 * look, which carries the voting tile (QR + link, the total) and the four poll states on top of the
 * usual layout (one rule for the preset, no per-look exceptions).
 */
const LOOK_BUDGET = 12 * 1024;
const PRESET_LOOK_BUDGET: Record<string, number> = { poll: 14 * 1024 };

const PRESETS = [
  "lower-third",
  "poll",
  "classic-bar",
  "minimal-line",
  "slide-in-accent",
  "news-strap",
  "corner-bug",
  "fullscreen-title",
  "leaderboard",
  "ticker",
  "scoreboard",
  "schedule-board",
  "now-next",
  "next-up",
  "countdown",
  "sponsor-slideshow",
  "sponsor-wall",
];
const DIR = join(process.cwd(), "presets", "templates");
const KEY = /^[a-z][a-z0-9_-]{0,31}$/;
const OPTION = /^[a-zA-Z0-9_-]{1,64}$/;
const HEX = /^#(?:[0-9a-f]{6}|[0-9a-f]{8})$/i;
const FONT_NAMES = new Set(FONTS.map(([n]) => n));
/** Clock formats: HH, mm and ss tokens with ":", "." or " " between them (as the server checks). */
const CLOCK_FORMAT = /^(?:HH|mm|ss)(?:[:. ](?:HH|mm|ss))*$/;
/** Values the server sets as row marks (`_status`, `_current`; `_duration` is whole seconds). */
const STATUSES = new Set(["previous", "now", "next", "delayed", "upcoming", "cancelled"]);
/** Weights the renderer's font build ships for every curated family. */
const WEIGHTS = new Set(["400", "600", "700", "normal", "bold"]);
const SLUG = new Map(FONTS.map(([n, s]) => [n, s]));

type Json = Record<string, unknown>;

function read(id: string, file: string): string {
  return readFileSync(join(DIR, id, file), "utf8");
}

const stripComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");

/** Bodies of every `@keyframes name { ... }` block (brace-balanced). */
function keyframeBodies(css: string): string[] {
  const out: string[] = [];
  const re = /@keyframes\s+[\w-]+\s*\{/g;
  while (re.exec(css)) {
    let depth = 1;
    let i = re.lastIndex;
    for (; i < css.length && depth; i++) {
      if (css[i] === "{") depth++;
      else if (css[i] === "}") depth--;
    }
    out.push(css.slice(re.lastIndex, i - 1));
  }
  return out;
}

/**
 * `:empty` as Chromium (and Selectors 3) define it: no element children and no text of non-zero
 * length. An empty field binding leaves one empty Text node, which still counts as empty; jsdom's
 * own `:empty` disagrees, so it is checked by hand.
 */
function emptyPerSpec(el: Element): boolean {
  return Array.from(el.childNodes).every((n) => n.nodeType !== 1 && !(n.nodeType === 3 && (n as Text).data.length));
}

/** `data-each` row templates in template html: the list key and the `.col` keys bound inside. */
function rowTemplates(html: string): { key: string; cols: string[] }[] {
  const t = document.createElement("template");
  t.innerHTML = html;
  return Array.from(t.content.querySelectorAll("[data-each]")).map((el) => ({
    key: el.getAttribute("data-each") || "",
    cols: [...el.outerHTML.matchAll(/\{\{\s*\.([\w-]+)\s*\}\}/g)].map((m) => m[1]),
  }));
}

/** Sample cell of a list column: "<column label> <n>", or an asset URL for an image column. */
function sampleCell(c: Json, i: number): string {
  return c.kind === "image" ? `/assets/${c.key as string}-${i}.png` : `${c.label as string} ${i}`;
}

/** A value per field: its label for text-like fields, three sample rows for a list (like the
 *  server's preview sample), the default for a timer, the format for a clock. */
function sampleValue(f: Json): string {
  if (f.type === "list") {
    const cols = f.columns as Json[];
    return JSON.stringify([1, 2, 3].map((i) => Object.fromEntries(cols.map((c) => [c.key as string, sampleCell(c, i)]))));
  }
  if (f.type === "timer") return f.default as string;
  if (f.type === "clock") return f.format as string;
  return f.label as string;
}

/** Every attribute value under `root`, joined. */
function attrText(root: Element): string {
  return Array.from(root.querySelectorAll("*"))
    .flatMap((el) => Array.from(el.attributes).map((a) => a.value))
    .join("\n");
}

afterEach(() => {
  vi.restoreAllMocks();
});

/** The style library index: styles in order and the presets with their type. */
const LIBRARY = JSON.parse(readFileSync(join(process.cwd(), "presets", "library.json"), "utf8")) as {
  styles: { id: string; name: string; description: string }[];
  presets: { id: string; kind?: string; legacy?: boolean }[];
};
const LIBRARY_KINDS = new Set(LIBRARY.presets.filter((p) => p.kind).map((p) => p.id));
/** Variables every library preset (one with a type) declares; brand kits and styles set them. */
const SHARED = ["accent", "secondary", "text", "plate", "font-heading", "font-body", "radius", "speed"];

interface Look {
  variant: string;
  html: string;
  /** Comments stripped. */
  css: string;
  /** The variant's variable values (registered, or its vars.json while not registered yet). */
  vars: Record<string, string>;
}

/** Look folders on disk (`variants/<id>/` with a template.html), registered or not. */
function lookDirs(id: string): string[] {
  const dir = join(DIR, id, "variants");
  if (!existsSync(dir)) return [];
  return readdirSync(dir).filter((v) => existsSync(join(dir, v, "template.html")));
}

/** A look's vars.json, if it has one. */
function varsJson(id: string, variant: string): Record<string, string> | undefined {
  const file = join(DIR, id, "variants", variant, "vars.json");
  return existsSync(file) ? (JSON.parse(readFileSync(file, "utf8")) as Record<string, string>) : undefined;
}

/**
 * A preset's looks: schema 2 has `variants/<id>/` (every registered one, plus folders on disk not
 * registered yet, so design work is checked before it is registered), schema 1 one look at the
 * folder root.
 */
function looks(id: string, meta: Json): Look[] {
  if (meta.schemaVersion === 1)
    return [{ variant: "default", html: read(id, "template.html"), css: stripComments(read(id, "template.css")), vars: {} }];
  const registered = (meta.variants as Json[]).map((v) => [v.id as string, v.vars as Record<string, string>] as const);
  const ids = new Set(registered.map(([v]) => v));
  const pending = lookDirs(id)
    .filter((v) => !ids.has(v))
    .map((v) => [v, varsJson(id, v) ?? {}] as const);
  return [...registered, ...pending].map(([variant, vars]) => ({
    variant,
    html: read(id, `variants/${variant}/template.html`),
    css: stripComments(read(id, `variants/${variant}/template.css`)),
    vars,
  }));
}

/** Field keys bound in a look's html (`{{key}}`, and `data-each` lists). */
function boundKeys(html: string): Set<string> {
  return new Set([
    // `{{.col}}` (a row's column) and `{{#}}` (its number) belong to list rows.
    ...[...html.matchAll(/\{\{\s*(\w[\w.-]*)\s*\}\}/g)].map((m) => m[1]),
    ...rowTemplates(html).map((r) => r.key),
  ]);
}

describe.each(PRESETS)("preset %s", (id) => {
  const meta = JSON.parse(read(id, "template.json")) as Json;
  const fields = meta.fields as Json[];
  const variables = meta.variables as Json[];
  const all = looks(id, meta);

  it("template.json has the exact shape", () => {
    const v2 = meta.schemaVersion === 2;
    expect(Object.keys(meta).sort()).toEqual(
      ["autoOutMs", "description", "fields", "name", "preset", "schemaVersion", "updatedAt", "variables"]
        .concat(v2 ? ["defaultVariant", "variants"] : [])
        .concat("design" in meta ? ["design"] : [])
        .sort(),
    );
    // The optional design box: {w, h, x?, y?, sizing} (core::model::TemplateDesign).
    if ("design" in meta) {
      const d = meta.design as Json;
      for (const k of Object.keys(d)) expect(["w", "h", "x", "y", "sizing"]).toContain(k);
      for (const k of ["w", "h"]) {
        expect(Number.isInteger(d[k])).toBe(true);
        expect(d[k]).toBeGreaterThanOrEqual(16);
        expect(d[k]).toBeLessThanOrEqual(8192);
      }
      for (const k of ["x", "y"]) if (d[k] != null) expect(Number.isFinite(d[k])).toBe(true);
      expect(["fit", "reflow"]).toContain(d.sizing);
    }
    expect([1, 2]).toContain(meta.schemaVersion);
    expect(typeof meta.name).toBe("string");
    expect((meta.name as string).length).toBeGreaterThan(0);
    expect(meta.description).toMatch(/^[A-Z].+\.$/);
    // Bumped whenever the look, variables or design change (projects offer the update).
    const ref = meta.preset as Json;
    expect(Object.keys(ref).sort()).toEqual(["id", "version"]);
    expect(ref.id).toBe(id);
    expect(Number.isInteger(ref.version) && (ref.version as number) >= 1).toBe(true);
    expect(meta.autoOutMs).toBeNull();
    expect(meta.updatedAt).toBe("");
    if (v2) {
      const variants = meta.variants as Json[];
      expect(variants.length).toBeGreaterThan(0);
      expect(variants.length).toBeLessThanOrEqual(12);
      for (const v of variants) {
        expect(Object.keys(v).sort()).toEqual(["id", "name", "vars"]);
        expect(v.id).toMatch(/^[a-z][a-z0-9-]{0,31}$/);
        expect((v.name as string).length).toBeGreaterThan(0);
      }
      expect(new Set(variants.map((v) => v.id)).size).toBe(variants.length);
      expect(variants.map((v) => v.id)).toContain(meta.defaultVariant);
      // A library preset's looks are the classic one (existing presets) and the styles.
      if (LIBRARY_KINDS.has(id)) {
        const styles = new Set(["classic", ...LIBRARY.styles.map((s) => s.id)]);
        for (const v of variants) expect(styles).toContain(v.id);
      }
    }
    expect(fields.length).toBeGreaterThan(0);
    for (const f of fields) {
      expect((f.label as string).length).toBeGreaterThan(0);
      switch (f.type) {
        case "text":
        case "multiline":
        case "number":
        case "image":
          expect(Object.keys(f).sort()).toEqual(["default", "key", "label", "type"]);
          expect(f.default).toBe("");
          break;
        case "list": {
          expect(Object.keys(f).sort()).toEqual(["columns", "default", "key", "label", "type"]);
          const cols = f.columns as Json[];
          expect(cols.length).toBeGreaterThan(0);
          for (const c of cols) {
            // `kind` is optional and only ever "image" (an asset picker in the editor).
            if ("kind" in c) {
              expect(Object.keys(c).sort()).toEqual(["key", "kind", "label"]);
              expect(c.kind).toBe("image");
            } else expect(Object.keys(c).sort()).toEqual(["key", "label"]);
            expect(c.key).toMatch(KEY);
            expect((c.label as string).length).toBeGreaterThan(0);
          }
          expect(new Set(cols.map((c) => c.key)).size).toBe(cols.length);
          // No invented rows: a preset's list starts empty.
          expect(JSON.parse(f.default as string)).toEqual([]);
          break;
        }
        case "timer": {
          expect(Object.keys(f).sort()).toEqual(["default", "key", "label", "type"]);
          const t = JSON.parse(f.default as string) as Json;
          expect(Object.keys(t).sort()).toEqual(["at", "base", "dir", "end", "run"]);
          expect(t.run).toBe(false);
          expect(t.at).toBe(0);
          expect(Number.isInteger(t.base) && (t.base as number) >= 0).toBe(true);
          expect(["up", "down"]).toContain(t.dir);
          expect(["stop", "hide"]).toContain(t.end);
          break;
        }
        case "clock":
          expect(Object.keys(f).sort()).toEqual(["default", "format", "key", "label", "type"]);
          expect(f.default).toBe("");
          expect(f.format).toMatch(CLOCK_FORMAT);
          break;
        default:
          throw new Error(`field type ${String(f.type)} is not covered by the preset checks`);
      }
    }
  });

  it("every style look on disk is registered in template.json with its vars.json", () => {
    if (meta.schemaVersion !== 2) return;
    const registered = new Map((meta.variants as Json[]).map((v) => [v.id as string, v]));
    const fix = (v: string) => `run: node scripts/register-variant.mjs ${id} ${v}`;
    for (const v of lookDirs(id)) {
      if (v === "classic") continue;
      expect(LIBRARY.styles.map((s) => s.id), `variants/${v} is not a style in library.json`).toContain(v);
      const own = varsJson(id, v);
      expect(own, `variants/${v}/vars.json is missing`).toBeTruthy();
      const entry = registered.get(v);
      expect(entry, `variants/${v} is not registered (${fix(v)})`).toBeTruthy();
      expect(entry!.vars, `variants/${v}: template.json differs from vars.json (${fix(v)})`).toEqual(own);
      expect(entry!.name).toBe(LIBRARY.styles.find((s) => s.id === v)!.name);
    }
    // Classic first, then the styles in library order (the register script's order).
    const order = ["classic", ...LIBRARY.styles.map((s) => s.id)];
    const ids = (meta.variants as Json[]).map((v) => v.id as string);
    expect(ids).toEqual([...ids].sort((a, b) => order.indexOf(a) - order.indexOf(b)));
  });

  it("keys are valid and unique across fields and variables", () => {
    const keys = [...fields, ...variables].map((x) => x.key as string);
    for (const k of keys) expect(k).toMatch(KEY);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("variables have valid kinds and defaults; variant values are valid", () => {
    const keys = variables.map((v) => v.key);
    // Library presets share the style variables (corner-bug's `text` is a field: no text colour);
    // the others keep the classic trio.
    const required = LIBRARY_KINDS.has(id) ? SHARED.filter((k) => !fields.some((f) => f.key === k)) : ["accent", "font", "scale"];
    for (const k of required) expect(keys).toContain(k);
    const valid = (v: Json, d: string) => {
      switch (v.type) {
        case "color":
          expect(d).toMatch(HEX);
          break;
        case "font":
          expect(FONT_NAMES).toContain(d);
          break;
        case "range": {
          const [min, max, step] = [v.min, v.max, v.step] as number[];
          expect(d.trim()).not.toBe("");
          const n = Number(d);
          expect(n).toBeGreaterThanOrEqual(min);
          expect(n).toBeLessThanOrEqual(max);
          const steps = (n - min) / step;
          expect(Math.abs(steps - Math.round(steps))).toBeLessThan(1e-9);
          break;
        }
        case "select":
          expect((v.options as Json[]).map((o) => o.value)).toContain(d);
          break;
        default:
          throw new Error(`unknown variable type ${String(v.type)}`);
      }
    };
    for (const v of variables) {
      expect(typeof v.label).toBe("string");
      expect(typeof v.default).toBe("string");
      switch (v.type) {
        case "color":
        case "font":
          expect(Object.keys(v).sort()).toEqual(["default", "key", "label", "type"]);
          break;
        case "range": {
          expect(Object.keys(v).sort()).toEqual(["default", "key", "label", "max", "min", "step", "type", "unit"]);
          expect(v.min as number).toBeLessThan(v.max as number);
          expect(v.step as number).toBeGreaterThan(0);
          expect(typeof v.unit).toBe("string");
          break;
        }
        case "select": {
          expect(Object.keys(v).sort()).toEqual(["default", "key", "label", "options", "type"]);
          const opts = v.options as Json[];
          expect(opts.length).toBeGreaterThan(1);
          for (const o of opts) {
            expect(Object.keys(o).sort()).toEqual(["label", "value"]);
            expect(o.value).toMatch(OPTION);
          }
          const values = opts.map((o) => o.value);
          expect(new Set(values).size).toBe(values.length);
          break;
        }
      }
      valid(v, v.default as string);
    }
    for (const look of all)
      for (const [k, d] of Object.entries(look.vars)) {
        const v = variables.find((x) => x.key === k);
        expect(v, `${look.variant}: variable ${k}`).toBeTruthy();
        expect(typeof d, `${look.variant}: variable ${k}`).toBe("string");
        valid(v!, d);
      }
    // Scale is unitless so CSS can use it as a factor: calc(1px * var(--scale)); so is the
    // style speed factor (except the ticker's, which is its loop time in seconds).
    const scale = variables.find((v) => v.key === "scale");
    if (scale) expect(scale).toMatchObject({ type: "range", unit: "", default: "1" });
    const speed = variables.find((v) => v.key === "speed");
    if (speed && id !== "ticker") expect(speed).toMatchObject({ type: "range", unit: "", min: 0.5, max: 2, default: "1" });
  });

  it("every binding is a declared field and every field is bound by some look", () => {
    const declared = fields.map((f) => f.key as string);
    const union = new Set<string>();
    for (const l of all) {
      const each = rowTemplates(l.html).map((r) => r.key);
      for (const k of boundKeys(l.html)) {
        expect(declared, `${l.variant}: {{${k}}}`).toContain(k);
        union.add(k);
      }
      for (const f of fields) {
        if (f.type === "list") {
          if (boundKeys(l.html).has(f.key as string)) expect(each, `${f.key as string} has a data-each`).toContain(f.key);
        } else expect(each).not.toContain(f.key);
      }
    }
    expect([...union].sort()).toEqual([...declared].sort());
  });

  it("list rows bind only their own columns, and every column", () => {
    for (const l of all)
      for (const r of rowTemplates(l.html)) {
        const f = fields.find((x) => x.key === r.key)!;
        const cols = (f.columns as Json[]).map((c) => c.key as string);
        expect([...new Set(r.cols)].sort(), l.variant).toEqual([...cols].sort());
      }
  });

  it("every custom property is defined or declared, and every variable is used by some look", () => {
    const declared = new Set(variables.map((v) => v.key as string));
    const usedAll = new Set<string>();
    for (const l of all) {
      const defined = new Set([...l.css.matchAll(/(?:^|[{;\s])--([\w-]+)\s*:/g)].map((m) => m[1]));
      // Used through var() or a container style query (`@container style(--key: value)`).
      const used = new Set([...l.css.matchAll(/(?:var|style)\(\s*--([\w-]+)/g)].map((m) => m[1]));
      // From the renderer: list rows get --i (their index), the host --auto-out-ms when the
      // template has an auto-out (e.g. "5000ms").
      // The rows' parent gets --n (the row count).
      if (fields.some((f) => f.type === "list")) ["i", "n"].forEach((k) => defined.add(k));
      defined.add("auto-out-ms");
      for (const u of used) if (!defined.has(u)) expect(declared, `${l.variant}: --${u}`).toContain(u);
      for (const d of declared) expect(defined, `${l.variant}: variable ${d} shadowed in CSS`).not.toContain(d);
      used.forEach((u) => usedAll.add(u));
    }
    // The shared style variables of a library preset may go unused by a look that has no use for
    // them (a brand kit sets them on every template).
    for (const d of declared) if (!(LIBRARY_KINDS.has(id) && SHARED.includes(d))) expect(usedAll, `variable ${d} unused`).toContain(d);
  });

  describe.each(all.map((l) => [l.variant, l] as const))("look %s", (_name, look) => {
    const { html, css } = look;
    const bound = boundKeys(html);
    const own = fields.filter((f) => bound.has(f.key as string));

    it("has in and out rules, and the out animation is on the root", () => {
      const root = compile(html, {}, () => "").root;
      expect(root).not.toBeNull();
      const cls = root!.classList[0];
      expect(cls).toBeTruthy();
      expect(css).toContain('[data-state="in"]');
      const out = new RegExp(`\\.${cls}\\[data-state="out"\\]\\s*\\{([^}]*)\\}`).exec(css);
      expect(out, "out rule on the root").not.toBeNull();
      const decl = out![1];
      expect(decl).toMatch(/animation:[^;]*forwards/);
      const ms = Number(/(\d+)ms/.exec(decl)![1]);
      expect(ms).toBeGreaterThanOrEqual(250);
      expect(ms).toBeLessThanOrEqual(400);
      // Every state rule is scoped by the root's own data-state.
      for (const m of css.matchAll(/([^{}]*?)\[data-state="(?:in|out)"\]/g)) {
        expect(m[1].trim().split(/[\s,]+/).pop()).toBe(`.${cls}`);
      }
    });

    it("keyframes animate only transform and opacity", () => {
      const bodies = keyframeBodies(css);
      expect(bodies.length).toBeGreaterThan(0);
      for (const b of bodies) {
        for (const m of b.matchAll(/[{;]\s*([\w-]+)\s*:/g)) expect(["transform", "opacity"]).toContain(m[1]);
      }
    });

    it("uses only shipped font weights and styles", () => {
      // The families the look uses: named in its CSS, or a font variable's value in this look.
      const fams = familiesInCss(css);
      for (const v of variables)
        if (v.type === "font") {
          const name = look.vars[v.key as string] ?? (v.default as string);
          const slug = SLUG.get(name);
          if (slug && new RegExp(`var\\(\\s*--${v.key as string}\\b`).test(css)) fams.add(slug);
        }
      // A weight must be shipped by some family the look uses (400/600/700 when it names none).
      const shipped = new Set(WEIGHTS);
      for (const s of fams) for (const w of weightsOf(s)) shipped.add(String(w));
      const weights = [
        ...[...css.matchAll(/font-weight\s*:\s*([^;}\s]+)/g)].map((m) => m[1]),
        ...[...css.matchAll(/(?:^|[;{\s])font\s*:[^;{}]*?\b([1-9]00)\b/g)].map((m) => m[1]),
      ];
      for (const w of weights) expect(shipped, `font-weight ${w} (families: ${[...fams].join(", ") || "none"})`).toContain(w);
      // Italic only with a family that ships italic faces (no synthesized slant on air).
      if (/font(?:-style)?\s*:[^;{}]*\bitalic\b/.test(css))
        expect([...fams].some((s) => italicsOf(s).length > 0), "italic needs a family with italic faces").toBe(true);
    });

    it("loads nothing external and runs no script", () => {
      expect(css).not.toMatch(/url\(/i);
      expect(css).not.toMatch(/@import/i);
      expect(css).not.toMatch(/@font-face/i);
      expect(html).not.toMatch(/<script/i);
      expect(html).not.toMatch(/\son[a-z]+\s*=/i);
      expect(html).not.toMatch(/https?:/i);
    });

    it("stays within the size budget of a look (12 KB of HTML + CSS; Poll 14 KB)", () => {
      if (meta.schemaVersion !== 2) return;
      const dir = `variants/${look.variant}/`;
      const budget = PRESET_LOOK_BUDGET[id] ?? LOOK_BUDGET;
      expect(read(id, dir + "template.html").length + read(id, dir + "template.css").length).toBeLessThanOrEqual(budget);
    });

    it("compiles with the renderer and binds its fields without warnings", () => {
      const warn = vi.spyOn(console, "warn");
      const types = Object.fromEntries(fields.map((f) => [f.key, f.type])) as Record<string, FieldType>;
      const values = Object.fromEntries(fields.map((f) => [f.key, sampleValue(f)])) as Record<string, string>;
      const c = compile(html, types, (k) => values[k] ?? "");
      expect(warn).not.toHaveBeenCalled();
      expect(c.root).not.toBeNull();
      expect(c.frag.children.length).toBe(1); // a single root element
      for (const f of own) expect(c.binds.get(f.key as string)?.length).toBeGreaterThan(0);
      expect(c.lists.length).toBe(rowTemplates(html).length);
      for (const l of c.lists) expect(l.rows.length).toBe(3);
      const div = document.createElement("div");
      div.append(c.frag);
      const text = div.textContent ?? "";
      // Image cells and status fields are bound into attributes (src, data-status).
      const attrs = attrText(div);
      for (const f of own) {
        if (f.type === "list") {
          for (const col of f.columns as Json[]) {
            for (const i of [1, 2, 3]) expect(col.kind === "image" ? attrs : text + "\n" + attrs).toContain(sampleCell(col, i));
          }
        } else if (f.type === "timer") expect(text).toContain(timerText(f.default as string, 0));
        else if (f.type === "clock") expect(text).toMatch(/\d\d:\d\d/);
        else if (f.type === "image") expect(attrs).toContain(f.label as string);
        else expect(text + "\n" + attrs).toContain(f.label as string);
      }
    });

    it("status and slideshow styling targets elements that carry the marks", () => {
      // Row marks (`_status`, `_current`, `_duration` → data-*) land on data-each rows; a status can
      // also be bound into the data-status attribute of a plain element.
      const t = document.createElement("template");
      t.innerHTML = html;
      const marked = new Set<string>();
      for (const el of Array.from(t.content.querySelectorAll("[data-each], [data-status]")))
        for (const c of Array.from(el.classList)) marked.add(c);
      for (const m of css.matchAll(/(\.[\w-]+)?(?::not\()?\[data-(status|current|duration|winner|highlight)(?:="([^"]*)")?\]/g)) {
        const [, cls, name, value] = m;
        // Bare only as a child test inside :has(> …), whose subject is a row container.
        if (!cls) expect(css.slice(Math.max(0, m.index - 8), m.index), `[data-${name}] needs a class`).toMatch(/:has\(>\s*$/);
        else expect(marked, `${cls}[data-${name}] is not a row or a bound status element`).toContain(cls.slice(1));
        if (name === "status") expect(STATUSES).toContain(value);
        else if (name === "current") expect(value).toBe("true");
        else if (name === "duration") expect(value).toMatch(/^[1-9]\d{0,3}$/);
      }
      // A bound data-status attribute holds exactly one field.
      for (const el of Array.from(t.content.querySelectorAll("[data-status]")))
        expect(el.getAttribute("data-status")).toMatch(/^\{\{\s*\w+\s*\}\}$/);
    });

    it("elements hidden with :empty are empty when their field is", () => {
      // A list gets one row with every cell empty, so elements inside rows exist too.
      const types = Object.fromEntries(fields.map((f) => [f.key, f.type])) as Record<string, FieldType>;
      const c = compile(html, types, (k) => (types[k] === "list" ? "[{}]" : ""));
      const div = document.createElement("div");
      div.append(c.frag);
      const classes = [...css.matchAll(/\.([\w-]+):empty/g)].map((m) => m[1]);
      expect(classes.length).toBeGreaterThan(0);
      for (const cls of classes) {
        const el = div.querySelector(`.${cls}`);
        expect(el, cls).not.toBeNull();
        expect(emptyPerSpec(el!), `.${cls} should match :empty`).toBe(true);
      }
    });
  });

  it("sample data names real fields and holds values as the server sends them", () => {
    let sample: Json;
    try {
      sample = JSON.parse(read(id, "sample.json")) as Json;
    } catch {
      // Only library presets ship sample data.
      expect(LIBRARY_KINDS.has(id)).toBe(false);
      return;
    }
    for (const [k, v] of Object.entries(sample)) {
      const f = fields.find((x) => x.key === k);
      expect(f, `sample ${k}`).toBeTruthy();
      expect(typeof v).toBe("string");
      if (f!.type === "list") expect(Array.isArray(JSON.parse(v as string))).toBe(true);
      if (f!.type === "timer") expect(Object.keys(JSON.parse(v as string) as Json).sort()).toEqual(["at", "base", "dir", "end", "run"]);
    }
  });
});

// On-air graphics always animate as designed: the OS reduced-motion setting (Windows "Animation
// effects: off") is for the app UI only, so no preset CSS may react to it.
describe("reduced motion", () => {
  it("no preset CSS reacts to prefers-reduced-motion", () => {
    const hits: string[] = [];
    const walk = (dir: string) => {
      for (const e of readdirSync(dir, { withFileTypes: true })) {
        const p = join(dir, e.name);
        if (e.isDirectory()) walk(p);
        else if (e.name.endsWith(".css") && /prefers-reduced-motion/i.test(readFileSync(p, "utf8"))) hits.push(p);
      }
    };
    walk(DIR);
    expect(hits).toEqual([]);
  });
});

describe("style library", () => {
  it("lists the styles in order and every typed preset once", () => {
    expect(LIBRARY.styles.map((s) => s.id)).toEqual(["minimal", "corporate", "news", "elegant", "swiss", "editorial", "inline", "frame", "pill"]);
    for (const s of LIBRARY.styles) expect(s.description).toMatch(/^[A-Z].+\.$/);
    const kinds = LIBRARY.presets.filter((p) => p.kind).map((p) => p.kind);
    expect(new Set(kinds).size).toBe(kinds.length);
    expect(kinds.sort()).toEqual(["bug", "countdown", "lowerThird", "nowNext", "poll", "scoreboard", "slideshow", "sponsorWall", "ticker", "title"]);
    for (const p of LIBRARY.presets) expect(PRESETS).toContain(p.id as (typeof PRESETS)[number]);
  });

  it("every typed preset ships the Frame look", () => {
    for (const id of LIBRARY_KINDS) {
      const meta = JSON.parse(read(id, "template.json")) as Json;
      expect((meta.variants as Json[]).map((v) => v.id), id).toContain("frame");
    }
  });
});

// Built-in recipes (presets/recipes/<id>/recipe.json): every layer, field, variable and binding
// they name must exist in the presets they use (the server validates the shape strictly).
const RECIPES = ["event-schedule", "sponsor-slideshow", "sponsor-wall", "countdown-to-doors"];
/** Columns a schedule source exposes to bindings (`countdown` is a timer state). */
const SCHEDULE_COLUMNS = ["title", "subtitle", "location", "start", "end", "day", "new_start", "label", "status", "countdown"];
const RECIPES_DIR = join(process.cwd(), "presets", "recipes");

describe.each(RECIPES)("recipe %s", (id) => {
  const r = JSON.parse(readFileSync(join(RECIPES_DIR, id, "recipe.json"), "utf8")) as Json;
  const source = r.source as Json;
  const layers = r.layers as Json[];

  it("has the manifest shape", () => {
    expect(Object.keys(r).sort()).toEqual(
      ["description", "id", "layers", "name", "panel", "rules", "schemaVersion", "screen", "source", "version"].sort(),
    );
    expect(r.schemaVersion).toBe(1);
    expect(r.id).toBe(id);
    expect(r.version).toBe(1);
    expect(r.description).toMatch(/^[A-Z].+\.$/);
    expect(["slideshow", "schedule", "countdown"]).toContain(r.panel);
    expect(["text", "schedule"]).toContain(source.kind);
    if (r.panel === "slideshow") expect(source.kind).toBe("text");
    else expect(source.kind).toBe("schedule");
    expect(layers.length).toBeGreaterThan(0);
  });

  it("a text source has valid columns, rows and auto-advance", () => {
    if (source.kind !== "text") return;
    const cols = source.columns as Json[];
    for (const c of cols) {
      expect(c.key).toMatch(KEY);
      if ("kind" in c) expect(c.kind).toBe("image");
    }
    const keys = cols.map((c) => c.key as string);
    for (const row of source.rows as Json[]) for (const k of Object.keys(row)) expect(keys).toContain(k);
    const auto = source.auto as Json;
    expect(auto.everyMs as number).toBeGreaterThanOrEqual(1000);
  });

  it("layers use real presets, fields, variables and source columns", () => {
    const columns =
      source.kind === "schedule" ? SCHEDULE_COLUMNS : (source.columns as Json[]).map((c) => c.key as string);
    for (const l of layers) {
      const meta = JSON.parse(read(l.preset as string, "template.json")) as Json;
      const fields = new Map((meta.fields as Json[]).map((f) => [f.key as string, f]));
      const vars = new Map((meta.variables as Json[]).map((v) => [v.key as string, v]));
      for (const k of Object.keys(l.fields as Json)) expect(fields.has(k), `field ${k}`).toBe(true);
      for (const [k, v] of Object.entries(l.vars as Json)) {
        expect(vars.has(k), `variable ${k}`).toBe(true);
        expect(typeof v).toBe("string");
      }
      for (const [k, b] of Object.entries(l.bindings as Record<string, Json>)) {
        const f = fields.get(k);
        expect(f, `binding ${k}`).toBeTruthy();
        expect(b.sourceId).toBe("$source");
        if (f!.type === "list") {
          expect(b.column).toBe("");
          expect(b.row).toBeUndefined();
          // Every column the template's rows show comes from the source.
          for (const c of f!.columns as Json[]) expect(columns, `${k}.${c.key as string}`).toContain(c.key);
        } else {
          expect(columns, `${k} → ${b.column as string}`).toContain(b.column);
          // A timer only binds the countdown, and the countdown only a timer.
          expect(b.column === "countdown").toBe(f!.type === "timer");
          if (source.kind === "schedule") expect(["now", "next"]).toContain(b.row);
          else expect(b.row).toBeUndefined();
        }
      }
    }
  });

  it("rules point at this recipe's source, screen and layers", () => {
    const refs = JSON.stringify(r.rules).match(/"\$[^"]*"/g) ?? [];
    for (const ref of refs) {
      const m = /^"\$(source|screen|layer:(\d+))"$/.exec(ref);
      expect(m, ref).not.toBeNull();
      if (m![2] !== undefined) expect(Number(m![2])).toBeLessThan(layers.length);
    }
    for (const rule of r.rules as Json[]) {
      expect(rule.enabled).toBe(false);
      expect((rule.then as Json[]).length).toBeGreaterThan(0);
    }
  });
});
