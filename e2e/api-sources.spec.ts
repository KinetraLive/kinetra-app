// API sources against the real server, its scriptable mock API (`/__mock/api/<name>`) and
// the renderer: a JSON and a CSV source bound to a list layer update the output; unchanged data
// (same body, or an ETag 304) sends the renderer nothing; 500s back off (doubling) and recover;
// a 429's Retry-After is respected; a secret header reaches the API but no file in the project;
// "API value changed" fires a flow. Every test runs on its own fixture server (`apitiming` is
// server-wide), with test timing below the 1 s minimum so polls are quick.

import type { Page, WebSocket } from "@playwright/test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { expect, isolatedTest as test, type Fixture } from "./fixture";

// The first test may compile the fixture server.
test.describe.configure({ timeout: 300_000 });
import { openBus } from "./studio-helpers";
import { data, ev, flow, node, save, take, twoLayers } from "./flow-helpers";
import type { ApiStatus, FlowInfo, ScreenDoc, SourceDoc } from "../src/lib/types";

const FULL = { x: 0, y: 0, w: 1920, h: 1080 };
const names = (page: Page) => page.locator('.lb__row:not([data-row="out"]) .lb__name');

const COLUMNS = (csv: boolean) =>
  ["name", "team", "score"].map((key) => ({
    key,
    label: key[0].toUpperCase() + key.slice(1),
    kind: "text" as const,
    path: csv ? key : `/${key}`,
  }));

function apiDoc(
  live: Fixture,
  name: string,
  opts: { csv?: boolean; headers?: { name: string; value?: string }[] } = {},
) {
  return {
    name,
    kind: "api",
    request: { url: `${live.base}/__mock/api/${name}`, method: "GET", headers: opts.headers ?? [] },
    format: opts.csv ? "csv" : "json",
    mapping: {
      rows: opts.csv ? "" : "/items",
      columns: COLUMNS(!!opts.csv),
      csv: { header: true, delimiter: "auto" },
    },
    poll: { intervalMs: 1000, onlyWhenUsed: true, paused: false },
  };
}

const json = (rows: [string, string, number][]) =>
  JSON.stringify({ items: rows.map(([name, team, score]) => ({ name, team, score })) });
const mock = (live: Fixture, name: string, script: Record<string, unknown>) =>
  live.cmd(`mockapi ${name} ${JSON.stringify(script)}`);
const hits = async (live: Fixture, name: string) => Number(await live.cmd(`mockhits ${name}`));
const status = async (live: Fixture, id: string) => JSON.parse(await live.cmd(`apistatus ${id}`)) as ApiStatus;

