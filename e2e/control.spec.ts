// Control and text sources against the real server: text sources bound to layer fields, stepping with Next, "on take"
// staging, PANIC (instant cut) and persisted Control field edits. Every test builds its own
// screen; the shared test screen is untouched.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test, type Fixture } from "./fixture";
import type { ScreenDoc, ScreenLayer } from "../src/lib/types";

const FULL = { x: 0, y: 0, w: 1920, h: 1080 };
const roots = (page: Page) => page.locator("[data-state]");

let classicBar = "";
let seq = 0;

async function guests(live: Fixture): Promise<string> {
  return live.cmd(
    `addsource ${JSON.stringify({
      name: "Guests",
      columns: [
        { key: "name", label: "Name" },
        { key: "title", label: "Title" },
      ],
      rows: [
        { name: "Grace Hopper", title: "Rear Admiral" },
        { name: "Ada Lovelace", title: "Analyst" },
      ],
    })}`,
  );
}

async function makeScreen(live: Fixture, layer: ScreenLayer): Promise<ScreenDoc> {
  const doc: ScreenDoc = {
    id: `m5${Date.now().toString(36)}${seq++}`,
    name: "M5 screen",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [layer],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc;
}

const barLayer = (fields: Record<string, string>, bindings?: ScreenLayer["bindings"]): ScreenLayer => ({
  id: "bar",
  name: "Bar",
  kind: "template",
  templateId: classicBar,
  rect: FULL,
  fields,
  vars: {},
  ...(bindings ? { bindings } : {}),
});

async function openScreen(page: Page, base: string, id: string) {
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${id}`);
  await (await ws).waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
}

async function settled(page: Page) {
  for (const el of await roots(page).all()) {
    await el.evaluate((e) => Promise.all(e.getAnimations({ subtree: true }).map((a) => a.finished)));
  }
}

/** Counts animationstart events anywhere in the page (shadow roots included) from now on. */
function countAnimations(page: Page) {
  return page.evaluate(() => {
    const w = window as unknown as { __starts: number };
    w.__starts = 0;
    const count = () => w.__starts++;
    document.addEventListener("animationstart", count, true);
    for (const d of document.querySelectorAll("div")) d.shadowRoot?.addEventListener("animationstart", count, true);
  });
}
const starts = (page: Page) => page.evaluate(() => (window as unknown as { __starts: number }).__starts);

test.beforeEach(async ({ live }) => {
  classicBar ||= await live.cmd("addtemplate classic-bar");
  await live.cmd("liveedit onTake");
});

test("a bound field follows the source's current row", async ({ page, live }) => {
  await live.cmd("liveedit immediate");
  const src = await guests(live);
  const doc = await makeScreen(
    live,
    barLayer({ name: "", title: "" }, { name: { sourceId: src, column: "name" }, title: { sourceId: src, column: "title" } }),
  );
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} bar`);
  await expect(page.getByText("Grace Hopper")).toBeVisible();
  await settled(page);
  await countAnimations(page);

  expect(await live.cmd(`stepsource ${src} 1`)).toBe("1");
  await expect(page.getByText("Ada Lovelace")).toBeVisible();
  await expect(page.getByText("Analyst", { exact: true })).toBeVisible();
  expect(await starts(page), "a field update never replays the animation").toBe(0);
  // Clamped at the end: no wrap.
  expect(await live.cmd(`stepsource ${src} 1`)).toBe("1");
});

test("under 'on take' a step waits for the next TAKE", async ({ page, live }) => {
  const src = await guests(live);
  const doc = await makeScreen(live, barLayer({ name: "", title: "" }, { name: { sourceId: src, column: "name" } }));
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} bar`);
  await expect(page.getByText("Grace Hopper")).toBeVisible();

  await live.cmd(`stepsource ${src} 1`);
  await page.waitForTimeout(400);
  await expect(page.getByText("Grace Hopper")).toBeVisible();

  // TAKE on the on-air layer applies the staged change in place.
  await settled(page);
  await countAnimations(page);
  await live.cmd(`layershow ${doc.id} bar`);
  await expect(page.getByText("Ada Lovelace")).toBeVisible();
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  expect(await starts(page), "no in-animation replay").toBe(0);
});

test("PANIC cuts everything at once, and the next take animates in", async ({ page, live }) => {
  const doc = await makeScreen(live, barLayer({ name: "Ada", title: "Analyst" }));
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} bar`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await settled(page);
  await countAnimations(page);

  await live.cmd("panic");
  // Gone at once: no out-animation to wait for.
  await expect(roots(page)).toHaveCount(0, { timeout: 500 });
  expect(await starts(page), "no out-animation").toBe(0);

  await live.cmd(`layershow ${doc.id} bar`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await countAnimations(page);
  await expect.poll(() => roots(page).evaluate((e) => e.getAnimations({ subtree: true }).length)).toBeGreaterThan(0);
});

test("a Control field edit is saved to the screen file", async ({ page, live }) => {
  await live.cmd("liveedit immediate");
  const doc = await makeScreen(live, barLayer({ name: "Ada", title: "Analyst" }));
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} bar`);
  await live.cmd(`setfields ${doc.id} bar {"name":"Grace Hopper"}`);
  await expect(page.getByText("Grace Hopper")).toBeVisible();

  const file = join(await live.cmd("dir"), "screens", `${doc.id}.json`);
  // Coalesced: written within about a second, not per keystroke.
  await expect
    .poll(() => {
      const d = JSON.parse(readFileSync(file, "utf8")) as { layers: ScreenLayer[] };
      return d.layers[0].fields.name;
    }, { timeout: 5_000 })
    .toBe("Grace Hopper");

  // A reload (reconnect) shows the saved value without replaying the on-air layer.
  await page.reload();
  await expect(page.getByText("Grace Hopper")).toBeVisible();
});
