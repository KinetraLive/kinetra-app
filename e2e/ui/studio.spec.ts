// Studio mode screenshots (`pnpm shots:ui`): the single view with Preview and Program side by side
// (T-bar part-way), the layer rows with cues and "Program changed since cue", the settings
// popover, the exit question and the grid with "Preview pending" badges. Both themes, into
// review/studio/. The Preview iframe is the real renderer showing the mock's Preview, mirrored into
// the fixture as its own screen (harness.ts, mock-tauri.ts).

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { mirrorScreens, open, studioScenario } from "./harness";

const OUT = join(process.cwd(), "review", "studio");
mkdirSync(OUT, { recursive: true });

async function shot(page: Page, name: string) {
  await page.waitForTimeout(600);
  await page.screenshot({ path: join(OUT, `${name}.png`) });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, studio mode`, () => {
    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server);
    });

    test("single view, layer rows, settings, exit question", async ({ page }) => {
      await open(page, "/control", studioScenario(theme, "single"));
      const pvw = page.frameLocator('iframe[title="Main program Preview"]');
      const pgm = page.frameLocator('iframe[title="Main program Program"]');
      await expect(pvw.locator('[data-state="in"]').first()).toBeAttached({ timeout: 15_000 });
      await expect(pgm.locator('[data-state="in"]').first()).toBeAttached({ timeout: 15_000 });
      await expect(page.locator("iframe")).toHaveCount(2);
      await expect(page.getByRole("slider", { name: "T-bar" })).toHaveAttribute("aria-valuenow", "45");
      await page.waitForTimeout(1200);
      await shot(page, `${theme}-70-studio-single`);

      // The rows: Breaking strap cued in with a staged edit, the LIVE bug cued out (changed since).
      await page.getByRole("button", { name: "Fields and hotkey of Breaking strap" }).click();
      await expect(page.getByText("Program changed since cue")).toBeVisible();
      await page.getByRole("complementary", { name: "Selected screen" }).screenshot({
        path: join(OUT, `${theme}-71-studio-layers.png`),
      });

      await page.getByRole("button", { name: "Studio mode settings" }).click();
      await expect(page.getByRole("dialog", { name: "Studio mode" })).toBeVisible();
      await shot(page, `${theme}-72-studio-settings`);
      await page.keyboard.press("Escape");

      await page.getByRole("switch", { name: "Studio mode" }).click();
      const exit = page.getByRole("dialog", { name: "Turn Studio mode off?" });
      await expect(exit).toBeVisible();
      await shot(page, `${theme}-73-studio-exit`);
      await exit.getByRole("button", { name: "Cancel" }).click();
      await expect(exit).toBeHidden();
    });

    test("grid with Preview pending badges", async ({ page }) => {
      await open(page, "/control", studioScenario(theme, "grid"));
      await expect(page.getByText("Preview pending")).toHaveCount(2);
      await page.waitForTimeout(1500);
      await shot(page, `${theme}-74-studio-grid`);
    });
  });
}
