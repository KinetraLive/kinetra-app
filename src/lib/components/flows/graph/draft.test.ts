import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FLOW_NODES_MAX, type FlowEdge, type FlowNode, type NodeKind } from "$lib/types";
import { clockOf, newNode, nodeSummary, portLabel } from "./catalog";
import { CATEGORIES, NODES, catalogInfo } from "./nodes";
import { DebugState, FLASH_MS, REDUCED_FLASH_MS, debugSeverity } from "./debug.svelte";
import {
  History,
  HISTORY_MAX,
  addNodes,
  canAdd,
  connect,
  graphKey,
  moveNodes,
  nudge,
  pruneEdges,
  removeElements,
  setNodeKind,
  stableStringify,
  type Graph,
} from "./draft";
import { paletteGroups } from "./palette";
import { categoryOf, checkConnection, portsOf } from "./ports";
import type { Lookups } from "../linear";

const node = (id: string, k: NodeKind, x = 0, y = 0): FlowNode => ({ ...k, id, position: { x, y } }) as FlowNode;
const cue = node("cue", { type: "trigger.cue", config: { label: null } });
const take = node("take", { type: "action.take", config: { screenId: "s", layerId: "l", forMs: null } }, 240);
const sw = node("sw", { type: "logic.switch", config: { cases: ["a", "b"] } }, 480);
const e1: FlowEdge = { id: "e1", from: { node: "cue", port: "fired" }, to: { node: "take", port: "in" }, kind: "event" };
const e2: FlowEdge = { id: "e2", from: { node: "sw", port: "case1" }, to: { node: "take", port: "in" }, kind: "event" };
const base: Graph = { nodes: [cue, take, sw], edges: [e1, e2] };

describe("graph operations", () => {
  it("add, remove (with its wires), unchanged → same graph", () => {
    const n = node("log", { type: "action.log", config: { message: "x", toast: false } });
    const g = addNodes(base, [n]);
    expect(g.nodes.map((x) => x.id)).toEqual(["cue", "take", "sw", "log"]);
    expect(g.nodes[0]).toBe(cue);
    expect(addNodes(base, [])).toBe(base);
    const r = removeElements(base, ["take"]);
    expect(r.nodes.map((x) => x.id)).toEqual(["cue", "sw"]);
    expect(r.edges).toEqual([]);
    expect(removeElements(base, [], ["e2"]).edges).toEqual([e1]);
    expect(removeElements(base, ["ghost"])).toBe(base);
  });

  it("moves round to whole pixels and keep unmoved nodes", () => {
    const g = moveNodes(base, new Map([["take", { x: 300.4, y: 10.6 }]]));
    expect(g.nodes[1].position).toEqual({ x: 300, y: 11 });
    expect(g.nodes[0]).toBe(cue);
    expect(moveNodes(base, new Map([["take", { x: 240, y: 0 }]]))).toBe(base);
    expect(nudge(base, ["cue", "sw"], 8, -80).nodes.map((n) => n.position)).toEqual([
      { x: 8, y: -80 },
      { x: 240, y: 0 },
      { x: 488, y: -80 },
    ]);
  });

  it("connect checks the ports and gives the wire an id and kind", () => {
    const r = connect(base, { node: "sw", port: "default" }, { node: "take", port: "in" }, () => "new1");
    expect(r).toMatchObject({ ok: true, edge: { id: "new1", kind: "event" } });
    if (r.ok) expect(r.graph.edges).toHaveLength(3);
    expect(connect(base, { node: "cue", port: "fired" }, { node: "take", port: "in" })).toEqual({
      ok: false,
      reason: "These ports are already connected.",
    });
  });

  it("a config change that removes a port drops its wires", () => {
    const g = setNodeKind(base, "sw", { type: "logic.switch", config: { cases: ["a"] } });
    expect(g.edges).toEqual([e1]);
    expect(g.nodes[2].position).toEqual({ x: 480, y: 0 });
    // The same config (any key order) is no change.
    expect(setNodeKind(base, "take", { type: "action.take", config: { forMs: null, layerId: "l", screenId: "s" } })).toBe(base);
    expect(pruneEdges(base)).toBe(base);
  });

  it("node limit", () => {
    const many = { nodes: Array.from({ length: FLOW_NODES_MAX }, (_, i) => node(`n${i}`, cue)), edges: [] };
    expect(canAdd(many)).toBe(false);
    expect(canAdd(base)).toBe(true);
  });

  it("stable keys ignore key order and undefined", () => {
    expect(stableStringify({ b: 1, a: [{ d: 2, c: undefined }] })).toBe('{"a":[{"d":2}],"b":1}');
    expect(graphKey(base)).toBe(graphKey(structuredClone(base)));
  });
});

