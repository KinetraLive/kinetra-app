// Tall dialogs fit the window (`pnpm shots:ui`): at 1366×768 and 1920×1080 a dialog sits inside
// the window and never scrolls as a whole, so its footer (Name, Create, Cancel) is always in view;
// only an inner list may scroll. Screenshots of the New template gallery at both sizes go to
// review/dialogs/.

import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { m7Scenario, m9Scenario, mirrorScreens, open, screensScenario } from "./harness";

const OUT = join(process.cwd(), "review", "dialogs");
const SIZES = [
  { width: 1366, height: 768 },
  { width: 1920, height: 1080 },
];

/** Inside the window, not scrolling as a whole, and `action` visible and not covered. */
async function fits(page: Page, dialog: Locator, action: Locator) {
  const vp = page.viewportSize()!;
  await expect(dialog).toBeVisible();
  await expect(action).toBeVisible();
  // Let lazy content (thumbnails, cards) settle before measuring.
  await page.waitForTimeout(400);
  const box = (await dialog.boundingBox())!;
  expect(box.y, "dialog top inside the window").toBeGreaterThanOrEqual(0);
  expect(box.y + box.height, "dialog bottom inside the window").toBeLessThanOrEqual(vp.height + 1);
  const scrolls = await dialog.evaluate((el) => el.scrollHeight > el.clientHeight + 1);
  expect(scrolls, "the dialog itself scrolls (its footer can scroll away)").toBe(false);
  const a = (await action.boundingBox())!;
  expect(a.y + a.height, "action inside the window").toBeLessThanOrEqual(vp.height);
  const hit = await action.evaluate((el) => {
    const r = el.getBoundingClientRect();
    const top = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    return !!top && (el === top || el.contains(top));
  });
  expect(hit, "action not covered").toBe(true);
}

for (const size of SIZES) {
  test.describe(`${size.width}×${size.height}`, () => {
    test.use({ viewport: size });

    test("New template: the cards scroll, Name and Create stay in view", async ({ page }) => {
      await open(page, "/templates", screensScenario("dark"));
      await page.getByRole("button", { name: "New template" }).click();
      const dialog = page.getByRole("dialog", { name: "New template" });
      await expect(dialog.getByRole("option").first()).toBeVisible();
      await page.screenshot({ path: join(OUT, `new-template-${size.width}x${size.height}.png`) });
      await dialog.getByRole("option", { name: /^Lower third/ }).click();
      const nameInput = dialog.getByLabel("Name");
      await expect(nameInput).toBeFocused();
      await expect(nameInput).toHaveValue("Lower third");
      await fits(page, dialog, dialog.getByRole("button", { name: "Create" }));
      // The cards scroll on their own at the small size.
      if (size.height < 1000) {
        const body = dialog.locator(".body");
        expect(await body.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true);
      }
      await page.screenshot({ path: join(OUT, `new-template-chosen-${size.width}x${size.height}.png`) });
    });

    test("New screen", async ({ page }) => {
      await open(page, "/screens", screensScenario("dark"));
      await page.getByRole("button", { name: "New screen" }).first().click();
      const dialog = page.getByRole("dialog", { name: "New screen" });
      await fits(page, dialog, dialog.getByRole("button", { name: "Create" }));
    });

    test("New source", async ({ page }) => {
      await open(page, "/sources", screensScenario("dark"));
      await page.getByRole("button", { name: "New source" }).first().click();
      const dialog = page.getByRole("dialog", { name: "New source" });
      await fits(page, dialog, dialog.getByRole("button", { name: "Cancel" }));
    });

    test("Add automation (recipes)", async ({ page, context, server }) => {
      await mirrorScreens(page, context, server, true);
      await open(page, "/control", {
        ...m7Scenario("dark"),
        control: { selectedScreenId: "s6event001", view: "single", gridColumns: 2, liveEdit: "onTake", hotkeys: {} },
      });
      await page.getByRole("button", { name: "Add automation…" }).first().click();
      const dialog = page.getByRole("dialog", { name: "Add automation" });
      await dialog.getByRole("listbox", { name: "Recipes" }).getByRole("option").first().click();
      await fits(page, dialog, dialog.getByRole("button", { name: "Cancel" }));
    });

    test("Edit rule: Save stays in view while the rule scrolls", async ({ page, context, server }) => {
      await mirrorScreens(page, context, server, true);
      await open(page, "/control", {
        ...m7Scenario("dark"),
        control: { selectedScreenId: "s6event001", view: "single", gridColumns: 2, liveEdit: "onTake", hotkeys: {} },
      });
      await page.locator('[data-recipe="rcp1event0"] .toggle').click();
      const recipe = page.locator('[data-recipe="rcp1event0"]');
      await recipe.locator('[data-flow="rul2before"]').getByRole("button", { name: /^Actions for/ }).click();
      await page.getByRole("menuitem", { name: "Edit", exact: true }).click();
      const dialog = page.getByRole("dialog", { name: "Edit rule" });
      const save = dialog.getByRole("button", { name: "Save" });
      await expect(save).toBeVisible();
      for (const at of ["top", "bottom"] as const) {
        await dialog.evaluate((el, end) => (el.scrollTop = end === "top" ? 0 : el.scrollHeight), at);
        await page.waitForTimeout(150);
        const vp = page.viewportSize()!;
        const b = (await save.boundingBox())!;
        expect(b.y + b.height, `Save inside the window (${at})`).toBeLessThanOrEqual(vp.height);
        const hit = await save.evaluate((el) => {
          const r = el.getBoundingClientRect();
          const top = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
          return !!top && (el === top || el.contains(top));
        });
        expect(hit, `Save not covered (${at})`).toBe(true);
      }
      await page.screenshot({ path: join(OUT, `edit-rule-${size.width}x${size.height}.png`) });
    });

    test("New flow from a template", async ({ page, context, server }) => {
      await mirrorScreens(page, context, server, true);
      await open(page, "/flows", m9Scenario("dark"));
      await page.getByRole("button", { name: "New flow" }).click();
      await page.getByRole("dialog", { name: "New flow" }).getByRole("button", { name: /^From a template/ }).click();
      const dialog = page.getByRole("dialog", { name: "New flow from a template" });
      await expect(dialog.locator("[data-flow-template]").first()).toBeVisible({ timeout: 15_000 });
      await fits(page, dialog, dialog.getByRole("button", { name: "Cancel" }));
    });
  });
}
