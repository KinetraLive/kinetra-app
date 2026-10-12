// OSC control against the real fixture server, with real UDP from Node (OSC bytes encoded by
// hand: padded strings + `,i` / `,f` / `,s` type tags). Covers a take by address showing the layer
// in the renderer, `/opacity 0.5`, `/kinetra/clear-all`, a flow with `trigger.osc` on a custom
// wildcard address, and on-air feedback packets arriving at a Node UDP socket. The accept rule for
// other computers (LAN access + allowlist) is unit-tested in Rust (`core::osc`, `server::osc`):
// sending from a non-loopback address would need a LAN bind (a firewall prompt).

import { createSocket, type Socket } from "node:dgram";
import type { Page } from "@playwright/test";
import { expect, isolatedTest as test, type Fixture } from "./fixture";
import type { NewFlow, OscSettings, ScreenDoc, ScreenLayer } from "../src/lib/types";

const TEMPLATE = {
  name: "OSC plate",
  html: '<div class="lt"><div class="plate">{{name}}</div></div>',
  css: `.lt{position:absolute;inset:0}
.plate{position:absolute;inset:0;background:#ffffff}
.lt[data-state="in"]{animation:in 200ms both}
.lt[data-state="out"]{animation:out 150ms forwards}
@keyframes in{from{opacity:0}}
@keyframes out{to{opacity:0}}`,
  fields: [{ key: "name", label: "Name", type: "text", default: "" }],
};

type Arg = number | string | { f: number };

function str(s: string): Buffer {
  const b = Buffer.from(`${s}\0`, "utf8");
  return Buffer.concat([b, Buffer.alloc((4 - (b.length % 4)) % 4)]);
}

/** One OSC message: integers as `i`, `{ f }` as `f`, strings as `s`. */
function osc(address: string, ...args: Arg[]): Buffer {
  let tags = ",";
  const parts: Buffer[] = [];
  for (const a of args) {
    if (typeof a === "string") {
      tags += "s";
      parts.push(str(a));
    } else {
      const b = Buffer.alloc(4);
      if (typeof a === "number") {
        tags += "i";
        b.writeInt32BE(a);
      } else {
        tags += "f";
        b.writeFloatBE(a.f);
      }
      parts.push(b);
    }
  }
  return Buffer.concat([str(address), str(tags), ...parts]);
}

/** Address and first int argument of a feedback message. */
function parse(b: Buffer): { address: string; value: number } {
  const end = b.indexOf(0);
  const address = b.toString("utf8", 0, end);
  let at = end + 1 + ((4 - ((end + 1) % 4)) % 4);
  const tagEnd = b.indexOf(0, at);
  const tags = b.toString("utf8", at, tagEnd);
  at = tagEnd + 1 + ((4 - ((tagEnd + 1 - at) % 4)) % 4);
  expect(tags).toBe(",i");
  return { address, value: b.readInt32BE(at) };
}

async function udp(): Promise<Socket> {
  const s = createSocket("udp4");
  await new Promise<void>((r) => s.bind(0, "127.0.0.1", () => r()));
  return s;
}

/** A UDP port free right now on loopback. */
async function freePort(): Promise<number> {
  const s = await udp();
  const port = s.address().port;
  await new Promise<void>((r) => s.close(() => r()));
  return port;
}

async function enable(own: Fixture, feedback: OscSettings["feedback"] = null): Promise<number> {
  const port = await freePort();
  const settings: OscSettings = { enabled: true, port, allow: [], feedback };
  await own.cmd(`setosc ${JSON.stringify(settings)}`);
  await expect
    .poll(async () => JSON.parse(await own.cmd("oscstatus")))
    .toEqual({ state: "listening", port, lan: false });
  return port;
}

async function send(port: number, packet: Buffer) {
  const s = await udp();
  await new Promise<void>((r, j) => s.send(packet, port, "127.0.0.1", (e) => (e ? j(e) : r())));
  s.close();
}

async function screen(own: Fixture): Promise<ScreenDoc> {
  const templateId = await own.cmd(`addtemplatedoc ${JSON.stringify(TEMPLATE)}`);
  const layer = (id: string): ScreenLayer => ({
    id,
    name: id,
    kind: "template",
    templateId,
    rect: { x: 0, y: 0, w: 1920, h: 1080 },
    fields: { name: id },
    vars: {},
  });
  const doc: ScreenDoc = {
    id: `osc${Date.now().toString(36)}`,
    name: "OSC",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [layer("l1"), layer("l2")],
    updatedAt: "",
  };
  await own.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc;
}

