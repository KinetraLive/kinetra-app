// Flow templates against the real server: the built-in gallery, adding templates pointed at a
// screen's layers (armed per rule), running an added cue flow, a multi-flow template's names and
// arming, and a missing slot refused with its label.

import { expect, test } from "./fixture";
import { local, openScreen, roots, state, twoLayers } from "./flow-helpers";
import type { FlowInfo, FlowTemplateInfo } from "../src/lib/types";

const flows = async (live: { cmd(l: string): Promise<string> }): Promise<FlowInfo[]> =>
  JSON.parse(await live.cmd("flows"));

const add = async (live: { cmd(l: string): Promise<string> }, req: object): Promise<string[]> =>
  JSON.parse(await live.cmd(`addflowtemplate ${JSON.stringify(req)}`));

test("the gallery lists every built-in template in order", async ({ live }) => {
  const list: FlowTemplateInfo[] = JSON.parse(await live.cmd("flowtemplates"));
  expect(list).toHaveLength(18);
  expect(list[0].id).toBe("show-open");
  expect(list.map((t) => t.id)).toContain("manual-hold");
  for (const t of list) {
    expect(t.chain.length).toBeGreaterThan(0);
    expect(t.flows).toBeGreaterThanOrEqual(1);
  }
});

test("an added stinger runs from its cue and clears itself", async ({ page, live }) => {
  const doc = await twoLayers(live);
  await openScreen(page, live.base, doc.id);
  const [id] = await add(live, { templateId: "stinger-once", slots: { stinger: `${doc.id}/a` } });
  const info = (await flows(live)).find((f) => f.id === id)!;
  expect(info.armed).toBe(true);
  expect(info.name).toBe("Stinger without double-fire");

  await live.cmd(`runflow ${id}`);
  await expect(page.getByText("Layer A")).toBeVisible();
  // Take for 3 s: off air again on its own.
  await expect(roots(page)).toHaveCount(0, { timeout: 8_000 });
  await expect.poll(() => state(live, id)).toBe("idle");
});

test("clean all clears every screen", async ({ page, live }) => {
  const doc = await twoLayers(live);
  await openScreen(page, live.base, doc.id);
  const [stinger] = await add(live, { templateId: "stinger-once", slots: { stinger: `${doc.id}/b` } });
  const [clean] = await add(live, { templateId: "clean-all-log", slots: {} });
  await live.cmd(`runflow ${stinger}`);
  await expect(page.getByText("Layer B")).toBeVisible();
  await live.cmd(`runflow ${clean}`);
  await expect(roots(page)).toHaveCount(0, { timeout: 4_000 });
});

test("a multi-flow template names and arms each flow", async ({ live }) => {
  const now = Date.now();
  const src = await live.cmd(
    `addschedule ${JSON.stringify({
      name: "Hold stage",
      items: [{ id: "k", title: "Keynote", start: local(now + 60 * 60_000), end: local(now + 90 * 60_000) }],
    })}`,
  );
  const doc = await twoLayers(live);
  const t: FlowTemplateInfo = JSON.parse(await live.cmd("flowtemplates")).find(
    (x: FlowTemplateInfo) => x.id === "manual-hold",
  );
  const slots = Object.fromEntries(
    t.slots.map((s) => [s.key, s.kind === "scheduleSource" ? src : `${doc.id}/a`]),
  );
  const ids = await add(live, { templateId: "manual-hold", name: "Hold", slots });
  expect(ids).toHaveLength(3);
  const got = (await flows(live)).filter((f) => ids.includes(f.id));
  const byName = Object.fromEntries(got.map((f) => [f.name, f.armed]));
  expect(Object.keys(byName).sort()).toEqual(["Hold · hold", "Hold · release", "Hold · take"]);
  expect(byName["Hold · take"]).toBe(false);
  expect(byName["Hold · hold"]).toBe(true);
  expect(byName["Hold · release"]).toBe(true);
});

test("a missing slot is refused with its label", async ({ live }) => {
  await expect(live.cmd(`addflowtemplate ${JSON.stringify({ templateId: "next-guest", slots: {} })}`)).rejects.toThrow(
    /pick a/,
  );
});
