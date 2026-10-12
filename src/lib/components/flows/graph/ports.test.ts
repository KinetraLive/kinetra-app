// ports.ts must agree with the Rust port table (core/flow_nodes.rs). ports.fixture.json is written
// by `cargo test ports_fixture` (UPDATE_PORTS_FIXTURE=1 regenerates it): one sample config per
// node type with its ports, and the data-type compatibility table.

import { describe, expect, it } from "vitest";
import type { DataType, FlowEdge, FlowNode, NodeKind } from "$lib/types";
import { NODES } from "./nodes";
import fixture from "./ports.fixture.json";
import { accepts, checkConnection, compatible, outputOf, portsOf, portsWithColumns, type Port } from "./ports";

type FxPort = { port: string; kind: "event" | "data"; dataType?: DataType; accepts?: DataType[] };
type FxNode = { type: string; config: unknown; inputs: FxPort[]; outputs: FxPort[]; columnOutputs: boolean };

const TYPES: DataType[] = ["text", "number", "bool", "time", "row", "rows"];
const fx = fixture as unknown as { compatible: { from: DataType; to: DataType; ok: boolean }[]; nodes: FxNode[] };

describe("port table parity with Rust (ports.fixture.json)", () => {
  it("covers every node type of the catalogue", () => {
    expect(new Set(fx.nodes.map((n) => n.type))).toEqual(new Set(NODES.map((n) => n.type)));
  });

  for (const n of fx.nodes) {
    it(`${n.type}: same ports, kinds, types and accepted types`, () => {
      const kind = { type: n.type, config: n.config } as NodeKind;
      const p = portsOf(kind);
      const shape = (ports: Port[], input: boolean) =>
        ports.map((x) => ({
          port: x.name,
          kind: x.kind,
          ...(x.type ? { dataType: x.type } : {}),
          ...(input && x.kind === "data" ? { accepts: TYPES.filter((t) => accepts(kind, x, t)) } : {}),
        }));
      expect(shape(p.inputs, true)).toEqual(n.inputs.map((x) => ({ ...x, ...(x.accepts ? { accepts: [...x.accepts] } : {}) })));
      expect(shape(p.outputs, false)).toEqual(n.outputs);
      expect(p.columnOutputs).toBe(n.columnOutputs);
    });
  }

  it("the implicit-conversion table", () => {
    expect(fx.compatible).toHaveLength(36);
    for (const c of fx.compatible) expect(compatible(c.from, c.to), `${c.from} → ${c.to}`).toBe(c.ok);
  });
});

// ---- Connection rules ------------------------------------------------------------------------------

const node = (id: string, k: NodeKind): FlowNode => ({ ...k, id, position: { x: 0, y: 0 } }) as FlowNode;
const cue = node("cue", { type: "trigger.cue", config: { label: null } });
const take = node("take", { type: "action.take", config: { screenId: "s", layerId: "l", forMs: null } });
const branch = node("if", { type: "logic.branch", config: {} });
const now = node("now", { type: "data.scheduleItem", config: { sourceId: "sch", which: "now" } });
const math = node("math", { type: "data.math", config: { op: "add", a: 0, b: 0 } });
const sched = node("sched", { type: "action.schedule", config: { sourceId: "sch", item: { pick: "input" }, op: { op: "end" } } });
const delay = node("delay", { type: "logic.delay", config: { ms: 100 } });
const rows = node("rows", { type: "data.sourceRow", config: { sourceId: "src", row: { pick: "current" } } });
const set = node("set", { type: "action.setFields", config: { screenId: "s", layerId: "l", fields: { items: "" } } });
const graph = (edges: FlowEdge[] = []) => ({ nodes: [cue, take, branch, now, math, sched, delay, rows, set], edges });
const r = (n: string, p: string) => ({ node: n, port: p });