/** A leaderboard layer bound to the source, on air, with the renderer page open on it. */
async function onAir(page: Page, live: Fixture, sourceId: string): Promise<{ doc: ScreenDoc; ws: WebSocket }> {
  const tpl = await live.cmd("addtemplate leaderboard");
  const doc: ScreenDoc = {
    id: `api${Date.now().toString(36)}`,
    name: "API",
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
  return { doc, ws };
}

/** Every frame the renderer receives from now on. */
function frames(ws: WebSocket) {
  const got: string[] = [];
  ws.on("framereceived", (f) => got.push(String(f.payload)));
  return got;
}

test("a JSON source bound to a list layer updates the renderer when the API's data changes", async ({
  page,
  own,
}) => {
  await own.cmd("apitiming 200");
  await mock(own, "scores", { body: json([["Ada", "A", 12], ["Grace", "B", 40]]) });
  const id = await own.cmd(`apisource ${JSON.stringify(apiDoc(own, "scores"))}`);
  await onAir(page, own, id);
  await expect(names(page)).toHaveText(["Ada", "Grace"]);
  await expect.poll(async () => (await status(own, id)).state).toBe("ok");
  expect((await status(own, id)).rows).toBe(2);

  await mock(own, "scores", { body: json([["Hedy", "C", 7], ["Grace", "B", 41], ["Ada", "A", 12]]) });
  await expect(names(page)).toHaveText(["Hedy", "Grace", "Ada"]);
  // The source file caches the last good rows (numbers mapped to text).
  const saved = JSON.parse(await own.cmd(`getsource ${id}`)) as SourceDoc;
  expect(saved.kind === "api" && saved.rows.map((r) => r.score)).toEqual(["7", "41", "12"]);
});

test("a CSV source bound to a list layer updates the renderer", async ({ page, own }) => {
  await own.cmd("apitiming 200");
  const csv = (body: string) => ({ contentType: "text/csv", body });
  await mock(own, "table", csv('name,team,score\r\n"Lovelace, Ada",A,12\r\nGrace,B,40\r\n'));
  const id = await own.cmd(`apisource ${JSON.stringify(apiDoc(own, "table", { csv: true }))}`);
  await onAir(page, own, id);
  await expect(names(page)).toHaveText(["Lovelace, Ada", "Grace"]);
  // Another delimiter, detected from the header line.
  await mock(own, "table", csv("name;team;score\nKatherine;D;99\n"));
  await expect(names(page)).toHaveText(["Katherine"]);
});

test("unchanged data sends the renderer nothing (same body, then ETag 304)", async ({ page, own }) => {
  await own.cmd("apitiming 150");
  const body = json([["Ada", "A", 12]]);
  await mock(own, "same", { body });
  const id = await own.cmd(`apisource ${JSON.stringify(apiDoc(own, "same"))}`);
  const { ws } = await onAir(page, own, id);
  await expect(names(page)).toHaveText(["Ada"]);
  await page.waitForTimeout(400); // the take's own messages settle
  const got = frames(ws);

  let n = await hits(own, "same");
  await expect.poll(() => hits(own, "same")).toBeGreaterThanOrEqual(n + 4);
  await mock(own, "same", { body, etag: '"v1"' });
  n = await hits(own, "same");
  await expect.poll(() => hits(own, "same")).toBeGreaterThanOrEqual(n + 4);
  expect(got).toEqual([]);
  expect((await status(own, id)).state).toBe("ok");
});

test("500s back off (doubling up to the cap), then the first success recovers", async ({ page, own }) => {
  await own.cmd("apitiming 100 1600");
  await mock(own, "flaky", { body: json([["Ada", "A", 12]]) });
  const id = await own.cmd(`apisource ${JSON.stringify(apiDoc(own, "flaky"))}`);
  await onAir(page, own, id);
  await expect(names(page)).toHaveText(["Ada"]);

  await mock(own, "flaky", { status: 500, body: "down" });
  await expect.poll(async () => (await status(own, id)).state).toBe("backoff");
  const s = await status(own, id);
  expect(s.httpStatus).toBe(500);
  expect(s.nextAt).not.toBeNull();
  // At a fixed 100 ms, 2.5 s would be ~25 requests; doubling from 100 ms (cap 1.6 s) is ≤ 7.
  const n = await hits(own, "flaky");
  await page.waitForTimeout(2500);
  expect((await hits(own, "flaky")) - n).toBeLessThanOrEqual(7);
  // The output keeps the last good rows meanwhile.
  await expect(names(page)).toHaveText(["Ada"]);

  await mock(own, "flaky", { body: json([["Grace", "B", 40]]) });
  await expect(names(page)).toHaveText(["Grace"], { timeout: 4_000 });
  await expect.poll(async () => (await status(own, id)).state).toBe("ok");
  expect((await status(own, id)).message).toBeNull();
});

test("a 429 waits for its Retry-After before asking again", async ({ page, own }) => {
  await own.cmd("apitiming 100 30000");
  await mock(own, "limited", { body: json([["Ada", "A", 12]]) });
  const id = await own.cmd(`apisource ${JSON.stringify(apiDoc(own, "limited"))}`);
  await onAir(page, own, id);
  await expect(names(page)).toHaveText(["Ada"]);

  await mock(own, "limited", { status: 429, headers: { "Retry-After": "2" } });
  await expect.poll(async () => (await status(own, id)).httpStatus).toBe(429);
  const at = Date.now();
  const n = await hits(own, "limited");
  const s = await status(own, id);
  expect(s.state).toBe("backoff");
  expect(s.nextAt! - s.lastAttemptAt!).toBeGreaterThanOrEqual(1900);
  await mock(own, "limited", { body: json([["Grace", "B", 40]]) });
  await page.waitForTimeout(1500);
  expect(await hits(own, "limited")).toBe(n);
  await expect(names(page)).toHaveText(["Grace"], { timeout: 3_000 });
  expect(Date.now() - at).toBeGreaterThanOrEqual(1500);
});

function allFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? allFiles(p) : [p];
  });
}

