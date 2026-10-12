// The flow editor against the real server (the editor UI itself is covered by vitest and the
// screenshot harness): a draft is stored but not run until Deploy, check_flow names the nodes of an
// invalid draft and Deploy refuses it, Discard draft, live debug batches only while subscribed and
// throttled, and external edits of flows/<id>.json (deployed, invalid kept out, a draft kept).

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test, type Fixture } from "./fixture";
import { cue, data, delay, ev, flow, node, openScreen, roots, save, state, take, text, twoLayers } from "./flow-helpers";
import type { FilesChanged, FlowCheck, FlowDebug, FlowEdge, FlowInfo, FlowNode } from "../src/lib/types";

// ---- Helpers ------------------------------------------------------------------------------------

const info = async (live: Fixture, id: string) =>
  (JSON.parse(await live.cmd("flows")) as FlowInfo[]).find((f) => f.id === id);
const graph = (nodes: FlowNode[], edges: FlowEdge[]) => JSON.stringify({ nodes, edges });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Clears the screen, runs the flow's cue and expects exactly that layer on air. */
async function runTakes(page: Page, live: Fixture, screenId: string, id: string, layer: "A" | "B") {
  await live.cmd(`clearscreen ${screenId}`);
  await expect(roots(page)).toHaveCount(0, { timeout: 4_000 });
  await live.cmd(`runflow ${id}`);
  await expect(page.getByText(`Layer ${layer}`)).toBeVisible();
  await expect(roots(page)).toHaveCount(1);
  await expect.poll(() => state(live, id)).toBe("idle");
}

/** The `flow-debug` batches of this flow emitted since the last call. */
const debugOf = async (live: Fixture, id: string) =>
  (JSON.parse(await live.cmd("debugevents")) as FlowDebug[]).filter((b) => b.id === id);

/** Waits for a `project-files-changed` of this flow with that change; returns it. */
async function fileChange(live: Fixture, id: string, change: FilesChanged["change"]): Promise<FilesChanged> {
  const until = Date.now() + 5_000;
  while (Date.now() < until) {
    const list = JSON.parse(await live.cmd("files")) as FilesChanged[];
    const hit = list.find((c) => c.kind === "flow" && c.id === id && c.change === change);
    if (hit) return hit;
    await sleep(50);
  }
  throw new Error(`no project-files-changed ${change} for flow ${id}`);
}

/** The flow file on disk (null while it is being replaced). */
function readFlowFile(path: string): Record<string, unknown> | null {
  try {
    return JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
  } catch {
    return null;
  }
}

/** cue ▶ take A. */
const linearA = (screenId: string) => flow("Editor", [cue(), take("ta", screenId, "a")], [ev("cue", "fired", "ta")]);

// ---- Draft and deploy ---------------------------------------------------------------------------

