// Live data (WebSocket) sources against the real server, its WebSocket mock (`/__mock/ws`) and the
// renderer: a source bound to a list layer updates the output; 120 messages a second reach the
// renderer as at most `rate` updates a second; the filter picks the messages; the same data again
// sends nothing; the live rows never reach the project's files. Each test runs on its own fixture
// server (the mock is server-wide).

import type { Page, WebSocket } from "@playwright/test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { expect, isolatedTest as test, type Fixture } from "./fixture";
import { openBus } from "./studio-helpers";
import type { ScreenDoc, SocketStatus } from "../src/lib/types";

// The first test may compile the fixture server.
test.describe.configure({ timeout: 300_000 });

const FULL = { x: 0, y: 0, w: 1920, h: 1080 };
const names = (page: Page) => page.locator('.lb__row:not([data-row="out"]) .lb__name');

function socketDoc(live: Fixture, name: string, over: Record<string, unknown> = {}) {
  return {
    name,
    kind: "socket",
    url: `${live.base.replace(/^http/, "ws")}/__mock/ws`,
    mapping: {
      rows: "/items",
      columns: ["name", "team", "score"].map((key) => ({ key, label: key, kind: "text", path: `/${key}` })),
      csv: { header: true, delimiter: "auto" },
    },
    rate: 10,
    paused: false,
    ...over,
  };
}

const msg = (rows: [string, string, number][], type = "standings") =>
  JSON.stringify({ type, items: rows.map(([name, team, score]) => ({ name, team, score })) });
const status = async (live: Fixture, id: string) => JSON.parse(await live.cmd(`socketstatus ${id}`)) as SocketStatus;

/** A leaderboard layer bound to the source, on air, with the renderer page open on it. */
async function onAir(page: Page, live: Fixture, sourceId: string): Promise<WebSocket> {
  const tpl = await live.cmd("addtemplate leaderboard");
  const doc: ScreenDoc = {
    id: `ws${Date.now().toString(36)}`,
    name: "Live",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [
      {
        id: "l1",
        name: "Board",
        kind: "template",
        templateId: tpl,
        rect: FULL,
        fields: { title: "Standings" },
        vars: {},
        bindings: { rows: { sourceId, column: "" } },
      },
    ],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  const ws = await openBus(page, live.base, doc.id, "program");
  await live.cmd(`layershow ${doc.id} l1`);
  return ws;
}

/** Messages the renderer receives from now on. */
function frames(ws: WebSocket) {
  const got: string[] = [];
  ws.on("framereceived", (f) => got.push(String(f.payload)));
  return got;
}

/** Every file under `dir` containing `needle`. */
function filesWith(dir: string, needle: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...filesWith(p, needle));
    else if (readFileSync(p, "utf8").includes(needle)) out.push(p);
  }
  return out;
}

test("a bound live data source updates the renderer, paced, and keeps its rows out of the files", async ({ page, own }) => {
  const id = await own.cmd(`socketsource ${JSON.stringify(socketDoc(own, "Scores"))}`);
  expect(await own.cmd("sockettasks")).toBe("0");
  await onAir(page, own, id);
  await expect.poll(async () => (await status(own, id)).state).toBe("connected");
  await own.cmd(`wspush ${msg([["Ada", "A", 12], ["Grace", "B", 40]])}`);
  await expect(names(page)).toHaveText(["Ada", "Grace"]);
  expect((await status(own, id)).state).toBe("live");

  // 120 messages a second for 2 s: at most ~10 updates a second are applied.
  const before = Number(await own.cmd("socketapplied"));
  const started = Date.now();
  for (let i = 0; i < 240; i++) {
    await own.cmd(`wspush ${msg([["Ada", "A", i], ["Grace", "B", 40]])}`);
    await page.waitForTimeout(8);
  }
  const secs = (Date.now() - started) / 1000;
  await expect(page.locator('.lb__row:not([data-row="out"]) .lb__score').first()).toHaveText("239");
  const applied = Number(await own.cmd("socketapplied")) - before;
  expect(applied).toBeGreaterThanOrEqual(3);
  expect(applied).toBeLessThanOrEqual(Math.ceil(10 * secs) + 2);

  // The live rows are never written to the project.
  const dir = await own.cmd("dir");
  expect(filesWith(dir, "Grace")).toEqual([]);
});

test("the filter picks the messages, and the same data again sends the renderer nothing", async ({ page, own }) => {
  const id = await own.cmd(
    `socketsource ${JSON.stringify(socketDoc(own, "Filtered", { filter: { pointer: "/type", equals: "standings" } }))}`,
  );
  const ws = await onAir(page, own, id);
  await expect.poll(async () => (await status(own, id)).state).toBe("connected");
  await own.cmd(`wspush ${msg([["Ada", "A", 12]])}`);
  await expect(names(page)).toHaveText(["Ada"]);
  await own.cmd(`wspush ${msg([["Nope", "X", 0]], "ping")}`);
  await own.cmd(`wspush ${msg([["Hedy", "C", 7]])}`);
  await expect(names(page)).toHaveText(["Hedy"]);
  await page.waitForTimeout(400); // the update's own messages settle
  const got = frames(ws);
  for (let i = 0; i < 10; i++) await own.cmd(`wspush ${msg([["Hedy", "C", 7]])}`);
  await page.waitForTimeout(600);
  expect(got).toEqual([]);
});
