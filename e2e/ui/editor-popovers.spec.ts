// The layout editor's menus and popovers are never clipped or covered (`pnpm shots:ui`). The Add
// layer menu once opened inside the Layers column, which scrolls (`overflow: auto`): the column
// cut it off at its edge and the canvas covered the rest. Each panel here opens from a column,
// and at every sample point across it the element on top is the panel itself; it stays inside the
// window and gives the page no horizontal scrollbar. Screenshots into review/popovers/.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { EVENT_SCREEN, m7Scenario, mirrorScreens, open, screensScenario } from "./harness";

const OUT = join(process.cwd(), "review", "popovers");
mkdirSync(OUT, { recursive: true });

/** Sample points (a 5 × 5 grid) inside the panel's box, and whether each one hits the panel. */
async function onTop(panel: Locator): Promise<{ hits: number; points: number }> {
  return panel.evaluate((el) => {
    const r = el.getBoundingClientRect();
    let hits = 0;
    let points = 0;
    for (let i = 0; i < 5; i++) {
      for (let j = 0; j < 5; j++) {
        const x = r.left + 4 + ((r.width - 8) * i) / 4;
        const y = r.top + 4 + ((r.height - 8) * j) / 4;
        points++;
        const hit = document.elementFromPoint(x, y);
        if (hit && el.contains(hit)) hits++;
      }
    }
    return { hits, points };
  });
}

async function check(page: Page, panel: Locator, name: string) {
  await expect(panel).toBeVisible();
  await page.waitForTimeout(300);
  await page.screenshot({ path: join(OUT, `${name}.png`) });
  const box = (await panel.boundingBox())!;
  const view = page.viewportSize()!;
  const { hits, points } = await onTop(panel);
  expect(hits, `${name}: the panel is on top at every sample point`).toBe(points);
  expect(box.x, `${name}: inside the window (left)`).toBeGreaterThanOrEqual(0);
  expect(box.y, `${name}: inside the window (top)`).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width, `${name}: inside the window (right)`).toBeLessThanOrEqual(view.width);
  expect(box.y + box.height, `${name}: inside the window (bottom)`).toBeLessThanOrEqual(view.height);
  const scroll = await page.evaluate(() => ({
    page: document.documentElement.scrollWidth > window.innerWidth,
    columns: [...document.querySelectorAll("aside")].some((a) => a.scrollWidth > a.clientWidth + 1),
  }));
  expect(scroll, `${name}: no horizontal scrollbar on the page or a column`).toEqual({ page: false, columns: false });
}

test.use({ viewport: { width: 1280, height: 800 } });

test.describe("layout editor popovers", () => {
  test("Add layer menu opens above the canvas, unclipped", async ({ page, context, server }) => {
    await mirrorScreens(page, context, server);
    await open(page, "/screens/s1main01ab", screensScenario("dark"));
    const out = page.frameLocator('iframe[title="Screen output"]');
    await expect(out.locator('[data-state="in"]')).toHaveCount(2, { timeout: 15_000 });
    await page.getByRole("button", { name: "Add layer" }).click();
    await check(page, page.getByRole("menu", { name: "Add a layer from a template" }), "add-layer");
    // Escape closes it and gives focus back to the button.
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Add layer" })).toBeFocused();
  });

  test("Add layer menu in a short window", async ({ page, context, server }) => {
    await page.setViewportSize({ width: 1100, height: 420 });
    await mirrorScreens(page, context, server);
    await open(page, "/screens/s1main01ab", screensScenario("dark"));
    await page.getByRole("button", { name: "Add layer" }).click();
    await check(page, page.getByRole("menu", { name: "Add a layer from a template" }), "add-layer-short");
  });

  test("Bind to source menu opens from the Inspector, unclipped", async ({ page, context, server }) => {
    await mirrorScreens(page, context, server);
    await open(page, "/screens/s1main01ab", screensScenario("dark"));
    const out = page.frameLocator('iframe[title="Screen output"]');
    await expect(out.locator('[data-state="in"]')).toHaveCount(2, { timeout: 15_000 });
    // The guest's fields are bound already (Unbind); the strap's aren't.
    await page.getByRole("complementary", { name: "Layers" }).getByText("Breaking strap", { exact: true }).click();
    const inspector = page.getByRole("complementary", { name: "Inspector" });
    await inspector.getByRole("button", { name: /^Bind .* to source$/ }).first().click();
    const menu = page.getByRole("menu", { name: /^Bind .* to a source$/ });
    await check(page, menu, "bind-menu");
    // The columns step (a click inside the panel doesn't close it).
    await menu.getByRole("menuitem").first().click();
    await check(page, page.getByRole("menu", { name: /^Columns of / }), "bind-menu-columns");
  });

  test("Link to data table popover opens from the Inspector, unclipped", async ({ page, context, server }) => {
    await mirrorScreens(page, context, server, true);
    await open(page, `/screens/${EVENT_SCREEN}`, m7Scenario("dark"));
    await page.getByRole("complementary", { name: "Layers" }).getByText("Schedule board", { exact: true }).first().click();
    const inspector = page.getByRole("complementary", { name: "Inspector" });
    await inspector.getByRole("button", { name: /^Link (options of .*|.* to a data table…)$/ }).first().click();
    const panel = page.getByRole("dialog", { name: /^Link .* to a data table$/ });
    await check(page, panel, "list-bind");
    // Focus starts on its first select, and Escape closes it.
    await expect(panel.locator("select").first()).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(panel).toHaveCount(0);
  });

  test("Link to data table popover low in a short window", async ({ page, context, server }) => {
    await page.setViewportSize({ width: 1280, height: 560 });
    await mirrorScreens(page, context, server, true);
    await open(page, `/screens/${EVENT_SCREEN}`, m7Scenario("dark"));
    await page.getByRole("complementary", { name: "Layers" }).getByText("Schedule board", { exact: true }).first().click();
    const inspector = page.getByRole("complementary", { name: "Inspector" });
    const button = inspector.getByRole("button", { name: /^Link (options of .*|.* to a data table…)$/ }).first();
    // The field near the bottom of the window: little room below it.
    await button.evaluate((el) => {
      const col = el.closest("aside")!;
      col.scrollTop += el.getBoundingClientRect().bottom - (window.innerHeight - 60);
    });
    await button.click();
    await check(page, page.getByRole("dialog", { name: /^Link .* to a data table$/ }), "list-bind-short");
  });
});
