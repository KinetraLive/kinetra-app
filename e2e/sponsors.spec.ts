// The sponsor templates in the real renderer against the real output server: the sponsor wall
// (Classic and Frame) with 1, 4 and 9 logos, tiers and an empty logo; the sponsor rotator
// (sponsor-slideshow) paused halfway through its crossfade and slide transitions, and as a small
// corner layer; both with transparent tiles and plates on a colored screen; long names wrapped
// and fitted in every rotator look; the logo background off (default) and on. Baselines are
// e2e/__screenshots__/sponsors-*.png; every shot is also written to review/sponsors/ (the fade,
// fit and logo background ones to review/sponsors-fix/ too).
//   pnpm --dir renderer build, then pnpm exec playwright test e2e/sponsors.spec.ts
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, styleTest as test, type Fixture } from "./fixture";
import { BG, asFields, layer, openScreen, roots, sample, saveScreen, settle } from "./style-helpers";

const REVIEW = join(process.cwd(), "review", "sponsors");
/** Shots of the slideshow fade, long-name fit and logo background fixes (also in REVIEW). */
const REVIEW_FIX = join(process.cwd(), "review", "sponsors-fix");

/** A sample sponsor row: the wall sample's logos and names, in turn. */
const WALL = JSON.parse(sample("sponsor-wall").logos) as Record<string, string>[];
const rows = (n: number, extra: (i: number) => Record<string, string> = () => ({})) =>
  Array.from({ length: n }, (_, i) => ({ ...WALL[i % WALL.length], tier: "", ...extra(i) }));

/** Project template per preset, created once per worker. */
const templates = new Map<string, string>();
async function templateOf(live: Fixture, id: string) {
  let t = templates.get(id);
  if (!t) templates.set(id, (t = await live.cmd(`addtemplate ${id}`)));
  return t;
}

interface Shot {
  preset: string;
  variant: string;
  fields: Record<string, unknown>;
  vars?: Record<string, string>;
  rect?: { x: number; y: number; w: number; h: number };
  sizing?: "fit" | "reflow";
  /** Screen background (default BG.dark). */
  bg?: string;
}

/** One layer on a 1920×1080 screen, on air → the screen id and the page errors. */
async function onAir(page: Page, live: Fixture, s: Shot) {
  const t = await templateOf(live, s.preset);
  const fields = { ...sample(s.preset), ...asFields(s.fields) };
  const l = layer("l1", t, 1920, 1080, fields, {
    variant: s.variant,
    vars: s.vars ?? {},
    ...(s.rect ? { rect: s.rect } : {}),
    ...(s.sizing ? { sizing: s.sizing } : {}),
  });
  const doc = await saveScreen(live, 1920, 1080, s.bg ?? BG.dark, [l]);
  await page.setViewportSize({ width: 1920, height: 1080 });
  const errors = await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  return { screen: doc.id, errors };
}

/** Compares with the baseline and writes the same shot for review (`fix`: also to REVIEW_FIX). */
async function shot(page: Page, name: string, fix = false) {
  await expect(page).toHaveScreenshot(`sponsors-${name}.png`);
  for (const dir of fix ? [REVIEW, REVIEW_FIX] : [REVIEW]) {
    mkdirSync(dir, { recursive: true });
    await page.screenshot({ path: join(dir, `${name}.png`) });
  }
}

/** Tiles laid out (display not none) in the wall. */
const visibleTiles = (page: Page) =>
  page.locator(".sw__tile").evaluateAll((els) => els.filter((e) => getComputedStyle(e).display !== "none").length);

for (const variant of ["classic", "frame"]) {
  test.describe(`sponsor wall ${variant}`, () => {
    for (const n of [1, 4, 9]) {
      test(`${n} logos`, async ({ page, styleServer }) => {
        const { errors } = await onAir(page, styleServer, { preset: "sponsor-wall", variant, fields: { logos: rows(n) } });
        await settle(page);
        await expect(page.locator(".sw__grid")).toHaveAttribute("data-count", String(n));
        expect(await visibleTiles(page)).toBe(n);
        await shot(page, `wall-${variant}-${n}`);
        expect(errors).toEqual([]);
      });
    }

    test("tiers: main, then partner, then the rest, larger first", async ({ page, styleServer }) => {
      // Mixed order and case on purpose: the template sorts and sizes by tier.
      const tiers = ["supporter", "partner", "", "MAIN", "supporter", "Partner", "other", "supporter"];
      const { errors } = await onAir(page, styleServer, {
        preset: "sponsor-wall",
        variant,
        fields: { logos: rows(tiers.length, (i) => ({ tier: tiers[i] })) },
      });
      await settle(page);
      const boxes = await page.locator(".sw__tile").evaluateAll((els) =>
        els.map((e) => {
          const r = e.getBoundingClientRect();
          return { tier: e.getAttribute("data-tier")!.toLowerCase(), top: r.top, w: r.width };
        }),
      );
      const of = (t: string) => boxes.filter((b) => (t === "rest" ? !["main", "partner"].includes(b.tier) : b.tier === t));
      const [main, partner, rest] = [of("main"), of("partner"), of("rest")];
      expect(main[0].w).toBeGreaterThan(partner[0].w);
      expect(partner[0].w).toBeGreaterThan(rest[0].w);
      expect(Math.max(...main.map((b) => b.top))).toBeLessThan(Math.min(...partner.map((b) => b.top)));
      expect(Math.max(...partner.map((b) => b.top))).toBeLessThan(Math.min(...rest.map((b) => b.top)));
      await shot(page, `wall-${variant}-tiers`);
      expect(errors).toEqual([]);
    });
  });
}

