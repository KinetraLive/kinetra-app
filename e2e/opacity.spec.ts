// Layer opacity against the real server: a white plate on a black screen renders at half
// brightness with opacity 50 (fit and reflow), and an opacity change on air under `immediate`
// (through `run_control`, the path MIDI faders and OSC use) updates in place: no replayed
// in-animation, the same template root, still `in`.

import type { Page } from "@playwright/test";
import { expect, test, type Fixture } from "./fixture";
import type { ContentSizing, Rect, ScreenDoc, ScreenLayer } from "../src/lib/types";

const TEMPLATE = {
  name: "Opacity plate",
  html: '<div class="lt"><div class="plate"></div></div>',
  css: `.lt{position:absolute;inset:0}
.plate{position:absolute;inset:0;background:#ffffff}
.lt[data-state="in"]{animation:in 300ms cubic-bezier(.2,0,0,1) both}
.lt[data-state="out"]{animation:out 200ms forwards}
@keyframes in{from{opacity:0}}
@keyframes out{to{opacity:0}}`,
  fields: [],
  design: { w: 400, h: 200, sizing: "fit" },
};

const RECT: Rect = { x: 100, y: 100, w: 400, h: 200 };

let templateId = "";
let seq = 0;

function layer(sizing: ContentSizing, opacity?: number): ScreenLayer {
  return { id: "l1", name: "Plate", kind: "template", templateId, rect: RECT, fields: {}, vars: {}, sizing, opacity };
}

async function makeScreen(live: Fixture, l: ScreenLayer): Promise<ScreenDoc> {
  const doc: ScreenDoc = {
    id: `op${Date.now().toString(36)}${seq++}`.slice(0, 20),
    name: "Opacity",
    width: 1920,
    height: 1080,
    background: "#000000",
    layers: [l],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc;
}

async function openScreen(page: Page, base: string, id: string) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${id}`);
  await (await ws).waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
  return errors;
}

const root = (page: Page) => page.locator(".lt");

async function settled(page: Page) {
  await root(page).evaluate((e) => Promise.all(e.getAnimations({ subtree: true }).map((a) => a.finished)));
}

/** The red channel at the plate's center, read from a real screenshot (decoded in the page). */
async function brightness(page: Page): Promise<number> {
  const png = await page.screenshot({ clip: clip({ x: RECT.x + RECT.w / 2 - 2, y: RECT.y + RECT.h / 2 - 2, w: 4, h: 4 }) });
  return page.evaluate(async (b64) => {
    const el = new Image();
      el.src = `data:image/png;base64,${b64}`;
      await el.decode(); // an <img>: the page's CSP allows data: images (not fetch)
      const img = await createImageBitmap(el);
    const c = new OffscreenCanvas(img.width, img.height);
    const ctx = c.getContext("2d")!;
    ctx.drawImage(img, 0, 0);
    return ctx.getImageData(1, 1, 1, 1).data[0];
  }, png.toString("base64"));
}

function clip(r: Rect) {
  return { x: r.x, y: r.y, width: r.w, height: r.h };
}

test.beforeEach(async ({ live }) => {
  templateId ||= await live.cmd(`addtemplatedoc ${JSON.stringify(TEMPLATE)}`);
  await live.cmd("liveedit onTake");
});

for (const sizing of ["fit", "reflow"] as const) {
  test(`opacity 50 renders at half alpha (${sizing})`, async ({ page, live }) => {
    const doc = await makeScreen(live, layer(sizing, 50));
    const errors = await openScreen(page, live.base, doc.id);
    await live.cmd(`layershow ${doc.id} l1`);
    await expect(root(page)).toHaveAttribute("data-state", "in");
    await settled(page);
    expect(Math.abs((await brightness(page)) - 128)).toBeLessThanOrEqual(3);
    // On the inner box, never on the host (crossfades and studio takes animate the host).
    const where = await page.evaluate(() => {
      const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot)!;
      const inner = host.shadowRoot!.querySelector("style + div") as HTMLElement;
      return { host: getComputedStyle(host).opacity, inner: getComputedStyle(inner).opacity };
    });
    expect(where).toEqual({ host: "1", inner: "0.5" });
    expect(JSON.parse(await live.cmd(`layerview ${doc.id} l1`)).opacity).toBe(50);
    await page.screenshot({ path: `review/opacity/${sizing}-50.png`, clip: clip({ x: 0, y: 0, w: 600, h: 400 }) });
    expect(errors).toEqual([]);
  });
}

test("an opacity change on air under immediate updates in place without a replay", async ({ page, live }) => {
  await live.cmd("liveedit immediate");
  const doc = await makeScreen(live, layer("reflow"));
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(root(page)).toHaveAttribute("data-state", "in");
  await settled(page);
  expect(await brightness(page)).toBeGreaterThanOrEqual(252);
  const before = await root(page).elementHandle();
  await page.evaluate(() => {
    const w = window as unknown as { __starts: number };
    w.__starts = 0;
    for (const d of document.querySelectorAll("div"))
      d.shadowRoot?.addEventListener("animationstart", () => w.__starts++, true);
  });
  const target = JSON.stringify({ kind: "value", screenId: doc.id, layerId: "l1", target: { kind: "opacity" } });

  await live.cmd(`runcontrol ${target} level:0.5`);
  await expect.poll(() => brightness(page)).toBeLessThan(140);
  expect(Math.abs((await brightness(page)) - 128)).toBeLessThanOrEqual(3);
  // Paced (≤ 30/s): the next value lands within a frame or two.
  await page.waitForTimeout(50);
  await live.cmd(`runcontrol ${target} level:0.8`);
  await expect.poll(() => brightness(page)).toBeGreaterThan(190);
  expect(Math.abs((await brightness(page)) - 204)).toBeLessThanOrEqual(3);
  await page.waitForTimeout(50);
  await live.cmd(`runcontrol ${target} press`);
  await expect.poll(() => brightness(page)).toBeGreaterThanOrEqual(252);

  await page.waitForTimeout(300);
  expect(await page.evaluate(() => (window as unknown as { __starts: number }).__starts)).toBe(0);
  await expect(root(page)).toHaveAttribute("data-state", "in");
  expect(await root(page).evaluate((el, prev) => el === prev, before)).toBe(true);
  // Saved to the screen (100 % = absent).
  expect(JSON.parse(await live.cmd(`layerview ${doc.id} l1`)).opacity).toBe(100);
});
