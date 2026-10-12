// Template variants against the real server and renderer (migration: variants-migration.spec.ts): switching
// looks off air and on air (onTake staging, immediate crossfade and out-and-in as the renderer
// performs them), field values kept across looks, and a flow's Set variant bypassing onTake.
// Swaps are recorded in the page with MutationObservers installed before the command.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test, type Fixture } from "./fixture";
import { cue, ev, flow, node, openScreen, roots, save, state } from "./flow-helpers";
import { looks, oneLayer, recording, settled, startRecording, twoLooks, type Rec } from "./variant-helpers";
import type { LiveLayerView, ScreenLayer } from "../src/lib/types";

const view = async (live: Fixture, screen: string, layer = "l1") =>
  JSON.parse(await live.cmd(`layerview ${screen} ${layer}`)) as LiveLayerView;

/** Index of the first event matching `f`, -1 if none. */
const at = (rec: Rec, f: (e: Rec["events"][number]) => boolean) => rec.events.findIndex(f);

let tpl: { template: string; compact: string } | undefined;
let dir = "";
test.beforeEach(async ({ live }) => {
  tpl ??= await twoLooks(live);
  dir ||= await live.cmd("dir");
  await live.cmd("liveedit onTake");
});
test.afterEach(async ({ live }) => {
  await live.cmd("liveedit onTake");
});

// ---- Switching looks ----------------------------------------------------------------------------

test("off air, a variant applies at once: the next take shows it", async ({ page, live }) => {
  const doc = await oneLayer(live, tpl!.template);
  await openScreen(page, live.base, doc.id);
  await live.cmd(`setvariant ${doc.id} l1 ${tpl!.compact}`);
  expect(await view(live, doc.id)).toMatchObject({ onAir: false, pending: false, variant: tpl!.compact });
  // Saved with the screen.
  await expect
    .poll(() => JSON.parse(readFileSync(join(dir, "screens", `${doc.id}.json`), "utf8")).layers[0].variant, { timeout: 3_000 })
    .toBe(tpl!.compact);

  await live.cmd(`layershow ${doc.id} l1`);
  await expect(page.locator('[data-look="compact"]')).toHaveAttribute("data-state", "in");
  expect(await looks(page)).toEqual(["compact"]);
  await expect(page.getByText("Ada Lovelace")).toBeVisible();
});

test("on air under onTake, a variant waits for the take, which swaps without an out cycle", async ({ page, live }) => {
  const doc = await oneLayer(live, tpl!.template);
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(page.locator('[data-look="default"]')).toHaveAttribute("data-state", "in");
  await settled(page);

  await startRecording(page);
  await live.cmd(`setvariant ${doc.id} l1 ${tpl!.compact}`);
  // Staged: the view reports the next take's variant, the output is untouched.
  expect(await view(live, doc.id)).toMatchObject({ onAir: true, pending: true, variant: tpl!.compact });
  await page.waitForTimeout(400);
  expect(await looks(page)).toEqual(["default"]);
  expect((await recording(page)).events).toEqual([]);

  await live.cmd(`layershow ${doc.id} l1`);
  await expect.poll(() => looks(page)).toEqual(["compact"]);
  await expect(roots(page)).toHaveCount(1);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await page.waitForTimeout(400);
  const rec = await recording(page);
  expect(rec.max).toBe(1);
  expect(rec.events.filter((e) => e.kind === "state" && e.v === "out")).toEqual([]);
  expect(rec.events.filter((e) => e.kind === "hosts")).toEqual([]);
  // Recompiled in place on the same host, its in-animation already finished (not replayed).
  expect(rec.events.filter((e) => e.kind === "tree")).toEqual([expect.objectContaining({ look: "compact", running: 0 })]);
  expect(await view(live, doc.id)).toMatchObject({ onAir: true, pending: false, variant: tpl!.compact });
});

test("on air under immediate, a crossfade overlaps two hosts briefly, then leaves one", async ({ page, live }) => {
  await live.cmd("liveedit immediate");
  const doc = await oneLayer(live, tpl!.template);
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(page.locator('[data-look="default"]')).toHaveAttribute("data-state", "in");
  await settled(page);

  await startRecording(page);
  const t0 = Date.now();
  await live.cmd(`setvariant ${doc.id} l1 ${tpl!.compact} crossfade`);
  await expect.poll(() => looks(page)).toEqual(["compact"]);
  await expect(roots(page)).toHaveCount(1);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  const rec = await recording(page);
  expect(rec.max).toBe(2);
  const both = at(rec, (e) => e.kind === "hosts" && e.hosts?.length === 2);
  const one = at(rec, (e) => e.kind === "hosts" && e.hosts?.join() === "compact:in");
  expect(rec.events[both].hosts).toEqual(["default:in", "compact:in"]);
  expect(one).toBeGreaterThan(both);
  // The overlap lasts about the crossfade (300 ms), the old look never plays out.
  const overlap = rec.events[one].t - rec.events[both].t;
  expect(overlap).toBeGreaterThan(200);
  expect(overlap).toBeLessThan(1_000);
  expect(rec.events.filter((e) => e.kind === "state" && e.v === "out")).toEqual([]);
  console.log(`crossfade: first DOM change ${(rec.events[0].t - t0).toFixed(1)} ms after the command; overlap ${overlap.toFixed(0)} ms`);
});

