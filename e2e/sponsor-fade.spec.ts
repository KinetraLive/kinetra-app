// The sponsor slideshow's slide change really crossfades, in the real renderer against the real
// output server: the outgoing and incoming slides' computed opacity is sampled every frame while
// the current mark moves, and both must pass through intermediate values. With the OS "reduce
// motion" setting emulated too: on-air graphics ignore it (it is for the app UI only).
// Two paths: an output screen bound to a text source advanced by auto-advance (the rotator on
// air), and the template editor's preview layer, whose list value the editor re-marks itself
// (`fields` with `_current` on the next row, what its `previewDraft` sends).
//   pnpm --dir renderer build, then pnpm exec playwright test e2e/sponsor-fade.spec.ts
import type { Page } from "@playwright/test";
import { expect, presetTest, styleTest, type Fixture } from "./fixture";
import { openScreen as openTestScreen } from "./preset-helpers";
import { BG, asFields, layer, openScreen, roots, sample, saveScreen, settle } from "./style-helpers";

const SLIDES = JSON.parse(sample("sponsor-slideshow").slides) as Record<string, string>[];
/** The sample slides with only row `i` current. */
const marked = (i: number) =>
  SLIDES.map((s, j) => {
    const { _current: _, ...rest } = s;
    return j === i ? { ...rest, _current: "true" } : rest;
  });

interface Frame {
  t: number;
  /** Computed opacity of the outgoing and the incoming slide. */
  out: number;
  in: number;
}

/**
 * Waits (in the page) until slide `to` becomes current, then samples both slides' computed
 * opacity every frame for `ms`. Start it before triggering the change.
 */
function sampleChange(page: Page, from: number, to: number, ms = 1000): Promise<Frame[]> {
  return page.evaluate(
    async ({ from, to, ms }) => {
      const frame = () => new Promise((r) => requestAnimationFrame(r));
      const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot)!;
      const slide = (i: number) => host.shadowRoot!.querySelectorAll<HTMLElement>(".ss__slide")[i];
      const deadline = performance.now() + 15_000;
      while (slide(to)?.getAttribute("data-current") !== "true") {
        if (performance.now() > deadline) throw new Error("the slide never became current");
        await frame();
      }
      const out: { t: number; out: number; in: number }[] = [];
      const t0 = performance.now();
      while (performance.now() - t0 < ms) {
        out.push({
          t: Math.round(performance.now() - t0),
          out: +getComputedStyle(slide(from)).opacity,
          in: +getComputedStyle(slide(to)).opacity,
        });
        await frame();
      }
      return out;
    },
    { from, to, ms },
  );
}

const mid = (v: number) => v > 0.05 && v < 0.95;

/** Both slides pass through intermediate opacities over several frames, overlapping (a
 *  crossfade, not a cut), and end with the incoming slide shown and the outgoing one gone. */
function expectCrossfade(frames: Frame[]) {
  const log = JSON.stringify(frames);
  expect(frames.filter((f) => mid(f.out)).length, log).toBeGreaterThanOrEqual(5);
  expect(frames.filter((f) => mid(f.in)).length, log).toBeGreaterThanOrEqual(5);
  expect(frames.filter((f) => mid(f.out) && mid(f.in)).length, log).toBeGreaterThanOrEqual(3);
  // The outgoing slide only fades out, the incoming one only in.
  for (let i = 1; i < frames.length; i++) {
    expect(frames[i].out, log).toBeLessThanOrEqual(frames[i - 1].out + 1e-6);
    expect(frames[i].in, log).toBeGreaterThanOrEqual(frames[i - 1].in - 1e-6);
  }
  const last = frames[frames.length - 1];
  expect(last.out, log).toBe(0);
  expect(last.in, log).toBe(1);
}

for (const reducedMotion of ["no-preference", "reduce"] as const) {
  for (const variant of ["classic", "frame"]) {
    styleTest(`output: auto-advance crossfades the slides, ${variant}, reduced motion ${reducedMotion}`, async ({ page, styleServer }) => {
      await page.emulateMedia({ reducedMotion });
      const live: Fixture = styleServer;
      const t = await live.cmd("addtemplate sponsor-slideshow");
      const src = await live.cmd(
        `addsource ${JSON.stringify({
          name: "Sponsors",
          // A text column: source image cells must be project assets; the sample logos are data URLs.
          columns: [
            { key: "logo", label: "Logo" },
            { key: "name", label: "Name" },
            { key: "tagline", label: "Tagline" },
          ],
          rows: SLIDES.map(({ logo, name, tagline }) => ({ logo, name, tagline })),
        })}`,
      );
      const l = layer("l1", t, 1920, 1080, { label: "Presented by" }, {
        variant,
        vars: { motion: "crossfade", "motion-ms": "700" },
        bindings: { slides: { sourceId: src, column: "" } },
      } as never);
      const doc = await saveScreen(live, 1920, 1080, BG.dark, [l]);
      const errors = await openScreen(page, live.base, doc.id);
      await live.cmd(`layershow ${doc.id} l1`);
      await expect(roots(page)).toHaveAttribute("data-state", "in");
      await settle(page);
      expect(await page.evaluate(() => matchMedia("(prefers-reduced-motion: reduce)").matches)).toBe(reducedMotion === "reduce");

      // Automation goes on air directly (no take), like the rotator recipe's auto-advance.
      const frames = sampleChange(page, 0, 1);
      await live.cmd(`autoadvance ${src} {"everyMs":1000,"loop":true,"transition":"update"}`);
      const got = await frames;
      await live.cmd(`autoadvance ${src} null`);
      expectCrossfade(got);
      expect(errors).toEqual([]);
    });
  }
}

for (const reducedMotion of ["no-preference", "reduce"] as const) {
  for (const variant of ["classic", "frame"]) {
    presetTest(`editor preview: moving the mark crossfades the slides, ${variant}, reduced motion ${reducedMotion}`, async ({ page, presetLive }) => {
      await page.emulateMedia({ reducedMotion });
      await presetLive.cmd(`template sponsor-slideshow ${variant}`);
      await presetLive.cmd(`vars ${JSON.stringify({ motion: "crossfade", "motion-ms": "700", progress: "none" })}`);
      await presetLive.cmd(`fields ${JSON.stringify(asFields({ label: "Presented by", slides: marked(0) }))}`);
      const errors = await openTestScreen(page, presetLive.base);
      await presetLive.cmd("show");
      await expect(roots(page)).toHaveAttribute("data-state", "in");
      await settle(page);

      const frames = sampleChange(page, 0, 1);
      await presetLive.cmd(`fields ${JSON.stringify(asFields({ slides: marked(1) }))}`);
      expectCrossfade(await frames);
      expect(errors).toEqual([]);
    });
  }
}
