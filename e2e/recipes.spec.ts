// Recipes against the real server: the schedule engine (a boundary flips NOW/UP NEXT on air),
// operator actions (Delay, Start now, Undo), a recipe's flow firing, an image slideshow served
// from /assets, and installing a recipe. The fixture's `clock <skew>` moves schedule time so a
// boundary is seconds away.

import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deflateSync } from "node:zlib";
import type { Page } from "@playwright/test";
import { expect, test, type Fixture } from "./fixture";
import type { FlowInfo, RecipeInstance, ScheduleView, ScreenDoc, ScreenLayer } from "../src/lib/types";

const FULL = { x: 0, y: 0, w: 1920, h: 1080 };
const roots = (page: Page) => page.locator("[data-state]");

/** Local date-time `YYYY-MM-DDTHH:MM` of an epoch ms (the fixture uses this machine's zone). */
function local(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

const presets: Record<string, string> = {};
async function preset(live: Fixture, id: string) {
  return (presets[id] ||= await live.cmd(`addtemplate ${id}`));
}

let seq = 0;
async function screenWith(live: Fixture, layers: ScreenLayer[]): Promise<ScreenDoc> {
  const doc: ScreenDoc = {
    id: `m7${Date.now().toString(36)}${seq++}`,
    name: "M7",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers,
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc;
}

async function openScreen(page: Page, base: string, id: string) {
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${id}`);
  await (await ws).waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
}

/** A Now & Next layer bound to a schedule's NOW and UP NEXT items. */
function nowNext(templateId: string, sourceId: string): ScreenLayer {
  const b = (column: string, row: "now" | "next") => ({ sourceId, column, row });
  return {
    id: "nn",
    name: "Now & Next",
    kind: "template",
    templateId,
    rect: FULL,
    fields: {},
    vars: {},
    bindings: {
      now_title: b("title", "now"),
      next_title: b("title", "next"),
      next_label: b("label", "next"),
      next_status: b("status", "next"),
    },
  };
}

test.afterEach(async ({ live }) => {
  await live.cmd("clock 0");
});

test("a schedule boundary moves NOW and UP NEXT on air", async ({ page, live }) => {
  // Put schedule time 2 s before a whole minute: item A ends there and B starts.
  const real = Date.now();
  const boundary = Math.ceil((real + 65_000) / 60_000) * 60_000;
  await live.cmd(`clock ${boundary - 2_000 - real}`);
  const src = await live.cmd(
    `addschedule ${JSON.stringify({
      name: "Day",
      items: [
        { id: "a", title: "Opening talk", start: local(boundary - 30 * 60_000), end: local(boundary) },
        { id: "b", title: "Keynote", start: local(boundary), end: local(boundary + 45 * 60_000) },
        { id: "c", title: "Lunch", start: local(boundary + 45 * 60_000), end: local(boundary + 90 * 60_000) },
      ],
    })}`,
  );
  const doc = await screenWith(live, [nowNext(await preset(live, "now-next"), src)]);
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} nn`);
  await expect(page.locator(".nn__block--now .nn__title")).toHaveText("Opening talk");
  await expect(page.locator(".nn__block--next .nn__title")).toHaveText("Keynote");
  // The engine wakes at the boundary (no polling) and re-applies the bound fields on air.
  await expect(page.locator(".nn__block--now .nn__title")).toHaveText("Keynote", { timeout: 6_000 });
  await expect(page.locator(".nn__block--next .nn__title")).toHaveText("Lunch");
  await expect(roots(page)).toHaveAttribute("data-state", "in");
});

