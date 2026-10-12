// Shared by the flow specs (flows, flow-loops, migration, flow-editor): a screen with two classic bars, opening
// a renderer page, local date-times, and flow graph builders.

import type { Page } from "@playwright/test";
import type { Fixture } from "./fixture";
import type { FlowEdge, FlowNode, NewFlow, ScreenDoc, ScreenLayer } from "../src/lib/types";

const FULL = { x: 0, y: 0, w: 1920, h: 1080 };
export const roots = (page: Page) => page.locator("[data-state]");

/** Local date-time `YYYY-MM-DDTHH:MM` of an epoch ms (the fixture uses this machine's zone). */
export function local(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

const presets = new Map<Fixture, Record<string, string>>();
async function preset(live: Fixture, id: string) {
  const ids = presets.get(live) ?? {};
  presets.set(live, ids);
  return (ids[id] ||= await live.cmd(`addtemplate ${id}`));
}

const layerOf = (id: string, templateId: string, name: string): ScreenLayer => ({
  id,
  name,
  kind: "template",
  templateId,
  rect: FULL,
  fields: { name, title: "Flows" },
  vars: {},
});

let seq = 0;
/** A screen with two classic bars: `a` reads "Layer A", `b` "Layer B". */
export async function twoLayers(live: Fixture, id = `m8${Date.now().toString(36)}${seq++}`): Promise<ScreenDoc> {
  const t = await preset(live, "classic-bar");
  const doc: ScreenDoc = {
    id,
    name: "M8",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [layerOf("a", t, "Layer A"), layerOf("b", t, "Layer B")],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc;
}

export async function openScreen(page: Page, base: string, id: string) {
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${id}`);
  await (await ws).waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
}

// ---- Graph helpers ------------------------------------------------------------------------------

let x = 0;
export const node = (id: string, kind: Pick<FlowNode, "type" | "config">): FlowNode =>
  ({ id, ...kind, position: { x: (x += 240), y: 0 } }) as FlowNode;
/** An event wire `e-<from>-<fromPort>-<to>`. */
export const ev = (from: string, fromPort: string, to: string, toPort = "in"): FlowEdge => ({
  id: `e-${from}-${fromPort}-${to}`,
  from: { node: from, port: fromPort },
  to: { node: to, port: toPort },
  kind: "event",
});
export const data = (from: string, fromPort: string, to: string, toPort: string): FlowEdge => ({
  ...ev(from, fromPort, to, toPort),
  kind: "data",
});
export const cue = (id = "cue") => node(id, { type: "trigger.cue", config: { label: null } });
export const take = (id: string, screenId: string, layerId: string) =>
  node(id, { type: "action.take", config: { screenId, layerId, forMs: null } });
export const delay = (id: string, ms: number) => node(id, { type: "logic.delay", config: { ms } });
export const text = (id: string, value: string) =>
  node(id, { type: "data.constant", config: { value: { kind: "text", value } } });

export const clear = (id: string, screenId: string, layerId: string) =>
  node(id, { type: "action.clear", config: { screenId, layerId } });
/** A Repeat loop: `count` rounds (`null` = forever), `delayMs` between rounds. */
export const repeat = (id: string, count: number | null, delayMs: number) =>
  node(id, { type: "logic.repeat", config: { count, delayMs } });
export const stopFlow = (id: string, flowId: string) => node(id, { type: "action.stopFlow", config: { flowId } });

/**
 * Counts takes in a renderer page: every time a layer root's `data-state` turns `in`, keyed by
 * the root's text ("Layer A" → `A`). Call before `openScreen`; read with {@link takes}.
 */
export async function watchTakes(page: Page) {
  await page.addInitScript(() => {
    const counts: Record<string, number> = {};
    (window as unknown as { __takes: Record<string, number> }).__takes = counts;
    const bump = (el: Element) => {
      const k = /Layer ([A-Z])/.exec(el.textContent ?? "")?.[1] ?? "?";
      counts[k] = (counts[k] ?? 0) + 1;
    };
    const attach = Element.prototype.attachShadow;
    Element.prototype.attachShadow = function (init) {
      const shadow = attach.call(this, init);
      new MutationObserver((records) => {
        for (const r of records) {
          if (r.type === "attributes") {
            const el = r.target as Element;
            if (el.getAttribute("data-state") === "in" && r.oldValue !== "in") bump(el);
          } else {
            for (const n of r.addedNodes) if (n instanceof Element && n.getAttribute("data-state") === "in") bump(n);
          }
        }
      }).observe(shadow, {
        subtree: true,
        childList: true,
        attributes: true,
        attributeFilter: ["data-state"],
        attributeOldValue: true,
      });
      return shadow;
    };
  });
}
/** Takes counted by {@link watchTakes} for a layer letter. */
export const takes = (page: Page, letter: string) =>
  page.evaluate((k) => (window as unknown as { __takes: Record<string, number> }).__takes[k] ?? 0, letter);

export function flow(name: string, nodes: FlowNode[], edges: FlowEdge[], armed = true): NewFlow {
  return { name, armed, retrigger: "restart", nodes, edges };
}

/** Saves a new flow → its id. */
export const save = (live: Fixture, f: NewFlow) => live.cmd(`saveflow ${JSON.stringify(f)}`);
/** Replaces a flow's deployed graph. */
export const deploy = (live: Fixture, id: string, f: NewFlow) =>
  live.cmd(`updateflow ${JSON.stringify({ id, name: f.name, retrigger: f.retrigger, nodes: f.nodes, edges: f.edges })}`);
export const state = (live: Fixture, id: string) => live.cmd(`flowstate ${id}`);
