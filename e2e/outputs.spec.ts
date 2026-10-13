// Display outputs against the real server: the renderer's output mode (`?output=`: fit to the
// window, background, cursor hidden, key matte, dual fill + key from one connection), the static
// test pattern page (/pattern, renderer/pattern.html) and the idle page (static/output-idle.html).
// Nothing here opens a real output window: the pages are loaded in Edge at the window's size.

import type { Page } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { gzipSync } from "node:zlib";
import { expect, test, type Fixture } from "./fixture";
import type { Rect, ScreenDoc, ScreenLayer } from "../src/lib/types";

const plate = (name: string, color: string) => ({
  name,
  html: '<div class="lt"><div class="plate"></div></div>',
  css: `.lt{position:absolute;inset:0}
.plate{position:absolute;inset:0;background:${color}}
.lt[data-state="in"]{animation:in 400ms linear both}
.lt[data-state="out"]{animation:out 200ms forwards}
@keyframes in{from{opacity:0}}
@keyframes out{to{opacity:0}}`,
  fields: [],
  design: { w: 400, h: 200, sizing: "fit" },
});

/** Red, opaque, at A; white at 50 % opacity at B (screen px, 1920×1080). */
const A: Rect = { x: 100, y: 100, w: 400, h: 200 };
const B: Rect = { x: 700, y: 100, w: 400, h: 200 };
const center = (r: Rect) => [r.x + r.w / 2, r.y + r.h / 2] as const;

let red = "";
let white = "";
let seq = 0;