test("a draft runs only after Deploy; check_flow names the nodes of an invalid one", async ({ page, live }) => {
  const doc = await twoLayers(live);
  await openScreen(page, live.base, doc.id);
  const id = await save(live, linearA(doc.id));
  await runTakes(page, live, doc.id, id, "A");

  // cue ▶ branch; condition ● = compare("same", "same") → then take B, else take A.
  const nodes = [
    cue(),
    node("br", { type: "logic.branch", config: {} }),
    text("k1", "same"),
    text("k2", "same"),
    node("cmp", { type: "data.compare", config: { op: "eq", a: "", b: "" } }),
    take("ta", doc.id, "a"),
    take("tb", doc.id, "b"),
  ];
  const edges = [
    ev("cue", "fired", "br"),
    data("k1", "value", "cmp", "a"),
    data("k2", "value", "cmp", "b"),
    data("cmp", "bool", "br", "condition"),
    ev("br", "then", "tb"),
    ev("br", "else", "ta"),
  ];
  await live.cmd(`saveflowdraft ${id} ${graph(nodes, edges)}`);
  expect((await info(live, id))?.draft?.nodes).toHaveLength(7);
  // Not deployed: the cue still runs the deployed graph.
  await runTakes(page, live, doc.id, id, "A");

  // An invalid draft (a text into the Branch's bool input; cmp ⇄ cmp2 a data cycle) is stored...
  const badNodes = [
    ...nodes.filter((n) => n.id !== "k2"),
    node("cmp2", { type: "data.compare", config: { op: "eq", a: "", b: "" } }),
  ];
  const badEdges = [
    ev("cue", "fired", "br"),
    data("k1", "value", "br", "condition"),
    data("cmp", "bool", "cmp2", "a"),
    data("cmp2", "bool", "cmp", "a"),
    ev("br", "then", "tb"),
    ev("br", "else", "ta"),
  ];
  await live.cmd(`saveflowdraft ${id} ${graph(badNodes, badEdges)}`);
  expect((await info(live, id))?.draft?.nodes.map((n) => n.id)).toContain("cmp2");
  // ... check_flow reports each error on its node ...
  const check = JSON.parse(await live.cmd(`checkflow ${graph(badNodes, badEdges)}`)) as FlowCheck;
  const on = (n: string) => check.errors.filter((e) => e.node === n).map((e) => e.message);
  expect(on("br").join(" ")).toContain("can't go into a bool input");
  expect(on("cmp").join(" ")).toContain("Data loop");
  expect(on("cmp2").join(" ")).toContain("Data loop");
  expect(check.errors.map((e) => e.node).sort()).toEqual(["br", "cmp", "cmp2"]);
  // ... and Deploy refuses it with the first error; the deployed graph keeps running.
  await expect(live.cmd(`deployflow ${id}`)).rejects.toThrow(/can't go into a bool input/);
  expect((await info(live, id))?.draft).toBeTruthy();
  await runTakes(page, live, doc.id, id, "A");

  // Fixed: no errors, deployed, the draft gone, and the Branch takes B.
  expect((JSON.parse(await live.cmd(`checkflow ${graph(nodes, edges)}`)) as FlowCheck).errors).toEqual([]);
  await live.cmd(`saveflowdraft ${id} ${graph(nodes, edges)}`);
  await live.cmd(`deployflow ${id}`);
  const after = await info(live, id);
  expect(after?.draft ?? null).toBeNull();
  expect(after?.nodes.map((n) => n.id)).toEqual(nodes.map((n) => n.id));
  await runTakes(page, live, doc.id, id, "B");
});

test("Discard draft keeps the deployed graph", async ({ page, live }) => {
  const doc = await twoLayers(live);
  await openScreen(page, live.base, doc.id);
  const id = await save(live, linearA(doc.id));
  const deployed = (await info(live, id))!;

  await live.cmd(`saveflowdraft ${id} ${graph([cue(), take("tb", doc.id, "b")], [ev("cue", "fired", "tb")])}`);
  expect((await info(live, id))?.draft).toBeTruthy();
  await live.cmd(`discardflowdraft ${id}`);
  const after = (await info(live, id))!;
  expect(after.draft ?? null).toBeNull();
  expect(after.nodes).toEqual(deployed.nodes);
  expect(after.edges).toEqual(deployed.edges);
  await runTakes(page, live, doc.id, id, "A");
});

// ---- Live debug ---------------------------------------------------------------------------------

test("live debug batches only while subscribed, at most one per 100 ms", async ({ live }) => {
  const doc = await twoLayers(live);
  const id = await save(live, linearA(doc.id));
  await live.cmd("debugevents");

  // Not subscribed: nothing recorded.
  await live.cmd(`runflow ${id}`);
  await expect.poll(() => state(live, id)).toBe("idle");
  await sleep(300);
  expect(await debugOf(live, id)).toEqual([]);

  // Subscribed: one batch with the trigger, the action and the event wire between them.
  await live.cmd(`subscribedebug ${id}`);
  await live.cmd(`runflow ${id}`);
  const batches: FlowDebug[] = [];
  await expect
    .poll(async () => {
      batches.push(...(await debugOf(live, id)));
      return batches.length;
    })
    .toBeGreaterThan(0);
  await sleep(300);
  batches.push(...(await debugOf(live, id)));
  expect(batches).toHaveLength(1);
  expect(batches[0].nodes.map((n) => n.node).sort()).toEqual(["cue", "ta"]);
  expect(batches[0].edges).toEqual(["e-cue-fired-ta"]);
  expect(batches[0].errors).toEqual([]);

  // A burst of 30 runs in ~1 s (retrigger: restart) is coalesced: one batch per 100 ms at most.
  const t0 = Date.now();
  for (let i = 0; i < 30; i++) {
    await live.cmd(`runflow ${id}`);
    await sleep(33);
  }
  await sleep(300);
  const burst = await debugOf(live, id);
  const elapsed = Date.now() - t0;
  expect(burst.length).toBeGreaterThan(0);
  expect(burst.length).toBeLessThanOrEqual(Math.floor(elapsed / 100) + 1);
  expect(burst.length / (elapsed / 1000)).toBeLessThanOrEqual(11);

  // Unsubscribed: nothing more.
  await live.cmd(`unsubscribedebug ${id}`);
  await live.cmd(`runflow ${id}`);
  await expect.poll(() => state(live, id)).toBe("idle");
  await sleep(300);
  expect(await debugOf(live, id)).toEqual([]);

  // Deleting a subscribed flow (with a batch pending) is fine.
  await live.cmd(`subscribedebug ${id}`);
  await live.cmd(`runflow ${id}`);
  await live.cmd(`deleteflow ${id}`);
  await sleep(300);
  await live.cmd("debugevents");
  expect(await info(live, id)).toBeUndefined();
  await expect(live.cmd(`subscribedebug ${id}`)).rejects.toThrow(/not found/i);
  await live.cmd(`unsubscribedebug ${id}`);
  await live.cmd(`clearscreen ${doc.id}`);
});

// ---- External edits -----------------------------------------------------------------------------

test("an external edit of flows/<id>.json deploys; an invalid one is kept out; a draft is kept", async ({
  page,
  live,
}) => {
  const doc = await twoLayers(live);
  await openScreen(page, live.base, doc.id);
  const id = await save(live, linearA(doc.id));
  const path = join(await live.cmd("dir"), "flows", `${id}.json`);
  await live.cmd("files");
  const write = (nodes: FlowNode[], edges: FlowEdge[], extra: Record<string, unknown> = {}) => {
    const file = readFlowFile(path)!;
    delete file.draft;
    writeFileSync(path, JSON.stringify({ ...file, nodes, edges, ...extra }, null, 2));
  };

  // A valid change of the deployed graph is deployed.
  write([cue(), take("tb", doc.id, "b")], [ev("cue", "fired", "tb")]);
  expect((await fileChange(live, id, "changed")).message ?? null).toBeNull();
  expect((await info(live, id))?.nodes.map((n) => n.id)).toEqual(["cue", "tb"]);
  await runTakes(page, live, doc.id, id, "B");

  // An invalid file: reported with why, and the last good graph keeps running.
  write([cue(), take("tb", doc.id, "b")], [ev("cue", "fired", "tb"), ev("cue", "fired", "ghost")]);
  expect((await fileChange(live, id, "invalid")).message).toContain("ghost");
  expect((await info(live, id))?.nodes.map((n) => n.id)).toEqual(["cue", "tb"]);
  await runTakes(page, live, doc.id, id, "B");

  // With a draft saved (and flushed), an edit of the deployed graph in a file without the draft
  // deploys the file and keeps the draft (written back to the file).
  const draftNodes = [cue(), delay("d", 50), take("tb", doc.id, "b")];
  const draftEdges = [ev("cue", "fired", "d"), ev("d", "out", "tb")];
  await live.cmd(`saveflowdraft ${id} ${graph(draftNodes, draftEdges)}`);
  await expect.poll(() => readFlowFile(path)?.draft ?? null, { timeout: 5_000 }).not.toBeNull();
  write([cue(), take("ta", doc.id, "a")], [ev("cue", "fired", "ta")]);
  await fileChange(live, id, "changed");
  const after = (await info(live, id))!;
  expect(after.nodes.map((n) => n.id)).toEqual(["cue", "ta"]);
  expect(after.draft?.nodes.map((n) => n.id)).toEqual(["cue", "d", "tb"]);
  await runTakes(page, live, doc.id, id, "A");
  await expect
    .poll(() => (readFlowFile(path)?.draft as { nodes?: unknown[] } | undefined)?.nodes?.length ?? 0, {
      timeout: 5_000,
    })
    .toBe(3);

  // Deploying the kept draft runs it.
  await live.cmd(`deployflow ${id}`);
  await runTakes(page, live, doc.id, id, "B");
  await live.cmd(`clearscreen ${doc.id}`);
});