test("operator actions: delay, start now, undo", async ({ page, live }) => {
  const now = Date.now();
  const m = 60_000;
  const src = await live.cmd(
    `addschedule ${JSON.stringify({
      name: "Stage",
      items: [
        { id: "a", title: "Band A", start: local(now - 20 * m), end: local(now + 20 * m) },
        { id: "b", title: "Band B", start: local(now + 30 * m), end: local(now + 60 * m) },
        { id: "c", title: "Band C", start: local(now + 70 * m), end: local(now + 100 * m) },
      ],
    })}`,
  );
  const doc = await screenWith(live, [nowNext(await preset(live, "now-next"), src)]);
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} nn`);
  await expect(page.locator(".nn__block--next .nn__title")).toHaveText("Band B");

  // Delay the next act: it shows DELAYED (label and status) at once.
  expect(await live.cmd(`scheduleaction ${src} {"do":"delay","itemId":"b","minutes":10}`)).toBe("a b");
  await expect(page.locator(".nn__block--next")).toHaveAttribute("data-status", "delayed");
  await expect(page.locator(".nn__block--next .nn__tag")).toHaveText("DELAYED");

  // Start it now: Band B is NOW, Band A becomes PREVIOUS, Band C is up next.
  expect(await live.cmd(`scheduleaction ${src} {"do":"startNow","itemId":"b"}`)).toBe("b c");
  await expect(page.locator(".nn__block--now .nn__title")).toHaveText("Band B");
  await expect(page.locator(".nn__block--next .nn__title")).toHaveText("Band C");
  const view = JSON.parse(await live.cmd(`scheduleview ${src}`)) as ScheduleView;
  expect(view.items.find((i) => i.id === "a")?.status).toBe("previous");

  // Undo brings Band A back as NOW.
  expect(await live.cmd(`scheduleaction ${src} {"do":"undo"}`)).toBe("a b");
  await expect(page.locator(".nn__block--now .nn__title")).toHaveText("Band A");
});

test("installing the Event schedule recipe creates its source, templates, layers and flows", async ({ page, live }) => {
  const id = await live.cmd(
    `installrecipe ${JSON.stringify({ recipeId: "event-schedule", name: "Day 1", screen: { kind: "new", name: "Event", width: 1920, height: 1080 } })}`,
  );
  const inst = JSON.parse(await live.cmd(`recipeinstance ${id}`)) as RecipeInstance;
  expect(inst.panel).toBe("schedule");
  expect(inst.sourceId).toBeTruthy();
  expect(inst.layerIds).toHaveLength(3);
  expect(inst.templateIds).toHaveLength(3);
  expect(inst.flowIds.length).toBeGreaterThanOrEqual(2);
  const flows = (JSON.parse(await live.cmd("flows")) as FlowInfo[]).filter((f) => inst.flowIds.includes(f.id));
  expect(flows).toHaveLength(inst.flowIds.length);
  // Installed deployed and disarmed, owned by the recipe.
  expect(flows.every((f) => !f.armed && f.origin === id && f.warnings.length === 0)).toBe(true);
  const view = JSON.parse(await live.cmd(`scheduleview ${inst.sourceId}`)) as ScheduleView;
  expect(view.nowId).toBeTruthy(); // sample items around now
  expect(view.nextId).toBeTruthy();

  // Its flow "when an item starts, take Now & Next": take for 1.5 s, arm it, start the next item.
  const flow = flows.find((f) => f.nodes.some((n) => n.type === "trigger.itemStatus"))!;
  expect(flow).toBeTruthy();
  const nodes = flow.nodes.map((n) => (n.type === "action.take" ? { ...n, config: { ...n.config, forMs: 1500 } } : n));
  await live.cmd(`updateflow ${JSON.stringify({ id: flow.id, name: flow.name, retrigger: flow.retrigger, nodes, edges: flow.edges })}`);
  await live.cmd(`armflow ${flow.id} true`);
  await openScreen(page, live.base, inst.screenId);
  await expect(roots(page)).toHaveCount(0);
  await live.cmd(`scheduleaction ${inst.sourceId} ${JSON.stringify({ do: "startNow", itemId: view.nextId })}`);
  await expect(page.locator(".nn")).toHaveAttribute("data-state", "in", { timeout: 4_000 });
  // forMs: cleared again by the flow.
  await expect(page.locator(".nn")).toHaveCount(0, { timeout: 6_000 });
  const after = (JSON.parse(await live.cmd("flows")) as FlowInfo[]).find((f) => f.id === flow.id)!;
  expect(after.lastFiredAt).toBeTruthy();

  // A rule added from the recipe's panel belongs to it.
  const own = await live.cmd(
    `saveflow ${JSON.stringify({
      name: "Clear at the end",
      armed: false,
      retrigger: "restart",
      origin: id,
      nodes: [
        { id: "t", type: "trigger.itemStatus", config: { sourceId: inst.sourceId, status: "previous" }, position: { x: 0, y: 0 } },
        { id: "c", type: "action.clearScreen", config: { screenId: inst.screenId }, position: { x: 240, y: 0 } },
      ],
      edges: [{ id: "e", from: { node: "t", port: "fired" }, to: { node: "c", port: "in" }, kind: "event" }],
    })}`,
  );
  const withOwn = JSON.parse(await live.cmd(`recipeinstance ${id}`)) as RecipeInstance;
  expect(withOwn.flowIds).toContain(own);
});

/** A solid-colour 8×8 PNG (valid file, distinct per colour). */
function png(dir: string, name: string, rgb: [number, number, number]): string {
  const w = 8;
  const raw = Buffer.alloc((w * 3 + 1) * w);
  for (let y = 0; y < w; y++) for (let x = 0; x < w; x++) raw.set(rgb, y * (w * 3 + 1) + 1 + x * 3);
  const crcTable = Array.from({ length: 256 }, (_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc = (b: Buffer) => {
    let c = 0xffffffff;
    for (const x of b) c = crcTable[(c ^ x) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
  const chunk = (type: string, data: Buffer) => {
    const t = Buffer.from(type, "ascii");
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const c = Buffer.alloc(4);
    c.writeUInt32BE(crc(Buffer.concat([t, data])));
    return Buffer.concat([len, t, data, c]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(w, 4);
  ihdr.set([8, 2, 0, 0, 0], 8);
  const file = Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
  const path = join(dir, name);
  writeFileSync(path, file);
  return path;
}

test("an image slideshow crossfades through slides served from /assets", async ({ page, live }) => {
  const dir = mkdtempSync(join(tmpdir(), "kinetra-e2e-"));
  const urls: string[] = [];
  for (const [n, rgb] of [["red.png", [220, 30, 30]], ["blue.png", [30, 60, 220]]] as const) {
    urls.push(await live.cmd(`importasset ${png(dir, n, [...rgb])}`));
  }
  expect(urls[0]).toMatch(/^\/assets\/.+\.png$/);
  const res = await page.request.get(`${live.base}${urls[0]}`);
  expect(res.status()).toBe(200);
  expect(res.headers()["content-type"]).toBe("image/png");
  const etag = res.headers()["etag"];
  expect((await page.request.get(`${live.base}${urls[0]}`, { headers: { "If-None-Match": etag } })).status()).toBe(304);

  const src = await live.cmd(
    `addsource ${JSON.stringify({
      name: "Sponsors",
      columns: [
        { key: "logo", label: "Logo", kind: "image" },
        { key: "name", label: "Name" },
        { key: "tagline", label: "Tagline" },
      ],
      rows: [
        { logo: urls[0], name: "Red Co", tagline: "" },
        { logo: urls[1], name: "Blue Ltd", tagline: "" },
      ],
    })}`,
  );
  const doc = await screenWith(live, [
    {
      id: "ss",
      name: "Slideshow",
      kind: "template",
      templateId: await preset(live, "sponsor-slideshow"),
      rect: FULL,
      fields: { label: "Presented by" },
      vars: {},
      bindings: { slides: { sourceId: src, column: "" } },
    },
  ]);
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} ss`);
  const slides = page.locator(".ss__slide");
  await expect(slides).toHaveCount(2);
  await expect(slides.nth(0)).toHaveAttribute("data-current", "true");
  // Both images load (decoded once, kept in the DOM for the crossfade).
  await expect.poll(() => slides.locator("img").evaluateAll((imgs) => imgs.map((i) => (i as HTMLImageElement).naturalWidth))).toEqual([8, 8]);

  await live.cmd(`autoadvance ${src} {"everyMs":1000,"loop":true,"transition":"update"}`);
  await expect(slides.nth(1)).toHaveAttribute("data-current", "true", { timeout: 3_000 });
  await expect(slides.nth(0)).not.toHaveAttribute("data-current", "true");
  await expect(slides.nth(1)).toHaveAttribute("data-duration", "1");
  await live.cmd(`autoadvance ${src} null`);
});


