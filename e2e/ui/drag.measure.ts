// Layout editor drag cost (budget): `pnpm perf:ui`. The built app with the mock backend, the
// real renderer in the canvas (fixture server). Moves and resizes a layer box at pointer rate
// (~60 Hz) and reports frame gaps, long tasks, `set_layer_rect` rate and the save after the drop.
// Then moves a two-layer selection (one set_layer_rect per layer per update) with the layout tools
// on (grid, guides, safe areas, smart guides).
// Budget: no main-thread task > 16 ms, ≤ 20 rect updates/s in total, exactly one save ~500 ms after
// the drop. The first resize of the (reflow) guest lower third also saves once when the drag begins:
// resizing makes it Scale to fit, saved before the drag's live rects so the output scales.

import type { Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { mirrorScreens, open, screensScenario } from "./harness";

interface Probe {
  frames: number[];
  longTasks: number[];
  calls: { cmd: string; t: number }[];
}

/** Starts recording frame times, long tasks and backend calls in the page. */
function startProbe(page: Page) {
  return page.evaluate(() => {
    const w = window as unknown as {
      __probe: Probe;
      __TAURI_INTERNALS__: { invoke: (cmd: string, args?: unknown) => Promise<unknown> };
    };
    const probe: Probe = { frames: [], longTasks: [], calls: [] };
    w.__probe = probe;
    const inner = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = (cmd, args) => {
      probe.calls.push({ cmd, t: performance.now() });
      return inner(cmd, args);
    };
    new PerformanceObserver((l) => {
      for (const e of l.getEntries()) probe.longTasks.push(e.duration);
    }).observe({ type: "longtask" });
    const tick = (t: number) => {
      probe.frames.push(t);
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
}

async function readProbe(page: Page): Promise<Probe> {
  return page.evaluate(() => (window as unknown as { __probe: Probe }).__probe);
}

/** Drags from (x, y) by (dx, dy) in `steps` pointer moves, one per ~16 ms. */
async function drag(page: Page, x: number, y: number, dx: number, dy: number, steps: number) {
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= steps; i++) {
    await page.mouse.move(x + (dx * i) / steps, y + (dy * i) / steps);
    await page.waitForTimeout(16);
  }
  await page.mouse.up();
}

function report(name: string, p: Probe, from: number, to: number, dropAt: number) {
  const frames = p.frames.filter((t) => t >= from && t <= to);
  const gaps = frames.slice(1).map((t, i) => t - frames[i]).sort((a, b) => a - b);
  const pct = (q: number) => gaps[Math.min(gaps.length - 1, Math.floor(gaps.length * q))] ?? 0;
  const rects = p.calls.filter((c) => c.cmd === "set_layer_rect" && c.t >= from && c.t <= dropAt + 60);
  const saves = p.calls.filter((c) => c.cmd === "save_screen" && c.t >= dropAt);
  const early = p.calls.filter((c) => c.cmd === "save_screen" && c.t >= from && c.t < dropAt);
  const secs = (dropAt - from) / 1000;
  // Max updates in any 1 s window during the drag.
  let maxPerSec = 0;
  for (const c of rects) maxPerSec = Math.max(maxPerSec, rects.filter((d) => d.t >= c.t && d.t < c.t + 1000).length);
  const out = {
    frames: frames.length,
    gapP50: +pct(0.5).toFixed(1),
    gapP95: +pct(0.95).toFixed(1),
    gapMax: +(gaps.at(-1) ?? 0).toFixed(1),
    gapsOver25ms: gaps.filter((g) => g > 25).length,
    longTasks: p.longTasks.length,
    rectCalls: rects.length,
    rectPerSecAvg: +(rects.length / secs).toFixed(1),
    rectPerSecMax: maxPerSec,
    saves: saves.length,
    savesWhileDragging: early.length,
    saveAfterDropMs: saves.length ? Math.round(saves[0].t - dropAt) : null,
  };
  console.log(`${name}: ${JSON.stringify(out)}`);
  return out;
}

test("layout editor: move and resize at pointer rate", async ({ page, context, server }) => {
  test.setTimeout(120_000);
  await mirrorScreens(page, context, server);
  await open(page, "/screens/s1main01ab", {
    ...screensScenario("dark"),
    screenEditor: { grid: true, gridSize: 16, actionSafe: true, titleSafe: true, centerCross: true },
  });
  const out = page.frameLocator('iframe[title="Screen output"]');
  await expect(out.locator('[data-state="in"]')).toHaveCount(2, { timeout: 15_000 });
  const guest = page.getByRole("button", { name: /^Guest lower third,/ });
  await guest.click();
  await page.waitForTimeout(1500); // the click's own save settles
  await startProbe(page);

  // "resize-center": Alt (from the centre) + Shift (aspect), snapping on.
  for (const kind of ["move", "resize", "resize-center", "multi"] as const) {
    if (kind === "multi") {
      await page.getByRole("button", { name: /^LIVE bug,/ }).click({ modifiers: ["Shift"] });
      await page.waitForTimeout(800);
    }
    const box = await guest.boundingBox();
    if (!box) throw new Error("no layer box");
    const from = await page.evaluate(() => performance.now());
    if (kind === "move" || kind === "multi") {
      await drag(page, box.x + box.width / 2, box.y + box.height / 2, 160, -90, 120);
    } else {
      const h = await page.locator('[data-handle="se"]').boundingBox();
      if (!h) throw new Error("no resize handle");
      const center = kind === "resize-center";
      if (center) {
        await page.keyboard.down("Alt");
        await page.keyboard.down("Shift");
      }
      // The first resize reshapes the box to the graphic's aspect (wider): a shorter drag keeps its
      // corner handle on the canvas for the next one.
      await drag(page, h.x + h.width / 2, h.y + h.height / 2, center ? -60 : 60, center ? -30 : 30, 120);
      if (center) {
        await page.keyboard.up("Shift");
        await page.keyboard.up("Alt");
      }
    }
    const dropAt = await page.evaluate(() => performance.now());
    await page.waitForTimeout(1200);
    const to = await page.evaluate(() => performance.now());
    const r = report(kind, await readProbe(page), from, to, dropAt);
    expect(r.longTasks, "long tasks (> 50 ms)").toBe(0);
    if (kind === "multi") expect(r.rectCalls, "both layers sent").toBeGreaterThan(2);
    // 20/s while dragging, plus the drop's flush (each dragged layer's final rect at once).
    const layers = kind === "multi" ? 2 : 1;
    expect(r.rectPerSecMax, "set_layer_rect per second").toBeLessThanOrEqual(20 + layers);
    expect(r.saves, "saves after the drop").toBe(1);
    // Only the first resize switches the layer to Scale to fit (one save as the drag begins).
    expect(r.savesWhileDragging, "saves while dragging").toBe(kind === "resize" ? 1 : 0);
  }
});
