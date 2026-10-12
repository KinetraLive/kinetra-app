// Control mode cost (budget): `pnpm perf:ui`. The built app with the mock backend and the real
// renderer in the previews. Measures the hotkey → TAKE/CLEAR handler (no main-thread task > 16 ms,
// command dispatched in the same task), the number of live preview iframes (≤ 6 in grid, 1 in
// single view) and that nothing runs at rest (no timers, intervals or animation frames).

import type { Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { mirrorScreens, open, screensScenario } from "./harness";

/** Counts timers and animation frames scheduled by the app page from now on. */
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

test("control: hotkey TAKE task time, iframe count, nothing at rest", async ({ page, context, server }) => {
  test.setTimeout(120_000);
  await mirrorScreens(page, context, server);
  await open(page, "/control", {
    ...screensScenario("dark"),
    control: {
      selectedScreenId: "s1main01ab",
      gridColumns: 2,
      liveEdit: "onTake",
      hotkeys: { "s1main01ab/l1strap": "F3" },
    },
  });
  await expect(page.getByRole("button", { name: "Take Breaking strap" })).toBeVisible();
  await page.waitForTimeout(1500);

  const iframes = await page.locator("iframe").count();
  console.log(`grid iframes: ${iframes}`);
  expect(iframes).toBeLessThanOrEqual(6);

  // Hotkey handler: time from keydown to the invoke call, and long tasks around it.
  const timings = await page.evaluate(async () => {
    const w = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (cmd: string, args?: unknown) => Promise<unknown> };
    };
    const inner = w.__TAURI_INTERNALS__.invoke;
    const out: number[] = [];
    let t0 = 0;
    w.__TAURI_INTERNALS__.invoke = (cmd, args) => {
      if (cmd === "live_show" || cmd === "live_hide") out.push(performance.now() - t0);
      return inner(cmd, args);
    };
    const long: number[] = [];
    new PerformanceObserver((l) => l.getEntries().forEach((e) => long.push(e.duration))).observe({ type: "longtask" });
    for (let i = 0; i < 10; i++) {
      t0 = performance.now();
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "F3", code: "F3", bubbles: true }));
      await new Promise((r) => setTimeout(r, 250));
    }
    w.__TAURI_INTERNALS__.invoke = inner;
    return { dispatchMs: out, longTasks: long };
  });
  console.log(`hotkey → invoke: ${JSON.stringify(timings.dispatchMs.map((x) => +x.toFixed(2)))} long tasks: ${timings.longTasks.length}`);
  expect(timings.dispatchMs.length).toBe(10);
  expect(Math.max(...timings.dispatchMs)).toBeLessThan(16);
  expect(timings.longTasks.length).toBe(0);

  // At rest: nothing scheduled for 3 s.
  await page.waitForTimeout(1500);
  await countScheduling(page);
  await page.waitForTimeout(3000);
  const sched = await page.evaluate(() => (window as unknown as { __sched: unknown }).__sched);
  console.log(`scheduled at rest (3 s): ${JSON.stringify(sched)}`);
  expect(sched).toEqual({ t: 0, i: 0, r: 0 });

  // Single view: exactly one live iframe.
  await page.getByRole("radio", { name: "Single" }).or(page.getByRole("button", { name: "Single" })).first().click();
  await page.waitForTimeout(1200);
  const single = await page.locator("iframe").count();
  console.log(`single iframes: ${single}`);
  expect(single).toBe(1);
});
