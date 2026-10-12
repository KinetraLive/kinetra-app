// Repeat loops and Stop flow against the real server and renderer: a counted Repeat takes a layer
// once per round and then fires `done`; a forever loop ends by the Stop button, by a Stop flow
// node in another flow and by PANIC; the 100 ms pacing guard (one warning per run in the debug
// events, ≤ ~10 rounds/s); and a flow that runs itself is refused. Takes are counted in the page
// (a MutationObserver per shadow root on `data-state`); negative checks wait just past the next
// round the loop would have started.

import { expect, test } from "./fixture";
import {
  clear,
  cue,
  data,
  delay,
  deploy,
  ev,
  flow,
  node,
  openScreen,
  repeat,
  roots,
  save,
  state,
  stopFlow,
  take,
  takes,
  twoLayers,
  watchTakes,
} from "./flow-helpers";
import type { Fixture } from "./fixture";
import type { FlowCheck, FlowDebug, FlowEdge, FlowNode, LiveLayerView } from "../src/lib/types";

const PACED = "Looping faster than every 100 ms — paced.";

// Every loop a test started is stopped, so nothing keeps running into the next test.
const started: string[] = [];
test.afterEach(async ({ live }) => {
  for (const id of started.splice(0)) await live.cmd(`stopflow ${id}`).catch(() => {});
});
async function run(live: Fixture, id: string) {
  started.push(id);
  await live.cmd(`runflow ${id}`);
}

/** cue ▶ Repeat(count, delayMs) ▶ body: take A → delay `onMs` → clear A. */
function loop(
  screenId: string,
  count: number | null,
  delayMs: number,
  onMs: number,
  extra: { nodes: FlowNode[]; edges: FlowEdge[] } = { nodes: [], edges: [] },
) {
  return flow(
    count === null ? "Forever" : `Repeat ${count}`,
    [
      cue(),
      repeat("rep", count, delayMs),
      take("ta", screenId, "a"),
      delay("d", onMs),
      clear("ca", screenId, "a"),
      ...extra.nodes,
    ],
    [ev("cue", "fired", "rep"), ev("rep", "body", "ta"), ev("ta", "done", "d"), ev("d", "out", "ca"), ...extra.edges],
  );
}

test("Repeat 3× takes and clears a layer three times, then `done` takes the other once", async ({ page, live }) => {
  const doc = await twoLayers(live);
  await watchTakes(page);
  await openScreen(page, live.base, doc.id);
  const id = await save(
    live,
    loop(doc.id, 3, 300, 200, {
      nodes: [take("tb", doc.id, "b")],
      edges: [ev("rep", "done", "tb")],
    }),
  );
  await run(live, id);
  await expect(page.getByText("Layer B")).toBeVisible({ timeout: 5_000 });
  await expect.poll(() => state(live, id)).toBe("idle");
  expect(await takes(page, "A")).toBe(3);
  expect(await takes(page, "B")).toBe(1);
  // A is cleared after its last round; only B stays on air.
  await expect(roots(page)).toHaveCount(1, { timeout: 4_000 });
});

test("a forever Repeat ends with the Stop button: idle, no more takes", async ({ page, live }) => {
  const doc = await twoLayers(live);
  await watchTakes(page);
  await openScreen(page, live.base, doc.id);
  const id = await save(live, loop(doc.id, null, 150, 150));
  await run(live, id);
  await expect.poll(() => takes(page, "A")).toBeGreaterThanOrEqual(2);
  expect(await state(live, id)).toBe("running");
  await live.cmd(`stopflow ${id}`);
  await expect.poll(() => state(live, id)).toBe("idle");
  const n = await takes(page, "A");
  // Past two more rounds' worth of time: nothing else was taken.
  await page.waitForTimeout(800);
  expect(await takes(page, "A")).toBe(n);
});

test("Stop flow in another cue flow stops a forever loop, then its `done` runs", async ({ page, live }) => {
  const doc = await twoLayers(live);
  await watchTakes(page);
  await openScreen(page, live.base, doc.id);
  const loopId = await save(live, loop(doc.id, null, 150, 150));
  const stopperId = await save(
    live,
    flow(
      "Stop loop",
      [cue(), stopFlow("sf", loopId), take("tb", doc.id, "b")],
      [ev("cue", "fired", "sf"), ev("sf", "done", "tb")],
    ),
  );
  await run(live, loopId);
  await expect.poll(() => takes(page, "A")).toBeGreaterThanOrEqual(1);
  await run(live, stopperId);
  await expect.poll(() => state(live, loopId)).toBe("idle");
  await expect(page.getByText("Layer B")).toBeVisible();
  expect(await takes(page, "B")).toBe(1);
  const n = await takes(page, "A");
  await page.waitForTimeout(800);
  expect(await takes(page, "A")).toBe(n);
  // Stopping a flow that isn't running is nothing to do: `done` still fires (B re-taken on air,
  // which is no new `in`), and the stopper ends.
  await run(live, stopperId);
  await expect.poll(() => state(live, stopperId)).toBe("idle");
  expect(await state(live, loopId)).toBe("idle");
});

