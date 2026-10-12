import type { Page } from "@playwright/test";
import { expect, test } from "./fixture";

// Playwright's CSS locators pierce open shadow roots, so these find the template root inside a
// layer's shadow DOM.
const root = (page: Page) => page.locator("[data-state]");

/** Opens a screen and waits until the renderer has received its snapshot. */
async function openScreen(page: Page, base: string, screen = "test") {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${screen}`);
  const socket = await ws;
  await socket.waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"') || String(f.payload).includes('"t":"error"'));
  return errors;
}

/** Resolves once every animation on the template root (and its subtree) has finished. */
async function animationsSettled(page: Page) {
  await root(page).first().evaluate((el) =>
    Promise.all(el.getAnimations({ subtree: true }).map((a) => a.finished)),
  );
}

/** Counts `animationstart` events on the template root from now on. */
async function countAnimationStarts(page: Page) {
  await root(page).first().evaluate((el) => {
    const w = window as unknown as { __starts: number; __root: Element };
    w.__starts = 0;
    w.__root = el;
    el.addEventListener("animationstart", () => w.__starts++);
    el.querySelectorAll("*").forEach((c) => c.addEventListener("animationstart", () => w.__starts++));
  });
}

test("an empty screen is transparent and shows nothing", async ({ page, live }) => {
  const errors = await openScreen(page, live.base);
  await expect(root(page)).toHaveCount(0);
  const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  expect(bg).toBe("rgba(0, 0, 0, 0)");
  expect(errors).toEqual([]);
});

test("show plays the lower third in and matches the baseline", async ({ page, live }) => {
  await openScreen(page, live.base);
  await live.cmd("show");
  await expect(root(page)).toHaveAttribute("data-state", "in");
  await animationsSettled(page);
  await expect(page.getByText("Ada Lovelace")).toBeVisible();
  await expect(page).toHaveScreenshot("lower-third.png", { omitBackground: true });
});

test("descenders and accented capitals are not clipped", async ({ page, live }) => {
  await openScreen(page, live.base);
  await live.cmd(`fields {"name":"Ludvig Rørvik · Åsa Ødegård","title":"Typography: gjpqy ÅÄÖ Ÿ É"}`);
  await live.cmd("show");
  await animationsSettled(page);
  await expect(page).toHaveScreenshot("lower-third-glyphs.png", { omitBackground: true });
});

test("a field edit on air updates in place without replaying the animation", async ({ page, live }) => {
  await openScreen(page, live.base);
  await live.cmd("show");
  await animationsSettled(page);
  await countAnimationStarts(page);

  await live.cmd(`fields {"name":"Grace Hopper"}`);
  await expect(page.getByText("Grace Hopper")).toBeVisible();
  const same = await root(page).first().evaluate((el) => {
    const w = window as unknown as { __starts: number; __root: Element };
    return { sameNode: w.__root === el, starts: w.__starts };
  });
  expect(same).toEqual({ sameNode: true, starts: 0 });
});

test("field values are escaped", async ({ page, live }) => {
  await openScreen(page, live.base);
  await live.cmd(`fields {"name":"<img src=x onerror=\\"window.__pwned=1\\">"}`);
  await live.cmd("show");
  await expect(page.getByText("<img src=x")).toBeVisible();
  await expect(page.locator("img")).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as { __pwned?: number }).__pwned)).toBeUndefined();
});

test("hide plays out and removes the layer from the DOM", async ({ page, live }) => {
  await openScreen(page, live.base);
  await live.cmd("show");
  await animationsSettled(page);
  await live.cmd("hide");
  await expect(root(page)).toHaveCount(0, { timeout: 4000 });
});

test("a server restart reconnects without replaying the on-air animation", async ({ page, live }) => {
  await openScreen(page, live.base);
  await live.cmd("show");
  await animationsSettled(page);
  await countAnimationStarts(page);

  const reconnected = page.waitForEvent("websocket", { timeout: 10_000 });
  await live.cmd("restart");
  const socket = await reconnected;
  await socket.waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
  await page.waitForTimeout(300);

  const after = await root(page).first().evaluate((el) => {
    const w = window as unknown as { __starts: number; __root: Element };
    return { state: el.getAttribute("data-state"), sameNode: w.__root === el, starts: w.__starts };
  });
  expect(after).toEqual({ state: "in", sameNode: true, starts: 0 });
});

test("an unknown screen stays transparent without errors", async ({ page, live }) => {
  const errors = await openScreen(page, live.base, "does-not-exist");
  await page.waitForTimeout(500);
  await expect(root(page)).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("show reaches the renderer within 50 ms", async ({ page, live }) => {
  await openScreen(page, live.base);
  // Record (wall clock) when data-state="in" first appears anywhere, including shadow roots.
  await page.evaluate(() => {
    const w = window as unknown as { __shownAt?: number };
    const seen = () => {
      for (const host of document.querySelectorAll("*")) {
        if (host.shadowRoot?.querySelector('[data-state="in"]')) return true;
      }
      return !!document.querySelector('[data-state="in"]');
    };
    const tick = () => {
      if (seen()) w.__shownAt = performance.timeOrigin + performance.now();
    };
    const mo = new MutationObserver(() => {
      tick();
      if (w.__shownAt) mo.disconnect();
    });
    mo.observe(document, { subtree: true, childList: true, attributes: true });
    // Shadow roots don't report to a document observer; catch attribute changes inside them too.
    const orig = Element.prototype.attachShadow;
    Element.prototype.attachShadow = function (init) {
      const sr = orig.call(this, init);
      new MutationObserver(tick).observe(sr, { subtree: true, childList: true, attributes: true });
      return sr;
    };
  });
  const sent = Date.now();
  await live.cmd("show");
  await expect.poll(() => page.evaluate(() => (window as unknown as { __shownAt?: number }).__shownAt)).toBeTruthy();
  const shownAt = await page.evaluate(() => (window as unknown as { __shownAt: number }).__shownAt);
  const latency = shownAt - sent;
  console.log(`show latency: ${latency.toFixed(1)} ms`);
  expect(latency).toBeLessThan(50);
});

test("50 show/hide cycles leave no nodes behind", async ({ page, live }) => {
  // 50 real in/out animations (~0.8 s each).
  test.setTimeout(120_000);
  await openScreen(page, live.base);
  const count = () =>
    page.evaluate(() => {
      let n = document.querySelectorAll("*").length;
      for (const host of document.querySelectorAll("*")) n += host.shadowRoot?.querySelectorAll("*").length ?? 0;
      return n;
    });
  const before = await count();
  for (let i = 0; i < 50; i++) {
    await live.cmd("show");
    await expect(root(page)).toHaveAttribute("data-state", "in");
    await live.cmd("hide");
    await expect(root(page)).toHaveCount(0, { timeout: 4000 });
  }
  expect(await count()).toBe(before);
});
