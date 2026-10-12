// Renderer cost in a real browser (Edge), against the real server. Not part of the regular e2e
// run (the file name doesn't match *.spec.ts); run explicitly:
//   pnpm perf:renderer
import { expect, test } from "./fixture";


test("renderer idle CPU and heap over 200 show/hide cycles", async ({ page, live }) => {
  test.setTimeout(400_000);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  const metrics = async () =>
    Object.fromEntries(((await cdp.send("Performance.getMetrics")).metrics).map((m) => [m.name, m.value]));
  const heapAfterGc = async () => {
    await cdp.send("HeapProfiler.collectGarbage");
    await page.waitForTimeout(300);
    return Math.round(((await metrics()).JSHeapUsedSize / 1024) * 10) / 10;
  };

  await page.goto(`${live.base}/screen/test`);
  await page.waitForTimeout(1000);
  await live.cmd("show");
  await expect(page.locator("[data-state]")).toHaveAttribute("data-state", "in");
  await page.waitForTimeout(1500);

  // Idle with a graphic on air: main-thread task time over 20 s.
  const a = await metrics();
  await page.waitForTimeout(20_000);
  const b = await metrics();
  const busyMs = (b.TaskDuration - a.TaskDuration) * 1000;
  console.log(`idle on air: ${busyMs.toFixed(1)} ms of main-thread work in 20 s (${((busyMs / 20_000) * 100).toFixed(3)}%)`);

  await live.cmd("hide");
  await expect(page.locator("[data-state]")).toHaveCount(0, { timeout: 4000 });
  const before = await heapAfterGc();
  const nodesBefore = (await metrics()).Nodes;
  const cycle = async (i: number) => {
    await live.cmd("show");
    await expect(page.locator("[data-state]")).toHaveAttribute("data-state", "in");
    await live.cmd(`fields {"name":"Cycle ${i}"}`);
    await live.cmd("hide");
    await expect(page.locator("[data-state]")).toHaveCount(0, { timeout: 4000 });
  };
  for (let i = 0; i < 50; i++) await cycle(i);
  const mid = await heapAfterGc();
  for (let i = 50; i < 200; i++) await cycle(i);
  const after = await heapAfterGc();
  console.log(`heap: start ${before} KB, after 50 ${mid} KB, after 200 ${after} KB`);
  const nodesAfter = (await metrics()).Nodes;
  console.log(`JS heap after GC: ${before} KB -> ${after} KB; DOM nodes (incl. detached): ${nodesBefore} -> ${nodesAfter}`);
  expect(busyMs / 20_000).toBeLessThan(0.005);
  // The first cycles warm up JIT/caches; after that the heap must stay flat.
  expect(after - mid).toBeLessThan(100);
});