test("PANIC stops a forever loop: nothing on air, no more takes", async ({ page, live }) => {
  const doc = await twoLayers(live);
  await watchTakes(page);
  await openScreen(page, live.base, doc.id);
  // Mostly on air (600 ms on, 100 ms between), so PANIC cuts a layer that is up.
  const id = await save(live, loop(doc.id, null, 100, 600));
  await run(live, id);
  await expect(page.getByText("Layer A")).toBeVisible();
  await live.cmd("panic");
  await expect(roots(page)).toHaveCount(0);
  await expect.poll(() => state(live, id)).toBe("idle");
  const n = await takes(page, "A");
  await page.waitForTimeout(1_000);
  expect(await takes(page, "A")).toBe(n);
  await expect(roots(page)).toHaveCount(0);
});

test("a Repeat with no delay and an instant body is paced to ≤ 10 rounds/s, warned once per run", async ({ live }) => {
  const doc = await twoLayers(live);
  // cue ▶ Repeat(forever, 0 ms) ▶ body: Set fields name ← round (off air: applies at once).
  const id = await save(
    live,
    flow(
      "Fast",
      [
        cue(),
        repeat("rep", null, 0),
        node("sf", {
          type: "action.setFields",
          config: { screenId: doc.id, layerId: "b", fields: { name: "" } },
        }),
      ],
      [ev("cue", "fired", "rep"), ev("rep", "body", "sf"), data("rep", "round", "sf", "name")],
    ),
  );
  const round = async () => {
    const v = JSON.parse(await live.cmd(`layerview ${doc.id} b`)) as LiveLayerView;
    return Number(v.fields.name);
  };
  const warnings = async () =>
    (JSON.parse(await live.cmd("debugevents")) as FlowDebug[])
      .filter((d) => d.id === id)
      .flatMap((d) => d.errors)
      .filter((e) => e.message === PACED);

  await live.cmd(`subscribedebug ${id}`);
  try {
    await live.cmd("debugevents"); // drop older batches
    for (let pass = 0; pass < 2; pass++) {
      const t0 = Date.now();
      await run(live, id);
      // Run for ~1.5 s, then stop and read the last round from the layer's field.
      await expect.poll(round).toBeGreaterThanOrEqual(10);
      await expect.poll(() => Date.now() - t0, { intervals: [100] }).toBeGreaterThanOrEqual(1_500);
      expect(await state(live, id)).toBe("running"); // no "Too many steps" over many rounds
      await live.cmd(`stopflow ${id}`);
      const elapsed = Date.now() - t0;
      await expect.poll(() => state(live, id)).toBe("idle");
      const rounds = await round();
      // One round per 100 ms at most (the first one starts at once), with a round of slack.
      expect(rounds).toBeGreaterThanOrEqual(10);
      expect(rounds).toBeLessThanOrEqual(Math.floor(elapsed / 100) + 2);
      // The last batches arrive ≤ 100 ms apart: one more poll interval collects them.
      await new Promise((r) => setTimeout(r, 250));
      const w = await warnings();
      expect(w, `pass ${pass}`).toHaveLength(1);
      expect(w[0].node).toBe("rep");
    }
  } finally {
    await live.cmd(`unsubscribedebug ${id}`);
  }
});

test("a flow can't run itself: deploy and check name the Run flow node", async ({ live }) => {
  const id = await save(live, flow("Selfish", [cue()], []));
  const graph = flow(
    "Selfish",
    [
      cue(),
      node("r", {
        type: "action.runFlow",
        config: { flowId: id, wait: false },
      }),
    ],
    [ev("cue", "fired", "r")],
  );
  await expect(deploy(live, id, graph)).rejects.toThrow(/A flow can't run itself; use Repeat/);
  const check = JSON.parse(
    await live.cmd(`checkflow ${JSON.stringify({ id, nodes: graph.nodes, edges: graph.edges })}`),
  ) as FlowCheck;
  expect(check.errors).toContainEqual({
    node: "r",
    message: "A flow can't run itself; use Repeat",
  });
  // The deployed graph is unchanged (still the cue alone).
  const flows = JSON.parse(await live.cmd("flows")) as {
    id: string;
    name: string;
  }[];
  expect(flows.some((f) => f.id === id)).toBe(true);
});
