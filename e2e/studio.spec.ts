// Studio mode (Preview / Program) against the real server and renderer: every operator edit goes
// to the Preview (its own output, /screen/<id>/preview, shows it settled) while Program stays
// untouched; Take moves it to Program with Cut, Crossfade or Play out & in as the Program
// renderer performs them; the T-bar sets host opacities and commits or aborts; automation goes
// straight to Program (both outputs follow; the cue is marked "Program changed"); PANIC; Preview
// after take; the Studio-off exits; a flow's Take preview. Program and Preview are two pages
// side by side; what the renderer does is recorded in the page (studio-helpers.ts).

import { expect, test, type Fixture } from "./fixture";
import { ev, flow, node, roots, save, take } from "./flow-helpers";
import {
  CUT,
  cue,
  hosts,
  layerView,
  op,
  openBoth,
  previewOf,
  rec,
  settledAll,
  shown,
  startRec,
  studio,
  studioOff,
  twoLayerScreen,
  type StRec,
} from "./studio-helpers";
import { twoLooks } from "./variant-helpers";
import type { BrowserContext, Page } from "@playwright/test";
import type { StudioSettings } from "../src/lib/types";

let tpl: { template: string; compact: string } | undefined;

test.beforeEach(async ({ live }) => {
  tpl ??= await twoLooks(live);
  await studioOff(live);
  await live.cmd("liveedit onTake");
});
test.afterEach(async ({ live }) => {
  await studioOff(live);
});

/**
 * A two-layer screen (a "Alpha", b "Bravo") with `onAir` taken to Program before Studio mode goes
 * on (with `s`), Program and Preview open and settled.
 */
async function setup(live: Fixture, page: Page, context: BrowserContext, s: Partial<StudioSettings> = {}, onAir = ["a"]) {
  const doc = await twoLayerScreen(live, tpl!.template);
  const { prog, pv } = await openBoth(page, context, live.base, doc.id);
  for (const l of onAir) await live.cmd(`layershow ${doc.id} ${l}`);
  const names = onAir.map((l) => `${l === "a" ? "Alpha" : "Bravo"}:default:in`);
  await expect.poll(() => shown(prog)).toEqual(names);
  await expect.poll(() => shown(pv)).toEqual(names);
  await settledAll(prog);
  await studio(live, s);
  // Program in front: a background page gets no animation frames and late animation events.
  await prog.bringToFront();
  return { id: doc.id, prog, pv };
}

const near = (x: number | undefined, p: number) => x !== undefined && Math.abs(x - p) <= 0.03;
/** Host opacities by layer name. */
const opacities = async (page: Page) => Object.fromEntries((await hosts(page)).map((h) => [h.name, h.opacity]));
/** Program DOM changes only (the opacity samples run every frame anyway). */
const changes = (r: StRec) => r.events;

// ---- Staging ------------------------------------------------------------------------------------

test("staging: cues and edits show settled in Preview, Program is untouched until the take", async ({ page, context, live }) => {
  const { id, prog, pv } = await setup(live, page, context);
  await startRec(prog);

  // Cue b in: Preview shows it settled (no in-animation running), Program doesn't change.
  let view = await cue(live, id, "b", true);
  expect(view.cues).toEqual([{ layerId: "b", visible: true, staged: false, programChanged: false }]);
  await expect.poll(() => shown(pv)).toEqual(["Alpha:default:in", "Bravo:default:in"]);
  const bravo = (await hosts(pv)).find((h) => h.name === "Bravo")!;
  expect(bravo).toMatchObject({ running: 0, opacity: 1 });

  // A field edit and a variant change of the on-air layer: Preview only.
  await live.cmd(`setfields ${id} a ${JSON.stringify({ name: "Alpha 2" })}`);
  await expect.poll(() => shown(pv)).toEqual(["Alpha 2:default:in", "Bravo:default:in"]);
  await live.cmd(`setvariant ${id} a ${tpl!.compact}`);
  await expect.poll(() => shown(pv)).toEqual(["Alpha 2:compact:in", "Bravo:default:in"]);
  expect((await hosts(pv)).every((h) => h.running === 0)).toBe(true);

  view = await previewOf(live, id);
  expect(view.cues).toEqual([
    { layerId: "a", visible: null, staged: true, programChanged: false },
    { layerId: "b", visible: true, staged: false, programChanged: false },
  ]);
  expect(view.layers.find((l) => l.id === "a")).toMatchObject({ onAir: true, variant: tpl!.compact, fields: { name: "Alpha 2" } });
  expect(view.layers.find((l) => l.id === "b")).toMatchObject({ onAir: true });

  // Program never moved.
  await prog.waitForTimeout(300);
  expect(await shown(prog)).toEqual(["Alpha:default:in"]);
  expect(changes(await rec(prog))).toEqual([]);
  expect(await layerView(live, id, "b")).toMatchObject({ onAir: false });

  // Take: Program gets all of it; the Preview diff is empty (keep).
  view = await op(live, id, { op: "take" });
  expect(view.cues).toEqual([]);
  await expect.poll(() => shown(prog)).toEqual(["Alpha 2:compact:in", "Bravo:default:in"]);
  expect(await layerView(live, id, "b")).toMatchObject({ onAir: true });
  expect(await shown(pv)).toEqual(["Alpha 2:compact:in", "Bravo:default:in"]);
});

