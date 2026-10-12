// The built-in presets in a real browser, against the real output server: each loads, plays
// in (baseline screenshot with its field labels as text) and out, restyles on air through `vars`
// without replaying, and collapses its empty optional lines. Plus auto-out.
import type { Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, presetTest as test } from "./fixture";
import { PRESETS, animationsSettled, labelFields, labelTexts, openScreen, root } from "./preset-helpers";

/** Selector of each preset's text element(s) and of the optional lines that collapse when empty. */
const SPEC: Record<(typeof PRESETS)[number], { text: string; optional: Record<string, string> }> = {
  "classic-bar": { text: ".cbar__name", optional: { title: ".cbar__title" } },
  "minimal-line": { text: ".ml__name", optional: { title: ".ml__title" } },
  "slide-in-accent": { text: ".sa__name", optional: { tag: ".sa__tag", title: ".sa__title" } },
  "news-strap": { text: ".ns__headline", optional: { subline: ".ns__sub" } },
  "corner-bug": { text: ".bug__text", optional: { text: ".bug__text" } },
  "fullscreen-title": { text: ".ft__title", optional: { kicker: ".ft__kicker", subtitle: ".ft__subtitle" } },
  leaderboard: { text: ".lb__name", optional: { title: ".lb__title" } },
  scoreboard: { text: ".sb__name", optional: { period: ".sb__period" } },
  ticker: { text: ".tk__item", optional: {} },
  "schedule-board": { text: ".sch__name", optional: {} },
  "now-next": { text: ".nn__title", optional: {} },
  "next-up": { text: ".nu__title", optional: {} },
  countdown: { text: ".cd__title", optional: {} },
  "sponsor-slideshow": { text: ".ss__name", optional: { label: ".ss__label" } },
  "sponsor-wall": { text: ".sw__title", optional: { title: ".sw__title" } },
};

/** A curated font none of the presets uses by default, and its stylesheet slug. */
const FONT = "Source Serif 4";
const FONT_SLUG = "source-serif-4";

/** The variable that sets a preset's main font. */
function fontVar(id: string): string {
  const meta = JSON.parse(readFileSync(join(process.cwd(), "presets", "templates", id, "template.json"), "utf8")) as {
    variables: { key: string }[];
  };
  return meta.variables.some((v) => v.key === "font") ? "font" : "font-heading";
}

async function load(page: Page, live: { base: string; cmd(l: string): Promise<unknown> }, id: string) {
  await live.cmd(`template ${id}`);
  await live.cmd(`fields ${JSON.stringify(labelFields(id))}`);
  return openScreen(page, live.base);
}

/** Counts `animationstart` events on the template root and its subtree from now on. */
async function countAnimationStarts(page: Page) {
  await root(page)
    .first()
    .evaluate((el) => {
      const w = window as unknown as { __starts: number; __root: Element };
      w.__starts = 0;
      w.__root = el;
      el.addEventListener("animationstart", () => w.__starts++);
      el.querySelectorAll("*").forEach((c) => c.addEventListener("animationstart", () => w.__starts++));
    });
}