describe("History", () => {
  const g = (n: number): Graph => ({ nodes: [node(`n${n}`, cue)], edges: [] });

  it("undo / redo; a new edit drops the redo branch", () => {
    const h = new History(g(0));
    expect(h.push(g(1))).toBe(true);
    h.push(g(2));
    expect(h.canUndo).toBe(true);
    expect(h.undo()?.nodes[0].id).toBe("n1");
    expect(h.undo()?.nodes[0].id).toBe("n0");
    expect(h.undo()).toBeNull();
    expect(h.redo()?.nodes[0].id).toBe("n1");
    h.push(g(5));
    expect(h.canRedo).toBe(false);
    expect(h.current.nodes[0].id).toBe("n5");
  });

  it("pushing the current graph records nothing", () => {
    const first = g(0);
    const h = new History(first);
    expect(h.push(first)).toBe(false);
    expect(h.canUndo).toBe(false);
  });

  it("edits of one group are one step", () => {
    const h = new History(g(0));
    h.push(g(1), "take:message");
    h.push(g(2), "take:message");
    h.push(g(3), "take:message");
    expect(h.undo()?.nodes[0].id).toBe("n0");
    h.redo();
    h.push(g(4), "other");
    expect(h.undo()?.nodes[0].id).toBe("n3");
  });

  it("is capped and resets", () => {
    const h = new History(g(0), 3);
    for (let i = 1; i <= 5; i++) h.push(g(i));
    let n = 0;
    while (h.undo()) n++;
    expect(n).toBe(3);
    expect(HISTORY_MAX).toBe(100);
    h.reset(g(9));
    expect(h.canUndo || h.canRedo).toBe(false);
    expect(h.current.nodes[0].id).toBe("n9");
  });
});