// ---- Take transitions ---------------------------------------------------------------------------

test("take with cut: the incoming appears settled, the outgoing vanishes without playing out", async ({ page, context, live }) => {
  const { id, prog } = await setup(live, page, context);
  await cue(live, id, "a", false);
  await cue(live, id, "b", true);
  await startRec(prog);
  await op(live, id, { op: "take", transition: CUT });
  await expect.poll(() => shown(prog)).toEqual(["Bravo:default:in"]);
  await prog.waitForTimeout(300);
  const r = await rec(prog);
  // One DOM change: Alpha gone and Bravo in, in the same task (never two hosts at once).
  expect(r.max).toBe(1);
  expect(r.events.filter((e) => e.kind === "hosts").map((e) => e.hosts)).toEqual([["Bravo:in:0"]]);
  // No out-animation of Alpha, no in-animation still running on Bravo.
  expect(r.events.filter((e) => e.kind === "anim" && e.v === "lt-out")).toEqual([]);
  expect((await hosts(prog))[0]).toMatchObject({ running: 0, opacity: 1 });
  expect(await layerView(live, id, "a")).toMatchObject({ onAir: false });
});

test("take with crossfade: two hosts cross over in opacity, then one", async ({ page, context, live }) => {
  const { id, prog } = await setup(live, page, context);
  await cue(live, id, "a", false);
  await cue(live, id, "b", true);
  await startRec(prog);
  await op(live, id, { op: "take", transition: { kind: "crossfade", ms: 500 } });
  await expect.poll(() => shown(prog)).toEqual(["Bravo:default:in"]);
  await prog.waitForTimeout(100);
  const r = await rec(prog);
  expect(r.max).toBe(2);
  const both = r.events.findIndex((e) => e.kind === "hosts" && e.hosts?.length === 2);
  const one = r.events.findIndex((e) => e.kind === "hosts" && e.hosts?.join() === "Bravo:in:0");
  expect(r.events[both].hosts).toEqual(["Alpha:in:0", "Bravo:in:0"]);
  expect(one).toBeGreaterThan(both);
  const overlap = r.events[one].t - r.events[both].t;
  expect(overlap).toBeGreaterThan(400);
  expect(overlap).toBeLessThan(1_500);
  // Mid-crossfade frames: Bravo partly in while Alpha is partly out, summing to ~1.
  const mid = r.samples.filter(([t, o]) => t > r.events[both].t && t < r.events[one].t && o.Alpha !== undefined && o.Bravo !== undefined);
  const partial = mid.filter(([, o]) => o.Bravo > 0.15 && o.Bravo < 0.85);
  expect(partial.length).toBeGreaterThan(3);
  for (const [, o] of partial) expect(Math.abs(o.Alpha + o.Bravo - 1)).toBeLessThan(0.1);
  // Bravo's opacity only rises.
  const bravo = mid.map(([, o]) => o.Bravo);
  expect(bravo.every((v, i) => i === 0 || v >= bravo[i - 1] - 1e-6)).toBe(true);
  expect(r.events.filter((e) => e.kind === "anim" && e.v === "lt-out")).toEqual([]);
  expect((await hosts(prog))[0]).toMatchObject({ opacity: 1, running: 0 });
});