/** Installs a recipe on a new 1920×1080 screen, opens it and takes its first layer. */
async function installAndShow(page: Page, live: Fixture, recipeId: string) {
  const id = await live.cmd(
    `installrecipe ${JSON.stringify({ recipeId, name: "Sponsors", screen: { kind: "new", name: "Sponsors", width: 1920, height: 1080 } })}`,
  );
  const inst = JSON.parse(await live.cmd(`recipeinstance ${id}`)) as RecipeInstance;
  const src = JSON.parse(await live.cmd(`getsource ${inst.sourceId}`)) as { kind: string; rows: Record<string, string>[] };
  const logos = src.rows.map((r) => r.logo);
  expect(logos).toHaveLength(4);
  for (const url of logos) {
    expect(url).toMatch(/^\/assets\/[a-z-]+(-\d+)?\.svg$/);
    const res = await page.request.get(`${live.base}${url}`);
    expect(res.status()).toBe(200);
    expect(res.headers()["content-type"]).toBe("image/svg+xml");
    expect(res.headers()["content-security-policy"]).toBe("script-src 'none'");
  }
  await openScreen(page, live.base, inst.screenId);
  await live.cmd(`layershow ${inst.screenId} ${inst.layerIds[0]}`);
  return { inst, logos };
}

/** naturalWidth of every <img> matching `sel` (0 = not loaded). */
const widths = (page: Page, sel: string) =>
  page.locator(sel).evaluateAll((imgs) => imgs.map((i) => (i as HTMLImageElement).naturalWidth));