describe("catalogue and palette", () => {
  const d = { screenId: "s", layerId: "l", scheduleId: "sch", textId: "src" };
  const k: Lookups = { screens: new Map(), sources: new Map(), flows: new Map() };

  it("every node type has a label, description and a new-node config with ports", () => {
    expect(NODES).toHaveLength(50);
    for (const info of NODES) {
      const n = newNode(info.type, d, { x: 10, y: 20 });
      expect(n.type).toBe(info.type);
      expect(n.id).toMatch(/^[A-Za-z0-9_-]{10}$/);
      expect(n.position).toEqual({ x: 10, y: 20 });
      expect(catalogInfo(info.type)?.label).toBe(info.label);
      const p = portsOf(n);
      expect(p.inputs.length + p.outputs.length).toBeGreaterThan(0);
      expect(typeof nodeSummary(n, k)).toBe("string");
    }
    expect(newNode("action.take", d, { x: 0, y: 0 }).config).toEqual({ screenId: "s", layerId: "l", forMs: null });
    expect(newNode("data.scheduleItem", d, { x: 0, y: 0 }).config).toEqual({ sourceId: "sch", which: "now" });
  });

  it("Take preview: an action on a screen (in; done, failed), the project's transition by default", () => {
    expect(catalogInfo("action.takePreview")).toEqual({
      type: "action.takePreview",
      label: "Take preview",
      description: "Takes a screen's Preview to Program (Studio mode).",
    });
    const n = newNode("action.takePreview", d, { x: 0, y: 0 });
    expect(n.config).toEqual({ screenId: "s", transition: null });
    const p = portsOf(n);
    expect(p.inputs.map((x) => [x.name, x.kind])).toEqual([["in", "event"]]);
    expect(p.outputs.map((x) => x.name)).toEqual(["done", "failed"]);
    expect(nodeSummary(n, k)).toBe("Take the Preview of a missing screen");
    expect(paletteGroups("preview").flatMap((g) => g.items.map((i) => i.label))).toContain("Take preview");
  });

  it("Set variant: an action that shows another look of a layer, aimed at the defaults", () => {
    const info = catalogInfo("action.setVariant");
    expect(info).toEqual({ type: "action.setVariant", label: "Set variant", description: "Shows another look of a layer." });
    expect(NODES.filter((x) => x.type.startsWith("action.")).map((x) => x.type)).toContain("action.setVariant");
    const n = newNode("action.setVariant", d, { x: 0, y: 0 });
    expect(n.config).toEqual({ screenId: "s", layerId: "l", variant: "", transition: "crossfade" });
    const p = portsOf(n);
    expect(p.inputs.map((x) => [x.name, x.kind])).toEqual([["in", "event"]]);
    expect(p.outputs.map((x) => [x.name, x.kind])).toEqual([
      ["done", "event"],
      ["failed", "event"],
    ]);
    expect(nodeSummary(n, k)).toBe("Show a variant on a missing layer");
    expect(paletteGroups("look").flatMap((g) => g.items.map((i) => i.label))).toContain("Set variant");
  });

  it("summaries and port labels", () => {
    expect(nodeSummary({ type: "data.compare", config: { op: "ge", a: "", b: "" } }, k)).toBe("a ≥ b");
    expect(nodeSummary({ type: "logic.delay", config: { ms: 4000 } }, k)).toBe("Wait 4 s");
    expect(nodeSummary({ type: "action.take", config: { screenId: "x", layerId: "y", forMs: null } }, k)).toBe(
      "Take a missing layer",
    );
    expect(nodeSummary({ type: "trigger.cue", config: { label: "Go" } }, k)).toBe("Button “Go”");
    expect(portLabel({ type: "logic.switch", config: { cases: ["Live", ""] } }, "case1")).toBe("(empty)");
    expect(portLabel({ type: "logic.sequence", config: { count: 2 } }, "s0")).toBe("1");
    expect(clockOf("2026-10-04T14:02:31")).toBe("14:02:31");
  });

  it("Repeat: a Logic node looping its body (3 rounds by default), ports in/stop → body/done + round", () => {
    const info = catalogInfo("logic.repeat")!;
    expect(info).toEqual({ type: "logic.repeat", label: "Repeat", description: "Runs its body again and again." });
    expect(paletteGroups("repeat").map((g) => [g.id, g.items.map((i) => i.label)])).toEqual([["logic", ["Repeat"]]]);
    const n = newNode("logic.repeat", d, { x: 0, y: 0 });
    expect(n.config).toEqual({ count: 3, delayMs: 0 });
    const p = portsOf(n);
    expect(p.inputs.map((x) => x.name)).toEqual(["in", "stop"]);
    expect(p.outputs.map((x) => [x.name, x.kind, x.type])).toEqual([
      ["body", "event", undefined],
      ["done", "event", undefined],
      ["round", "data", "number"],
    ]);
    expect(nodeSummary(n, k)).toBe("3 times");
    expect(nodeSummary({ type: "logic.repeat", config: { count: null, delayMs: 2000 } }, k)).toBe("Forever, 2 s apart");
    expect(nodeSummary({ type: "logic.repeat", config: { count: 1, delayMs: 0 } }, k)).toBe("Once");
    // Wired back into its own in: refused (it already loops); into its own stop: fine.
    const rep = { ...n, id: "rep" };
    const g = { nodes: [rep], edges: [] };
    expect(checkConnection(g, { node: "rep", port: "body" }, { node: "rep", port: "in" })).toEqual({
      ok: false,
      reason: "Repeat already loops; wire to stop instead.",
    });
    expect(checkConnection(g, { node: "rep", port: "body" }, { node: "rep", port: "stop" })).toEqual({ ok: true, kind: "event" });
  });

  it("Stop flow: an action with a flow to stop (none chosen yet)", () => {
    expect(catalogInfo("action.stopFlow")).toEqual({
      type: "action.stopFlow",
      label: "Stop flow",
      description: "Stops a running flow (this one too).",
    });
    const n = newNode("action.stopFlow", d, { x: 0, y: 0 });
    expect(n.config).toEqual({ flowId: "" });
    expect(portsOf(n).inputs.map((x) => x.name)).toEqual(["in"]);
    expect(portsOf(n).outputs.map((x) => x.name)).toEqual(["done", "failed"]);
    expect(nodeSummary({ type: "action.stopFlow", config: { flowId: "me" } }, { ...k, self: "me" })).toBe("Stop this flow");
    expect(paletteGroups("stop").flatMap((g) => g.items.map((i) => i.type))).toContain("action.stopFlow");
  });

  it("palette: four groups in order; search over label, type and description", () => {
    expect(paletteGroups("").map((g) => g.label)).toEqual(CATEGORIES.map((c) => c.label));
    const t = paletteGroups("time");
    expect(t.flatMap((g) => g.items.map((i) => i.label))).toEqual(
      expect.arrayContaining(["Time of day", "Time now", "Wait until", "Set timer"]),
    );
    expect(paletteGroups("clear").map((g) => g.id)).toEqual(["action"]);
    expect(paletteGroups("zzz")).toEqual([]);
    for (const g of paletteGroups("")) for (const i of g.items) expect(categoryOf(i.type)).toBe(g.id);
  });
});