test("take with play out & in: the outgoing plays out, the incoming plays in", async ({ page, context, live }) => {
  const { id, prog } = await setup(live, page, context);
  await cue(live, id, "a", false);
  await cue(live, id, "b", true);
  await startRec(prog);
  await op(live, id, { op: "take", transition: { kind: "outIn", ms: 300 } });
  await expect.poll(() => shown(prog)).toEqual(["Bravo:default:in"]);
  await settledAll(prog);
  const r = await rec(prog);
  expect(r.events.some((e) => e.kind === "state" && e.v === "out" && e.name === "Alpha")).toBe(true);
  expect(r.events.some((e) => e.kind === "anim" && e.v === "lt-out" && e.name === "Alpha")).toBe(true);
  expect(r.events.some((e) => e.kind === "anim" && e.v === "lt-in" && e.name === "Bravo")).toBe(true);
  // Bravo came in with its in-animation running (not settled).
  const added = r.events.find((e) => e.kind === "hosts" && e.hosts?.some((h) => h.startsWith("Bravo:in")));
  expect(added?.hosts?.find((h) => h.startsWith("Bravo"))).not.toBe("Bravo:in:0");
});

// ---- T-bar --------------------------------------------------------------------------------------

test("T-bar: prepared at 0, opacities follow p, commit takes; abort restores Program", async ({ page, context, live }) => {
  const { id, prog } = await setup(live, page, context);
  await cue(live, id, "a", false);
  await cue(live, id, "b", true);

  let view = await op(live, id, { op: "tbarBegin" });
  expect(view.tbar).toBe(0);
  await expect.poll(() => opacities(prog)).toEqual({ Alpha: 1, Bravo: 0 });
  expect((await hosts(prog)).find((h) => h.name === "Bravo")).toMatchObject({ state: "in", running: 0 });
  for (const p of [0.25, 0.5, 0.75]) {
    await op(live, id, { op: "tbarSet", p });
    await expect
      .poll(async () => {
        const o = await opacities(prog);
        return near(o.Bravo, p) && near(o.Alpha, 1 - p);
      })
      .toBe(true);
  }
  // Program's state is unchanged until the commit.
  expect(await layerView(live, id, "b")).toMatchObject({ onAir: false });
  view = await op(live, id, { op: "tbarEnd", commit: true });
  expect(view.tbar).toBeNull();
  expect(view.cues).toEqual([]);
  await expect.poll(() => opacities(prog)).toEqual({ Bravo: 1 });
  expect(await shown(prog)).toEqual(["Bravo:default:in"]);
  expect(await layerView(live, id, "a")).toMatchObject({ onAir: false });
  expect(await layerView(live, id, "b")).toMatchObject({ onAir: true });

  // Back the other way, then abort: Program restored, the diff kept.
  await cue(live, id, "b", false);
  await cue(live, id, "a", true);
  await op(live, id, { op: "tbarBegin" });
  await op(live, id, { op: "tbarSet", p: 0.5 });
  await expect.poll(async () => near((await opacities(prog)).Alpha, 0.5)).toBe(true);
  expect(near((await opacities(prog)).Bravo, 0.5)).toBe(true);
  view = await op(live, id, { op: "tbarEnd", commit: false });
  expect(view.tbar).toBeNull();
  expect(view.cues.map((c) => [c.layerId, c.visible])).toEqual([
    ["a", true],
    ["b", false],
  ]);
  await expect.poll(() => opacities(prog)).toEqual({ Bravo: 1 });
  expect(await shown(prog)).toEqual(["Bravo:default:in"]);
  expect(await layerView(live, id, "b")).toMatchObject({ onAir: true });
  expect(await layerView(live, id, "a")).toMatchObject({ onAir: false });
});

// ---- Preview output and automation --------------------------------------------------------------