test("the Sponsor wall recipe shows its bundled sample logos", async ({ page, live }) => {
  const { logos } = await installAndShow(page, live, "sponsor-wall");
  await expect(page.locator(".sw")).toHaveAttribute("data-state", "in");
  const tiles = page.locator(".sw__tile");
  await expect(tiles).toHaveCount(4);
  await expect.poll(() => widths(page, ".sw__logo")).toEqual([200, 200, 200, 200]);
  expect(await page.locator(".sw__logo").evaluateAll((imgs) => imgs.map((i) => i.getAttribute("src")))).toEqual(logos);
  await expect(page.locator(".sw__logo[data-empty]")).toHaveCount(0);
  for (let i = 0; i < 4; i++) await expect(tiles.nth(i)).toBeVisible();
});

test("the Sponsor slideshow recipe sits at its design box and shows its sample logos", async ({ page, live }) => {
  const { inst, logos } = await installAndShow(page, live, "sponsor-slideshow");
  const doc = JSON.parse(readFileSync(join(await live.cmd("dir"), "screens", `${inst.screenId}.json`), "utf8")) as ScreenDoc;
  const layer = doc.layers[0] as ScreenLayer & { sizing?: string };
  expect(layer.rect).toEqual({ x: 1285, y: 594, w: 520, h: 400 });
  expect(layer.sizing).toBe("fit");
  await expect(page.locator(".ss")).toHaveAttribute("data-state", "in");
  const slides = page.locator(".ss__slide");
  await expect(slides).toHaveCount(4);
  await expect.poll(() => widths(page, ".ss__logo")).toEqual([200, 200, 200, 200]);
  expect(await page.locator(".ss__logo").evaluateAll((imgs) => imgs.map((i) => i.getAttribute("src")))).toEqual(logos);
  await expect(page.locator(".ss__logo[data-empty]")).toHaveCount(0);
  // No slide is taken out (display: none); the current one is on screen with its logo.
  expect(await slides.evaluateAll((els) => els.map((el) => getComputedStyle(el).display))).not.toContain("none");
  await expect(slides.nth(0)).toHaveAttribute("data-current", "true");
  await expect(slides.nth(0).locator(".ss__logo")).toBeVisible();
  // Once the in-animation has settled, the card lies inside the layer's box.
  const inside = async () => {
    const c = await page.locator(".ss__card").boundingBox();
    return !!c && c.x >= 1284 && c.y >= 593 && c.x + c.width <= 1806 && c.y + c.height <= 995;
  };
  await expect.poll(inside, { timeout: 3_000 }).toBe(true);
  // For review: the card with its first sponsor, after the slide has faded in.
  await page.waitForTimeout(1_500);
  await page.screenshot({ path: test.info().outputPath("sponsor-slideshow-recipe.png") });
});
