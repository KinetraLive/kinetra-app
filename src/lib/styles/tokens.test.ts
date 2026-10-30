// @vitest-environment node
// Contrast guard for the Kinetra token file (src/app.css). Every colour pair the design relies on
// is checked in both themes, so a token change can't quietly break legibility or focus visibility.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

// vitest runs from the repo root.
const SRC = join(process.cwd(), "src");
const css = readFileSync(join(SRC, "app.css"), "utf8");

/** Custom properties declared directly in the block that starts at `selector {`. */
function block(selector: string): Record<string, string> {
  const start = css.indexOf(selector + " {");
  if (start < 0) throw new Error(`no block ${selector}`);
  const open = css.indexOf("{", start);
  const close = css.indexOf("}", open);
  const body = css.slice(open + 1, close);
  const vars: Record<string, string> = {};
  for (const m of body.matchAll(/(--[a-z0-9-]+):\s*([^;]+);/g)) vars[m[1]] = m[2].trim();
  return vars;
}

const base = block(":root"); // shared tokens + dark values (dark is the default)
const lightExplicit = block(':root[data-theme="light"]');
const lightMedia = block(':root:not([data-theme="dark"])');
const themes = { dark: base, light: { ...base, ...lightExplicit } };

function resolve(vars: Record<string, string>, name: string, depth = 0): string {
  const v = vars[name];
  if (!v) throw new Error(`undefined token ${name}`);
  const ref = /^var\((--[a-z0-9-]+)\)$/.exec(v);
  if (ref && depth < 5) return resolve(vars, ref[1], depth + 1);
  return v;
}

function luminance(hex: string): number {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) throw new Error(`not a #rrggbb colour: ${hex}`);
  const [r, g, b] = [0, 2, 4]
    .map((i) => parseInt(m[1].slice(i, i + 2), 16) / 255)
    .map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** HSL hue in degrees of a #rrggbb colour. */
function hue(hex: string): number {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) throw new Error(`not a #rrggbb colour: ${hex}`);
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(m[1].slice(i, i + 2), 16) / 255);
  const max = Math.max(r, g, b);
  const d = max - Math.min(r, g, b);
  if (d === 0) return 0;
  const h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return (h * 60 + 360) % 360;
}

/** Shortest distance between two hues on the colour wheel. */
function hueDistance(a: number, b: number): number {
  const d = Math.abs(a - b) % 360;
  return d > 180 ? 360 - d : d;
}

function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

const SURFACES = ["--bg", "--surface-1", "--surface-2", "--surface-3"];

describe.each(Object.entries(themes))("%s theme", (_name, vars) => {
  const c = (fg: string, bg: string) => contrast(resolve(vars, fg), resolve(vars, bg));

  it.each(SURFACES)("text and muted text are AA on %s", (s) => {
    expect(c("--text", s)).toBeGreaterThanOrEqual(4.5);
    expect(c("--text-muted", s)).toBeGreaterThanOrEqual(4.5);
  });

  it.each(SURFACES)("scrollbar thumbs reach 3:1 on %s", (s) => {
    expect(c("--scrollbar", s)).toBeGreaterThanOrEqual(3);
  });

  it.each(SURFACES)("the focus ring reaches 3:1 on %s", (s) => {
    expect(c("--focus", s)).toBeGreaterThanOrEqual(3);
  });

  it.each(SURFACES)("danger and warning text are AA on %s", (s) => {
    expect(c("--danger", s), "danger").toBeGreaterThanOrEqual(4.5);
    expect(c("--warning", s), "warning").toBeGreaterThanOrEqual(4.5);
  });

  it.each(["--bg", "--surface-1", "--surface-2"])("accent used as text is AA on %s", (s) => {
    expect(c("--accent-text", s)).toBeGreaterThanOrEqual(4.5);
  });

  it("ink on accent, accent hover, danger and program tally is AA", () => {
    expect(c("--accent-ink", "--accent")).toBeGreaterThanOrEqual(4.5);
    expect(c("--accent-ink", "--accent-hover")).toBeGreaterThanOrEqual(4.5);
    expect(c("--danger-ink", "--danger")).toBeGreaterThanOrEqual(4.5);
    expect(c("--danger-ink", "--danger-hover")).toBeGreaterThanOrEqual(4.5);
    expect(c("--tally-ink", "--tally-program")).toBeGreaterThanOrEqual(4.5);
  });

  it("danger is red, clear of tally green, warning and the accent, and not the tally red itself", () => {
    const danger = hue(resolve(vars, "--danger"));
    // Red by design (destructive = red); it never draws a dot or a status fill (TallyDot below).
    expect(hueDistance(danger, 0)).toBeLessThan(15);
    expect(resolve(vars, "--danger")).not.toBe(resolve(vars, "--tally-program"));
    for (const other of ["--tally-preview", "--accent", "--accent-text"]) {
      expect(hueDistance(danger, hue(resolve(vars, other))), other).toBeGreaterThanOrEqual(45);
    }
    // Red vs amber: ~40° apart reads clearly as two colours.
    expect(hueDistance(danger, hue(resolve(vars, "--warning")))).toBeGreaterThanOrEqual(35);
    expect(hueDistance(danger, hue(resolve(vars, "--danger-hover")))).toBeLessThan(10);
  });

  it("primary buttons are neutral and their ink is AA", () => {
    expect(c("--primary-ink", "--primary")).toBeGreaterThanOrEqual(4.5);
    expect(c("--primary-ink", "--primary-hover")).toBeGreaterThanOrEqual(4.5);
    for (const t of ["--primary", "--primary-hover"]) {
      const m = /^#(..)(..)(..)$/.exec(resolve(vars, t));
      const [r, g, b] = (m ?? []).slice(1).map((x) => parseInt(x, 16));
      expect(Math.max(r, g, b) - Math.min(r, g, b), t).toBeLessThanOrEqual(12);
    }
  });

  // Screen tiles and cards: the selected line sits on the tile (--surface-1 / its selected
  // --surface-2) and against the page (--bg); the tally lines sit outside the tile on the page or a
  // panel. All three reach 3:1 against each.
  it.each(["--bg", "--surface-1", "--surface-2"])("selected, Program and Preview lines reach 3:1 on %s", (s) => {
    for (const t of ["--selected-line", "--program-line", "--preview-line"]) {
      expect(c(t, s), `${t} on ${s}`).toBeGreaterThanOrEqual(3);
    }
  });

  it.each(SURFACES)("tally dots stay visible (3:1) on %s", (s) => {
    for (const t of ["program", "preview"]) {
      // A dot's visible edge is its ring when the theme has one, otherwise its fill.
      const ring = resolve(vars, `--tally-${t}-ring`);
      const edge = ring === "transparent" ? `--tally-${t}` : `--tally-${t}-ring`;
      expect(c(edge, s), `${t} on ${s}`).toBeGreaterThanOrEqual(3);
    }
  });
});

