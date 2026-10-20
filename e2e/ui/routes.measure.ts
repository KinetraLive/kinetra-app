// First opens of app routes (`pnpm perf:ui`): the built app with the mock backend. Each round loads
// the dashboard fresh (browser cache off, so every chunk is fetched as on a cold start) and opens
// every route once, timing a link click until the new route has rendered (the first DOM change
// after the URL changed), and the long tasks around it. The entry preloads only CSS for routes
// (vite.config.js `withoutLoaded`), so a route's JS arrives through its import chain; this checks
// that stays quick.

import type { Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { mirrorScreens, open, screensScenario } from "./harness";

const ROUTES = ["/templates", "/sources", "/screens", "/screens/s1main01ab", "/flows", "/polls", "/control"];
const ROUNDS = 7;

/** Clicks a link to `href` and resolves with ms until the route rendered, plus long tasks. */
function firstOpen(page: Page, href: string) {
  return page.evaluate(
    (href) =>
      new Promise<{ ms: number; longTasks: number }>((resolve, reject) => {
        const long: number[] = [];
        const po = new PerformanceObserver((l) => l.getEntries().forEach((e) => long.push(e.duration)));
        po.observe({ type: "longtask" });
        const a = document.createElement("a");
        a.href = href;
        document.body.append(a);
        const t0 = performance.now();
        const mo = new MutationObserver(() => {
          if (location.pathname !== href) return;
          const ms = performance.now() - t0;
          mo.disconnect();
          a.remove();
          // Long tasks are reported after they end.
          setTimeout(() => (po.disconnect(), resolve({ ms, longTasks: long.length })), 100);
        });
        mo.observe(document.body, { childList: true, subtree: true });
        setTimeout(() => reject(new Error(`${href} didn't open`)), 10_000);
        a.click();
      }),
    href,
  );
}

const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];

test("routes: first open of each route", async ({ page, context, server }) => {
  test.setTimeout(300_000);
  await mirrorScreens(page, context, server);
  const cdp = await context.newCDPSession(page);
  await cdp.send("Network.setCacheDisabled", { cacheDisabled: true });
  await open(page, "/dashboard", screensScenario("dark"));

  const times: Record<string, number[]> = Object.fromEntries(ROUTES.map((r) => [r, []]));
  let longTasks = 0;
  for (let round = 0; round < ROUNDS; round++) {
    await page.goto("/dashboard");
    await expect(page.locator("[data-nav]").first()).toBeVisible();
    await page.waitForTimeout(500);
    for (const href of ROUTES) {
      const r = await firstOpen(page, href);
      times[href].push(r.ms);
      longTasks += r.longTasks;
      await page.waitForTimeout(300);
    }
  }
  for (const href of ROUTES) {
    console.log(`${href.padEnd(20)} median ${median(times[href]).toFixed(1)} ms  max ${Math.max(...times[href]).toFixed(1)} ms`);
  }
  console.log(`long tasks: ${longTasks}`);
});
