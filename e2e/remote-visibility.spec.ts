// Remote control's exact `on_air`: an output reports `gone` when a layer's out-animation has
// really finished, and the core counts the layer as visible until then (not at the clear).
import type { Page } from "@playwright/test";
import { expect, test } from "./fixture";
import type { ScreenDoc } from "../src/lib/types";

const root = (page: Page) => page.locator("[data-state]");

test("a layer stays visible until the output reports its out-animation finished", async ({ page, live }) => {
  const templateId = await live.cmd("addtemplate classic-bar");
  const doc: ScreenDoc = {
    id: `rv${Date.now().toString(36)}`,
    name: "Remote",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [
      { id: "l1", name: "Bar", kind: "template", templateId, rect: { x: 0, y: 0, w: 1920, h: 1080 }, fields: { name: "Ada" }, vars: {} },
    ],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  await live.cmd("fadetracking on");
  const visible = () => live.cmd(`visible ${doc.id} l1`);

  const ws = page.waitForEvent("websocket");
  await page.goto(`${live.base}/screen/${doc.id}`);
  const socket = await ws;
  await socket.waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
  const gone: number[] = [];
  socket.on("framesent", (f) => {
    if (String(f.payload).includes('"t":"gone"')) gone.push(Date.now());
  });

  await live.cmd(`layershow ${doc.id} l1`);
  await expect(root(page)).toHaveCount(1);
  await root(page).first().evaluate((el) => Promise.all(el.getAnimations({ subtree: true }).map((a) => a.finished)));
  expect(await visible()).toBe("true");

  // When the layer leaves the page, by the page's own clock (same machine as the test).
  await root(page).first().evaluate((el) => {
    const w = window as unknown as { __removed?: number };
    const host = (el.getRootNode() as ShadowRoot).host;
    new MutationObserver(() => {
      if (!host.isConnected && !w.__removed) w.__removed = Date.now();
    }).observe(document.body, { childList: true, subtree: true });
  });
  const hiddenAt = Date.now();
  await live.cmd(`layerhide ${doc.id} l1`);
  // Off air in Program at once, but still on the output while it animates out.
  expect(await visible()).toBe("true");
  await expect(root(page)).toHaveCount(0, { timeout: 4000 });
  await expect.poll(() => gone.length).toBe(1);
  const removedAt = await page.evaluate(() => (window as unknown as { __removed: number }).__removed);
  // Sent as the element left the page: after the out-animation, not at the clear.
  expect(removedAt - hiddenAt).toBeGreaterThan(100);
  expect(Math.abs(gone[0] - removedAt)).toBeLessThan(100);
  await expect.poll(visible).toBe("false");

  // A cut sends nothing and isn't visible at all.
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(root(page)).toHaveCount(1);
  await live.cmd("panic");
  expect(await visible()).toBe("false");
  await page.waitForTimeout(300);
  expect(gone.length).toBe(1);
  await live.cmd("fadetracking off");
});
