// The Control API (docs/CONTROL_API.md) against the real fixture server: a Node WebSocket client
// (undici's, which can send the Authorization header a browser can't) takes a layer and the
// renderer shows it; set_fields + take reach the renderer's text; the state's diffs follow; Studio
// mode cues into Preview and studio_take takes it; a flow's API call trigger runs over REST; a
// browser page is refused. The guard, limits and close codes are unit-tested in Rust
// (`server::control`).

import type { Page } from "@playwright/test";
import { expect, isolatedTest as test, type Fixture } from "./fixture";
import type { NewFlow, ScreenDoc, ScreenLayer } from "../src/lib/types";

const TEMPLATE = {
  name: "API plate",
  html: '<div class="lt"><div class="plate">{{name}}</div></div>',
  css: `.lt{position:absolute;inset:0}
.plate{position:absolute;inset:0;background:#ffffff}
.lt[data-state="in"]{animation:in 200ms both}
.lt[data-state="out"]{animation:out 150ms forwards}
@keyframes in{from{opacity:0}}
@keyframes out{to{opacity:0}}`,
  fields: [{ key: "name", label: "Name", type: "text", default: "" }],
};

type Msg = Record<string, unknown> & { t: string };

/** A Control API client: every message it got, in order, and a way to wait for one. */
class Client {
  readonly got: Msg[] = [];
  #waiters: Array<() => void> = [];
  closed: number | null = null;

