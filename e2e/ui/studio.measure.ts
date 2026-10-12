// Studio mode cost (`pnpm perf:ui`): the built app with the mock backend, the real renderer in the
// Preview and Program iframes. Drags the T-bar at pointer rate (~60 Hz) from the bottom to the
// middle, back down and up to the top (commit), and reports frame gaps, long tasks, the slowest
// input handler and the `studio_op` tbarSet rate. Then checks the single view's iframe count and
// that nothing runs at rest.
// Budget: no main-thread task > 16 ms while dragging, ≤ 30 tbarSet per second, 2 iframes, nothing
// scheduled at rest.

import type { Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { mirrorScreens, open, studioScenario } from "./harness";

interface Probe {
  frames: number[];
  longTasks: number[];
  /** Input event processing times (handler work), ms. */
  handlers: number[];
  ops: { op: string; t: number }[];
}

function startProbe(page: Page) {
  return page.evaluate(() => {
    const w = window as unknown as {
      __probe: Probe;
      __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
    };
    const probe: Probe = { frames: [], longTasks: [], handlers: [], ops: [] };
    w.__probe = probe;
    const inner = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = (cmd, args) => {
      if (cmd === "studio_op") probe.ops.push({ op: String((args?.op as { op: string }).op), t: performance.now() });
      return inner(cmd, args);
    };
    new PerformanceObserver((l) => {
      for (const e of l.getEntries()) probe.longTasks.push(e.duration);
    }).observe({ type: "longtask" });
    new PerformanceObserver((l) => {
      for (const e of l.getEntries() as PerformanceEventTiming[]) probe.handlers.push(e.processingEnd - e.processingStart);
    }).observe({ type: "event", durationThreshold: 16, buffered: false } as PerformanceObserverInit);
    const tick = (t: number) => {
      probe.frames.push(t);
      if (!(window as unknown as { __probeStop?: boolean }).__probeStop) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
}

function countScheduling(page: Page) {
  return page.evaluate(() => {
    const w = window as unknown as Record<string, unknown> & { __sched: { t: number; i: number; r: number } };
    w.__sched = { t: 0, i: 0, r: 0 };
    const st = window.setTimeout;
    const si = window.setInterval;
    const raf = window.requestAnimationFrame;
    window.setTimeout = ((...a: Parameters<typeof setTimeout>) => (w.__sched.t++, st(...a))) as typeof setTimeout;
    window.setInterval = ((...a: Parameters<typeof setInterval>) => (w.__sched.i++, si(...a))) as typeof setInterval;
    window.requestAnimationFrame = (cb) => (w.__sched.r++, raf(cb));
  });
}

test("studio: T-bar drag cost, tbarSet rate, iframes, nothing at rest", async ({ page, context, server }) => {
  test.setTimeout(120_000);
  await mirrorScreens(page, context, server);
  await open(page, "/control", { ...studioScenario("dark", "single"), studioTbar: {} });
  const pgm = page.frameLocator('iframe[title="Main program Program"]');
  await expect(pgm.locator('[data-state="in"]').first()).toBeAttached({ timeout: 15_000 });
  await page.waitForTimeout(1500);

  const iframes = await page.locator("iframe").count();
  console.log(`studio single view iframes: ${iframes}`);
  expect(iframes).toBe(2);

  const bar = page.getByRole("slider", { name: "T-bar" });
  const box = await bar.boundingBox();
  if (!box) throw new Error("no T-bar");
  const x = box.x + box.width / 2;
  const bottom = box.y + box.height - 1;
  const top = box.y + 1;
  await startProbe(page);
  const from = await page.evaluate(() => performance.now());
  await page.mouse.move(x, bottom);
  await page.mouse.down();
  // 180 moves (~3 s at 60 Hz): up to the middle, back down, then all the way up.
  const path = [
    ...Array.from({ length: 60 }, (_, i) => bottom - ((bottom - top) / 2) * ((i + 1) / 60)),
    ...Array.from({ length: 40 }, (_, i) => (bottom + top) / 2 + ((bottom - top) / 4) * ((i + 1) / 40)),
    ...Array.from({ length: 80 }, (_, i) => bottom - (bottom - top) / 4 - ((bottom - top) * 0.75 * (i + 1)) / 80),
  ];
  for (const y of path) {
    await page.mouse.move(x, y);
    await page.waitForTimeout(16);
  }
  await page.mouse.up();
  const upAt = await page.evaluate(() => performance.now());
  await page.waitForTimeout(800);
  const p = await page.evaluate(() => (window as unknown as { __probe: Probe }).__probe);

  const frames = p.frames.filter((t) => t >= from && t <= upAt);
  const gaps = frames.slice(1).map((t, i) => t - frames[i]).sort((a, b) => a - b);
  const pct = (q: number) => gaps[Math.min(gaps.length - 1, Math.floor(gaps.length * q))] ?? 0;
  const sets = p.ops.filter((o) => o.op === "tbarSet");
  let maxPerSec = 0;
  for (const s of sets) maxPerSec = Math.max(maxPerSec, sets.filter((d) => d.t >= s.t && d.t < s.t + 1000).length);
  let minGap = Infinity;
  for (let i = 1; i < sets.length; i++) minGap = Math.min(minGap, sets[i].t - sets[i - 1].t);
  const out = {
    moves: path.length,
    frames: frames.length,
    gapP50: +pct(0.5).toFixed(1),
    gapP95: +pct(0.95).toFixed(1),
    gapMax: +(gaps.at(-1) ?? 0).toFixed(1),
    longTasks: p.longTasks.length,
    slowHandlers: p.handlers.length,
    handlerMaxMs: +Math.max(0, ...p.handlers).toFixed(1),
    tbarSet: sets.length,
    tbarSetPerSecMax: maxPerSec,
    tbarSetMinGapMs: +minGap.toFixed(1),
    ops: [...new Set(p.ops.map((o) => o.op))],
    last: p.ops.at(-1)?.op,
  };
  console.log(`T-bar drag: ${JSON.stringify(out)}`);
  expect(out.longTasks, "long tasks").toBe(0);
  expect(out.handlerMaxMs, "slowest input handler (ms)").toBeLessThan(16);
  expect(out.tbarSetPerSecMax, "tbarSet per second").toBeLessThanOrEqual(30);
  expect(out.ops[0]).toBe("tbarBegin");
  expect(out.last).toBe("tbarEnd");
  await expect(bar).toHaveAttribute("aria-valuenow", "0");

  // At rest: nothing scheduled for 3 s (the probe's frame loop stopped first).
  await page.evaluate(() => ((window as unknown as { __probeStop?: boolean }).__probeStop = true));
  await page.mouse.move(5, 5);
  await page.waitForTimeout(1500);
  await countScheduling(page);
  await page.waitForTimeout(3000);
  const sched = await page.evaluate(() => (window as unknown as { __sched: unknown }).__sched);
  console.log(`studio, scheduled at rest (3 s): ${JSON.stringify(sched)}`);
  expect(sched).toEqual({ t: 0, i: 0, r: 0 });
});