test("on air under immediate, out-and-in plays the old look out, then the new one in", async ({ page, live }) => {
  await live.cmd("liveedit immediate");
  const doc = await oneLayer(live, tpl!.template);
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(page.locator('[data-look="default"]')).toHaveAttribute("data-state", "in");
  await settled(page);

  await startRecording(page);
  await live.cmd(`setvariant ${doc.id} l1 ${tpl!.compact} outIn`);
  await expect.poll(() => looks(page)).toEqual(["compact"]);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  const rec = await recording(page);
  const out = at(rec, (e) => e.kind === "state" && e.v === "out" && e.look === "default");
  const inn = at(rec, (e) => e.kind === "hosts" && e.hosts?.join() === "compact:in");
  expect(out).toBe(0);
  expect(inn).toBeGreaterThan(out);
  // The new look waited for the old one's out-animation (250 ms) and never overlapped it.
  expect(rec.events[inn].t - rec.events[out].t).toBeGreaterThan(200);
  expect(rec.max).toBe(1);
  // …and then plays its own in-animation.
  await expect.poll(async () => (await recording(page)).events.some((e) => e.kind === "anim" && e.v === "lt-in" && e.look === "compact")).toBe(true);
});

test("field values survive switching looks, off air and on air", async ({ page, live }) => {
  const doc = await oneLayer(live, tpl!.template);
  await openScreen(page, live.base, doc.id);
  // The compact look doesn't use `sponsor`.
  await live.cmd(`setfields ${doc.id} l1 ${JSON.stringify({ name: "Grace Hopper", sponsor: "Initech" })}`);

  // Off air: compact, then default again.
  await live.cmd(`setvariant ${doc.id} l1 ${tpl!.compact}`);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(page.locator('[data-look="compact"]')).toBeVisible();
  await expect(page.getByText("Grace Hopper")).toBeVisible();
  await expect(page.getByText("Initech")).toHaveCount(0);
  expect((await view(live, doc.id)).fields).toMatchObject({ sponsor: "Initech" });
  await live.cmd(`layerhide ${doc.id} l1`);
  await expect(roots(page)).toHaveCount(0, { timeout: 4_000 });
  await live.cmd(`setvariant ${doc.id} l1 default`);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(page.getByText("Initech")).toBeVisible();

  // On air (immediate): compact, then default again.
  await live.cmd("liveedit immediate");
  await live.cmd(`setvariant ${doc.id} l1 ${tpl!.compact} crossfade`);
  await expect.poll(() => looks(page)).toEqual(["compact"]);
  await expect(page.getByText("Initech")).toHaveCount(0);
  await live.cmd(`setvariant ${doc.id} l1 default outIn`);
  await expect.poll(() => looks(page)).toEqual(["default"]);
  await expect(page.getByText("Initech")).toBeVisible();
  await expect(page.getByText("Grace Hopper")).toBeVisible();

  // The screen file kept the value all along (the default variant is not written).
  await expect
    .poll(() => {
      const file = JSON.parse(readFileSync(join(dir, "screens", `${doc.id}.json`), "utf8")) as { layers: ScreenLayer[] };
      const l = file.layers[0] as ScreenLayer & { variant?: string };
      return { sponsor: l.kind === "template" ? l.fields.sponsor : null, variant: l.variant ?? null };
    })
    .toEqual({ sponsor: "Initech", variant: null });
});

// ---- Flows --------------------------------------------------------------------------------------

test("a flow's Set variant swaps on air at once under onTake", async ({ page, live }) => {
  const doc = await oneLayer(live, tpl!.template);
  await openScreen(page, live.base, doc.id);
  await live.cmd(`layershow ${doc.id} l1`);
  await expect(page.locator('[data-look="default"]')).toHaveAttribute("data-state", "in");
  await settled(page);

  const id = await save(
    live,
    flow(
      "Go compact",
      [
        cue(),
        node("sv", {
          type: "action.setVariant",
          config: { screenId: doc.id, layerId: "l1", variant: tpl!.compact, transition: "outIn" },
        }),
      ],
      [ev("cue", "fired", "sv")],
    ),
  );
  await startRecording(page);
  await live.cmd(`runflow ${id}`);
  await expect.poll(() => looks(page)).toEqual(["compact"]);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  const rec = await recording(page);
  const out = at(rec, (e) => e.kind === "state" && e.v === "out" && e.look === "default");
  expect(out).toBe(0);
  expect(at(rec, (e) => e.kind === "hosts" && e.hosts?.join() === "compact:in")).toBeGreaterThan(out);
  expect(await view(live, doc.id)).toMatchObject({ onAir: true, pending: false, variant: tpl!.compact });
  await expect.poll(() => state(live, id)).toBe("idle");
});
