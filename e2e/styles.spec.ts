// The style library's screenshot matrix, in the real renderer against the real output server.
// For every style look on disk (presets/templates/<id>/variants/<style>/, styles from
// presets/library.json; not Classic): every library preset with its sample data, {16:9, 9:16} ×
// {light, dark background}, settled at the end of its play-in; edge cases (long text, missing
// optional fields, empty lists); one shot per type paused mid play-in. Baselines are
// e2e/__screenshots__/style-<style>-<preset>-….png. Plus a scan of every variant's CSS: animations
// and transitions may only touch transform and opacity.
//
// One style: pnpm --dir renderer build, then
//   pnpm exec playwright test e2e/styles.spec.ts -g "style <style> "
// (a new look: add --update-snapshots once, then review the PNGs it wrote).
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, styleTest as test, type Fixture } from "./fixture";
import { BG, LIBRARY, STYLE_LOOKS, asFields, freezeMidway, layer, openScreen, roots, sample, saveScreen, settle } from "./style-helpers";

const SIZES = { "16x9": [1920, 1080], "9x16": [1080, 1920] } as const;

const LONG_NAME = "Professor Alexandra Konstantinopoulou-Wennerström";

/** Edge cases, by type: field overrides on top of the sample data. */
const EDGE: { id: string; name: string; fields: Record<string, unknown> }[] = [
  {
    id: "lower-third",
    name: "long",
    fields: {
      name: LONG_NAME,
      title: "Director of International Co-productions and Distribution Strategy, Nordic Public Broadcasting Union",
    },
  },
  { id: "lower-third", name: "bare", fields: { title: "", photo: "", logo: "" } },
  {
    id: "fullscreen-title",
    name: "long",
    fields: { title: "The Future of Live Production: Remote Workflows, Cloud Switching and the Next Decade of Broadcast" },
  },
  { id: "corner-bug", name: "nologo", fields: { logo: "" } },
  {
    id: "poll",
    name: "long",
    fields: { question: "Which of these sessions from the first two days of the summit should we replay on the main stream tonight?" },
  },
  { id: "poll", name: "empty", fields: { options: [] } },
  { id: "poll", name: "closed", fields: { status: "closed" } },
  {
    id: "now-next",
    name: "long",
    fields: {
      now_title: "Keynote: How a small crew delivered a three-day festival to four platforms",
      next_title: "Panel: Remote production at scale with distributed teams and cloud switching",
    },
  },
  {
    id: "sponsor-slideshow",
    name: "long",
    fields: {
      slides: (JSON.parse(sample("sponsor-slideshow").slides) as Record<string, string>[]).map((r, i) =>
        i === 0
          ? { ...r, name: "Nordisk Kringkastingsteknikk og Mediesystemer AS", tagline: "Kringkastingsløsninger for scener, studioer og arenaer i hele Norden" }
          : r,
      ),
    },
  },
];

/** Project template per preset, created once per worker (and fixture server). */
const templates = new Map<string, string>();
async function templateOf(live: Fixture, id: string) {
  let t = templates.get(id);
  if (!t) templates.set(id, (t = await live.cmd(`addtemplate ${id}`)));
  return t;
}

/** A one-layer screen of preset `id` in the `style` variant, on air and settled. */
async function onAir(page: Page, live: Fixture, style: string, id: string, w: number, h: number, bg: string, fields: Record<string, string>) {
  const t = await templateOf(live, id);
  const doc = await saveScreen(live, w, h, bg, [layer("l1", t, w, h, fields, { variant: style })]);
  const view = JSON.parse(await live.cmd(`layerview ${doc.id} l1`)) as { variant?: string };
  // Not this style: the look is on disk but not in template.json (node scripts/register-variant.mjs).
  expect(view.variant, `${id} has no registered ${style} look`).toBe(style);
  await page.setViewportSize({ width: w, height: h });
  const errors = await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  return errors;
}

for (const { style, presets } of STYLE_LOOKS) {
  test.describe(`style ${style}`, () => {
    for (const id of presets) {
      for (const [ratio, [w, h]] of Object.entries(SIZES)) {
        for (const [tone, bg] of Object.entries(BG)) {
          test(`${id} ${ratio} ${tone}`, async ({ page, styleServer }) => {
            const errors = await onAir(page, styleServer, style, id, w, h, bg, sample(id));
            await settle(page);
            await expect(page).toHaveScreenshot(`style-${style}-${id}-${ratio}-${tone}.png`);
            expect(errors).toEqual([]);
          });
        }
      }

      test(`${id} mid play-in`, async ({ page, styleServer }) => {
        await onAir(page, styleServer, style, id, 1920, 1080, BG.dark, sample(id));
        const t = await freezeMidway(page);
        expect(t).toBeGreaterThan(0);
        await expect(page).toHaveScreenshot(`style-${style}-${id}-mid.png`);
      });
    }

    for (const e of EDGE.filter((x) => presets.includes(x.id))) {
      for (const [ratio, [w, h]] of Object.entries(SIZES)) {
        // Long text matters in both widths; the other cases once, at 16:9.
        if (e.name !== "long" && ratio !== "16x9") continue;
        test(`${e.id} ${e.name} ${ratio}`, async ({ page, styleServer }) => {
          const fields = { ...sample(e.id), ...asFields(e.fields) };
          const errors = await onAir(page, styleServer, style, e.id, w, h, ratio === "16x9" ? BG.dark : BG.light, fields);
          await settle(page);
          await expect(page).toHaveScreenshot(`style-${style}-${e.id}-${e.name}-${ratio}.png`);
          expect(errors).toEqual([]);
        });
      }
    }
  });
}

