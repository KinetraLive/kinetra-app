// Screen tile states without the viewfinder brackets (`pnpm shots:ui`): one Control tile per state,
// normal, selected, Preview (Studio, pending), Program (on air) and selected + Program, in both
// themes, into review/tally/. The grid is the Studio scenario (harness.ts): Confidence monitor is
// idle, Lower thirds has a cue pending, Opener is on air, Main program is on air (and cued).

import { mkdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { mirrorScreens, open, studioScenario } from "./harness";

const OUT = join(process.cwd(), "review", "tally");
mkdirSync(OUT, { recursive: true });

/** The tile with the room around it (the tally line and focus ring sit outside its edge). */
async function tile(page: Page, screen: string, name: string) {
  const box = await page.locator(`article[data-screen="${screen}"]`).boundingBox();
  if (!box) throw new Error(`no tile ${screen}`);
  const m = 12;
  await page.screenshot({
    path: join(OUT, `${name}.png`),
    clip: { x: box.x - m, y: box.y - m, width: box.width + 2 * m, height: box.height + 2 * m },
  });
}

for (const theme of ["dark", "light"] as const) {
  test(`${theme}: tile states`, async ({ page, context, server }) => {
    await mirrorScreens(page, context, server);
    // Main program selected (on air): normal, Preview, Program, selected + Program.
    await open(page, "/control", studioScenario(theme, "grid"));
    await expect(page.getByText("Preview pending")).toHaveCount(2);
    await page.waitForTimeout(1200);
    const at = (s: string) => page.locator(`article[data-screen="${s}"]`);
    await expect(at("s1main01ab")).toHaveAttribute("data-tally", "program");
    await expect(at("s1main01ab")).toHaveAttribute("data-selected", "");
    await expect(at("s2thirds01")).toHaveAttribute("data-tally", "preview");
    await expect(at("s3opener01")).toHaveAttribute("data-tally", "program");
    await expect(at("s4confid01")).not.toHaveAttribute("data-tally");
    await tile(page, "s4confid01", `${theme}-1-normal`);
    await tile(page, "s2thirds01", `${theme}-3-preview`);
    await tile(page, "s3opener01", `${theme}-4-program`);
    await tile(page, "s1main01ab", `${theme}-5-selected-program`);
    await page.screenshot({ path: join(OUT, `${theme}-0-grid.png`) });

    // Selected alone: pick the idle Confidence monitor.
    await at("s4confid01").locator("button.pick").click();
    await expect(at("s4confid01")).toHaveAttribute("data-selected", "");
    await page.waitForTimeout(400);
    await tile(page, "s4confid01", `${theme}-2-selected`);
  });
}
