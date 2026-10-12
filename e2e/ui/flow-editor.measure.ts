// Flow editor cost (budget): `pnpm perf:ui`. The built app with the mock backend opens a
// 200-node flow in the node editor (the real @xyflow/svelte canvas), drags a node for ~2 s at
// pointer rate (after a first, short drag that warms the code up, reported on its own) and reports
// frame gaps, long tasks and draft saves; then leaves the editor and
// compares the JS heap with the list page's before it opened (both after a forced GC).
// Budget: frame gaps p95 ≤ 20 ms, no long task (> 50 ms, the Long Tasks API's floor; the frame gaps
// show anything over 16 ms), save_flow_draft ≤ 1 per 0.8 s, heap back within ~5 MB.

import type { CDPSession, Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { bigFlow, open } from "./harness";

interface Probe {
  frames: number[];
  longTasks: number[];
  calls: { cmd: string; t: number }[];
}

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

async function heapMB(cdp: CDPSession): Promise<number> {
  for (let i = 0; i < 3; i++) await cdp.send("HeapProfiler.collectGarbage");
  const { usedSize } = (await cdp.send("Runtime.getHeapUsage")) as { usedSize: number };
  return usedSize / 1048576;
}

test("flow editor: drag a node in a 200-node flow; heap after leaving", async ({ page }) => {
  test.setTimeout(120_000);
  const flow = bigFlow(200);
  await open(page, "/flows", {
    theme: "dark",
    screens: [],
    lanAccess: false,
    selectedScreenId: null,
    testOnAir: false,
    flows: [flow],
  });
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("HeapProfiler.enable");
  await expect(page.getByRole("link", { name: flow.name })).toBeVisible({ timeout: 15_000 });
  await page.waitForTimeout(500);
  const before = await heapMB(cdp);

  // Into the editor (client-side navigation, as a user would).
  await page.getByRole("link", { name: flow.name }).click();
  const canvas = page.getByRole("application", { name: "Flow canvas" });
  await expect(canvas.locator(".svelte-flow__node")).toHaveCount(200, { timeout: 30_000 });
  await page.waitForTimeout(1500); // validation and the first paint settle
  const inEditor = await heapMB(cdp);
  // The first drag of a session also compiles the drag code and renders the inspector for the
  // first time: measured on its own (reported), then the drag the budget is about.
  await startProbe(page);
  const warmFrom = await page.evaluate(() => performance.now());
  const warm = await canvas.locator('[data-node="b109"] .head').boundingBox();
  if (warm) {
    await page.mouse.move(warm.x + 20, warm.y + 6);
    await page.mouse.down();
    for (let i = 1; i <= 10; i++) await page.mouse.move(warm.x + 20 + i * 2, warm.y + 6);
    await page.mouse.up();
    await page.waitForTimeout(1200);
  }

  // A node near the middle of the view: drag it by its header for ~2 s at ~60 Hz.
  const target = canvas.locator('[data-node="b110"] .head');
  await target.scrollIntoViewIfNeeded();
  const box = await target.boundingBox();
  if (!box) throw new Error("no node to drag");
  const from = await page.evaluate(() => performance.now());
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  const steps = 120;
  for (let i = 1; i <= steps; i++) {
    await page.mouse.move(x + (180 * i) / steps, y + (90 * Math.sin((i / steps) * Math.PI)) );
    await page.waitForTimeout(16);
  }
  await page.mouse.up();
  const dropAt = await page.evaluate(() => performance.now());
  await page.waitForTimeout(1500); // the debounced draft save happens ~800 ms after the drop
  const to = await page.evaluate(() => performance.now());
  const p = await page.evaluate(() => (window as unknown as { __probe: Probe }).__probe);

  const frames = p.frames.filter((t) => t >= from && t <= dropAt);
  const warmFrames = p.frames.filter((t) => t >= warmFrom && t < warmFrom + 1000);
  const firstDragGapMax = Math.max(0, ...warmFrames.slice(1).map((t, i) => t - warmFrames[i]));
  const gaps = frames
    .slice(1)
    .map((t, i) => t - frames[i])
    .sort((a, b) => a - b);
  const pct = (q: number) => gaps[Math.min(gaps.length - 1, Math.floor(gaps.length * q))] ?? 0;
  const saves = p.calls.filter((c) => c.cmd === "save_flow_draft" && c.t >= from && c.t <= to);
  const checks = p.calls.filter((c) => c.cmd === "check_flow" && c.t >= from && c.t <= to);
  const moved = await page.evaluate(() => {
    const el = document.querySelector<HTMLElement>('.svelte-flow__node[data-id="b110"]');
    return el?.style.transform ?? "";
  });

  // Leave the editor; the canvas, its listeners and the debug subscription go with it.
  await page.getByRole("link", { name: "Flows" }).first().click();
  await expect(page.getByRole("link", { name: flow.name })).toBeVisible();
  await page.waitForTimeout(800);
  const after = await heapMB(cdp);
  const unsub = await page.evaluate(() =>
    (window as unknown as { __probe: Probe }).__probe.calls.filter((c) => c.cmd === "unsubscribe_flow_debug").length,
  );

  const out = {
    frames: frames.length,
    gapP50: +pct(0.5).toFixed(1),
    gapP95: +pct(0.95).toFixed(1),
    gapMax: +(gaps.at(-1) ?? 0).toFixed(1),
    gapsOver16ms: gaps.filter((g) => g > 17).length,
    firstDragGapMax: +firstDragGapMax.toFixed(1),
    longTasks: p.longTasks.length,
    longTaskMax: Math.round(Math.max(0, ...p.longTasks)),
    saves: saves.length,
    saveAfterDropMs: saves.length ? Math.round(saves[0].t - dropAt) : null,
    checks: checks.length,
    heapListMB: +before.toFixed(1),
    heapEditorMB: +inEditor.toFixed(1),
    heapAfterMB: +after.toFixed(1),
    heapDeltaMB: +(after - before).toFixed(1),
    unsubscribed: unsub,
    movedTo: moved,
    slowFramesAtMs: frames.slice(1).map((t, i) => [Math.round(frames[i] - from), Math.round(t - frames[i])]).filter(([, g]) => g > 17),
  };
  console.log(`flow editor 200 nodes: ${JSON.stringify(out)}`);

  expect(out.longTasks, "long tasks (> 50 ms) while dragging").toBe(0);
  expect(out.gapP95, "frame gap p95 (ms)").toBeLessThanOrEqual(20);
  // One drag is one edit: one save ~800 ms after the drop (≤ 1 per 0.8 s in any case).
  expect(out.saves, "save_flow_draft calls").toBeLessThanOrEqual(Math.ceil((to - from) / 800));
  expect(out.saves).toBeGreaterThanOrEqual(1);
  expect(out.unsubscribed, "debug subscription released").toBe(1);
  expect(out.heapDeltaMB, "heap after leaving the editor (MB over the list page's)").toBeLessThanOrEqual(5);
});