test("renderer: 100-row list update time, clock wakeups on air, nothing at rest off air", async ({ page, live }) => {
  test.setTimeout(120_000);
  // Records when each `fields` frame reaches the page, before the renderer handles it.
  await page.addInitScript(() => {
    const w = window as unknown as { __arrived: number };
    const Orig = window.WebSocket;
    window.WebSocket = class extends Orig {
      constructor(url: string | URL, protocols?: string | string[]) {
        super(url, protocols);
        this.addEventListener("message", (e) => {
          if (typeof e.data === "string" && e.data.includes('"t":"fields"')) w.__arrived = performance.now();
        });
      }
    } as typeof WebSocket;
  });
  // A plain template: a 100-row list and a clock (no data-limit), on its own screen.
  const tpl = await live.cmd(
    `addtemplatedoc ${JSON.stringify({
      name: "Perf list",
      html: '<div class="p"><b>{{now}}</b><ul><li data-each="rows">{{#}} {{.name}} {{.score}}</li></ul></div>',
      css: ".p{font:16px sans-serif;color:#fff}",
      fields: [
        { key: "now", label: "Now", type: "clock", default: "", format: "HH:mm:ss" },
        { key: "rows", label: "Rows", type: "list", default: "[]", columns: [{ key: "name", label: "Name" }, { key: "score", label: "Score" }] },
      ],
    })}`,
  );
  const rows = (k: number) => JSON.stringify(Array.from({ length: 100 }, (_, i) => ({ name: `P${i}`, score: String(i * k) })));
  const doc = {
    id: `perf${Date.now().toString(36)}`,
    name: "Perf",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [{ id: "l1", name: "L", kind: "template", templateId: tpl, rect: { x: 0, y: 0, w: 1920, h: 1080 }, fields: { rows: rows(1) }, vars: {} }],
    updatedAt: "",
  };
  await live.cmd("liveedit immediate");
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  await page.goto(`${live.base}/screen/${doc.id}`);
  await page.waitForTimeout(800);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(page.locator("li")).toHaveCount(100);

  // 100-row update: renderer time from the `fields` frame arriving to the last row patched
  // (MutationObserver), and the command's round trip (Playwright → fixture → reply) separately.
  const times: number[] = [];
  const trips: number[] = [];
  for (let k = 2; k < 12; k++) {
    // Install the observer first (the update can land within 2 ms of the command).
    await page.evaluate(() => {
      (window as unknown as { __done: Promise<number> }).__done = new Promise<number>((resolve) => {
          const w = window as unknown as { __arrived: number };
          w.__arrived = 0;
          let t0 = 0;
          let last = t0;
          let timer = 0;
          const mo = new MutationObserver(() => {
            last = performance.now();
            if (!t0) t0 = w.__arrived || last;
            clearTimeout(timer);
            timer = window.setTimeout(() => (mo.disconnect(), resolve(last - t0)), 200);
          });
          const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot)!;
          // Only the list: the clock ticking in the same template must not count.
          mo.observe(host.shadowRoot!.querySelector("ul")!, { subtree: true, characterData: true, childList: true });
      });
    });
    const c0 = Date.now();
    await live.cmd(`setfields ${doc.id} l1 ${JSON.stringify({ rows: rows(k) })}`);
    trips.push(Date.now() - c0);
    times.push(await page.evaluate(() => (window as unknown as { __done: Promise<number> }).__done));
  }
  const long: number = await page.evaluate(
    () =>
      new Promise((resolve) => {
        const seen: number[] = [];
        new PerformanceObserver((l) => l.getEntries().forEach((e) => seen.push(e.duration))).observe({ type: "longtask", buffered: true });
        setTimeout(() => resolve(seen.length), 100);
      }),
  );
  console.log(`100-row update in the renderer, frame arrival → last mutation (ms): ${times.map((t) => t.toFixed(1)).join(", ")}; long tasks: ${long}`);
  console.log(`setfields command round trip (ms): ${trips.join(", ")}`);

  // Clock on air: count renderer timers over 5 s (expect ~1 per second).
  const countTimers = () =>
    page.evaluate(() => {
      const w = window as unknown as { __t: number };
      w.__t = 0;
      const st = window.setTimeout;
      window.setTimeout = ((...a: Parameters<typeof setTimeout>) => (w.__t++, st(...a))) as typeof setTimeout;
      const raf = window.requestAnimationFrame;
      window.requestAnimationFrame = (cb) => (w.__t += 1000, raf(cb));
    });
  await countTimers();
  await page.waitForTimeout(5000);
  const onAir = await page.evaluate(() => (window as unknown as { __t: number }).__t);
  console.log(`clock on air: ${onAir} timer wakeups in 5 s`);
  expect(onAir).toBeGreaterThanOrEqual(4);
  expect(onAir).toBeLessThanOrEqual(6);

  // Off air: nothing scheduled at rest.
  await live.cmd(`layerhide ${doc.id} l1`);
  await page.waitForTimeout(1500);
  await page.evaluate(() => ((window as unknown as { __t: number }).__t = 0));
  await page.waitForTimeout(3000);
  const rest = await page.evaluate(() => (window as unknown as { __t: number }).__t);
  console.log(`off air: ${rest} timers in 3 s`);
  expect(rest).toBe(0);
});