test("sponsor wall: a row without a logo has no tile, the current sponsor is marked", async ({ page, styleServer }) => {
  const logos = rows(6, (i) => ({ ...(i === 2 ? { logo: "" } : {}), ...(i === 4 ? { _current: "true" } : {}) }));
  const { errors } = await onAir(page, styleServer, { preset: "sponsor-wall", variant: "classic", fields: { title: "", logos } });
  await settle(page);
  expect(await visibleTiles(page)).toBe(5);
  await expect(page.locator(".sw__tile").nth(2)).toBeHidden();
  await expect(page.locator(".sw__title")).toBeHidden();
  await shot(page, "wall-classic-empty-logo");
  expect(errors).toEqual([]);
});

/** Moves the slideshow's current mark from slide 1 to slide 2 and pauses the slide
 *  transitions 200 ms into the 700 ms slide change (the ease is front-loaded: the
 *  motion is mid-way there). */
async function midChange(page: Page, live: Fixture, screen: string) {
  const slides = JSON.parse(sample("sponsor-slideshow").slides) as Record<string, string>[];
  const next = slides.map((s, i) => {
    const { _current: _, ...rest } = s;
    return i === 1 ? { ...rest, _current: "true" } : rest;
  });
  await live.cmd("liveedit immediate");
  await live.cmd(`setfields ${screen} l1 ${JSON.stringify(asFields({ slides: next }))}`);
  await expect(page.locator(".ss__slide").nth(1)).toHaveAttribute("data-current", "true");
  return page.evaluate(() => {
    const hosts = [...document.querySelectorAll("div")].filter((d) => d.shadowRoot);
    // Only the slide change: the play-in animations stay finished.
    const all = hosts.flatMap((h) => h.shadowRoot!.getAnimations()).filter((a) => a instanceof CSSTransition);
    for (const a of all) {
      a.pause();
      a.currentTime = 200;
    }
    return all.length;
  });
}

for (const variant of ["classic", "frame"]) {
  test.describe(`sponsor rotator ${variant}`, () => {
    for (const motion of ["crossfade", "slide"]) {
      test(`${motion}, halfway`, async ({ page, styleServer }) => {
        const { screen, errors } = await onAir(page, styleServer, {
          preset: "sponsor-slideshow",
          variant,
          fields: {},
          vars: { motion, "motion-ms": "700", progress: "none" },
        });
        await settle(page);
        expect(await midChange(page, styleServer, screen)).toBeGreaterThan(0);
        // Mid-change: both slides partly visible.
        const ops = await page.locator(".ss__slide").evaluateAll((els) => els.slice(0, 2).map((e) => +getComputedStyle(e).opacity));
        expect(ops[0]).toBeGreaterThan(0.05);
        expect(ops[1]).toBeGreaterThan(0.05);
        await shot(page, `rotator-${variant}-${motion}-mid`, true);
        expect(errors).toEqual([]);
      });
    }

    test("a small corner layer", async ({ page, styleServer }) => {
      const { errors } = await onAir(page, styleServer, {
        preset: "sponsor-slideshow",
        variant,
        fields: {},
        vars: { progress: "none" },
        rect: { x: 1504, y: 820, w: 338, h: 260 },
        sizing: "fit",
      });
      await settle(page);
      // The card stays inside its layer box.
      const box = await page.locator(".ss__card").boundingBox();
      expect(box!.x).toBeGreaterThanOrEqual(1504);
      expect(box!.y).toBeGreaterThanOrEqual(820);
      expect(box!.x + box!.width).toBeLessThanOrEqual(1504 + 338 + 0.5);
      expect(box!.y + box!.height).toBeLessThanOrEqual(820 + 260 + 0.5);
      await shot(page, `rotator-${variant}-corner`);
      expect(errors).toEqual([]);
    });
  });
}

