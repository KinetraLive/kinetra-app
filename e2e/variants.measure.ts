// Variant swap cost in a real browser (Edge), against the real server: an on-air layer switched 20
// times between two looks, crossfade and out-and-in alternating, each swap waited out. Reports the
// command → first renderer DOM change latency and checks that no host or node is left behind.
//   pnpm perf:renderer
import { expect, test } from "./fixture";
import { openScreen, roots } from "./flow-helpers";
import { looks, oneLayer, settled, twoLooks } from "./variant-helpers";

const pct = (xs: number[], p: number) => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.ceil((p / 100) * s.length) - 1)];
};

test("variant swaps: 20 switches leave one host, flat DOM, command → DOM latency", async ({ page, live }) => {
  test.setTimeout(120_000);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  const nodes = async () => {
    await cdp.send("HeapProfiler.collectGarbage");
    await page.waitForTimeout(200);
    const m = (await cdp.send("Performance.getMetrics")).metrics;
    return m.find((x) => x.name === "Nodes")?.value ?? -1;
  };
  // Elements in the page, shadow roots included.
  const elements = () =>
    page.evaluate(() => {
      const count = (r: Document | ShadowRoot): number =>
        [...r.querySelectorAll("*")].reduce((n, el) => n + 1 + (el.shadowRoot ? count(el.shadowRoot) : 0), 0);
      return count(document);
    });

  const { template, compact } = await twoLooks(live);
  const doc = await oneLayer(live, template);
  await live.cmd("liveedit immediate");
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await settled(page);

  const latencies: number[] = [];
  const durations: number[] = [];
  let afterFirst = { nodes: 0, elements: 0 };
  for (let i = 0; i < 20; i++) {
    const to = i % 2 === 0 ? compact : "default";
    const transition = i % 4 < 2 ? "crossfade" : "outIn";
    // Resolves with the epoch time of the first change under the stage (a host added, or the old
    // root's data-state), observed before the command is sent.
    await page.evaluate(() => {
      const w = window as unknown as { __first: Promise<number> };
      w.__first = new Promise<number>((resolve) => {
        const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot)!;
        const mo = new MutationObserver(() => {
          mo.disconnect();
          resolve(performance.timeOrigin + performance.now());
        });
        mo.observe(host.parentElement!, { childList: true });
        mo.observe(host.shadowRoot!, { subtree: true, childList: true, attributes: true, attributeFilter: ["data-state"] });
      });
    });
    const t0 = Date.now();
    await live.cmd(`setvariant ${doc.id} l1 ${to} ${transition}`);
    const first = await page.evaluate(() => (window as unknown as { __first: Promise<number> }).__first);
    latencies.push(first - t0);
    // The swap is over: one host, the new look, on air and settled.
    await expect.poll(() => looks(page), { intervals: [20] }).toEqual([to === "default" ? "default" : "compact"]);
    await expect(roots(page)).toHaveCount(1);
    await expect(roots(page)).toHaveAttribute("data-state", "in");
    await settled(page);
    durations.push(Date.now() - t0);
    if (i === 0) afterFirst = { nodes: await nodes(), elements: await elements() };
  }
  // One more alternation so the last look matches the first switch's (compact).
  await live.cmd(`setvariant ${doc.id} l1 ${compact} crossfade`);
  await expect.poll(() => looks(page), { intervals: [20] }).toEqual(["compact"]);
  await settled(page);
  const end = { nodes: await nodes(), elements: await elements() };

  console.log(
    `variant swap latency, command → first DOM change (ms): p50 ${pct(latencies, 50).toFixed(1)}, p95 ${pct(latencies, 95).toFixed(1)}, max ${Math.max(...latencies).toFixed(1)}`,
  );
  console.log(`swap to settled (ms, incl. polling): p50 ${pct(durations, 50)}, p95 ${pct(durations, 95)}`);
  console.log(`DOM after the first switch: ${afterFirst.nodes} nodes (${afterFirst.elements} elements); after 21: ${end.nodes} nodes (${end.elements} elements)`);
  await expect(roots(page)).toHaveCount(1);
  expect(end.elements).toBe(afterFirst.elements);
  expect(Math.abs(end.nodes - afterFirst.nodes)).toBeLessThanOrEqual(20);
  expect(pct(latencies, 95)).toBeLessThanOrEqual(50);
});
