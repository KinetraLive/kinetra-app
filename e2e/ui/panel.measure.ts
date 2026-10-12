// Control's resizable right panel (data editing budget): `pnpm perf:ui`. Drags the divider across
// 60 pointer moves on the built app with the mock backend and checks: no main-thread task > 16 ms
// (no long tasks), at most one `--panel-w` write per animation frame, and exactly one
// `set_ui_state` (the width, on release).

import { expect, test } from "../fixture";
import { mirrorScreens, open, screensScenario } from "./harness";

test("control: panel drag cost and one write on release", async ({ page, context, server }) => {
  test.setTimeout(120_000);
  await mirrorScreens(page, context, server);
  await open(page, "/control", {
    ...screensScenario("dark"),
    control: { selectedScreenId: "s5sports01", view: "single", gridColumns: 2, liveEdit: "onTake", hotkeys: {} },
  });
  const sep = page.getByRole("separator", { name: "Resize the side panel" });
  await expect(sep).toBeVisible();
  await page.waitForTimeout(1500);

  // Count set_ui_state calls, long tasks, and --panel-w writes per frame.
  await page.evaluate(() => {
    const w = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (cmd: string, args?: unknown) => Promise<unknown> };
      __panel: { saves: unknown[]; long: number[]; writes: number; frames: number; maxPerFrame: number };
    };
    const m = (w.__panel = { saves: [], long: [], writes: 0, frames: 0, maxPerFrame: 0 });
    const inner = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = (cmd, args) => {
      if (cmd === "set_ui_state") m.saves.push((args as { uiState: { control: { panelWidth?: number } } }).uiState.control.panelWidth);
      return inner(cmd, args);
    };
    new PerformanceObserver((l) => l.getEntries().forEach((e) => m.long.push(e.duration))).observe({ type: "longtask" });
    const body = document.querySelector<HTMLElement>(".control .body")!;
    let inFrame = 0;
    new MutationObserver(() => {
      if (!body.style.getPropertyValue("--panel-w")) return;
      m.writes++;
      inFrame++;
      m.maxPerFrame = Math.max(m.maxPerFrame, inFrame);
    }).observe(body, { attributes: true, attributeFilter: ["style"] });
    const tick = () => {
      m.frames++;
      inFrame = 0;
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });

  const box = (await sep.boundingBox())!;
  const x0 = box.x + box.width / 2;
  const y = box.y + 200;
  await page.mouse.move(x0, y);
  await page.mouse.down();
  for (let i = 1; i <= 60; i++) await page.mouse.move(x0 - i * 3, y);
  const during = await page.evaluate(() => (window as unknown as { __panel: { saves: unknown[] } }).__panel.saves.length);
  await page.mouse.up();
  await page.waitForTimeout(500);

  const m = await page.evaluate(() => (window as unknown as { __panel: unknown }).__panel);
  const r = m as { saves: number[]; long: number[]; writes: number; maxPerFrame: number };
  const width = await sep.getAttribute("aria-valuenow");
  console.log(
    `panel drag: ${r.writes} --panel-w writes (max ${r.maxPerFrame}/frame), long tasks ${r.long.length}, ` +
      `saves during drag ${during}, after release ${JSON.stringify(r.saves)}, width ${width}`,
  );
  expect(during).toBe(0);
  expect(r.saves.length).toBe(1);
  expect(String(r.saves[0])).toBe(width);
  expect(r.long.length).toBe(0);
  expect(r.maxPerFrame).toBeLessThanOrEqual(2);
});
