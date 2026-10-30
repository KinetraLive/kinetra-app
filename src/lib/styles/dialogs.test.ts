// Modal dialogs never fade in: a dialog's open animation may be held on its first frame for seconds
// on a busy first load, and a dialog at opacity 0 then shows the page through it (a tester saw
// Control's red on-air line over "Add automation"). Dialogs slide; only the backdrop (behind them)
// may fade. Checked in Edge by e2e/ui/dialog-stacking.spec.ts.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const SRC = join(process.cwd(), "src");

function svelteFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) return svelteFiles(p);
    return p.endsWith(".svelte") ? [p] : [];
  });
}

/** The text between the brace that opens at `from` and its matching close. */
function block(css: string, from: number): string {
  const open = css.indexOf("{", from);
  let depth = 0;
  for (let i = open; i < css.length; i++) {
    if (css[i] === "{") depth++;
    else if (css[i] === "}" && --depth === 0) return css.slice(open + 1, i);
  }
  return "";
}

/** The keyframes a dialog's own rules animate with (not its backdrop's). */
function dialogAnimations(css: string): string[] {
  const names: string[] = [];
  for (const m of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const [selector, body] = [m[1], m[2]];
    if (selector.includes("::backdrop") || selector.includes("@keyframes")) continue;
    if (!/\[open\]|\bdialog\b|\.(dialog|drawer|palette)\b/.test(selector)) continue;
    const a = /animation:\s*([\w-]+)/.exec(body);
    if (a && a[1] !== "none") names.push(a[1]);
  }
  return names;
}

const dialogs = svelteFiles(SRC).filter((f) => readFileSync(f, "utf8").includes("<dialog"));

describe("modal dialogs open opaque", () => {
  it("finds the dialogs and their animations", () => {
    expect(dialogs.length).toBeGreaterThan(15);
    const animated = dialogs.filter((f) => dialogAnimations(readFileSync(f, "utf8")).length > 0);
    expect(animated.length).toBeGreaterThan(14);
  });

  it.each(dialogs.map((f) => [f.slice(SRC.length + 1), f]))("%s never fades the dialog in", (_name, file) => {
    const css = /<style>([\s\S]*)<\/style>/.exec(readFileSync(file, "utf8"))?.[1] ?? "";
    for (const name of dialogAnimations(css)) {
      const at = css.indexOf(`@keyframes ${name} `);
      if (at < 0) continue; // A name with no keyframes here doesn't animate.
      expect(block(css, at), `@keyframes ${name}`).not.toMatch(/opacity:\s*0[;\s]/);
    }
  });
});
