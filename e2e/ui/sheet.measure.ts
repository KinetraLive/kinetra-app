// Spreadsheet editor cost (data editing budget): `pnpm perf:ui`. The built app with the mock
// backend, the Sources page on a 1000-row data table (6 columns). Types into a cell in the middle
// of the table and scrolls through it, and reports the event handling time per key (Event Timing:
// processing time of events that took ≥ 16 ms to paint), long tasks, frame gaps while
// scrolling, and the number of rendered rows (virtualised).
// Budget: no main-thread task > 16 ms while typing or scrolling.

import type { Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { open, screensScenario } from "./harness";
import type { SourceDoc } from "../../src/lib/types";

interface Probe {
  events: { name: string; processing: number; duration: number }[];
  longTasks: number[];
  frames: number[];
}

const ROWS = 1000;
const big: SourceDoc = {
  id: "srcbig0001",
  name: "Registrations",
  kind: "text",
  columns: ["Name", "Company", "Title", "City", "Badge", "Notes"].map((label) => ({ key: label.toLowerCase(), label })),
  rows: Array.from({ length: ROWS }, (_, i) => ({
    name: `Guest ${i + 1}`,
    company: `Company ${(i * 7) % 113}`,
    title: i % 3 === 0 ? "Producer" : i % 3 === 1 ? "Engineer" : "Director",
    city: ["Bergen", "Oslo", "Tromsø", "Stavanger"][i % 4],
    badge: String(1000 + i),
    notes: i % 5 === 0 ? "VIP, seat in the front row" : "",
  })),
  index: 0,
  updatedAt: new Date().toISOString(),
};

function startProbe(page: Page) {
  return page.evaluate(() => {
    const probe: Probe = { events: [], longTasks: [], frames: [] };
    (window as unknown as { __probe: Probe }).__probe = probe;
    new PerformanceObserver((l) => {
      for (const e of l.getEntries() as PerformanceEventTiming[])
        probe.events.push({ name: e.name, processing: e.processingEnd - e.processingStart, duration: e.duration });
    }).observe({ type: "event", durationThreshold: 16, buffered: false } as PerformanceObserverInit);
    new PerformanceObserver((l) => l.getEntries().forEach((e) => probe.longTasks.push(e.duration))).observe({ type: "longtask" });
    const tick = (t: number) => {
      probe.frames.push(t);
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
}

async function takeProbe(page: Page): Promise<Probe> {
  return page.evaluate(() => {
    const w = window as unknown as { __probe: Probe };
    const p = w.__probe;
    w.__probe = { events: [], longTasks: [], frames: [] };
    return p;
  });
}

function gaps(frames: number[]) {
  const g = frames.slice(1).map((t, i) => t - frames[i]).sort((a, b) => a - b);
  return { p50: +(g[Math.floor(g.length / 2)] ?? 0).toFixed(1), max: +(g.at(-1) ?? 0).toFixed(1), over25: g.filter((x) => x > 25).length };
}

test("sheet: typing and scrolling in a 1000-row data table", async ({ page }) => {
  test.setTimeout(120_000);
  const base = screensScenario("dark");
  await open(page, "/sources", { ...base, sourceDocs: [...(base.sourceDocs ?? []), big] });
  await page.getByRole("option", { name: /Registrations/ }).click();
  const grid = page.getByRole("grid", { name: "Rows of Registrations" });
  await expect(grid).toHaveAttribute("aria-rowcount", String(ROWS + 1));
  const rendered = await grid.locator('[role="row"].row').count();
  console.log(`rendered rows: ${rendered} of ${ROWS}`);
  expect(rendered).toBeLessThan(120);

  // Into the middle of the table, then type into a cell.
  await grid.locator('[role="gridcell"][data-r="0"][data-c="1"]').click();
  for (let i = 0; i < 25; i++) await page.keyboard.press("PageDown");
  await page.waitForTimeout(500);
  await startProbe(page);
  await page.keyboard.press("Enter");
  await page.keyboard.type(" and partners, Nordic office", { delay: 40 });
  await page.keyboard.press("Enter");
  await page.waitForTimeout(800); // the autosave (500 ms) lands too
  const typing = await takeProbe(page);
  const keyEvents = typing.events.filter((e) => e.name.startsWith("key") || e.name === "input" || e.name === "beforeinput");
  const maxProc = Math.max(0, ...keyEvents.map((e) => e.processing));
  console.log(
    `typing: max key/input handling ${maxProc.toFixed(1)} ms, long tasks ${typing.longTasks.length}, frame gaps ${JSON.stringify(gaps(typing.frames))}`,
  );
  const saved = await grid.locator('[role="gridcell"][data-c="1"]', { hasText: "Nordic office" }).count();
  expect(saved).toBe(1);

  // Scroll through the table with the wheel (~60 Hz), then jump with the keyboard.
  const box = await grid.boundingBox();
  if (!box) throw new Error("no grid");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await startProbe(page);
  for (let i = 0; i < 120; i++) {
    await page.mouse.wheel(0, i < 60 ? 240 : -180);
    await page.waitForTimeout(16);
  }
  await page.keyboard.press("Control+End");
  await page.keyboard.press("Control+Home");
  await page.waitForTimeout(300);
  const scrolling = await takeProbe(page);
  const scrollGaps = gaps(scrolling.frames);
  const maxScrollEvent = Math.max(0, ...scrolling.events.map((e) => e.processing));
  console.log(
    `scrolling: long tasks ${scrolling.longTasks.length}, max event handling ${maxScrollEvent.toFixed(1)} ms (incl. Ctrl+End / Ctrl+Home jumps), frame gaps ${JSON.stringify(scrollGaps)}, rendered rows ${await grid.locator('[role="row"].row').count()}`,
  );

  expect(maxProc, "key handling (ms)").toBeLessThanOrEqual(16);
  expect(typing.longTasks.length, "long tasks while typing").toBe(0);
  expect(scrolling.longTasks.length, "long tasks while scrolling").toBe(0);
  expect(maxScrollEvent, "event handling while scrolling (ms)").toBeLessThanOrEqual(16);
});