describe("checkConnection", () => {
  it("allows an event output into an event input and a typed value into a compatible input", () => {
    expect(checkConnection(graph(), r("cue", "fired"), r("take", "in"))).toEqual({ ok: true, kind: "event" });
    expect(checkConnection(graph(), r("math", "number"), r("cue", "value"))).toMatchObject({ ok: false });
    expect(checkConnection(graph(), r("now", "exists"), r("if", "condition"))).toEqual({ ok: true, kind: "data" });
    // number → text converts implicitly; rows into a Set fields input (list fields).
    expect(checkConnection(graph(), r("math", "number"), r("set", "items"))).toEqual({ ok: true, kind: "data" });
    expect(checkConnection(graph(), r("rows", "rows"), r("set", "items"))).toEqual({ ok: true, kind: "data" });
  });

  it("refuses with the reason, in the server's words", () => {
    expect(checkConnection(graph(), r("math", "number"), r("sched", "item"))).toEqual({
      ok: false,
      reason: "number can't go into a row input.",
    });
    expect(checkConnection(graph(), r("now", "title"), r("if", "condition"))).toEqual({
      ok: false,
      reason: "text can't go into a bool input.",
    });
    expect(checkConnection(graph(), r("cue", "fired"), r("if", "condition"))).toEqual({
      ok: false,
      reason: "An event can't go into a data input.",
    });
    expect(checkConnection(graph(), r("now", "exists"), r("take", "in"))).toEqual({
      ok: false,
      reason: "A value can't go into an event input.",
    });
    expect(checkConnection(graph(), r("cue", "nope"), r("take", "in"))).toMatchObject({ ok: false, reason: 'There is no output "nope".' });
    expect(checkConnection(graph(), r("cue", "fired"), r("ghost", "in"))).toMatchObject({ ok: false });
  });

  it("one wire per data input; no duplicate wires; event inputs take many", () => {
    const fed: FlowEdge = { id: "w", from: r("now", "exists"), to: r("if", "condition"), kind: "data" };
    expect(checkConnection(graph([fed]), r("now", "exists"), r("if", "condition"))).toMatchObject({
      ok: false,
      reason: "These ports are already connected.",
    });
    const other = node("other", { type: "data.layerState", config: { screenId: "s", layerId: "l" } });
    const g = { nodes: [...graph().nodes, other], edges: [fed] };
    expect(checkConnection(g, r("other", "onAir"), r("if", "condition"))).toEqual({ ok: false, reason: "This input already has a wire." });
    const ev: FlowEdge = { id: "e", from: r("cue", "fired"), to: r("take", "in"), kind: "event" };
    expect(checkConnection(graph([ev]), r("if", "then"), r("take", "in"))).toEqual({ ok: true, kind: "event" });
  });

  it("a node runs itself again only through a waiting node; never feeds itself a value", () => {
    expect(checkConnection(graph(), r("take", "done"), r("take", "in"))).toMatchObject({ ok: false });
    expect(checkConnection(graph(), r("delay", "out"), r("delay", "in"))).toEqual({ ok: true, kind: "event" });
    expect(checkConnection(graph(), r("rows", "index"), r("rows", "index"))).toMatchObject({ ok: false });
  });
});

describe("dynamic ports", () => {
  it("source row columns are text outputs; the editor lists the source's columns", () => {
    const k = rows as NodeKind;
    expect(outputOf(k, "name")).toEqual({ name: "name", kind: "data", type: "text" });
    expect(outputOf(k, "Not A Key")).toBeUndefined();
    expect(portsWithColumns(k, ["name", "title", "row"]).outputs.map((p) => p.name)).toEqual(["row", "rows", "index", "name", "title"]);
  });

  it("switch cases, sequence and random outputs, set fields inputs sorted by key", () => {
    expect(portsOf({ type: "logic.switch", config: { cases: ["a", "b"] } }).outputs.map((p) => p.name)).toEqual(["case0", "case1", "default"]);
    expect(portsOf({ type: "logic.sequence", config: { count: 3 } }).outputs.map((p) => p.name)).toEqual(["s0", "s1", "s2"]);
    expect(portsOf({ type: "logic.random", config: { count: 2, noRepeat: false } }).outputs.map((p) => p.name)).toEqual(["r0", "r1"]);
    const sf = portsOf({ type: "action.setFields", config: { screenId: "s", layerId: "l", fields: { title: "", name: "" } } });
    expect(sf.inputs.map((p) => p.name)).toEqual(["in", "name", "title"]);
  });
});