  private constructor(readonly ws: WebSocket) {
    ws.addEventListener("message", (e) => {
      this.got.push(JSON.parse(String(e.data)) as Msg);
      for (const w of this.#waiters.splice(0)) w();
    });
    ws.addEventListener("close", (e) => {
      this.closed = e.code;
      for (const w of this.#waiters.splice(0)) w();
    });
  }

  static async connect(base: string, token: string): Promise<Client> {
    // undici's WebSocket takes headers (Node 22+); browsers can't send Authorization.
    const Ctor = WebSocket as unknown as new (url: string, init: object) => WebSocket;
    const ws = new Ctor(`${base.replace("http", "ws")}/api/control`, {
      protocols: ["kinetra-control-local.v1"],
      headers: { Authorization: `Bearer ${token}` },
    });
    await new Promise<void>((resolve, reject) => {
      ws.addEventListener("open", () => resolve(), { once: true });
      ws.addEventListener("error", () => reject(new Error("connect failed")), { once: true });
    });
    const c = new Client(ws);
    c.send({ t: "hello", protocol: 1, client: "e2e/1.0" });
    return c;
  }

  send(m: object) {
    this.ws.send(JSON.stringify(m));
  }

  /** The first message (from `from` on) matching `f`. */
  async next(f: (m: Msg) => boolean, from = 0, timeoutMs = 5000): Promise<Msg> {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      const hit = this.got.slice(from).find(f);
      if (hit) return hit;
      if (this.closed !== null) throw new Error(`closed ${this.closed}`);
      if (Date.now() > deadline) throw new Error(`no matching message in ${timeoutMs} ms`);
      await new Promise<void>((r) => {
        this.#waiters.push(r);
        setTimeout(r, 100);
      });
    }
  }

  /** Sends a command and returns its result. */
  async command(corr: string, action: string, args: object = {}): Promise<Msg> {
    const from = this.got.length;
    this.send({ t: "command", corr, action, args });
    return this.next((m) => m.t === "result" && m.corr === corr, from);
  }
}

async function enable(own: Fixture): Promise<string> {
  await own.cmd("controlapi on");
  return own.cmd("controlapitoken");
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
    id: `api${Date.now().toString(36)}`,
    name: "API",
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

const root = (page: Page, text: string) => page.locator(".lt", { hasText: text });

test("a client takes, edits and follows the state; the renderer shows it", async ({ page, own }) => {
  const doc = await screen(own);
  const token = await enable(own);
  await openScreen(page, own.base, doc.id);
  const c = await Client.connect(own.base, token);
  const welcome = await c.next((m) => m.t === "welcome");
  expect(welcome.protocol).toBe(1);
  expect(welcome.features).toEqual(["studio_take", "automation", "timer", "run_flow"]);
  const snap = await c.next((m) => m.t === "snapshot");
  const l1 = `${doc.id}.l1`;
  const layers = (snap.state as { layers: Record<string, { on_air: boolean; name: string }> }).layers;
  expect(layers[l1]).toMatchObject({ on_air: false, name: "l1" });

  // Take: the result, the renderer, and a diff naming the command as its cause (timed).
  const sent = performance.now();
  const from = c.got.length;
  expect(await c.command("t1", "take", { layer: l1 })).toEqual({ t: "result", corr: "t1", ok: true });
  const diff = await c.next((m) => m.t === "diff" && m.cause === "t1", from);
  const latency = performance.now() - sent;
  console.log(`take → on_air diff at the client: ${latency.toFixed(1)} ms`);
  expect(diff.ops).toContainEqual({ op: "put", path: `/layers/${l1}/on_air`, value: true });
  await expect(root(page, "l1")).toHaveAttribute("data-state", "in");

  // Set fields and take another layer in one command.
  expect(
    await c.command("f1", "set_fields", { layer: `${doc.id}.l2`, values: { name: "Grace Hopper" }, take: true }),
  ).toMatchObject({ ok: true });
  await expect(root(page, "Grace Hopper")).toHaveAttribute("data-state", "in");

  // Refusals come back as results.
  expect(await c.command("x1", "take", { layer: `${doc.id}.nope` })).toMatchObject({ ok: false, code: "not_found" });

  // A change from elsewhere (OSC, the app) shows up as a diff too.
  const before = c.got.length;
  await own.cmd(`layerhide ${doc.id} l1`);
  await c.next((m) => m.t === "diff" && JSON.stringify(m.ops).includes(`"/layers/${l1}/on_air"`), before);
  await expect(root(page, "l1")).toHaveCount(0);
  c.ws.close();
});

test("Studio mode: take cues into Preview, studio_take puts it on air", async ({ page, own }) => {
  const doc = await screen(own);
  const token = await enable(own);
  await own.cmd(`studio {"enabled":true,"previewAfterTake":"keep","transition":{"kind":"cut","ms":300}}`);
  await openScreen(page, own.base, doc.id);
  const c = await Client.connect(own.base, token);
  await c.next((m) => m.t === "snapshot");
  const l1 = `${doc.id}.l1`;
  await c.command("c1", "take", { layer: l1 });
  await c.next((m) => m.t === "diff" && JSON.stringify(m.ops).includes(`"/preview/${doc.id}"`));
  expect(JSON.parse(await own.cmd(`layerview ${doc.id} l1`)).onAir).toBe(false);
  expect(await c.command("c2", "studio_take", { screen: doc.id, transition: "cut" })).toMatchObject({ ok: true });
  await expect(root(page, "l1")).toHaveAttribute("data-state", "in");
  c.ws.close();
});

test("a flow's API call trigger runs over REST with the value", async ({ page, own }) => {
  const doc = await screen(own);
  const token = await enable(own);
  await openScreen(page, own.base, doc.id);
  const flow: NewFlow = {
    name: "Goal",
    armed: true,
    retrigger: "restart",
    nodes: [
      { id: "r", type: "trigger.rest", config: {}, position: { x: 0, y: 0 } },
      { id: "f", type: "action.setFields", config: { screenId: doc.id, layerId: "l2", fields: { name: "" } }, position: { x: 240, y: 0 } },
      { id: "t", type: "action.take", config: { screenId: doc.id, layerId: "l2", forMs: null }, position: { x: 480, y: 0 } },
    ],
    edges: [
      { id: "e1", from: { node: "r", port: "fired" }, to: { node: "f", port: "in" }, kind: "event" },
      { id: "e2", from: { node: "r", port: "value" }, to: { node: "f", port: "name" }, kind: "data" },
      { id: "e3", from: { node: "f", port: "done" }, to: { node: "t", port: "in" }, kind: "event" },
    ],
  };
  const id = await own.cmd(`saveflow ${JSON.stringify(flow)}`);
  const res = await fetch(`${own.base}/api/flows/${id}/run`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ value: "Home 2–1" }),
  });
  expect(res.status).toBe(200);
  expect(await res.json()).toEqual({ ok: true });
  await expect(root(page, "Home 2–1")).toHaveAttribute("data-state", "in");
});

test("a web page can't use it, even on the same origin", async ({ page, own }) => {
  const doc = await screen(own);
  await enable(own);
  await openScreen(page, own.base, doc.id);
  // A browser WebSocket always sends Origin (refused before the upgrade) and can't send the token.
  const opened = await page.evaluate(
    (url) =>
      new Promise<boolean>((resolve) => {
        const ws = new WebSocket(url);
        ws.onopen = () => resolve(true);
        ws.onerror = () => resolve(false);
      }),
    `${own.base.replace("http", "ws")}/api/control`,
  );
  expect(opened).toBe(false);
  const status = await page.evaluate(
    (url) => fetch(url, { method: "POST" }).then((r) => r.status),
    `${own.base}/api/panic`,
  );
  expect(status).toBe(403);
  expect(JSON.parse(await own.cmd(`layerview ${doc.id} l1`)).onAir).toBe(false);
});