describe("live debug flashes", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  const batch = (nodes: string[], edges: string[] = [], errors: { node: string; message: string }[] = []) => ({
    id: "f",
    nodes: nodes.map((node) => ({ node, at: "2026-10-04T12:00:00Z" })),
    edges,
    errors,
  });

  it("flashes nodes and wires for FLASH_MS, then nothing runs", () => {
    const d = new DebugState();
    expect(d.pending).toBe(false);
    d.apply(batch(["a", "b"], ["w1"]));
    expect([...d.nodes]).toEqual(["a", "b"]);
    expect([...d.edges]).toEqual(["w1"]);
    expect(d.fired.get("a")).toBe("2026-10-04T12:00:00Z");
    vi.advanceTimersByTime(FLASH_MS - 1);
    expect(d.nodes.size).toBe(2);
    vi.advanceTimersByTime(1);
    expect(d.nodes.size).toBe(0);
    expect(d.edges.size).toBe(0);
    expect(d.pending).toBe(false);
    // The last-fired time stays.
    expect(d.fired.get("b")).toBe("2026-10-04T12:00:00Z");
  });

  it("a node firing again extends its flash; others expire on time", () => {
    const d = new DebugState();
    d.apply(batch(["a", "b"]));
    vi.advanceTimersByTime(200);
    d.apply(batch(["a"]));
    vi.advanceTimersByTime(FLASH_MS - 200);
    expect([...d.nodes]).toEqual(["a"]);
    vi.advanceTimersByTime(200);
    expect(d.nodes.size).toBe(0);
  });

  it("reduced motion holds the highlight for 1 s", () => {
    const d = new DebugState(() => true);
    d.apply(batch(["a"]));
    vi.advanceTimersByTime(REDUCED_FLASH_MS - 1);
    expect(d.nodes.has("a")).toBe(true);
    vi.advanceTimersByTime(1);
    expect(d.nodes.has("a")).toBe(false);
  });

  it("a paced Repeat is a warning, not an error: kept while the node runs again, gone on a new run", () => {
    const paced = "Looping faster than every 100 ms — paced.";
    expect(debugSeverity(paced)).toBe("warning");
    expect(debugSeverity("Skipped: the layer is gone")).toBe("error");
    const d = new DebugState();
    d.apply(batch(["rep"], [], [{ node: "rep", message: paced }]));
    expect(d.warnings.get("rep")).toBe(paced);
    expect(d.errors.size).toBe(0);
    d.apply(batch(["rep"]));
    expect(d.warnings.get("rep")).toBe(paced);
    d.newRun();
    expect(d.warnings.size).toBe(0);
    d.dispose();
  });

  it("run-time errors show until the node runs cleanly; clearFlashes keeps them", () => {
    const d = new DebugState();
    d.apply(batch(["t"], [], [{ node: "t", message: "Skipped: the layer is gone" }]));
    expect(d.errors.get("t")).toBe("Skipped: the layer is gone");
    d.clearFlashes();
    expect(d.nodes.size).toBe(0);
    expect(d.pending).toBe(false);
    expect(d.errors.size).toBe(1);
    d.apply(batch(["t"]));
    expect(d.errors.size).toBe(0);
    d.dispose();
    expect(d.pending).toBe(false);
  });
});
