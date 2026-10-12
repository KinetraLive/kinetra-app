// Studio mode cost in a real browser (Edge), against the real server:
// - take latency, `studioop take` → first Program DOM change, p50/p95 for cut and crossfade
//   (budget ≤ 50 ms), over 20 cycles of two takes (a ⇄ b), each waited out;
// - DOM flat after those 20 cycles;
// - a T-bar sweep of 60 `tbarSet` in 2 s, and a burst of 60 back to back: `tbar` messages the
//   Program page actually receives (≤ ~30/s), their handling time in the renderer and long tasks
//   (no task > 16 ms).
//   pnpm perf:renderer
import { expect, test } from "./fixture";
import { cue, hosts, op, openBus, settledAll, shown, studio, studioOff, twoLayerScreen } from "./studio-helpers";
import { twoLooks } from "./variant-helpers";
import type { TakeTransition } from "../src/lib/types";

const pct = (xs: number[], p: number) => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.ceil((p / 100) * s.length) - 1)];
};

interface Msg {
  /** Epoch ms when the frame's handler started. */
  t: number;
  /** Handler time (ms): parse + the renderer's handling, in one task. */
  dt: number;
  k: string;
}

test.afterEach(async ({ live }) => {
  await studioOff(live);
});

test("studio: take latency, flat DOM over 20 cycles, T-bar message rate and cost", async ({ page, live }) => {
  test.setTimeout(180_000);
  // Times every frame the renderer handles (its socket sets `onmessage`).
  await page.addInitScript(() => {
    const w = window as unknown as { __msgs: Msg[] };
    w.__msgs = [];
    const d = Object.getOwnPropertyDescriptor(WebSocket.prototype, "onmessage")!;
    Object.defineProperty(WebSocket.prototype, "onmessage", {
      configurable: true,
      get() {
        return d.get!.call(this);
      },
      set(fn: ((e: MessageEvent) => void) | null) {
        d.set!.call(
          this,
          fn &&
            ((e: MessageEvent) => {
              const t = performance.now();
              fn.call(this, e);
              w.__msgs.push({
                t: performance.timeOrigin + t,
                dt: performance.now() - t,
                k: /"t":"(\w+)"/.exec(String(e.data))?.[1] ?? "?",
              });
            }),
        );
      },
    });
  });
  const elements = () =>
    page.evaluate(() => {
      const count = (r: Document | ShadowRoot): number =>
        [...r.querySelectorAll("*")].reduce((n, el) => n + 1 + (el.shadowRoot ? count(el.shadowRoot) : 0), 0);
      return count(document);
    });
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  const nodes = async () => {
    await cdp.send("HeapProfiler.collectGarbage");
    await page.waitForTimeout(200);
    return (await cdp.send("Performance.getMetrics")).metrics.find((x) => x.name === "Nodes")?.value ?? -1;
  };

  const { template } = await twoLooks(live);
  const doc = await twoLayerScreen(live, template);
  const id = doc.id;
  await studioOff(live);
  await openBus(page, live.base, id, "program");
  await live.cmd(`layershow ${id} a`);
  await expect.poll(() => shown(page)).toEqual(["Alpha:default:in"]);
  await settledAll(page);
  await studio(live, { previewAfterTake: "keep" });
  // Long tasks (the API's floor is 50 ms) from here on.
  await page.evaluate(() => {
    const w = window as unknown as { __long: number[] };
    w.__long = [];
    new PerformanceObserver((l) => l.getEntries().forEach((e) => w.__long.push(e.duration))).observe({ type: "longtask" });
  });

  let on = "a";
  /** One take from `on` to the other layer; returns command → first Program DOM change (ms). */
  const takeOnce = async (transition: TakeTransition) => {
    const next = on === "a" ? "b" : "a";
    await cue(live, id, on, false);
    await cue(live, id, next, true);
    await page.evaluate(() => {
      const w = window as unknown as { __first: Promise<number> };
      const box = document.querySelector("body > div > div")!;
      w.__first = new Promise<number>((resolve) => {
        const mo = new MutationObserver(() => {
          mo.disconnect();
          resolve(performance.timeOrigin + performance.now());
        });
        mo.observe(box, { childList: true });
        for (const h of box.children)
          if (h.shadowRoot) mo.observe(h.shadowRoot, { subtree: true, childList: true, attributes: true, attributeFilter: ["data-state"] });
      });
    });
    const t0 = Date.now();
    await op(live, id, { op: "take", transition });
    const first = await page.evaluate(() => (window as unknown as { __first: Promise<number> }).__first);
    on = next;
    const name = on === "a" ? "Alpha" : "Bravo";
    await expect.poll(() => hosts(page), { intervals: [20] }).toEqual([expect.objectContaining({ name, state: "in", opacity: 1 })]);
    await settledAll(page);
    return first - t0;
  };

  // Warm-up: the first studio message loads the renderer's studio chunk.
  await takeOnce({ kind: "cut", ms: 100 });
  await takeOnce({ kind: "cut", ms: 100 });

  const lat: Record<string, number[]> = { cut: [], crossfade: [] };
  let afterFirst = { nodes: 0, elements: 0 };
  for (let i = 0; i < 20; i++) {
    lat.cut.push(await takeOnce({ kind: "cut", ms: 100 }));
    lat.crossfade.push(await takeOnce({ kind: "crossfade", ms: 300 }));
    if (i === 0) afterFirst = { nodes: await nodes(), elements: await elements() };
  }
  const end = { nodes: await nodes(), elements: await elements() };
  for (const k of ["cut", "crossfade"])
    console.log(
      `take latency, ${k}, command → first Program DOM change (ms): p50 ${pct(lat[k], 50).toFixed(1)}, p95 ${pct(lat[k], 95).toFixed(1)}, max ${Math.max(...lat[k]).toFixed(1)}`,
    );
  console.log(`DOM after the first take cycle: ${afterFirst.nodes} nodes (${afterFirst.elements} elements); after 20: ${end.nodes} nodes (${end.elements} elements)`);

  // T-bar sweeps (Program on b; cue a in, b out).
  const sweep = async (label: string, gapMs: number) => {
    const next = on === "a" ? "b" : "a";
    await cue(live, id, on, false);
    await cue(live, id, next, true);
    await op(live, id, { op: "tbarBegin" });
    await expect.poll(async () => (await hosts(page)).length).toBe(2);
    await page.evaluate(() => {
      const w = window as unknown as { __msgs: Msg[]; __long: number[]; __gaps: number[]; __raf: boolean };
      w.__msgs = [];
      w.__long = [];
      w.__gaps = [];
      w.__raf = true;
      let last = performance.now();
      const tick = (t: number) => {
        w.__gaps.push(t - last);
        last = t;
        if (w.__raf) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    const t0 = Date.now();
    for (let k = 1; k <= 60; k++) {
      await op(live, id, { op: "tbarSet", p: k / 60 });
      const wait = t0 + k * gapMs - Date.now();
      if (wait > 0) await new Promise((r) => setTimeout(r, wait));
    }
    const sent = Date.now() - t0;
    await page.waitForTimeout(150);
    const m = await page.evaluate(() => {
      const w = window as unknown as { __msgs: Msg[]; __long: number[]; __gaps: number[]; __raf: boolean };
      w.__raf = false;
      return { msgs: w.__msgs.filter((x) => x.k === "tbar"), long: w.__long, gaps: w.__gaps.slice(1) };
    });
    // Most `tbar` messages in any 1 s window.
    const ts = m.msgs.map((x) => x.t);
    const perSecond = Math.max(0, ...ts.map((t) => ts.filter((u) => u >= t && u < t + 1000).length));
    const span = ts.length > 1 ? ts[ts.length - 1] - ts[0] : 0;
    const dts = m.msgs.map((x) => x.dt);
    console.log(
      `T-bar ${label}: 60 tbarSet in ${sent} ms → ${m.msgs.length} tbar messages received over ${span.toFixed(0)} ms (max ${perSecond} in any 1 s); handling p50 ${pct(dts, 50).toFixed(2)} ms, max ${Math.max(...dts).toFixed(2)} ms; long tasks ${m.long.length}; frame gap p95 ${pct(m.gaps, 95).toFixed(1)} ms, max ${Math.max(...m.gaps).toFixed(1)} ms`,
    );
    await expect.poll(async () => (await hosts(page)).find((h) => h.name === (next === "a" ? "Alpha" : "Bravo"))?.opacity).toBe(1);
    await op(live, id, { op: "tbarEnd", commit: true });
    on = next;
    await expect.poll(() => hosts(page)).toEqual([expect.objectContaining({ name: on === "a" ? "Alpha" : "Bravo", opacity: 1 })]);
    return { perSecond, count: m.msgs.length, maxDt: Math.max(...dts), long: m.long.length };
  };
  const paced = await sweep("sweep (2 s)", 2000 / 60);
  const burst = await sweep("burst", 0);

  await expect(page.locator("[data-state]")).toHaveCount(1);
  expect(end.elements).toBe(afterFirst.elements);
  expect(Math.abs(end.nodes - afterFirst.nodes)).toBeLessThanOrEqual(20);
  expect(pct(lat.cut, 95)).toBeLessThanOrEqual(50);
  expect(pct(lat.crossfade, 95)).toBeLessThanOrEqual(50);
  for (const s of [paced, burst]) {
    expect(s.perSecond).toBeLessThanOrEqual(32);
    expect(s.maxDt).toBeLessThanOrEqual(16);
    expect(s.long).toBe(0);
  }
  // The last position always arrives (coalesced, never dropped).
  expect(burst.count).toBeGreaterThan(0);
});
