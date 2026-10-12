// Screens against the real server: project templates, screen files, live rules and the folder
// watcher, through the fixture's project commands (see src-tauri/examples/fixture_server.rs).
// Every test builds its own screen, so the shared test screen is untouched.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test, type Fixture } from "./fixture";
import type { Rect, ScreenDoc, ScreenLayer } from "../src/lib/types";

/** Template layer with field values; the box is the template's frame. */
function layer(id: string, templateId: string, rect: Rect, fields: Record<string, string>): ScreenLayer {
  return { id, name: id, kind: "template", templateId, rect, fields, vars: {} };
}

let seq = 0;
async function makeScreen(live: Fixture, layers: ScreenLayer[]): Promise<ScreenDoc> {
  const doc: ScreenDoc = {
    id: `e2e${Date.now().toString(36)}${seq++}`.slice(0, 20),
    name: "E2E screen",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers,
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc;
}

/** Opens a screen and waits for its snapshot. */
async function openScreen(page: Page, base: string, id: string) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${id}`);
  await (await ws).waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
  return errors;
}

/** The layer hosts' boxes in page px (viewport 1920×1080, so the stage is at scale 1), by z. */
function hostBoxes(page: Page) {
  return page.evaluate(() =>
    [...document.querySelectorAll("div")]
      .filter((d) => d.shadowRoot)
      .sort((a, b) => Number(a.style.zIndex) - Number(b.style.zIndex))
      .map((d) => {
        const r = d.getBoundingClientRect();
        return { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) };
      }),
  );
}

const roots = (page: Page) => page.locator("[data-state]");

async function settled(page: Page) {
  for (const el of await roots(page).all()) {
    await el.evaluate((e) => Promise.all(e.getAnimations({ subtree: true }).map((a) => a.finished)));
  }
}

let classicBar = "";
let cornerBug = "";

test.beforeEach(async ({ live }) => {
  classicBar ||= await live.cmd("addtemplate classic-bar");
  cornerBug ||= await live.cmd("addtemplate corner-bug");
  await live.cmd("liveedit onTake");
});

test("layers are drawn at their boxes, in z order", async ({ page, live }) => {
  const a: Rect = { x: 0, y: 540, w: 960, h: 540 };
  const b: Rect = { x: 1280, y: 0, w: 640, h: 360 };
  const doc = await makeScreen(live, [
    layer("bar", classicBar, a, { name: "Grace Hopper", title: "Rear Admiral" }),
    layer("bug", cornerBug, b, { text: "LIVE" }),
  ]);
  const errors = await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} bar`);
  await live.cmd(`layershow ${doc.id} bug`);
  await expect(roots(page)).toHaveCount(2);
  await expect(roots(page).nth(0)).toHaveAttribute("data-state", "in");
  await expect(roots(page).nth(1)).toHaveAttribute("data-state", "in");
  await settled(page);
  expect(await hostBoxes(page)).toEqual([a, b]);
  await expect(page.getByText("Grace Hopper")).toBeVisible();
  await expect(page).toHaveScreenshot("screen-layer-boxes.png", { omitBackground: true });
  expect(errors).toEqual([]);
});

test("a box moved on air under immediate edits moves without replaying", async ({ page, live }) => {
  await live.cmd("liveedit immediate");
  const doc = await makeScreen(live, [
    layer("bar", classicBar, { x: 0, y: 0, w: 1920, h: 1080 }, { name: "Ada", title: "Analyst" }),
  ]);
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} bar`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await settled(page);
  const root = await roots(page).elementHandle();

  await page.evaluate(() => {
    const w = window as unknown as { __starts: number };
    w.__starts = 0;
    document.addEventListener("animationstart", () => w.__starts++, true);
    for (const d of document.querySelectorAll("div")) {
      d.shadowRoot?.addEventListener("animationstart", () => w.__starts++, true);
    }
  });
  await live.cmd(`rect ${doc.id} bar {"x":-200,"y":-120,"w":1920,"h":1080}`);
  await expect.poll(() => hostBoxes(page)).toEqual([{ x: -200, y: -120, w: 1920, h: 1080 }]);
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => (window as unknown as { __starts: number }).__starts)).toBe(0);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  // Same node: nothing was recompiled.
  expect(await roots(page).evaluate((el, prev) => el === prev, root)).toBe(true);
});

test("on-air edits under onTake wait for the next take", async ({ page, live }) => {
  const full: Rect = { x: 0, y: 0, w: 1920, h: 1080 };
  const doc = await makeScreen(live, [layer("bar", classicBar, full, { name: "Ada", title: "Analyst" })]);
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} bar`);
  await expect(page.getByText("Ada", { exact: true })).toBeVisible();

  // Staged: a new name and a new box change nothing on air.
  const moved: Rect = { x: 100, y: -100, w: 1920, h: 1080 };
  await live.cmd(
    `savescreen ${JSON.stringify({ ...doc, layers: [layer("bar", classicBar, moved, { name: "Grace", title: "Analyst" })] })}`,
  );
  await live.cmd(`rect ${doc.id} bar ${JSON.stringify(moved)}`);
  await page.waitForTimeout(400);
  await expect(page.getByText("Ada", { exact: true })).toBeVisible();
  expect(await hostBoxes(page)).toEqual([full]);

  // Out and in again: the take applies the staged changes before the graphic comes in.
  await live.cmd(`layerhide ${doc.id} bar`);
  await expect(roots(page)).toHaveCount(0);
  await live.cmd(`layershow ${doc.id} bar`);
  await expect(page.getByText("Grace", { exact: true })).toBeVisible();
  expect(await hostBoxes(page)).toEqual([moved]);
});

test("a screen file edited outside Kinetra reloads the output", async ({ page, live }) => {
  await live.cmd("liveedit immediate");
  const doc = await makeScreen(live, [
    layer("bar", classicBar, { x: 0, y: 0, w: 1920, h: 1080 }, { name: "Ada", title: "Analyst" }),
  ]);
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} bar`);
  await expect(page.getByText("Ada", { exact: true })).toBeVisible();

  const file = join(await live.cmd("dir"), "screens", `${doc.id}.json`);
  const onDisk = JSON.parse(readFileSync(file, "utf8")) as { layers: ScreenLayer[] };
  if (onDisk.layers[0].kind === "template") onDisk.layers[0].fields.name = "Edited in VS Code";
  writeFileSync(file, JSON.stringify(onDisk, null, 2));
  await expect(page.getByText("Edited in VS Code")).toBeVisible({ timeout: 5_000 });
  await expect(roots(page)).toHaveAttribute("data-state", "in");
});
