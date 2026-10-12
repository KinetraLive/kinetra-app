// Content sizing against the real server: a template with a design box (640×180) rendered in
// `fit` (scaled uniformly into the layer's box, centered) and `reflow` (the content fills the box)
// at a small corner box and a full-width bar; a sizing switch on air under `immediate` updates in
// place without replaying the in-animation. Screenshots of the four cases go to review/sizing/
// (review artifacts, not baselines).

import type { Page } from "@playwright/test";
import { expect, test, type Fixture } from "./fixture";
import type { ContentSizing, Rect, ScreenDoc, ScreenLayer } from "../src/lib/types";

const DESIGN = { w: 640, h: 180 };

const TEMPLATE = {
  name: "Sizing bar",
  html: '<div class="lt"><div class="plate"><span class="name">{{name}}</span></div></div>',
  css: `.lt{position:absolute;inset:0;container-type:size;font-family:system-ui,sans-serif}
.plate{position:absolute;inset:0;box-sizing:border-box;background:#14161c;border-left:4cqw solid #ffb81c;display:flex;align-items:center;padding:0 6cqw}
.name{color:#fff;font-size:36cqh;font-weight:700;white-space:nowrap}
.lt[data-state="in"]{animation:in 300ms cubic-bezier(.2,0,0,1) both}
.lt[data-state="out"]{animation:out 200ms forwards}
@keyframes in{from{opacity:0}}
@keyframes out{to{opacity:0}}`,
  fields: [{ key: "name", label: "Name", type: "text", default: "" }],
  design: { ...DESIGN, x: 96, y: 840, sizing: "fit" },
};

const CORNER: Rect = { x: 1560, y: 40, w: 320, h: 160 };
const BAR: Rect = { x: 0, y: 860, w: 1920, h: 180 };

let templateId = "";
let seq = 0;

function layer(rect: Rect, sizing?: ContentSizing): ScreenLayer {
  return { id: "l1", name: "Bar", kind: "template", templateId, rect, fields: { name: "Ada Lovelace" }, vars: {}, sizing };
}