for (const id of PRESETS) {
  test.describe(`preset ${id}`, () => {
    test("plays in to the baseline, then out and leaves the DOM", async ({ page, presetLive }) => {
      const errors = await load(page, presetLive, id);
      await presetLive.cmd("show");
      await expect(root(page)).toHaveAttribute("data-state", "in");
      await animationsSettled(page);
      for (const label of labelTexts(id)) {
        await expect(page.getByText(label, { exact: true }).first()).toBeVisible();
      }
      // The ticker scrolls forever: its baseline is taken with animations stopped at their start.
      await expect(page).toHaveScreenshot(`preset-${id}.png`, {
        omitBackground: true,
        ...(id === "ticker" ? { animations: "disabled" as const } : {}),
        // Live clocks change every second: masked.
        mask: [page.locator(".sch__clock")],
      });

      await presetLive.cmd("hide");
      await expect(root(page)).toHaveAttribute("data-state", "out");
      await expect(root(page)).toHaveCount(0, { timeout: 4000 });
      expect(errors).toEqual([]);
    });

    test("a vars change on air restyles without replaying and loads the new font", async ({ page, presetLive }) => {
      await load(page, presetLive, id);
      await presetLive.cmd("show");
      await animationsSettled(page);
      await countAnimationStarts(page);

      const fontCss = page.waitForResponse((r) => r.url().endsWith(`/fonts/${FONT_SLUG}.css`) && r.ok());
      // Older presets name their font `font`; library presets `font-heading`.
      const fontKey = fontVar(id);
      await presetLive.cmd(`vars {"accent":"#ff0000","${fontKey}":"${FONT}"}`);
      await fontCss;

      const text = root(page).locator(SPEC[id].text).first();
      await expect(text).toHaveCSS("font-family", new RegExp(FONT));
      await expect
        .poll(() => root(page).first().evaluate((el) => getComputedStyle(el).getPropertyValue("--accent").trim()))
        .toBe("#ff0000");
      // The face is actually downloaded and used (curated fonts load only when text needs them).
      await expect
        .poll(() =>
          page.evaluate(
            (family) => [...document.fonts].some((f) => f.family.replace(/"/g, "") === family && f.status === "loaded"),
            FONT,
          ),
        )
        .toBe(true);

      const after = await root(page)
        .first()
        .evaluate((el) => {
          const w = window as unknown as { __starts: number; __root: Element };
          return { state: el.getAttribute("data-state"), sameNode: w.__root === el, starts: w.__starts };
        });
      expect(after).toEqual({ state: "in", sameNode: true, starts: 0 });
    });

    test("empty optional lines collapse", async ({ page, presetLive }) => {
      await load(page, presetLive, id);
      await presetLive.cmd("show");
      await animationsSettled(page);
      for (const [key, sel] of Object.entries(SPEC[id].optional)) {
        const el = root(page).locator(sel).first();
        await expect(el).toBeVisible();
        await presetLive.cmd(`fields ${JSON.stringify({ [key]: "" })}`);
        await expect(el).toHaveCSS("display", "none");
        await expect(el).toBeHidden();
      }
    });
  });
}

test("auto-out takes the layer off air without a hide", async ({ page, presetLive }) => {
  await load(page, presetLive, "classic-bar");
  await presetLive.cmd("autoout 600");
  await presetLive.cmd("show");
  await expect(root(page)).toHaveAttribute("data-state", "in");
  await expect(root(page)).toHaveCount(0, { timeout: 2000 });
});

test("the auto-out reaches CSS as --auto-out-ms: the Inline bar drains over it", async ({ page, presetLive }) => {
  await presetLive.cmd("template lower-third inline");
  await openScreen(page, presetLive.base);
  const bar = () =>
    page.locator(".lt__plate").evaluate((el) => ({
      ms: ((el.getRootNode() as ShadowRoot).host as HTMLElement).style.getPropertyValue("--auto-out-ms"),
      anim: getComputedStyle(el, "::after").animationName,
      duration: getComputedStyle(el, "::after").animationDuration,
    }));
  await presetLive.cmd("show");
  await expect(root(page)).toHaveAttribute("data-state", "in");
  expect(await bar()).toMatchObject({ ms: "", anim: "lt-bar" }); // no auto-out: the take fill
  await presetLive.cmd("autoout 4000"); // on air: the layer is re-sent, nothing replays
  await expect.poll(bar).toEqual({ ms: "4000ms", anim: "lt-drain", duration: "4s" });
  await expect(root(page)).toHaveAttribute("data-state", "in");
  await presetLive.cmd("hide");
  await expect(root(page)).toHaveCount(0);
  await presetLive.cmd("autoout 1500");
  await presetLive.cmd("show"); // the countdown runs: drained, then off air by itself
  await expect.poll(bar).toEqual({ ms: "1500ms", anim: "lt-drain", duration: "1.5s" });
  await expect(root(page)).toHaveCount(0, { timeout: 4000 });
});