test("Preview output: staged content settled; automation reaches both; Program changed since cue", async ({ page, context, live }) => {
  const { id, prog, pv } = await setup(live, page, context);
  await live.cmd(`setfields ${id} a ${JSON.stringify({ name: "Alpha staged" })}`);
  await expect.poll(() => shown(pv)).toEqual(["Alpha staged:default:in"]);
  expect((await hosts(pv))[0]).toMatchObject({ running: 0, opacity: 1 });
  expect(await shown(prog)).toEqual(["Alpha:default:in"]);

  // Automation (a flow's take of b): straight to Program, and the Preview follows.
  const go = await save(live, flow("Take B", [node("cue", { type: "trigger.cue", config: { label: null } }), take("tb", id, "b")], [ev("cue", "fired", "tb")]));
  await live.cmd(`runflow ${go}`);
  await expect.poll(() => shown(prog)).toEqual(["Alpha:default:in", "Bravo:default:in"]);
  await expect.poll(() => shown(pv)).toEqual(["Alpha staged:default:in", "Bravo:default:in"]);
  expect((await hosts(pv)).find((h) => h.name === "Bravo")).toMatchObject({ running: 0 });
  // b was not cued: no marker.
  expect((await previewOf(live, id)).cues).toEqual([{ layerId: "a", visible: null, staged: true, programChanged: false }]);

  // Automation changes the cued layer a: Program shows it, Preview keeps the staged name on top.
  const set = await save(
    live,
    flow(
      "Set title",
      [node("cue", { type: "trigger.cue", config: { label: null } }), node("sf", { type: "action.setFields", config: { screenId: id, layerId: "a", fields: { title: "Automated" } } })],
      [ev("cue", "fired", "sf")],
    ),
  );
  await live.cmd(`runflow ${set}`);
  await expect.poll(async () => (await hosts(prog)).find((h) => h.name === "Alpha")?.title).toBe("Automated");
  await expect.poll(async () => (await hosts(pv)).find((h) => h.name === "Alpha staged")?.title).toBe("Automated");
  expect((await previewOf(live, id)).cues).toEqual([{ layerId: "a", visible: null, staged: true, programChanged: true }]);

  // The take clears the marker.
  expect((await op(live, id, { op: "take" })).cues).toEqual([]);
  await expect.poll(async () => (await hosts(prog)).map((h) => `${h.name}/${h.title}`)).toEqual(["Alpha staged/Automated", "Bravo/Studio"]);
});

// ---- PANIC --------------------------------------------------------------------------------------

test("PANIC during a prepared T-bar with cues: Program cut, T-bar gone, every Preview empty", async ({ page, context, live }) => {
  const { id, prog, pv } = await setup(live, page, context);
  const other = await twoLayerScreen(live, tpl!.template);
  await cue(live, other.id, "a", true);
  await live.cmd(`setfields ${id} a ${JSON.stringify({ name: "Alpha staged" })}`);
  await cue(live, id, "b", true);
  await op(live, id, { op: "tbarBegin" });
  await op(live, id, { op: "tbarSet", p: 0.5 });
  await expect.poll(async () => (await hosts(prog)).length).toBe(3);

  await live.cmd("panic");
  await expect(roots(prog)).toHaveCount(0);
  expect(await hosts(prog)).toEqual([]);
  await expect(roots(pv)).toHaveCount(0);
  for (const s of [id, other.id]) {
    const v = await previewOf(live, s);
    expect(v.cues).toEqual([]);
    expect(v.tbar).toBeNull();
    expect(v.layers.every((l) => !l.onAir)).toBe(true);
  }
  // Nothing comes back late (no prepared host, no crossfade finishing).
  await prog.waitForTimeout(400);
  expect(await hosts(prog)).toEqual([]);
});

// ---- Preview after take -------------------------------------------------------------------------

test("Preview after take: keep mirrors the new Program, clear cues every on-air layer out", async ({ page, context, live }) => {
  const { id, prog, pv } = await setup(live, page, context, { previewAfterTake: "keep" });
  await cue(live, id, "b", true);
  expect((await op(live, id, { op: "take" })).cues).toEqual([]);
  await expect.poll(() => shown(prog)).toEqual(["Alpha:default:in", "Bravo:default:in"]);
  await expect.poll(() => shown(pv)).toEqual(["Alpha:default:in", "Bravo:default:in"]);

  await studio(live, { previewAfterTake: "clear" });
  await cue(live, id, "a", false);
  const v = await op(live, id, { op: "take" });
  expect(v.cues).toEqual([{ layerId: "b", visible: false, staged: false, programChanged: false }]);
  expect(v.layers.every((l) => !l.onAir)).toBe(true);
  await expect.poll(() => shown(prog)).toEqual(["Bravo:default:in"]);
  await expect(roots(pv)).toHaveCount(0);
});