async function makeScreen(live: Fixture): Promise<string> {
  const l = (id: string, templateId: string, rect: Rect, opacity?: number): ScreenLayer => ({
    id,
    name: id,
    kind: "template",
    templateId,
    rect,
    fields: {},
    vars: {},
    sizing: "fit",
    opacity,
  });
  const doc: ScreenDoc = {
    id: `out${Date.now().toString(36)}${seq++}`.slice(0, 20),
    name: "Output",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [l("a", red, A), l("b", white, B, 50)],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc.id;
}

async function showAll(live: Fixture, id: string) {
  await live.cmd(`layershow ${id} a`);
  await live.cmd(`layershow ${id} b`);
}

/** Opens a screen in output mode and waits for its snapshot. */
async function openOutput(page: Page, base: string, id: string, query: string) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${id}?${query}`);
  await (await ws).waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
  return errors;
}

/** Every template root on the page (one per layer and half). */
async function settled(page: Page, n: number) {
  await expect(page.locator('.lt[data-state="in"]')).toHaveCount(n);
  await page.evaluate(() =>
    Promise.all(
      [...document.querySelectorAll("div")]
        .flatMap((d) => (d.shadowRoot ? [...d.shadowRoot.querySelectorAll(".lt")] : []))
        .flatMap((r) => r.getAnimations({ subtree: true }).map((a) => a.finished)),
    ),
  );
}

/** RGB at page points, read from one real screenshot (decoded in the page). */
async function pixels(page: Page, pts: ReadonlyArray<readonly [number, number]>): Promise<number[][]> {
  const png = await page.screenshot();
  return page.evaluate(
    async ([b64, pts]) => {
      const el = new Image();
      el.src = `data:image/png;base64,${b64}`;
      await el.decode(); // an <img>: the page's CSP allows data: images (not fetch)
      const img = await createImageBitmap(el);
      const c = new OffscreenCanvas(img.width, img.height);
      const ctx = c.getContext("2d")!;
      ctx.drawImage(img, 0, 0);
      return pts.map(([x, y]) => [...ctx.getImageData(Math.round(x), Math.round(y), 1, 1).data.slice(0, 3)]);
    },
    [png.toString("base64"), pts] as const,
  );
}

const near = (got: number[], want: number[], tol = 3) => {
  expect(
    got.map((v, i) => Math.abs(v - want[i]) <= tol),
    `${got} ≈ ${want}`,
  ).toEqual([true, true, true]);
};

test.describe("output mode", () => {
  test.beforeEach(async ({ live }) => {
    red ||= await live.cmd(`addtemplatedoc ${JSON.stringify(plate("Red plate", "#ff0000"))}`);
    white ||= await live.cmd(`addtemplatedoc ${JSON.stringify(plate("White plate", "#ffffff"))}`);
    await live.cmd("liveedit immediate");
  });

  test.afterEach(async ({ live }) => {
    await live.cmd("liveedit onTake");
  });

  for (const [label, bg, rgb] of [
    ["black", "000000", [0, 0, 0]],
    ["green", "00b140", [0, 177, 64]],
    ["a custom color", "3a1f6b", [58, 31, 107]],
  ] as const) {
    test(`fill on ${label}: the screen over the background, cursor hidden`, async ({ page, live }) => {
      const id = await makeScreen(live);
      const errors = await openOutput(page, live.base, id, `output=o1&bg=${bg}&mode=fill`);
      await showAll(live, id);
      await settled(page, 2);
      const [back, a, b] = await pixels(page, [[1800, 1000], center(A), center(B)]);
      near(back, [...rgb]);
      near(a, [255, 0, 0]);
      near(
        b,
        rgb.map((c) => (255 + c) / 2),
      );
      expect(
        await page.evaluate(() => [
          getComputedStyle(document.body).cursor,
          getComputedStyle(document.documentElement).cursor,
        ]),
      ).toEqual(["none", "none"]);
      expect(errors).toEqual([]);
    });
  }

  test("key: every layer as its alpha matte, white on black", async ({ page, live }) => {
    const id = await makeScreen(live);
    const errors = await openOutput(page, live.base, id, "output=o2&bg=00b140&mode=key");
    await showAll(live, id);
    await settled(page, 2);
    const [back, a, b] = await pixels(page, [[1800, 1000], center(A), center(B)]);
    near(back, [0, 0, 0]);
    near(a, [255, 255, 255]);
    near(b, [128, 128, 128]);
    await expect(page).toHaveScreenshot("output-key.png");
    expect(errors).toEqual([]);
  });

  test("a browser source without ?output is unchanged and never loads the output chunk", async ({ page, live }) => {
    const id = await makeScreen(live);
    const chunks: string[] = [];
    page.on("request", (r) => chunks.push(r.url()));
    const ws = page.waitForEvent("websocket");
    await page.goto(`${live.base}/screen/${id}`);
    await (await ws).waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
    expect(chunks.filter((u) => /\/output-[^/]*\.js/.test(u))).toEqual([]);
    expect(
      await page.evaluate(() => [
        getComputedStyle(document.body).backgroundColor,
        getComputedStyle(document.body).cursor,
      ]),
    ).toEqual(["rgba(0, 0, 0, 0)", "auto"]);
  });

  test("letterbox: a 16:9 screen in a square window is centered on the background", async ({ page, live }) => {
    await page.setViewportSize({ width: 1000, height: 1000 });
    const id = await makeScreen(live);
    await openOutput(page, live.base, id, "output=o1&bg=00b140&mode=fill");
    await showAll(live, id);
    await settled(page, 2);
    const k = 1000 / 1920;
    const top = (1000 - 1080 * k) / 2;
    const [bar, bottom, a] = await pixels(page, [
      [500, top / 2],
      [500, 1000 - top / 2],
      [center(A)[0] * k, top + center(A)[1] * k],
    ]);
    near(bar, [0, 177, 64]);
    near(bottom, [0, 177, 64]);
    near(a, [255, 0, 0]);
    // Resizing refits (resize only; nothing runs at rest).
    await page.setViewportSize({ width: 1920, height: 1080 });
    await expect.poll(async () => (await pixels(page, [center(A)]))[0]).toEqual([255, 0, 0]);
  });

  for (const dual of ["h", "v", "hr", "vr"] as const) {
    const horizontal = dual[0] === "h";
    const size = horizontal ? { width: 3840, height: 1080 } : { width: 1920, height: 2160 };
    /** A point of the screen in the second half of the window. */
    const second = (p: readonly [number, number]) =>
      horizontal ? ([p[0] + 1920, p[1]] as const) : ([p[0], p[1] + 1080] as const);
    const first = (p: readonly [number, number]) => p;
    // r: the key's half comes first (its display is left of / above the fill's).
    const [fillAt, keyAt] = dual.endsWith("r") ? [second, first] : [first, second];
    const halves = dual.endsWith("r") ? "key in the first half, fill in the second" : "fill in the first half, key in the second";

    test(`dual ${dual}: ${halves}, from one connection`, async ({ page, live }) => {
      await page.setViewportSize(size);
      const id = await makeScreen(live);
      const sockets: string[] = [];
      page.on("websocket", (w) => sockets.push(w.url()));
      const errors = await openOutput(page, live.base, id, `output=o1&bg=000000&mode=fill&dual=${dual}`);
      await showAll(live, id);
      await settled(page, 4);
      const [fa, fb, fback, ka, kb, kback] = await pixels(page, [
        fillAt(center(A)),
        fillAt(center(B)),
        fillAt([1800, 1000]),
        keyAt(center(A)),
        keyAt(center(B)),
        keyAt([1800, 1000]),
      ]);
      near(fa, [255, 0, 0]);
      near(fb, [128, 128, 128]);
      near(fback, [0, 0, 0]);
      near(ka, [255, 255, 255]);
      near(kb, [128, 128, 128]);
      near(kback, [0, 0, 0]);
      expect(sockets).toHaveLength(1);
      await expect(page).toHaveScreenshot(`output-dual-${dual}.png`);

      // A show animates both halves in the same frame: sample both roots' opacity in each rAF.
      await live.cmd(`layerhide ${id} a`);
      await expect(page.locator('.lt[data-state="in"]')).toHaveCount(2);
      await page.waitForTimeout(400);
      await page.evaluate(() => {
        const w = window as unknown as { __pairs: number[][] };
        w.__pairs = [];
        const t0 = performance.now();
        const roots = () =>
          [...document.querySelectorAll("div")].flatMap((d) =>
            d.shadowRoot ? [...d.shadowRoot.querySelectorAll(".lt")] : [],
          );
        const frame = () => {
          // The two red-plate roots (layer "a"): one per half.
          const reds = roots().filter(
            (r) => getComputedStyle(r.querySelector(".plate")!).backgroundColor === "rgb(255, 0, 0)",
          );
          if (reds.length === 2) w.__pairs.push(reds.map((r) => +getComputedStyle(r).opacity));
          if (performance.now() - t0 < 1500) requestAnimationFrame(frame);
        };
        requestAnimationFrame(frame);
      });
      await live.cmd(`layershow ${id} a`);
      await page.waitForTimeout(1600);
      const pairs = await page.evaluate(() => (window as unknown as { __pairs: number[][] }).__pairs);
      const mid = pairs.filter(([x]) => x > 0 && x < 1);
      expect(mid.length).toBeGreaterThan(3);
      for (const [x, y] of pairs) expect(Math.abs(x - y)).toBeLessThan(1e-6);
      expect(errors).toEqual([]);
    });
  }
});

// ---- Test patterns and the idle page --------------------------------------------------------

/** The built page (static; the server serves the same file at /pattern). */
const PATTERN = pathToFileURL(join(process.cwd(), "renderer/dist/pattern.html")).href;

const NAME_Q = "name=Output%201&display=DELL%20U2720Q&res=2560x1440&hz=60";

for (const [shot, query, size] of [
  ["bars", "kind=bars&w=1920&h=1080&mode=fill", { width: 1920, height: 1080 }],
  ["grid", "kind=grid&w=1920&h=1080&mode=fill", { width: 1920, height: 1080 }],
  ["safe", "kind=safe&w=1920&h=1080&mode=fill", { width: 1920, height: 1080 }],
  ["name", `kind=name&w=1920&h=1080&mode=fill&${NAME_Q}`, { width: 1920, height: 1080 }],
  ["safe-key", "kind=safe&w=1920&h=1080&mode=key", { width: 1920, height: 1080 }],
  ["grid-letterbox", "kind=grid&w=1920&h=1080&mode=fill", { width: 1000, height: 1000 }],
  ["name-dual-h", `kind=name&w=1920&h=1080&mode=fill&dual=h&${NAME_Q}`, { width: 3840, height: 1080 }],
  ["bars-dual-v", "kind=bars&w=1920&h=1080&mode=fill&dual=v", { width: 1920, height: 2160 }],
  ["bars-dual-hr", "kind=bars&w=1920&h=1080&mode=fill&dual=hr", { width: 3840, height: 1080 }],
  ["bars-dual-vr", "kind=bars&w=1920&h=1080&mode=fill&dual=vr", { width: 1920, height: 2160 }],
] as const) {
  test(`test pattern: ${shot}`, async ({ page }) => {
    await page.setViewportSize(size);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto(`${PATTERN}?${query}`);
    await expect(page.locator("svg")).toHaveCount(query.includes("dual") ? 2 : 1);
    expect(await page.evaluate(() => getComputedStyle(document.body).cursor)).toBe("none");
    if (shot === "bars") {
      // 75 % white and blue at the ends of the top row.
      const [w, b] = await pixels(page, [
        [100, 300],
        [1820, 300],
      ]);
      near(w, [191, 191, 191]);
      near(b, [0, 0, 191]);
    }
    if (shot.startsWith("bars-dual")) {
      // The key half: bars are opaque, so the matte is white. Each half's top-left bar (75 %
      // white) and its blue bar (top right): the key's blue is white too.
      const halves: Record<string, [number, number][]> = {
        "bars-dual-v": [[0, 0], [0, 1080]],
        "bars-dual-hr": [[1920, 0], [0, 0]],
        "bars-dual-vr": [[0, 1080], [0, 0]],
      };
      const [[fx, fy], [kx, ky]] = halves[shot];
      const [fill, fillBlue, key, keyBlue] = await pixels(page, [
        [fx + 100, fy + 300],
        [fx + 1820, fy + 300],
        [kx + 100, ky + 300],
        [kx + 1820, ky + 300],
      ]);
      near(fill, [191, 191, 191]);
      near(fillBlue, [0, 0, 191]);
      near(key, [255, 255, 255]);
      near(keyBlue, [255, 255, 255]);
    }
    if (shot.startsWith("name")) await expect(page.getByText("Output 1").first()).toBeVisible();
    await expect(page).toHaveScreenshot(`output-pattern-${shot}.png`);
    expect(errors).toEqual([]);
  });
}

test("test pattern text is escaped", async ({ page }) => {
  await page.goto(`${PATTERN}?kind=name&name=${encodeURIComponent("<img src=x onerror=window.__pwned=1>")}`);
  await expect(page.locator("img")).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as { __pwned?: number }).__pwned)).toBeUndefined();
});

const idle = (q: string) => `${pathToFileURL(join(process.cwd(), "static/output-idle.html")).href}?${q}`;

test("idle page: background, key black, Identify text", async ({ page }) => {
  await page.goto(idle("bg=00b140"));
  near((await pixels(page, [[960, 540]]))[0], [0, 177, 64]);
  expect(await page.evaluate(() => getComputedStyle(document.body).cursor)).toBe("none");
  await page.goto(idle("bg=00b140&mode=key"));
  near((await pixels(page, [[960, 540]]))[0], [0, 0, 0]);
  await page.goto(idle("bg=00b140&dual=h"));
  const [l, r] = await pixels(page, [
    [480, 540],
    [1440, 540],
  ]);
  near(l, [0, 177, 64]);
  near(r, [0, 0, 0]);
  // Reversed: the black (key) half first.
  await page.goto(idle("bg=00b140&dual=hr"));
  const [rl, rr] = await pixels(page, [
    [480, 540],
    [1440, 540],
  ]);
  near(rl, [0, 0, 0]);
  near(rr, [0, 177, 64]);
  await page.goto(idle("bg=00b140&dual=vr"));
  const [top, bottom] = await pixels(page, [
    [960, 270],
    [960, 810],
  ]);
  near(top, [0, 0, 0]);
  near(bottom, [0, 177, 64]);
  await page.goto(idle(`name=${encodeURIComponent("DELL U2720Q")}&res=2560x1440&hz=60`));
  await expect(page.getByText("DELL U2720Q")).toBeVisible();
  await expect(page.getByText("2560×1440 · 60 Hz")).toBeVisible();
  await expect(page).toHaveScreenshot("output-idle-identify.png");
});

test("renderer sizes: main bundle ≤ 9 KB gz, pattern page ≤ 4 KB gz", () => {
  const dist = join(process.cwd(), "renderer/dist");
  const gz = (p: string) => gzipSync(readFileSync(p), { level: 9 }).length;
  const js = readdirSync(join(dist, "assets")).filter((f) => f.endsWith(".js"));
  const sizes = Object.fromEntries(js.map((f) => [f.split("-")[0], gz(join(dist, "assets", f))]));
  const pattern = gz(join(dist, "pattern.html"));
  console.log(`renderer gz: ${JSON.stringify(sizes)}, pattern.html ${pattern}`);
  expect(sizes.renderer).toBeLessThanOrEqual(9000);
  expect(sizes.output).toBeLessThanOrEqual(1024);
  expect(pattern).toBeLessThanOrEqual(4096);
});

test("the server serves the test pattern at /pattern", async ({ request, live }) => {
  const r = await request.get(`${live.base}/pattern?kind=grid&w=1920&h=1080`);
  expect(r.status()).toBe(200);
  expect(r.headers()["content-type"]).toContain("text/html");
  expect(await r.text()).toContain("<title>Kinetra test pattern</title>");
});
