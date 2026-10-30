// Dialogs always cover the on-air tally lines behind them (`pnpm shots:ui`), also on a busy first
// load. A tester saw Control's red Program line over "New cue" / "Add automation" for a few
// seconds the first time. Every dialog is modal (showModal: the browser's top layer, above any
// z-index), so the line can only show THROUGH a dialog that is still transparent: its open
// animation held on its first frame while the page is busy. Here each dialog is opened on a fresh
// Control page with screens on air, its animations are frozen at that first frame, and: the
// dialog is modal, it is what's under the tile's edge, and no Program red shows inside it.

import { join } from "node:path";
import type { Locator, Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { mirrorScreens, open, screensScenario } from "./harness";

const control = () => ({
  ...screensScenario("dark"),
  control: { view: "grid" as const, selectedScreenId: "s1main01ab", gridColumns: 2, liveEdit: "onTake" as const, hotkeys: {} },
});

/** From now on, a dialog's open animations (its own and its backdrop's) freeze at their first
 *  frame as they start: the frame a busy first load can hold for seconds. */
async function freezeOnOpen(page: Page) {
  await page.evaluate(() => {
    const w = window as unknown as { __frozen: number };
    w.__frozen = 0;
    document.addEventListener(
      "animationstart",
      (e) => {
        if (!(e.target instanceof HTMLDialogElement)) return;
        for (const anim of e.target.getAnimations({ subtree: true })) {
          anim.pause();
          anim.currentTime = 0;
          w.__frozen++;
        }
      },
      true,
    );
  });
}

/** Points on the Program tally lines (the tiles' top and bottom edges) that lie inside the dialog. */
async function linePointsUnder(page: Page, dialog: Locator): Promise<{ x: number; y: number }[]> {
  const d = (await dialog.boundingBox())!;
  const tiles = await page.locator('[data-tally="program"]').evaluateAll((els) =>
    els.map((el) => {
      const r = el.getBoundingClientRect();
      const s = getComputedStyle(el, "::before");
      const inset = parseFloat(s.top) || 0;
      const w = parseFloat(s.borderTopWidth) || 2;
      return { left: r.left, right: r.right, top: r.top + inset + w / 2, bottom: r.bottom - inset - w / 2 };
    }),
  );
  const pts: { x: number; y: number }[] = [];
  for (const t of tiles) {
    for (const y of [t.top, t.bottom]) {
      if (y <= d.y + 1 || y >= d.y + d.height - 1) continue;
      for (let x = Math.max(t.left, d.x) + 8; x < Math.min(t.right, d.x + d.width) - 8; x += 6) pts.push({ x, y });
    }
  }
  return pts;
}

async function covers(page: Page, dialog: Locator, name: string) {
  await expect.poll(() => page.evaluate(() => (window as unknown as { __frozen: number }).__frozen)).toBeGreaterThan(0);
  await page.screenshot({ path: join(process.cwd(), "review", "dialog-stacking", `${name}.png`) });
  expect(await dialog.evaluate((el) => el.matches(":modal")), "the dialog is modal (top layer)").toBe(true);
  expect(await dialog.evaluate((el) => getComputedStyle(el).opacity), "the dialog is opaque from its first frame").toBe("1");
  const pts = await linePointsUnder(page, dialog);
  expect(pts.length, "a tally line runs under the dialog").toBeGreaterThan(10);
  // What's under each point is the dialog, and none of them shows the Program red.
  const png = (await page.screenshot()).toString("base64");
  const red = await page.evaluate(
    async ([b64, points]) => {
      const img = new Image();
      img.src = `data:image/png;base64,${b64}`;
      await img.decode();
      const c = document.createElement("canvas");
      c.width = img.naturalWidth;
      c.height = img.naturalHeight;
      const ctx = c.getContext("2d")!;
      ctx.drawImage(img, 0, 0);
      const k = img.naturalWidth / window.innerWidth;
      return (points as { x: number; y: number }[]).filter((p) => {
        const [r, g, b] = ctx.getImageData(Math.round(p.x * k), Math.round(p.y * k), 1, 1).data;
        return r > 150 && g < 90 && b < 90;
      }).length;
    },
    [png, pts] as const,
  );
  expect(red, "Program red shows through the dialog").toBe(0);
  for (const p of pts.slice(0, 5)) {
    const inside = await dialog.evaluate((el, q) => el.contains(document.elementFromPoint(q.x, q.y)), p);
    expect(inside, "the dialog is on top of the tally line").toBe(true);
  }
}

test.use({ viewport: { width: 1440, height: 900 } });

test.beforeEach(async ({ page, context, server }) => {
  await mirrorScreens(page, context, server);
});

test("New cue covers the on-air tally line from its first frame", async ({ page }) => {
  await open(page, "/control", control());
  await expect(page.locator('[data-tally="program"]').first()).toBeVisible({ timeout: 15_000 });
  // The harness opens with reduced motion; the open animations are what this checks.
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await freezeOnOpen(page);
  await page.getByRole("button", { name: "New cue" }).first().click();
  const dialog = page.locator("dialog[open]");
  await expect(dialog).toBeVisible();
  await covers(page, dialog, "new-cue");
});

test("Add automation covers the on-air tally line from its first frame", async ({ page }) => {
  await open(page, "/control", control());
  await expect(page.locator('[data-tally="program"]').first()).toBeVisible({ timeout: 15_000 });
  // The harness opens with reduced motion; the open animations are what this checks.
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await freezeOnOpen(page);
  await page.getByRole("button", { name: /^Add automation/ }).first().click();
  const dialog = page.locator("dialog[open]");
  await expect(dialog).toBeVisible();
  await covers(page, dialog, "add-automation");
});
