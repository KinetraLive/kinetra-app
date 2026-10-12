// Flows against the real server: a graph with a Branch on a Compare of the schedule's NOW item,
// Automation off holding back a time-of-day trigger while a cue still runs, a deploy cancelling an
// instance parked in a Delay, and PANIC stopping an instance (migration: migration.spec.ts). Every
// test builds its own screen; negative checks wait just past the moment the flow would have acted.

import { expect, test } from "./fixture";
import { cue, data, delay, deploy, ev, flow, local, node, openScreen, roots, save, state, take, twoLayers } from "./flow-helpers";
import type { FlowInfo } from "../src/lib/types";

test.afterEach(async ({ live }) => {
  await live.cmd("automationoff false");
  await live.cmd("clock 0");
});

// ---- Logic --------------------------------------------------------------------------------------

test("a Branch on a Compare of the NOW item takes the matching layer", async ({ page, live }) => {
  const now = Date.now();
  const m = 60_000;
  const src = await live.cmd(
    `addschedule ${JSON.stringify({
      name: "Stage",
      items: [
        { id: "k", title: "Keynote", start: local(now - 10 * m), end: local(now + 20 * m) },
        { id: "l", title: "Lunch", start: local(now + 30 * m), end: local(now + 90 * m) },
      ],
    })}`,
  );
  const doc = await twoLayers(live);
  await openScreen(page, live.base, doc.id);
  // cue ▶ branch; branch.condition ● = compare(scheduleItem(now).title, constant).
  const graph = (expected: string) =>
    flow(
      "Branchy",
      [
        cue(),
        node("br", { type: "logic.branch", config: {} }),
        node("si", { type: "data.scheduleItem", config: { sourceId: src, which: "now" } }),
        node("k", { type: "data.constant", config: { value: { kind: "text", value: expected } } }),
        node("cmp", { type: "data.compare", config: { op: "eq", a: "", b: "" } }),
        take("ta", doc.id, "a"),
        take("tb", doc.id, "b"),
      ],
      [
        ev("cue", "fired", "br"),
        data("si", "title", "cmp", "a"),
        data("k", "value", "cmp", "b"),
        data("cmp", "bool", "br", "condition"),
        ev("br", "then", "ta"),
        ev("br", "else", "tb"),
      ],
    );
  const id = await save(live, graph("Keynote"));
  await live.cmd(`runflow ${id}`);
  await expect(page.getByText("Layer A")).toBeVisible();
  await expect(roots(page)).toHaveCount(1);
  await expect.poll(() => state(live, id)).toBe("idle");

  await live.cmd(`clearscreen ${doc.id}`);
  await expect(roots(page)).toHaveCount(0, { timeout: 4_000 });
  await deploy(live, id, graph("Lunch"));
  await live.cmd(`runflow ${id}`);
  await expect(page.getByText("Layer B")).toBeVisible();
  await expect(roots(page)).toHaveCount(1);
});

// ---- Automation off -----------------------------------------------------------------------------

test("Automation off holds back a time-of-day trigger while a cue still runs", async ({ page, live }) => {
  // Put schedule time 2 s before a whole minute: the time-of-day trigger is due there.
  const real = Date.now();
  const boundary = Math.ceil((real + 65_000) / 60_000) * 60_000;
  const doc = await twoLayers(live);
  await openScreen(page, live.base, doc.id);
  const timed = (at: number) =>
    flow(
      "At the minute",
      [node("t", { type: "trigger.timeOfDay", config: { at: local(at).slice(11), days: [] } }), take("ta", doc.id, "a")],
      [ev("t", "fired", "ta")],
    );
  const timedId = await save(live, timed(boundary));
  const cueId = await save(live, flow("Go B", [cue(), take("tb", doc.id, "b")], [ev("cue", "fired", "tb")]));

  await live.cmd("automationoff true");
  await live.cmd(`clock ${boundary - 2_000 - real}`);
  // The operator's cue still runs.
  await live.cmd(`runflow ${cueId}`);
  await expect(page.getByText("Layer B")).toBeVisible();
  // Past the boundary: nothing fired by itself.
  await page.waitForTimeout(3_500);
  await expect(page.getByText("Layer A")).toHaveCount(0);
  expect((JSON.parse(await live.cmd("flows")) as FlowInfo[]).find((f) => f.id === timedId)?.lastFiredAt).toBeNull();

  // Back on, the next due time fires (missed ones are not caught up).
  await live.cmd("automationoff false");
  const next = boundary + 60_000;
  await deploy(live, timedId, timed(next));
  await live.cmd(`clock ${next - 2_000 - Date.now()}`);
  await expect(page.getByText("Layer A")).toBeVisible({ timeout: 6_000 });
});

// ---- Deploy and PANIC ---------------------------------------------------------------------------

test("deploying a flow cancels its instance parked in a Delay", async ({ page, live }) => {
  const doc = await twoLayers(live);
  await openScreen(page, live.base, doc.id);
  const graph = (ms: number) =>
    flow("Later", [cue(), delay("d", ms), take("ta", doc.id, "a")], [ev("cue", "fired", "d"), ev("d", "out", "ta")]);
  const id = await save(live, graph(1_500));
  await live.cmd(`runflow ${id}`);
  expect(await state(live, id)).toBe("running");
  await deploy(live, id, graph(1_600));
  await expect.poll(() => state(live, id)).toBe("idle");
  await page.waitForTimeout(2_200);
  await expect(roots(page)).toHaveCount(0);
});

test("PANIC stops a running instance", async ({ page, live }) => {
  const doc = await twoLayers(live);
  await openScreen(page, live.base, doc.id);
  const id = await save(
    live,
    flow(
      "A then B",
      [cue(), take("ta", doc.id, "a"), delay("d", 1_500), take("tb", doc.id, "b")],
      [ev("cue", "fired", "ta"), ev("ta", "done", "d"), ev("d", "out", "tb")],
    ),
  );
  await live.cmd(`runflow ${id}`);
  await expect(page.getByText("Layer A")).toBeVisible();
  expect(await state(live, id)).toBe("running");
  await live.cmd("panic");
  await expect(roots(page)).toHaveCount(0);
  await expect.poll(() => state(live, id)).toBe("idle");
  await page.waitForTimeout(2_000);
  await expect(roots(page)).toHaveCount(0);
});