async function makeScreen(live: Fixture, l: ScreenLayer): Promise<ScreenDoc> {
  const doc: ScreenDoc = {
    id: `sz${Date.now().toString(36)}${seq++}`.slice(0, 20),
    name: "Sizing",
    width: 1920,
    height: 1080,
    background: "#3a3f4b",
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

/** The template root's box in page px (the viewport is 1920×1080, so the stage is at scale 1). */
function rootBox(page: Page) {
  return root(page).evaluate((e) => {
    const r = e.getBoundingClientRect();
    return { x: r.x, y: r.y, w: r.width, h: r.height };
  });
}

/** What `fit` should give: the design box scaled by min(W/w, H/h), centered in the rect. */
function fitted(r: Rect) {
  const s = Math.min(r.w / DESIGN.w, r.h / DESIGN.h);
  return { x: r.x + (r.w - DESIGN.w * s) / 2, y: r.y + (r.h - DESIGN.h * s) / 2, w: DESIGN.w * s, h: DESIGN.h * s };
}

function near(got: { x: number; y: number; w: number; h: number }, want: typeof got) {
  for (const k of ["x", "y", "w", "h"] as const) expect(got[k], k).toBeCloseTo(want[k], 0);
}

async function settled(page: Page) {
  await root(page).evaluate((e) => Promise.all(e.getAnimations({ subtree: true }).map((a) => a.finished)));
}

/** Outlines the layer's box (review screenshots only; the output never shows it). */
function outline(page: Page, r: Rect) {
  return page.evaluate((r) => {
    const d = document.createElement("div");
    d.style.cssText = `position:fixed;left:${r.x}px;top:${r.y}px;width:${r.w}px;height:${r.h}px;outline:2px dashed #7dd3fc;outline-offset:-1px;pointer-events:none;z-index:2147483647`;
    document.body.append(d);
  }, r);
}

test.beforeEach(async ({ live }) => {
  templateId ||= await live.cmd(`addtemplatedoc ${JSON.stringify(TEMPLATE)}`);
  await live.cmd("liveedit onTake");
});

for (const [name, rect] of [
  ["corner", CORNER],
  ["bar", BAR],
] as const) {
  for (const sizing of ["fit", "reflow"] as const) {
    test(`${sizing} at a ${name} box`, async ({ page, live }) => {
      const doc = await makeScreen(live, layer(rect, sizing));
      const errors = await openScreen(page, live.base, doc.id);
      await live.cmd(`layershow ${doc.id} l1`);
      await expect(root(page)).toHaveAttribute("data-state", "in");
      await settled(page);
      const box = await rootBox(page);
      if (sizing === "fit") {
        near(box, fitted(rect));
        // The template's CSS sees the design size: container units resolve against 640×180.
        const fs = await page.locator(".name").evaluate((e) => parseFloat(getComputedStyle(e).fontSize));
        expect(fs).toBeCloseTo(0.36 * DESIGN.h, 0);
      } else {
        near(box, rect);
      }
      const view = JSON.parse(await live.cmd(`layerview ${doc.id} l1`));
      expect(view.sizing).toBe(sizing);
      expect(view.design).toEqual(DESIGN);
      await outline(page, rect);
      await page.screenshot({ path: `review/sizing/${sizing}-${name}.png` });
      expect(errors).toEqual([]);
    });
  }
}

test("a sizing switch on air under immediate updates in place without a replay", async ({ page, live }) => {
  await live.cmd("liveedit immediate");
  const doc = await makeScreen(live, layer(CORNER));
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(root(page)).toHaveAttribute("data-state", "in");
  await settled(page);
  near(await rootBox(page), CORNER);
  const before = await root(page).elementHandle();
  await page.evaluate(() => {
    const w = window as unknown as { __starts: number };
    w.__starts = 0;
    for (const d of document.querySelectorAll("div"))
      d.shadowRoot?.addEventListener("animationstart", () => w.__starts++, true);
  });

  await live.cmd(`savescreen ${JSON.stringify({ ...doc, layers: [layer(CORNER, "fit")] })}`);
  await expect.poll(async () => (await rootBox(page)).h).toBeCloseTo(fitted(CORNER).h, 0);
  near(await rootBox(page), fitted(CORNER));
  // And back to reflow, then a resize while fitted re-fits.
  await live.cmd(`savescreen ${JSON.stringify({ ...doc, layers: [layer(CORNER, "reflow")] })}`);
  await expect.poll(async () => (await rootBox(page)).h).toBeCloseTo(CORNER.h, 0);
  await live.cmd(`savescreen ${JSON.stringify({ ...doc, layers: [layer(CORNER, "fit")] })}`);
  await live.cmd(`rect ${doc.id} l1 ${JSON.stringify(BAR)}`);
  await expect.poll(async () => (await rootBox(page)).w).toBeCloseTo(fitted(BAR).w, 0);
  near(await rootBox(page), fitted(BAR));

  await page.waitForTimeout(300);
  expect(await page.evaluate(() => (window as unknown as { __starts: number }).__starts)).toBe(0);
  await expect(root(page)).toHaveAttribute("data-state", "in");
  expect(await root(page).evaluate((el, prev) => el === prev, before)).toBe(true);
});

test("under onTake a sizing change waits for the next take", async ({ page, live }) => {
  const doc = await makeScreen(live, layer(BAR));
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(root(page)).toHaveAttribute("data-state", "in");
  await settled(page);
  await live.cmd(`savescreen ${JSON.stringify({ ...doc, layers: [layer(BAR, "fit")] })}`);
  expect(JSON.parse(await live.cmd(`layerview ${doc.id} l1`)).pending).toBe(true);
  await page.waitForTimeout(150);
  near(await rootBox(page), BAR);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect.poll(async () => (await rootBox(page)).w).toBeCloseTo(fitted(BAR).w, 0);
  near(await rootBox(page), fitted(BAR));
});

/** The painted parts in page px: the plate and the name's glyphs. */
function painted(page: Page) {
  return page.evaluate(() => {
    const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot)!;
    const sr = host.shadowRoot!;
    const box = (r: DOMRect) => ({ x: r.x, y: r.y, w: r.width, h: r.height });
    const range = document.createRange();
    range.selectNodeContents(sr.querySelector(".name")!);
    return { plate: box(sr.querySelector(".plate")!.getBoundingClientRect()), name: box(range.getBoundingClientRect()) };
  });
}

// The layout editor saves a resize as `sizing: "fit"` with a box of the design's aspect: the
// graphic itself gets bigger or smaller with the box (half the box, half the graphic), anchored at
// the box like the editor's handles.
test("a resized layer (fit, the design's aspect) scales its graphic with the box", async ({ page, live }) => {
  const at: Rect = { x: 96, y: 840, w: DESIGN.w, h: DESIGN.h };
  const doc = await makeScreen(live, layer(at));
  const errors = await openScreen(page, live.base, doc.id);
  await live.cmd("liveedit immediate");
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(root(page)).toHaveAttribute("data-state", "in");
  await settled(page);
  const base = await painted(page);
  near(base.plate, at);

  for (const k of [0.5, 1.5]) {
    const r: Rect = { x: at.x, y: at.y - (k > 1 ? 120 : 0), w: Math.round(DESIGN.w * k), h: Math.round(DESIGN.h * k) };
    await live.cmd(`savescreen ${JSON.stringify({ ...doc, layers: [layer(r, "fit")] })}`);
    await expect.poll(async () => (await painted(page)).plate.w).toBeCloseTo(r.w, 0);
    const got = await painted(page);
    // The plate is the box; the type scales by the same factor and keeps its place in it.
    near(got.plate, r);
    expect(got.name.w / base.name.w).toBeCloseTo(k, 1);
    expect(got.name.h / base.name.h).toBeCloseTo(k, 1);
    expect((got.name.x - r.x) / (base.name.x - at.x)).toBeCloseTo(k, 1);
    await outline(page, r);
    await page.screenshot({ path: `review/sizing/resized-${k}.png` });
  }
  expect(errors).toEqual([]);
});

// A user's own template without a design: fit takes the screen as its design, so a full-screen
// graphic resized to half the screen draws at half size.
test("a template without a design scales from the full screen", async ({ page, live }) => {
  const own = await live.cmd(`addtemplatedoc ${JSON.stringify({ ...TEMPLATE, name: "Own graphic", design: undefined })}`);
  const full: Rect = { x: 0, y: 0, w: 1920, h: 1080 };
  const l = (rect: Rect, sizing?: ContentSizing) => ({ ...layer(rect, sizing), templateId: own });
  const doc = await makeScreen(live, l(full));
  await openScreen(page, live.base, doc.id);
  await live.cmd("liveedit immediate");
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(root(page)).toHaveAttribute("data-state", "in");
  await settled(page);
  const base = await painted(page);
  const half: Rect = { x: 480, y: 270, w: 960, h: 540 };
  await live.cmd(`savescreen ${JSON.stringify({ ...doc, layers: [l(half, "fit")] })}`);
  await expect.poll(async () => (await painted(page)).plate.w).toBeCloseTo(960, 0);
  const got = await painted(page);
  near(got.plate, half);
  expect(got.name.w / base.name.w).toBeCloseTo(0.5, 1);
});