// ---- Motion rule: animated properties ----------------------------------------------------------

/** Properties a keyframe or a transition may animate. */
const ANIMATABLE = new Set(["transform", "opacity"]);
/** Declarations allowed inside a keyframe besides those. */
const KEYFRAME_META = new Set(["animation-timing-function"]);

/** `{…}` body that starts at `open` (the index of "{"), and the index after its "}". */
function block(css: string, open: number): [string, number] {
  let depth = 0;
  for (let i = open; i < css.length; i++) {
    if (css[i] === "{") depth++;
    else if (css[i] === "}" && --depth === 0) return [css.slice(open + 1, i), i + 1];
  }
  return [css.slice(open + 1), css.length];
}

/** Splits on commas outside parentheses. */
function topLevel(v: string): string[] {
  const out: string[] = [];
  let depth = 0;
  let cur = "";
  for (const c of v) {
    if (c === "(") depth++;
    if (c === ")") depth--;
    if (c === "," && depth === 0) {
      out.push(cur);
      cur = "";
    } else cur += c;
  }
  return [...out, cur];
}

const TIME = /^-?[\d.]+m?s$/;
const EASING = /^(ease|ease-in|ease-out|ease-in-out|linear|step-start|step-end)$/;
const GLOBAL = /^(initial|inherit|unset|revert|revert-layer)$/;

/** Problems in one stylesheet: animated properties other than transform/opacity. */
function motionProblems(raw: string): string[] {
  const css = raw.replace(/\/\*[\s\S]*?\*\//g, "");
  const out: string[] = [];
  const kf = /@keyframes\s+([\w-]+)\s*\{/g;
  for (let m: RegExpExecArray | null; (m = kf.exec(css)); ) {
    const [body, end] = block(css, m.index + m[0].length - 1);
    kf.lastIndex = end;
    for (const d of body.matchAll(/([\w-]+)\s*:/g)) {
      // Keyframe selectors ("from", "50%") are followed by "{", not ":".
      const prop = d[1].toLowerCase();
      if (!ANIMATABLE.has(prop) && !KEYFRAME_META.has(prop)) out.push(`@keyframes ${m[1]} animates "${prop}"`);
    }
  }
  for (const d of css.matchAll(/(?:^|[;{\s])transition(-property)?\s*:\s*([^;}]*)/g)) {
    const longhand = !!d[1];
    for (const part of topLevel(d[2].trim())) {
      const tokens = part
        .replace(/[\w-]+\([^)]*\)/g, " ") // var(...), cubic-bezier(...), steps(...)
        .trim()
        .split(/\s+/)
        .filter(Boolean);
      const props = longhand ? tokens : tokens.filter((t) => !TIME.test(t) && !EASING.test(t) && !GLOBAL.test(t));
      if (props.length === 1 && props[0] === "none") continue;
      if (!props.length && part.trim() && !GLOBAL.test(part.trim())) {
        out.push(`transition "${part.trim()}" animates all properties`);
        continue;
      }
      for (const p of props) if (!ANIMATABLE.has(p)) out.push(`transition animates "${p}"`);
    }
  }
  return out;
}

test.describe("motion rule", () => {
  test("variant CSS animates only transform and opacity", () => {
    const root = join(process.cwd(), "presets", "templates");
    const problems: string[] = [];
    let files = 0;
    for (const t of readdirSync(root)) {
      const variants = join(root, t, "variants");
      if (!existsSync(variants)) continue;
      for (const v of readdirSync(variants)) {
        const file = join(variants, v, "template.css");
        if (!existsSync(file)) continue;
        files++;
        for (const p of motionProblems(readFileSync(file, "utf8"))) problems.push(`${t}/variants/${v}: ${p}`);
      }
    }
    expect(files).toBeGreaterThanOrEqual(LIBRARY.length);
    expect(problems).toEqual([]);
  });

  test("no preset CSS reacts to prefers-reduced-motion (on-air graphics always animate)", () => {
    const hits: string[] = [];
    const walk = (dir: string) => {
      for (const e of readdirSync(dir, { withFileTypes: true })) {
        const p = join(dir, e.name);
        if (e.isDirectory()) walk(p);
        else if (e.name.endsWith(".css") && /prefers-reduced-motion/i.test(readFileSync(p, "utf8"))) hits.push(p);
      }
    };
    walk(join(process.cwd(), "presets", "templates"));
    expect(hits).toEqual([]);
  });

  test("the scan catches other animated properties", () => {
    expect(motionProblems("@keyframes a{from{opacity:0;transform:none}to{width:10px}}")).toEqual([
      '@keyframes a animates "width"',
    ]);
    expect(motionProblems(".a{transition:opacity 200ms var(--ease),transform 1s}")).toEqual([]);
    expect(motionProblems(".a{transition:200ms}")).toEqual(['transition "200ms" animates all properties']);
    expect(motionProblems(".a{transition: all 200ms ease}")).toEqual(['transition animates "all"']);
    expect(motionProblems(".a{transition-property:left, opacity}")).toEqual(['transition animates "left"']);
    expect(motionProblems(".a{transition:none}.b{transition-duration:1ms}")).toEqual([]);
    expect(motionProblems("/* transition: width */ .a{color:red}")).toEqual([]);
  });
});