async function openScreen(page: Page, base: string, id: string) {
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${id}`);
  await (await ws).waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
}

/** The template root of a layer (by its `name` field text). */
const root = (page: Page, text: string) => page.locator(".lt", { hasText: text });

test("take, opacity and clear all by OSC address show in the renderer", async ({ page, own }) => {
  const doc = await screen(own);
  const port = await enable(own);
  // Opacity on air goes out at once (onTake would stage it).
  await own.cmd("liveedit immediate");
  await openScreen(page, own.base, doc.id);
  const l1 = `/kinetra/layer/${doc.id}/l1`;

  await send(port, osc(`${l1}/take`, 1));
  await expect(root(page, "l1")).toHaveAttribute("data-state", "in");
  // The button's release (0) does nothing.
  await send(port, osc(`${l1}/clear`, 0));
  await send(port, osc(`${l1}/opacity`, { f: 0.5 }));
  await expect.poll(async () => JSON.parse(await own.cmd(`layerview ${doc.id} l1`)).opacity).toBe(50);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot)!;
        return getComputedStyle(host.shadowRoot!.querySelector("style + div") as HTMLElement).opacity;
      }),
    )
    .toBe("0.5");
  await expect(root(page, "l1")).toHaveAttribute("data-state", "in");

  await send(port, osc(`/kinetra/layer/${doc.id}/l2/toggle`));
  await expect(root(page, "l2")).toHaveAttribute("data-state", "in");
  await send(port, osc("/kinetra/clear-all"));
  await expect(page.locator(".lt")).toHaveCount(0);
  expect(JSON.parse(await own.cmd(`layerview ${doc.id} l1`)).onAir).toBe(false);
});

test("a flow with an OSC trigger on its own address fires with the first argument", async ({ page, own }) => {
  const doc = await screen(own);
  const port = await enable(own);
  await openScreen(page, own.base, doc.id);
  const flow: NewFlow = {
    name: "OSC name",
    armed: true,
    retrigger: "restart",
    nodes: [
      { id: "o", type: "trigger.osc", config: { address: "/show/*/name" }, position: { x: 0, y: 0 } },
      { id: "f", type: "action.setFields", config: { screenId: doc.id, layerId: "l2", fields: { name: "" } }, position: { x: 240, y: 0 } },
      { id: "t", type: "action.take", config: { screenId: doc.id, layerId: "l2", forMs: null }, position: { x: 480, y: 0 } },
    ],
    edges: [
      { id: "e1", from: { node: "o", port: "fired" }, to: { node: "f", port: "in" }, kind: "event" },
      { id: "e2", from: { node: "o", port: "value" }, to: { node: "f", port: "name" }, kind: "data" },
      { id: "e3", from: { node: "f", port: "done" }, to: { node: "t", port: "in" }, kind: "event" },
    ],
  };
  await own.cmd(`saveflow ${JSON.stringify(flow)}`);
  // Not matching (one segment too many): nothing.
  await send(port, osc("/show/a/b/name", "Nope"));
  await send(port, osc("/show/main/name", "Grace Hopper"));
  await expect(root(page, "Grace Hopper")).toHaveAttribute("data-state", "in");
  await expect(page.getByText("Nope")).toHaveCount(0);
});

test("on-air feedback reaches a UDP socket: the full state, then each change", async ({ own }) => {
  const doc = await screen(own);
  const rx = await udp();
  const got: Array<{ address: string; value: number; at: number }> = [];
  rx.on("message", (m) => got.push({ ...parse(m), at: performance.now() }));
  const port = await enable(own, { host: "127.0.0.1", port: rx.address().port });
  const onair = (l: string) => `/kinetra/layer/${doc.id}/${l}/onair`;

  // The full state once (every layer of every screen; ours are off air).
  await expect.poll(() => got.filter((g) => g.address.startsWith(`/kinetra/layer/${doc.id}/`)).length).toBe(2);
  expect(got.find((g) => g.address === onair("l1"))?.value).toBe(0);
  expect(got.find((g) => g.address === onair("l2"))?.value).toBe(0);

  got.length = 0;
  const t0 = performance.now();
  await send(port, osc(`/kinetra/layer/${doc.id}/l1/take`));
  await expect.poll(() => got.map((g) => [g.address, g.value])).toEqual([[onair("l1"), 1]]);
  console.log(`OSC take → feedback at a Node socket: ${(got[0].at - t0).toFixed(1)} ms`);
  await own.cmd(`layerhide ${doc.id} l1`);
  await expect.poll(() => got.map((g) => [g.address, g.value])).toEqual([
    [onair("l1"), 1],
    [onair("l1"), 0],
  ]);
  rx.close();
});
