// Logo review shots (not part of the e2e suite): the top bar, the welcome screen and Preferences ›
// About, dark and light, for the build's channel, into review/logo/<channel>-<theme>-<view>.png.
//
//   pnpm build && pnpm exec playwright test --config playwright.ui.config.ts e2e/ui/logo.spec.ts
//   KINETRA_CHANNEL=canary (same)   → the canary badge (and its tooltip shot)
//
// Mock backend only: no output server is needed for these views.

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { open } from "./harness";
import type { MockScenario } from "./mock-tauri";

const CHANNEL = process.env.KINETRA_CHANNEL === "canary" ? "canary" : "stable";
const DIR = join("review", "logo");
mkdirSync(DIR, { recursive: true });

const shot = (page: Page, name: string) => page.screenshot({ path: join(DIR, `${CHANNEL}-${name}.png`) });

for (const theme of ["dark", "light"] as const) {
  test.describe(`${CHANNEL} ${theme}`, () => {
    const base: MockScenario = { theme, screens: [], lanAccess: false, selectedScreenId: null, testOnAir: false };

    test("top bar", async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await open(page, "/dashboard", base);
      const bar = page.locator("header.topbar");
      await expect(bar.getByRole("link", { name: "Kinetra" })).toBeVisible();
      await page.mouse.move(700, 600);
      await bar.screenshot({ path: join(DIR, `${CHANNEL}-${theme}-topbar.png`) });
      await page.screenshot({ path: join(DIR, `${CHANNEL}-${theme}-topbar-context.png`), clip: { x: 0, y: 0, width: 720, height: 140 } });
      if (CHANNEL === "canary") {
        await bar.getByText("Canary").hover();
        const tip = page.getByRole("tooltip", { name: "Experimental build. Don't use for live shows." });
        await expect(tip).toBeVisible();
        // It fades in: wait for the end of the fade before the picture.
        await expect.poll(() => tip.evaluate((el) => getComputedStyle(el).opacity)).toBe("1");
        await page.screenshot({ path: join(DIR, `${CHANNEL}-${theme}-topbar-tooltip.png`), clip: { x: 0, y: 0, width: 720, height: 140 } });
      } else {
        await expect(page.getByText("Canary", { exact: true })).toHaveCount(0);
      }
    });

    test("welcome", async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await open(page, "/", { ...base, license: { signedIn: false, account: null, status: "none", plan: null, welcome: true, licenses: [] } } as MockScenario);
      await expect(page.getByRole("heading", { name: "Welcome to Kinetra" })).toBeVisible();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-welcome`);
    });

    test("about", async ({ page }) => {
      await page.setViewportSize({ width: 1200, height: 800 });
      await open(page, "/preferences?section=about", base);
      await expect(page.getByText("Third-party licenses")).toBeVisible();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-about`);
    });

    test("project picker", async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await open(page, "/", base);
      await expect(page.getByRole("heading", { name: "Kinetra" })).toBeVisible();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-picker`);
    });
  });
}
