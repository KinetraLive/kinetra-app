// Template editor layout at common window widths (pnpm shots:ui runs it): with many looks the
// looks row must scroll inside the left column, never widen it (10 looks once pushed the code
// under the preview and gave the window a horizontal scrollbar).

import { expect, test } from "@playwright/test";
import { loadTemplates, open } from "./harness";

const data = loadTemplates();
const most = [...data.templates].sort((a, b) => (b.variants?.length ?? 0) - (a.variants?.length ?? 0))[0];

for (const width of [1100, 1280, 1440]) {
  test(`template editor with ${most.variants?.length} looks fits ${width}px`, async ({ page }) => {
    expect(most.variants?.length ?? 0).toBeGreaterThanOrEqual(8);
    await page.setViewportSize({ width, height: 900 });
    await open(page, `/templates/${most.id}`, {
      theme: "dark",
      screens: [],
      lanAccess: false,
      selectedScreenId: null,
      testOnAir: false,
      templates: data.templates,
      presets: data.presets,
    });
    await page.getByRole("tab", { name: "CSS" }).click();
    const m = await page.evaluate(() => {
      const box = (s: string) => document.querySelector(s)!.getBoundingClientRect();
      const looks = document.querySelector('[role="tablist"][aria-label="Variants"]') as HTMLElement;
      return {
        page: document.documentElement.scrollWidth,
        left: box(".left").width,
        panel: box(".vpanel").width,
        looksScroll: looks.scrollWidth > looks.clientWidth,
      };
    });
    await page.screenshot({ path: `review/layout/editor-${width}.png` });
    expect(m.page).toBe(width);
    expect(m.panel).toBeLessThanOrEqual(m.left);
    expect(m.looksScroll).toBe(true);
  });
}