/** A light, colored screen (what transparent tiles and plates show through), with dark text. */
const SAND = "#e6dfd2";
/** A dark colored screen for the rotator. */
const TEAL = "#1d4a5c";
const CLEAR = { "logo-bg": "on", tile: "transparent", plate: "Transparent", text: "#1b1e24", accent: "#c2410c" };

for (const variant of ["classic", "frame"]) {
  test(`transparent: sponsor wall ${variant}, tiles and background`, async ({ page, styleServer }) => {
    const logos = rows(6, (i) => (i === 1 ? { _current: "true" } : {}));
    const { errors } = await onAir(page, styleServer, { preset: "sponsor-wall", variant, fields: { logos }, vars: CLEAR, bg: SAND });
    await settle(page);
    // The keyword reaches the layer as a CSS value (any case in, lowercase out) and paints nothing.
    const tile = page.locator(".sw__tile").first();
    expect(await tile.evaluate((e) => getComputedStyle(e).getPropertyValue("--tile").trim())).toBe("transparent");
    expect(await tile.evaluate((e) => getComputedStyle(e).getPropertyValue("--plate").trim())).toBe("transparent");
    expect(await tile.evaluate((e) => getComputedStyle(e).backgroundColor)).toBe("rgba(0, 0, 0, 0)");
    const plate = page.locator(variant === "frame" ? ".sw__plate" : ".sw__bg");
    expect(await plate.evaluate((e) => getComputedStyle(e).backgroundColor)).toBe("rgba(0, 0, 0, 0)");
    expect(await visibleTiles(page)).toBe(6);
    await shot(page, `transparent-wall-${variant}`);
    expect(errors).toEqual([]);
  });

  test(`transparent: sponsor rotator ${variant}, card`, async ({ page, styleServer }) => {
    const { errors } = await onAir(page, styleServer, {
      preset: "sponsor-slideshow",
      variant,
      fields: {},
      vars: { plate: "transparent", progress: "none" },
      bg: TEAL,
    });
    await settle(page);
    const card = page.locator(variant === "frame" ? ".ss__plate" : ".ss__card");
    expect(await card.evaluate((e) => getComputedStyle(e).backgroundColor)).toBe("rgba(0, 0, 0, 0)");
    await shot(page, `transparent-rotator-${variant}`);
    expect(errors).toEqual([]);
  });
}

// ---- Long names fit; logo background off (default) and on --------------------------------------

const LONG = {
  name: "Nordisk Kringkastingsteknikk og Mediesystemer AS",
  tagline: "Kringkastingsløsninger for scener, studioer og arenaer i hele Norden",
};
const SLIDESHOW_LOOKS = ["classic", "minimal", "corporate", "news", "elegant", "swiss", "editorial", "inline", "frame", "pill"];

/** The sample slides with a long name and tagline on the first (current) one. */
function longSlides() {
  const slides = JSON.parse(sample("sponsor-slideshow").slides) as Record<string, string>[];
  return slides.map((s, i) => (i === 0 ? { ...s, ...LONG } : s));
}

/**
 * The current slide's name and tagline: at most two lines, nothing overflowing its own box or the
 * card; the long name wrapped (and shrank as needed).
 */
async function expectFitted(page: Page) {
  const r = await page.evaluate(() => {
    const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot)!;
    const sr = host.shadowRoot!;
    const slide = sr.querySelector(".ss__slide")!;
    const card = (sr.querySelector(".ss__plate") ?? sr.querySelector(".ss__card"))!.getBoundingClientRect();
    return [".ss__name", ".ss__tagline"].map((sel) => {
      const el = slide.querySelector<HTMLElement>(sel)!;
      const cs = getComputedStyle(el);
      const b = el.getBoundingClientRect();
      const range = document.createRange();
      range.selectNodeContents(el);
      const text = range.getBoundingClientRect();
      const lh = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.2;
      return {
        sel,
        lines: Math.round(el.clientHeight / lh),
        shrunk: el.style.fontSize !== "",
        overflowX: el.scrollWidth - el.clientWidth,
        inCard: text.left >= card.left - 1 && text.right <= card.right + 1 && text.top >= card.top - 1 && text.bottom <= card.bottom + 1,
        inBox: text.left >= b.left - 1 && text.right <= b.right + 1,
      };
    });
  });
  const log = JSON.stringify(r);
  for (const x of r) {
    expect(x.lines, log).toBeLessThanOrEqual(2);
    expect(x.overflowX, log).toBeLessThanOrEqual(0);
    expect(x.inBox, log).toBe(true);
    expect(x.inCard, log).toBe(true);
  }
  expect(r[0].lines === 2 || r[0].shrunk, log).toBe(true);
}

