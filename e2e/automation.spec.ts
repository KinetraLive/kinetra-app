// Template data and automation against the real server: list fields (in-place rows, enter/exit), a leaderboard sorted from a
// text source, a countdown that hides its layer at zero, source auto-advance (update and retake)
// and a cue flow (a migrated macro). Every test builds its own screen; the shared test screen is
// untouched.

import type { Page } from "@playwright/test";
import { expect, test, type Fixture } from "./fixture";
import type { FlowInfo, NewFlow, ScreenDoc, ScreenLayer } from "../src/lib/types";

const FULL = { x: 0, y: 0, w: 1920, h: 1080 };
const roots = (page: Page) => page.locator("[data-state]");
const rows = (page: Page) => page.locator(".lb__row");

const ids: Record<string, string> = {};
async function preset(live: Fixture, id: string): Promise<string> {
  return (ids[id] ||= await live.cmd(`addtemplate ${id}`));
}

let seq = 0;
async function screenWith(live: Fixture, layer: ScreenLayer): Promise<ScreenDoc> {
  const doc: ScreenDoc = {
    id: `m6${Date.now().toString(36)}${seq++}`,
    name: "M6 screen",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [layer],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc;
}

const layer = (templateId: string, fields: Record<string, string>, bindings?: ScreenLayer["bindings"]): ScreenLayer => ({
  id: "l1",
  name: "Layer",
  kind: "template",
  templateId,
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
    await el.evaluate((e) =>
      Promise.all(
        e
          .getAnimations({ subtree: true })
          .filter((a) => a.effect?.getTiming().iterations !== Infinity)
          .map((a) => a.finished),
      ),
    );
  }
}

/** Counts animationstart events anywhere (shadow roots included) from now on. */
function countAnimations(page: Page) {
  return page.evaluate(() => {
    const w = window as unknown as { __starts: number };
    w.__starts = 0;
    const count = () => w.__starts++;
    for (const d of document.querySelectorAll("div")) d.shadowRoot?.addEventListener("animationstart", count, true);
  });
}
const starts = (page: Page) => page.evaluate(() => (window as unknown as { __starts: number }).__starts);

const board = (n: number) =>
  JSON.stringify(Array.from({ length: n }, (_, i) => ({ name: `P${i + 1}`, team: `T${i + 1}`, score: String(100 - i) })));

test.beforeEach(async ({ live }) => {
  await live.cmd("liveedit immediate");
});

test("list rows update in place; added rows enter, removed rows leave", async ({ page, live }) => {
  const doc = await screenWith(live, layer(await preset(live, "leaderboard"), { title: "Standings", rows: board(3) }));
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(rows(page)).toHaveCount(3);
  await settled(page);
  const first = await rows(page).first().elementHandle();

  // One score changes and a fourth row arrives: row 1 keeps its node, the new row enters.
  const four = JSON.parse(board(4)) as Record<string, string>[];
  four[0].score = "250";
  await live.cmd(`setfields ${doc.id} l1 ${JSON.stringify({ rows: JSON.stringify(four) })}`);
  await expect(rows(page)).toHaveCount(4);
  await expect(rows(page).first().locator(".lb__score")).toHaveText("250");
  expect(await rows(page).first().evaluate((el, prev) => el === prev, first)).toBe(true);
  await expect(rows(page).nth(3)).toHaveAttribute("data-row", "in");

  // Two rows go: they play out, then leave the DOM.
  await live.cmd(`setfields ${doc.id} l1 ${JSON.stringify({ rows: board(2) })}`);
  await expect(page.locator('.lb__row[data-row="out"]')).toHaveCount(2);
  await expect(rows(page)).toHaveCount(2, { timeout: 4_000 });
  await expect(roots(page)).toHaveAttribute("data-state", "in");
});

test("a leaderboard bound to a source ranks itself (numeric, descending, top 3)", async ({ page, live }) => {
  const src = await live.cmd(
    `addsource ${JSON.stringify({
      name: "Results",
      columns: [
        { key: "name", label: "Name" },
        { key: "team", label: "Team" },
        { key: "score", label: "Score" },
      ],
      rows: [
        { name: "Ada", team: "A", score: "12" },
        { name: "Grace", team: "B", score: "40" },
        { name: "Hedy", team: "C", score: "7" },
        { name: "Katherine", team: "D", score: "31" },
      ],
    })}`,
  );
  const doc = await screenWith(
    live,
    layer(await preset(live, "leaderboard"), { title: "Top 3" }, {
      rows: { sourceId: src, column: "", sort: { column: "score", numeric: true, desc: true }, limit: 3 },
    }),
  );
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(page.locator(".lb__name")).toHaveText(["Grace", "Katherine", "Ada"]);
  await expect(page.locator(".lb__rank")).toHaveText(["1", "2", "3"]);
});

test("a countdown with 'hide at zero' takes its layer off air when it reaches 0:00", async ({ page, live }) => {
  const doc = await screenWith(
    live,
    layer(await preset(live, "scoreboard"), { home: "Home", away: "Away", home_score: "1", away_score: "0", period: "1st" }),
  );
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await live.cmd(`settimer ${doc.id} l1 clock {"action":"configure","dir":"down","end":"hide"}`);
  await live.cmd(`settimer ${doc.id} l1 clock {"action":"set","ms":2000}`);
  await expect(page.locator(".sb__time")).toHaveText("0:02");
  await live.cmd(`settimer ${doc.id} l1 clock {"action":"start"}`);
  await expect(page.locator(".sb__time")).toHaveText("0:01", { timeout: 2_500 });
  // At zero the server takes the layer off air (whole second, ≤ 0.5 s snap).
  await expect(roots(page)).toHaveCount(0, { timeout: 5_000 });
});

async function guests(live: Fixture): Promise<string> {
  return live.cmd(
    `addsource ${JSON.stringify({
      name: "Rotation",
      columns: [{ key: "name", label: "Name" }],
      rows: [{ name: "Sponsor A" }, { name: "Sponsor B" }, { name: "Sponsor C" }],
    })}`,
  );
}

test("auto-advance 'update' rotates the bound field in place while on air", async ({ page, live }) => {
  const src = await guests(live);
  const doc = await screenWith(
    live,
    layer(await preset(live, "classic-bar"), { name: "", title: "Presented by" }, { name: { sourceId: src, column: "name" } }),
  );
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(page.getByText("Sponsor A")).toBeVisible();
  await settled(page);
  await countAnimations(page);
  await live.cmd(`autoadvance ${src} {"everyMs":1000,"loop":true,"transition":"update"}`);
  await expect(page.getByText("Sponsor B")).toBeVisible({ timeout: 3_000 });
  await expect(page.getByText("Sponsor C")).toBeVisible({ timeout: 3_000 });
  await expect(page.getByText("Sponsor A")).toBeVisible({ timeout: 3_000 }); // loops
  expect(await starts(page), "update never replays the animation").toBe(0);

  // Off air, it stops: no step while nothing bound is on air.
  await live.cmd(`layerhide ${doc.id} l1`);
  await expect(roots(page)).toHaveCount(0, { timeout: 4_000 });
  await live.cmd(`autoadvance ${src} null`);
});

test("auto-advance 'retake' plays the layer out and back in with the next row", async ({ page, live }) => {
  const src = await guests(live);
  const doc = await screenWith(
    live,
    layer(await preset(live, "classic-bar"), { name: "", title: "Presented by" }, { name: { sourceId: src, column: "name" } }),
  );
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(page.getByText("Sponsor A")).toBeVisible();
  const states: string[] = [];
  await page.exposeFunction("__state", (s: string) => states.push(s));
  await roots(page).evaluate((el) => {
    new MutationObserver(() => (window as unknown as { __state(s: string): void }).__state(el.getAttribute("data-state") ?? "")).observe(el, {
      attributes: true,
      attributeFilter: ["data-state"],
    });
  });
  await live.cmd(`autoadvance ${src} {"everyMs":1500,"loop":false,"transition":"retake"}`);
  await expect(page.getByText("Sponsor B")).toBeVisible({ timeout: 5_000 });
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  expect(states).toContain("out");
  await live.cmd(`autoadvance ${src} null`);
});

test("a cue flow takes a layer in, waits, and clears it; Stop cancels a running one", async ({ page, live }) => {
  const doc = await screenWith(live, layer(await preset(live, "classic-bar"), { name: "Ada", title: "Analyst" }));
  await openScreen(page, live.base, doc.id);
  // What a migrated macro looks like: a Cue trigger wired through take → wait → clear.
  const chain = (waitMs: number): NewFlow => ({
    name: "Intro",
    armed: true,
    retrigger: "restart",
    nodes: [
      { id: "cue", type: "trigger.cue", config: { label: "Intro" }, position: { x: 0, y: 0 } },
      { id: "in", type: "action.take", config: { screenId: doc.id, layerId: "l1", forMs: null }, position: { x: 240, y: 0 } },
      { id: "wait", type: "logic.delay", config: { ms: waitMs }, position: { x: 480, y: 0 } },
      { id: "out", type: "action.clear", config: { screenId: doc.id, layerId: "l1" }, position: { x: 720, y: 0 } },
    ],
    edges: [
      { id: "e1", from: { node: "cue", port: "fired" }, to: { node: "in", port: "in" }, kind: "event" },
      { id: "e2", from: { node: "in", port: "done" }, to: { node: "wait", port: "in" }, kind: "event" },
      { id: "e3", from: { node: "wait", port: "out" }, to: { node: "out", port: "in" }, kind: "event" },
    ],
  });
  const id = await live.cmd(`saveflow ${JSON.stringify(chain(1200))}`);
  await live.cmd(`runflow ${id}`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  expect(await live.cmd(`flowstate ${id}`)).toBe("running");
  await expect(roots(page)).toHaveCount(0, { timeout: 5_000 });
  await expect.poll(() => live.cmd(`flowstate ${id}`)).toBe("idle");
  const fired = (JSON.parse(await live.cmd("flows")) as FlowInfo[]).find((f) => f.id === id);
  expect(fired?.lastFiredAt).toBeTruthy();

  // Stop while it waits: the clear never comes.
  await live.cmd(`updateflow ${JSON.stringify({ id, ...chain(1500), name: "Intro" })}`);
  await live.cmd(`runflow ${id}`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await live.cmd(`stopflow ${id}`);
  expect(await live.cmd(`flowstate ${id}`)).toBe("idle");
  await page.waitForTimeout(2_000);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await live.cmd(`layerhide ${doc.id} l1`);

  // A disarmed cue doesn't run.
  await live.cmd(`armflow ${id} false`);
  await expect(live.cmd(`runflow ${id}`)).rejects.toThrow(/disarmed/i);
});