test("a secret header reaches the API but no file in the project", async ({ page, own }) => {
  await own.cmd("apitiming 200");
  const SECRET = "sk-live-e2e-7f3a9c";
  await mock(own, "private", { body: json([["Ada", "A", 12]]) });
  const id = await own.cmd(
    `apisource ${JSON.stringify(apiDoc(own, "private", { headers: [{ name: "Accept", value: "application/json" }] }))}`,
  );
  const doc = JSON.parse(await own.cmd(`setsecret ${id} X-Api-Key ${SECRET}`)) as SourceDoc;
  const header = doc.kind === "api" ? doc.request.headers.find((h) => h.name === "X-Api-Key") : undefined;
  expect(header?.stored).toBe(true);
  expect(header?.secret).toBeTruthy();
  expect(header?.value ?? null).toBeNull();

  await onAir(page, own, id);
  await expect(names(page)).toHaveText(["Ada"]);
  const req = JSON.parse(await own.cmd("mockreq private")) as Record<string, string>;
  expect(req["x-api-key"]).toBe(SECRET);
  expect(req["accept"]).toBe("application/json");

  // A test request redacts it in the response headers too.
  await mock(own, "private", { body: json([["Ada", "A", 12]]), headers: { "X-Api-Key": SECRET } });
  const saved = await own.cmd(`getsource ${id}`);
  expect(saved).not.toContain(SECRET);
  expect(await own.cmd(`testapi ${saved}`)).not.toContain(SECRET);

  const dir = await own.cmd("dir");
  for (const f of allFiles(dir)) expect(readFileSync(f).includes(SECRET), f).toBe(false);
});

test("API value changed fires a flow with the new value", async ({ page, own }) => {
  await own.cmd("apitiming 200");
  const guest = (name: string, score: number) => JSON.stringify({ items: [{ name, team: "A", score }] });
  await mock(own, "guest", { body: guest("Ada", 1) });
  const id = await own.cmd(`apisource ${JSON.stringify(apiDoc(own, "guest"))}`);
  const screen = await twoLayers(own);
  await openBus(page, own.base, screen.id, "program");
  // API value changed (name) ▶ Set fields on B (● value → name) ▶ Take B.
  const flowId = await save(
    own,
    flow(
      "On new guest",
      [
        node("api", { type: "trigger.apiChanged", config: { sourceId: id, column: "name" } }),
        node("sf", { type: "action.setFields", config: { screenId: screen.id, layerId: "b", fields: { name: "" } } }),
        take("tb", screen.id, "b"),
      ],
      [ev("api", "fired", "sf"), data("api", "value", "sf", "name"), ev("sf", "done", "tb")],
    ),
  );
  // The armed flow makes the source "in use": it polls with no layer bound. The first fetch is a
  // change too (no rows before it).
  await expect.poll(async () => (await status(own, id)).state).toBe("ok");
  await expect(page.getByText("Ada")).toBeVisible({ timeout: 4_000 });

  await mock(own, "guest", { body: guest("Grace Hopper", 1) });
  await expect(page.getByText("Grace Hopper")).toBeVisible({ timeout: 4_000 });

  // Another column changing doesn't fire it.
  const lastFired = async () =>
    (JSON.parse(await own.cmd("flows")) as FlowInfo[]).find((f) => f.id === flowId)?.lastFiredAt;
  const fired = await lastFired();
  expect(fired).toBeTruthy();
  await mock(own, "guest", { body: guest("Grace Hopper", 2) });
  const n = await hits(own, "guest");
  await expect.poll(() => hits(own, "guest")).toBeGreaterThanOrEqual(n + 3);
  expect(await lastFired()).toBe(fired);
});