const LAYOUTS = [
  { id: "full" },
  { id: "corner", rect: { x: 1504, y: 820, w: 338, h: 260 }, sizing: "fit" as const },
  { id: "reflow", rect: { x: 1300, y: 600, w: 560, h: 440 }, sizing: "reflow" as const },
];

for (const variant of SLIDESHOW_LOOKS) {
  test(`sponsor rotator ${variant}: a long name and tagline wrap to two lines and fit`, async ({ page, styleServer }) => {
    for (const lay of LAYOUTS) {
      const { errors } = await onAir(page, styleServer, {
        preset: "sponsor-slideshow",
        variant,
        fields: { slides: longSlides() },
        vars: { progress: "none" },
        ...(lay.rect ? { rect: lay.rect, sizing: lay.sizing } : {}),
      });
      await settle(page);
      await expectFitted(page);
      if (lay.id !== "reflow" && (variant === "classic" || variant === "frame")) await shot(page, `rotator-${variant}-long-${lay.id}`, true);
      expect(errors).toEqual([]);
    }
  });
}

/** The painted background of `sel` (or its ::before), or "none". */
const logoBox = (page: Page, sel: string) =>
  page
    .locator(sel)
    .first()
    .evaluate((el) => {
      const paint = (pseudo?: string) => {
        const c = getComputedStyle(el, pseudo).backgroundColor;
        return c === "rgba(0, 0, 0, 0)" || c === "transparent" ? null : c;
      };
      return paint() ?? paint("::before") ?? "none";
    });

for (const variant of ["classic", "frame"]) {
  for (const bg of ["off", "on"]) {
    test(`sponsor rotator ${variant}: logo background ${bg}`, async ({ page, styleServer }) => {
      const { errors } = await onAir(page, styleServer, {
        preset: "sponsor-slideshow",
        variant,
        fields: {},
        vars: { progress: "none", ...(bg === "on" ? { "logo-bg": "on" } : {}) },
      });
      await settle(page);
      const box = await logoBox(page, ".ss__media");
      if (bg === "off") expect(box).toBe("none");
      else expect(box).not.toBe("none");
      await shot(page, `rotator-${variant}-logo-bg-${bg}`, true);
      expect(errors).toEqual([]);
    });

    test(`sponsor wall ${variant}: logo background ${bg}`, async ({ page, styleServer }) => {
      const { errors } = await onAir(page, styleServer, {
        preset: "sponsor-wall",
        variant,
        fields: { logos: rows(6) },
        vars: bg === "on" ? { "logo-bg": "on" } : {},
      });
      await settle(page);
      const tile = await page.locator(".sw__tile").first().evaluate((e) => getComputedStyle(e).backgroundColor);
      if (bg === "off") expect(tile).toBe("rgba(0, 0, 0, 0)");
      else expect(tile).not.toBe("rgba(0, 0, 0, 0)");
      await shot(page, `wall-${variant}-logo-bg-${bg}`, true);
      expect(errors).toEqual([]);
    });
  }
}

test("sponsor rotator: no box behind the logo by default in any look; on paints the look's own", async ({ page, styleServer }) => {
  /** What paints right behind the logo when on, per look (default: a tile on .ss__media::before). */
  const behind: Record<string, string> = {
    corporate: ".ss__body",
    news: ".ss__navy",
    swiss: ".ss__slides",
    inline: ".ss__chip",
    pill: ".ss__chip",
    minimal: ".ss__logo",
  };
  for (const variant of SLIDESHOW_LOOKS) {
    const sel = behind[variant] ?? ".ss__media";
    const seen: Record<string, string> = {};
    for (const bg of ["off", "on"]) {
      await onAir(page, styleServer, { preset: "sponsor-slideshow", variant, fields: {}, vars: { progress: "none", "logo-bg": bg } });
      await settle(page);
      seen[bg] = await logoBox(page, sel);
      // Corporate's stage takes the card colour when off: one plate, no box of its own.
      if (variant === "corporate" && bg === "off") {
        const plate = await page.locator(".ss__body").evaluate((e) => getComputedStyle(e, "::after").backgroundColor);
        expect(seen.off, variant).toBe(plate);
      } else if (bg === "off") expect(seen.off, variant).toBe("none");
    }
    expect(seen.on, variant).not.toBe("none");
    expect(seen.on, variant).not.toBe(seen.off);
  }
});
