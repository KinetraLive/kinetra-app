// Control's toolbar stays on one line at every supported window width (`pnpm shots:ui`): PANIC is
// never wrapped to a second line. Below its natural width the bar collapses (hints, Columns, View
// and Live edit, the Screens text) and the collapsed controls are in the "More" menu. Checked at
// 1024 (the window's minimum), 1280 and 1440 px, with Studio mode off and on, and with remote
// control's chip in the top bar.

import type { Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { mirrorScreens, open, screensScenario, studioScenario } from "./harness";
import type { MockScenario } from "./mock-tauri";

/** PANIC, Clear screen and the bar: PANIC sits on the bar's only row, inside the window. */
async function oneLine(page: Page, width: number) {
  const panic = page.getByRole("button", { name: /^Panic/ });
  await expect(panic).toBeVisible();
  const bar = page.locator(".bar").filter({ has: panic });
  // The bar settles within a frame or two of a size change (fonts, key labels): retried.
  await expect(async () => {
  const [p, c, b] = await Promise.all([
    panic.boundingBox(),
    page.getByRole("button", { name: /^Clear/ }).first().boundingBox(),
    bar.boundingBox(),
  ]);
  if (!p || !c || !b) throw new Error("toolbar not laid out");
  // Same row as Clear screen (the first control on the right) and inside the window.
  expect(Math.abs(p.y - c.y), `PANIC on another row at ${width} px`).toBeLessThan(4);
  expect(p.x + p.width, `PANIC outside the window at ${width} px`).toBeLessThanOrEqual(width);
  // One row: the bar is no taller than one control row plus its padding.
  expect(b.height, `toolbar wraps at ${width} px`).toBeLessThan(p.height * 2);
  expect(await bar.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
  }).toPass({ timeout: 3000 });
}

// Live previews may still be loading through the mirrored routes when a short test ends.
test.afterEach(async ({ context }) => {
  await context.unrouteAll({ behavior: "ignoreErrors" });
});

const remoteOn: Partial<MockScenario> = {
  remote: { on: true, state: "live", runId: "r", resumed: false, crew: [], hasShowId: true },
};

for (const width of [1024, 1280, 1440]) {
  test(`toolbar on one line at ${width} px`, async ({ page, context, server }) => {
    await mirrorScreens(page, context, server);
    await page.setViewportSize({ width, height: 900 });
    await open(page, "/control", { ...screensScenario("dark"), ...remoteOn, control: { selectedScreenId: "s1main01ab" } });
    await expect(page.getByRole("button", { name: "Remote control: On" })).toBeVisible();
    await oneLine(page, width);
    // Whatever collapsed is still reachable: the More menu holds it.
    const collapse = Number(await page.locator(".bar[data-collapse]").getAttribute("data-collapse"));
    if (collapse >= 3) {
      await page.getByRole("button", { name: "More Control options" }).click();
      await expect(page.getByRole("menuitemcheckbox", { name: "Single view" })).toBeVisible();
      await expect(page.getByRole("menuitemcheckbox", { name: "Live edit: Immediate" })).toBeVisible();
      await page.keyboard.press("Escape");
    } else {
      await expect(page.getByRole("radiogroup", { name: "View" })).toBeVisible();
    }
  });

  test(`toolbar on one line at ${width} px in Studio mode`, async ({ page, context, server }) => {
    await mirrorScreens(page, context, server);
    await page.setViewportSize({ width, height: 900 });
    await open(page, "/control", { ...studioScenario("dark", "grid"), ...remoteOn });
    await expect(page.getByRole("switch", { name: "Studio mode" })).toHaveAttribute("aria-checked", "true");
    await oneLine(page, width);
  });
}

test("at full width nothing collapses", async ({ page, context, server }) => {
  await mirrorScreens(page, context, server);
  await page.setViewportSize({ width: 1920, height: 1000 });
  await open(page, "/control", { ...screensScenario("dark"), control: { selectedScreenId: "s1main01ab" } });
  await expect(page.locator(".bar[data-collapse]")).toHaveAttribute("data-collapse", "0");
  await expect(page.getByRole("button", { name: "More Control options" })).toHaveCount(0);
  await expect(page.getByText(/^(Edits to on-air layers wait for Take|Edits go to air at once)$/)).toBeVisible();
});