test("renderer: data-fit on a 100-character name patch, fonts loaded before the first play-in", async ({ page, live }) => {
  test.setTimeout(120_000);
  // Arrival of each `fields` frame (before the renderer handles it), and the font state at the
  // moment a root gets data-state="in".
  await page.addInitScript(() => {
    const w = window as unknown as { __arrived: number; __inAt: { fontsOk: boolean; size: string }[] };
    w.__inAt = [];
    const Orig = window.WebSocket;
    window.WebSocket = class extends Orig {
      constructor(url: string | URL, protocols?: string | string[]) {
        super(url, protocols);
        this.addEventListener("message", (e) => {
          if (typeof e.data === "string" && e.data.includes('"t":"fields"')) w.__arrived = performance.now();
        });
      }
    } as typeof WebSocket;
    const set = Element.prototype.setAttribute;
    Element.prototype.setAttribute = function (this: Element, n: string, v: string) {
      if (n === "data-state" && v === "in") {
        const h = this.querySelector("h1") as HTMLElement | null;
        w.__inAt.push({ fontsOk: document.fonts.check('600 16px "Space Grotesk"'), size: h?.style.fontSize ?? "" });
      }
      return set.call(this, n, v);
    };
  });
  const tpl = await live.cmd(
    `addtemplatedoc ${JSON.stringify({
      name: "Perf fit",
      html: '<div class="lt"><h1 class="name" data-fit>{{name}}</h1><p>{{title}}</p></div>',
      css: '.lt{width:900px;color:#fff;font-family:"Space Grotesk",sans-serif}.name{margin:0;font-size:96px;font-weight:600;line-height:1.1;white-space:nowrap;overflow:hidden}',
      fields: [
        { key: "name", label: "Name", type: "text", default: "" },
        { key: "title", label: "Title", type: "text", default: "" },
      ],
    })}`,
  );
  const long = (i: number) => `${i} ` + "Wolfeschlegelsteinhausenbergerdorff ".repeat(3).slice(0, 98 - String(i).length);
  const doc = {
    id: `fit${Date.now().toString(36)}`,
    name: "Fit",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [{ id: "l1", name: "L", kind: "template", templateId: tpl, rect: { x: 0, y: 0, w: 1920, h: 1080 }, fields: { name: long(0), title: "T" }, vars: {} }],
    updatedAt: "",
  };
  await live.cmd("liveedit immediate");
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  // On air before the page loads: the snapshot's first play-in must wait for the font.
  await live.cmd(`layershow ${doc.id} l1`);
  await page.goto(`${live.base}/screen/${doc.id}`);
  await expect(page.locator("h1")).toHaveCount(1);
  await expect.poll(() => page.evaluate(() => (window as unknown as { __inAt: unknown[] }).__inAt.length)).toBe(1);
  const first = await page.evaluate(() => (window as unknown as { __inAt: { fontsOk: boolean; size: string }[] }).__inAt[0]);
  console.log(`first play-in: Space Grotesk loaded ${first.fontsOk}, name font-size ${first.size || "(CSS)"}`);
  expect(first.fontsOk).toBe(true);
  expect(first.size).not.toBe("");

  const times: number[] = [];
  // A 100-character name (shrinks to the minimum), a short one (CSS size), a mid one (sized by
  // proportion, just below the CSS size).
  const names = (k: number) => [long(k), `Ada ${k}`, `Grace Brewster Hopper ${k}`][(k - 1) % 3];
  for (let k = 1; k <= 12; k++) {
    await page.evaluate(() => {
      (window as unknown as { __done: Promise<number> }).__done = new Promise<number>((resolve) => {
        const w = window as unknown as { __arrived: number };
        w.__arrived = 0;
        let t0 = 0;
        let last = 0;
        let timer = 0;
        const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot)!;
        const mo = new MutationObserver(() => {
          last = performance.now();
          if (!t0) t0 = w.__arrived || last;
          clearTimeout(timer);
          timer = window.setTimeout(() => (mo.disconnect(), resolve(last - t0)), 200);
        });
        mo.observe(host.shadowRoot!, { subtree: true, characterData: true, childList: true, attributes: true });
      });
    });
    await live.cmd(`setfields ${doc.id} l1 ${JSON.stringify({ name: names(k) })}`);
    times.push(await page.evaluate(() => (window as unknown as { __done: Promise<number> }).__done));
    const fits = await page.evaluate(() => {
      const h = [...document.querySelectorAll("div")].find((d) => d.shadowRoot)!.shadowRoot!.querySelector("h1")!;
      return { ok: h.scrollWidth <= h.clientWidth + 1, size: h.style.fontSize };
    });
    if (k % 3 === 2) expect(fits.size).toBe(""); // short name: back to the CSS size
    expect(fits.ok || Math.abs(parseFloat(fits.size) - 96 * 0.6) < 0.01).toBe(true);
    if (k % 3 === 0) expect(fits.ok && fits.size !== "").toBe(true);
  }
  console.log(`data-fit patch (text + fit), frame arrival → last mutation (ms): ${times.map((t) => t.toFixed(2)).join(", ")}`);
  // Budget: a 100-character name ≤ 4 ms. The first mid-length name also shapes the glyphs at a new
  // size (cold, ~3 ms measured); the warm ones are held to the same budget.
  const hundred = times.filter((_, i) => i % 3 === 0);
  const mid = times.filter((_, i) => i % 3 === 2);
  console.log(`100-char: max ${Math.max(...hundred).toFixed(2)} ms; mid-length: ${mid.map((t) => t.toFixed(2)).join(", ")} ms`);
  expect(Math.max(...hundred)).toBeLessThan(4);
  expect(Math.max(...mid.slice(1))).toBeLessThan(4);
});