// ---- Studio off ---------------------------------------------------------------------------------

test("Studio off with Preview changes: refused without an exit; take and discard", async ({ page, context, live }) => {
  const { id, prog, pv } = await setup(live, page, context);
  await cue(live, id, "b", true);
  await expect(studio(live, { enabled: false })).rejects.toThrow(/Preview has changes/);
  // Nothing changed.
  expect((await previewOf(live, id)).cues).toHaveLength(1);
  await expect(op(live, id, { op: "clear" })).resolves.toMatchObject({ cues: [] });

  // take: the Preview goes to Program, Studio mode is off.
  await cue(live, id, "b", true);
  await studio(live, { enabled: false }, "take");
  await expect.poll(() => shown(prog)).toEqual(["Alpha:default:in", "Bravo:default:in"]);
  await expect(op(live, id, { op: "take" })).rejects.toThrow(/Studio mode is off/);
  expect(JSON.parse(await live.cmd("uistate")).control.studio.enabled).toBe(false);

  // discard: Program unchanged, the diff gone.
  await studio(live);
  await cue(live, id, "a", false);
  await expect.poll(() => shown(pv)).toEqual(["Bravo:default:in"]);
  await studio(live, { enabled: false }, "discard");
  await studio(live);
  expect((await previewOf(live, id)).cues).toEqual([]);
  await prog.waitForTimeout(300);
  expect(await shown(prog)).toEqual(["Alpha:default:in", "Bravo:default:in"]);
});

test("Studio off with keepStaged: on-air staged content waits for the next take, cues dropped", async ({ page, context, live }) => {
  const { id, prog } = await setup(live, page, context);
  await live.cmd(`setfields ${id} a ${JSON.stringify({ name: "Kept" })}`);
  await cue(live, id, "b", true);
  await studio(live, { enabled: false }, "keepStaged");
  await prog.waitForTimeout(300);
  expect(await shown(prog)).toEqual(["Alpha:default:in"]);
  expect(await layerView(live, id, "a")).toMatchObject({ onAir: true, pending: true, fields: { name: "Kept" } });
  expect(await layerView(live, id, "b")).toMatchObject({ onAir: false });
  // The next take (onTake) applies it.
  await live.cmd(`layershow ${id} a`);
  await expect.poll(() => shown(prog)).toEqual(["Kept:default:in"]);
  expect(await layerView(live, id, "a")).toMatchObject({ pending: false });
});

// ---- Flows --------------------------------------------------------------------------------------

test("a flow's Take preview takes the Preview; with Studio mode off its failed port fires", async ({ page, context, live }) => {
  const { id, prog } = await setup(live, page, context, {}, []);
  const fid = await save(
    live,
    flow(
      "Take preview",
      [
        node("cue", { type: "trigger.cue", config: { label: null } }),
        node("tp", { type: "action.takePreview", config: { screenId: id, transition: CUT } }),
        take("ta", id, "a"),
      ],
      [ev("cue", "fired", "tp"), ev("tp", "failed", "ta")],
    ),
  );
  await cue(live, id, "b", true);
  await live.cmd(`runflow ${fid}`);
  await expect.poll(() => shown(prog)).toEqual(["Bravo:default:in"]);
  expect((await hosts(prog))[0]).toMatchObject({ running: 0 });
  expect((await previewOf(live, id)).cues).toEqual([]);
  await prog.waitForTimeout(400);
  // Not failed: a stays off.
  expect(await shown(prog)).toEqual(["Bravo:default:in"]);

  await studio(live, { enabled: false });
  await live.cmd(`runflow ${fid}`);
  await expect.poll(() => shown(prog)).toEqual(["Bravo:default:in", "Alpha:default:in"]);
});