describe("token references", () => {
  it("every var(--…) used in the app is defined somewhere", () => {
    // CSS fails silently on an undefined custom property, so a renamed or removed token would
    // otherwise go unnoticed. Defined = in app.css, or set by any component (custom properties
    // inherit, e.g. SettingsSection sets --settings-cols for its rows).
    const files: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) walk(p);
        else if (/\.(svelte|css)$/.test(p)) files.push(p);
      }
    };
    walk(join(SRC, "lib"));
    walk(join(SRC, "routes"));
    const defined = new Set([...css.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
    const sources = files.map((p) => [p, readFileSync(p, "utf8")] as const);
    for (const [, src] of sources) {
      for (const re of [/(--[a-z0-9-]+)\s*:/g, /style:(--[a-z0-9-]+)/g, /setProperty\(\s*["'](--[a-z0-9-]+)/g]) {
        for (const m of src.matchAll(re)) defined.add(m[1]);
      }
    }
    const missing = new Set<string>();
    for (const [p, src] of sources) {
      // var(--x, fallback) has a fallback, so only bare references must resolve.
      for (const m of src.matchAll(/var\((--[a-z0-9-]+)\s*\)/g)) {
        if (!defined.has(m[1])) missing.add(`${p}: ${m[1]}`);
      }
    }
    expect([...missing]).toEqual([]);
  });
});

describe("token file structure", () => {
  it("tally colours are identical in both themes", () => {
    expect(lightExplicit["--tally-program"]).toBeUndefined();
    expect(lightExplicit["--tally-preview"]).toBeUndefined();
    expect(base["--tally-program"]).toBe("#ff4545");
    expect(base["--tally-preview"]).toBe("#2fd27a");
  });

  it("the OS-preference light block matches the explicit light block", () => {
    expect(lightMedia).toEqual(lightExplicit);
  });

  it("selection is a border on selected objects; no viewfinder brackets anywhere; never navigation state", () => {
    // The selector group that shows the ::after (focus ring, selection border).
    const at = css.indexOf('.vf[aria-selected="true"]::after');
    const rule = css.slice(css.lastIndexOf("}", at) + 1, css.indexOf("{", at));
    expect(rule).toContain(".vf:focus-visible::after");
    expect(rule).toContain(".vf[data-selected]::after");
    expect(css).not.toMatch(/\.vf\[aria-current[^\]]*\]::after/);
    // No viewfinder brackets: no corner mask in any rule.
    const masks = [...css.matchAll(/([^{}]+)\{[^}]*\bmask:/g)].map((m) => m[1].replace(/\/\*[^]*?\*\//g, "").trim());
    expect(masks).toEqual([]);
  });

  it("has no display face: the logo is an outlined SVG and Chakra Petch is gone", () => {
    const users: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) walk(p);
        else if (/\.(svelte|css|ts|html)$/.test(p) && !p.endsWith("tokens.test.ts") && /--font-display\b|Chakra Petch/.test(readFileSync(p, "utf8"))) users.push(p);
      }
    };
    walk(SRC);
    expect(users).toEqual([]);
  });

  it("TallyDot never uses the danger colour", () => {
    const dot = readFileSync(join(SRC, "lib/components/TallyDot.svelte"), "utf8");
    expect(dot).not.toMatch(/--danger/);
  });

  it("components use no shadows except 1px inset borders", () => {
    const offenders: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) walk(p);
        else if (p.endsWith(".svelte")) {
          for (const m of readFileSync(p, "utf8").matchAll(/box-shadow:\s*([^;]+);/g)) {
            const v = m[1].trim();
            if (!/^inset 0 0 0 1px /.test(v) && v !== "none") {
              offenders.push(`${p}: ${v}`);
            }
          }
        }
      }
    };
    walk(join(SRC, "lib"));
    walk(join(SRC, "routes"));
    expect(offenders).toEqual([]);
  });
});
