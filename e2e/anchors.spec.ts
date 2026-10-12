// Position anchors against the real server: the core places anchored layers on every save, keeps
// them placed through size and resolution changes, and a drag (`rect`, the editor's live path)
// releases the axis it moved. Checked in the renderer (host boxes in screen px) and on disk.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test, type Fixture } from "./fixture";
import type { LayerAnchor, Rect, ScreenDoc, ScreenLayer } from "../src/lib/types";

let classicBar = "";
let seq = 0;

test.beforeEach(async ({ live }) => {
  classicBar ||= await live.cmd("addtemplate classic-bar");
  await live.cmd("liveedit immediate");
});

function bar(rect: Rect, anchor?: LayerAnchor): ScreenLayer {
  const l: ScreenLayer = { id: "bar", name: "bar", kind: "template", templateId: classicBar, rect, fields: { name: "Ada" }, vars: {} };
  if (anchor) l.anchor = anchor;
  return l;
}

async function save(live: Fixture, doc: ScreenDoc) {
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
}

function newDoc(layers: ScreenLayer[]): ScreenDoc {
  return {
    id: `anc${Date.now().toString(36)}${seq++}`.slice(0, 20),
    name: "Anchors",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers,
    updatedAt: "",
  };
}

async function onDisk(live: Fixture, id: string): Promise<ScreenDoc> {
  const file = join(await live.cmd("dir"), "screens", `${id}.json`);
  return JSON.parse(readFileSync(file, "utf8")) as ScreenDoc;
}

async function open(page: Page, base: string, id: string) {
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${id}`);
  await (await ws).waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
}

/** The layer host's box in screen px (undoing the stage's letterbox scale). */
function hostBox(page: Page) {
  return page.evaluate(() => {
    const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot);
    const stage = host?.parentElement;
    if (!host || !stage) return null;
    const s = stage.getBoundingClientRect();
    const k = s.width / stage.offsetWidth;
    const r = host.getBoundingClientRect();
    return {
      x: Math.round((r.x - s.x) / k),
      y: Math.round((r.y - s.y) / k),
      w: Math.round(r.width / k),
      h: Math.round(r.height / k),
    };
  });
}

const centre: LayerAnchor = { h: "center", v: "middle", safe: false };

test("a centred layer stays centred when its width changes", async ({ page, live }) => {
  const doc = newDoc([bar({ x: 0, y: 0, w: 800, h: 200 }, centre)]);
  await save(live, doc);
  await open(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} bar`);
  await expect.poll(() => hostBox(page)).toEqual({ x: 560, y: 440, w: 800, h: 200 });
  // The editor sends the old position with the new width: the core keeps it centred.
  await save(live, { ...doc, layers: [bar({ x: 560, y: 440, w: 1200, h: 200 }, centre)] });
  await expect.poll(() => hostBox(page)).toEqual({ x: 360, y: 440, w: 1200, h: 200 });
  const file = await onDisk(live, doc.id);
  expect(file.layers[0].rect).toEqual({ x: 360, y: 440, w: 1200, h: 200 });
  expect(file.layers[0].anchor).toEqual(centre);
});

test("a centred layer stays centred when the screen resolution changes", async ({ page, live }) => {
  const doc = newDoc([bar({ x: 0, y: 0, w: 800, h: 200 }, centre)]);
  await save(live, doc);
  await open(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} bar`);
  await expect.poll(() => hostBox(page)).toEqual({ x: 560, y: 440, w: 800, h: 200 });
  await save(live, { ...doc, width: 1280, height: 720, layers: [bar({ x: 560, y: 440, w: 800, h: 200 }, centre)] });
  await expect.poll(() => hostBox(page)).toEqual({ x: 240, y: 260, w: 800, h: 200 });
  expect((await onDisk(live, doc.id)).layers[0].rect).toEqual({ x: 240, y: 260, w: 800, h: 200 });
});

test("a horizontal drag releases h and keeps v", async ({ page, live }) => {
  const doc = newDoc([bar({ x: 0, y: 0, w: 800, h: 200 }, centre)]);
  await save(live, doc);
  await open(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} bar`);
  await expect.poll(() => hostBox(page)).toEqual({ x: 560, y: 440, w: 800, h: 200 });
  // The drag (memory + broadcast): not fought by the anchor.
  await live.cmd(`rect ${doc.id} bar {"x":100,"y":440,"w":800,"h":200}`);
  await expect.poll(() => hostBox(page)).toEqual({ x: 100, y: 440, w: 800, h: 200 });
  // The drop's save (an editor that still carries the old anchor): released the same way.
  await save(live, { ...doc, layers: [bar({ x: 100, y: 440, w: 800, h: 200 }, centre)] });
  const file = await onDisk(live, doc.id);
  expect(file.layers[0].anchor).toEqual({ h: null, v: "middle", safe: false });
  expect(file.layers[0].rect).toEqual({ x: 100, y: 440, w: 800, h: 200 });
  // v still anchored: a taller box stays vertically centred, x stays where it was dropped.
  await save(live, { ...doc, layers: [bar({ x: 100, y: 440, w: 800, h: 400 }, file.layers[0].anchor!)] });
  await expect.poll(() => hostBox(page)).toEqual({ x: 100, y: 340, w: 800, h: 400 });
});
