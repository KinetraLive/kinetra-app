// MIDI against the real server, through the same input path a controller uses (the fixture's
// in-memory device `TestPad`: `midiin` feeds it, `midiout` returns the LED messages it was sent):
// a mapped note toggles a layer and its LED turns red, two mappings on one pad run nothing, a CC
// fader drives opacity, and `trigger.midi` fires a flow.

import type { Page } from "@playwright/test";
import { expect, test, type Fixture } from "./fixture";
import type { ControlMapping, ScreenDoc } from "../src/lib/types";

const TEMPLATE = {
  name: "MIDI plate",
  html: '<div class="lt"><div class="plate"></div></div>',
  css: `.lt{position:absolute;inset:0}
.plate{position:absolute;inset:0;background:#ffffff}
.lt[data-state="in"]{animation:in 200ms both}
.lt[data-state="out"]{animation:out 150ms forwards}
@keyframes in{from{opacity:0}}
@keyframes out{to{opacity:0}}`,
  fields: [],
};

let templateId = "";
let seq = 0;

async function makeScreen(live: Fixture): Promise<string> {
  const doc: ScreenDoc = {
    id: `midi${Date.now().toString(36)}${seq++}`.slice(0, 20),
    name: "MIDI",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [
      { id: "l1", name: "Plate", kind: "template", templateId, rect: { x: 0, y: 0, w: 640, h: 360 }, fields: {}, vars: {} },
    ],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc.id;
}

async function openScreen(page: Page, base: string, id: string) {
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${id}`);
  await (await ws).waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
}

function note(id: string, number: number, commandId: string, feedback = false): ControlMapping {
  return { id, source: { device: "TestPad", channel: 1, kind: "note", number }, target: { kind: "command", commandId }, feedback };
}

async function sent(live: Fixture): Promise<number[][]> {
  return (JSON.parse(await live.cmd("midiout")) as { output: string; bytes: number[] }[]).map((m) => m.bytes);
}

const root = (page: Page) => page.locator(".lt");

test.beforeEach(async ({ live }) => {
  templateId ||= await live.cmd(`addtemplatedoc ${JSON.stringify(TEMPLATE)}`);
  await live.cmd("liveedit onTake");
  await live.cmd(`midiprefs {"inputs":{},"feedback":{"TestPad":{"output":null,"profile":"generic"}}}`);
});

test.afterEach(async ({ live }) => {
  await live.cmd(`savecontrols {"mappings":[]}`);
  await live.cmd(`midiprefs {"inputs":{},"feedback":{}}`);
  await live.cmd("midiout");
  await live.cmd("midievents");
});

test("a mapped note toggles a layer and its LED shows on air", async ({ page, live }) => {
  const screen = await makeScreen(live);
  await openScreen(page, live.base, screen);
  await live.cmd("midiout");
  await live.cmd(`savecontrols ${JSON.stringify({ mappings: [note("m1", 36, `live.toggle:${screen}/l1`, true)] })}`);
  // The full state once the mapping exists: ready (green, velocity 1 in the generic profile).
  expect(await sent(live)).toEqual([[0x90, 36, 1]]);

  await live.cmd("midiin TestPad 90 24 7f");
  await expect(root(page)).toHaveAttribute("data-state", "in");
  expect(await sent(live)).toEqual([[0x90, 36, 127]]);
  expect(JSON.parse(await live.cmd(`layerview ${screen} l1`)).onAir).toBe(true);

  // Note-off does nothing; the next note-on takes it off air and the LED goes back to green.
  await live.cmd("midiin TestPad 80 24 00");
  expect(await sent(live)).toEqual([]);
  await live.cmd("midiin TestPad 90 24 40");
  await expect(root(page)).toHaveCount(0);
  expect(await sent(live)).toEqual([[0x90, 36, 1]]);
});

test("two mappings on one pad run nothing and report a conflict", async ({ page, live }) => {
  const screen = await makeScreen(live);
  await openScreen(page, live.base, screen);
  const any: ControlMapping = { ...note("b", 40, "live.clearAll"), source: { device: null, channel: null, kind: "note", number: 40 } };
  await live.cmd(`savecontrols ${JSON.stringify({ mappings: [note("a", 40, `live.take:${screen}/l1`), any] })}`);
  await live.cmd("midievents");

  await live.cmd("midiin TestPad 90 28 7f");
  await page.waitForTimeout(150);
  await expect(root(page)).toHaveCount(0);
  expect(JSON.parse(await live.cmd(`layerview ${screen} l1`)).onAir).toBe(false);
  const events = JSON.parse(await live.cmd("midievents")) as { event: string; payload: { mappingIds: string[] } }[];
  expect(events.filter((e) => e.event === "control-conflict").map((e) => e.payload.mappingIds)).toEqual([["a", "b"]]);
});

test("a CC fader drives a layer's opacity", async ({ page, live }) => {
  await live.cmd("liveedit immediate");
  const screen = await makeScreen(live);
  await openScreen(page, live.base, screen);
  await live.cmd(`layershow ${screen} l1`);
  await expect(root(page)).toHaveAttribute("data-state", "in");
  const fader: ControlMapping = {
    id: "f1",
    source: { device: null, channel: null, kind: "cc", number: 7 },
    target: { kind: "value", screenId: screen, layerId: "l1", target: { kind: "opacity" } },
    feedback: false,
  };
  await live.cmd(`savecontrols ${JSON.stringify({ mappings: [fader] })}`);

  const inner = () =>
    page.evaluate(() => {
      const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot)!;
      return getComputedStyle(host.shadowRoot!.querySelector("style + div") as HTMLElement).opacity;
    });
  await live.cmd("midiin TestPad b0 07 40"); // 64 / 127 → 50 %
  await expect.poll(inner).toBe("0.5");
  expect(JSON.parse(await live.cmd(`layerview ${screen} l1`)).opacity).toBe(50);
  await page.waitForTimeout(50);
  await live.cmd("midiin TestPad b0 07 7f");
  await expect.poll(inner).toBe("1");
  await expect(root(page)).toHaveAttribute("data-state", "in");
});

test("trigger.midi fires a flow", async ({ page, live }) => {
  const screen = await makeScreen(live);
  await openScreen(page, live.base, screen);
  const flow = await live.cmd(
    `saveflow ${JSON.stringify({
      name: "Pad 5",
      armed: true,
      nodes: [
        { id: "m", type: "trigger.midi", config: { device: "TestPad", channel: null, kind: "note", number: 5 } },
        { id: "t", type: "action.take", config: { screenId: screen, layerId: "l1" } },
      ],
      edges: [{ id: "e1", from: { node: "m", port: "fired" }, to: { node: "t", port: "in" }, kind: "event" }],
    })}`,
  );
  try {
    // Another note, another device: nothing.
    await live.cmd("midiin TestPad 90 06 7f");
    await live.cmd("midiin Other 90 05 7f");
    await page.waitForTimeout(150);
    await expect(root(page)).toHaveCount(0);
    await live.cmd("midiin TestPad 99 05 7f");
    await expect(root(page)).toHaveAttribute("data-state", "in");
  } finally {
    await live.cmd(`deleteflow ${flow}`);
  }
});
