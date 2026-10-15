// The flow port table, mirrored from src-tauri/src/core/flow_nodes.rs (`ports`, `compatible`,
// `accepts`). Each node type's event and data ports; the Rust validator and this module must agree,
// which ports.test.ts checks against ports.fixture.json (written by a Rust test).
//
// Repeat (logic.repeat): in, stop → body (each round), done; data out round (number, from 1).
// Stop flow (action.stopFlow): the action ports only (in → done, failed).
// API value changed (trigger.apiChanged): fired; data out value (text: the column's new value)
// and row (row: the current row).
//
// Dynamic ports: switch `case0`.. per case, sequence `s0`.., random `r0`.., one `setFields` data
// input per field key of its config, and `data.sourceRow`'s column outputs (any key-shaped name is
// a text column; the editor shows the source's columns).
//
// `checkConnection` is what the editor allows when wiring (drag or "Connect to…"): output → input of
// the same kind, compatible data types, one wire per data input, no duplicate wire. Its reasons are
// the server's wording ("number can't go into a row input").

import type { DataType, FlowEdge, FlowNode, NodeKind, PortRef, WireKind } from "$lib/types";

export interface Port {
  name: string;
  kind: WireKind;
  /** Data ports only. */
  type?: DataType;
}

export interface Ports {
  inputs: Port[];
  outputs: Port[];
  /** `data.sourceRow`: every column key of its source is also a text output. */
  columnOutputs: boolean;
}

export type Category = "trigger" | "data" | "logic" | "action";

export const categoryOf = (type: string): Category => {
  const c = type.slice(0, type.indexOf("."));
  return c === "trigger" || c === "data" || c === "logic" ? c : "action";
};

/** `"a"`..`"h"`: the format inputs. */
export const FORMAT_INPUTS = ["a", "b", "c", "d", "e", "f", "g", "h"] as const;
export const SCHEDULE_ITEM_TEXT = ["title", "subtitle", "location", "start", "end", "status", "label"] as const;

const ev = (name: string): Port => ({ name, kind: "event" });
const d = (name: string, type: DataType): Port => ({ name, kind: "data", type });

const KEY = /^[a-z][a-z0-9_-]{0,31}$/;
/** Nodes that wait before firing on: an event cycle must pass through one (Repeat waits for its body). */
const WAITS = new Set(["logic.delay", "logic.waitUntil", "logic.debounce", "logic.throttle", "logic.repeat"]);
export const isWait = (type: string): boolean => WAITS.has(type);
export const isKey = (s: string): boolean => KEY.test(s);

/** Every port of a node (`data.sourceRow` columns excepted: see `columnOutputs`). */
export function portsOf(node: NodeKind): Ports {
  const p: Ports = { inputs: [], outputs: [], columnOutputs: false };
  const cat = categoryOf(node.type);
  if (cat === "trigger") p.outputs.push(ev("fired"));
  else if (cat === "action") {
    p.inputs.push(ev("in"));
    p.outputs.push(ev("done"), ev("failed"));
  } else if (cat === "logic") p.inputs.push(ev("in"));
  switch (node.type) {
    case "trigger.cue":
    case "trigger.runByFlow":
    case "trigger.rest":
      p.outputs.push(d("value", "text"));
      break;
    case "trigger.itemStatus":
    case "trigger.beforeNext":
      p.outputs.push(d("item", "row"));
      break;
    case "trigger.timeOfDay":
      p.outputs.push(d("at", "time"));
      break;
    case "trigger.sourceRow":
      p.outputs.push(d("row", "row"), d("index", "number"));
      break;
    case "trigger.apiChanged":
      p.outputs.push(d("value", "text"), d("row", "row"));
      break;
    // MIDI in: the velocity / CC value (0–127). OSC in: the first argument as text.
    case "trigger.midi":
      p.outputs.push(d("value", "number"));
      break;
    case "trigger.osc":
      p.outputs.push(d("value", "text"));
      break;
    case "trigger.timerZero":
    case "trigger.layer":
    case "trigger.slideshowWrap":
    case "trigger.mediaEnded":
      break;
    case "data.sourceRow":
      p.inputs.push(d("index", "number"));
      p.outputs.push(d("row", "row"), d("rows", "rows"), d("index", "number"));
      p.columnOutputs = true;
      break;
    case "data.scheduleItem":
      for (const n of SCHEDULE_ITEM_TEXT) p.outputs.push(d(n, "text"));
      p.outputs.push(d("exists", "bool"), d("item", "row"));
      break;
    case "data.fieldValue":
      p.outputs.push(d("value", "text"));
      break;
    case "data.layerState":
      p.outputs.push(d("onAir", "bool"), d("pending", "bool"));
      break;
    case "data.constant":
      p.outputs.push(d("value", node.config.value.kind));
      break;
    case "data.timeNow":
      p.outputs.push(d("now", "time"));
      break;
    case "data.format":
      for (const n of FORMAT_INPUTS) p.inputs.push(d(n, "text"));
      p.outputs.push(d("text", "text"));
      break;
    case "data.math":
      p.inputs.push(d("a", "number"), d("b", "number"));
      p.outputs.push(d("number", "number"));
      break;
    case "data.compare":
      p.inputs.push(d("a", "text"), d("b", "text"));
      p.outputs.push(d("bool", "bool"));
      break;
    case "logic.branch":
      p.inputs.push(d("condition", "bool"));
      p.outputs.push(ev("then"), ev("else"));
      break;
    case "logic.switch":
      p.inputs.push(d("value", "text"));
      node.config.cases.forEach((_, i) => p.outputs.push(ev(`case${i}`)));
      p.outputs.push(ev("default"));
      break;
    case "logic.gate":
      p.inputs.push(ev("open"), ev("close"));
      p.outputs.push(ev("out"));
      break;
    case "logic.delay":
      p.inputs.push(ev("cancel"), d("ms", "number"));
      p.outputs.push(ev("out"));
      break;
    case "logic.waitUntil":
      p.inputs.push(ev("release"));
      p.outputs.push(ev("out"), ev("timeout"));
      break;
    case "logic.debounce":
    case "logic.throttle":
      p.outputs.push(ev("out"));
      break;
    case "logic.sequence":
      for (let i = 0; i < node.config.count; i++) p.outputs.push(ev(`s${i}`));
      break;
    case "logic.once":
      p.inputs.push(ev("reset"));
      p.outputs.push(ev("out"));
      break;
    case "logic.counter":
      p.inputs.push(ev("reset"));
      p.outputs.push(ev("out"), d("count", "number"));
      break;
    case "logic.random":
      for (let i = 0; i < node.config.count; i++) p.outputs.push(ev(`r${i}`));
      break;
    case "logic.repeat":
      p.inputs.push(ev("stop"));
      p.outputs.push(ev("body"), ev("done"), d("round", "number"));
      break;
    case "action.setFields":
      // serde_json maps are ordered by key on the Rust side (BTreeMap).
      for (const k of Object.keys(node.config.fields).sort()) p.inputs.push(d(k, "text"));
      break;
    case "action.setTimer":
    case "action.media":
      p.inputs.push(d("ms", "number"));
      break;
    case "action.stepSource":
      p.inputs.push(d("index", "number"));
      break;
    case "action.schedule":
      p.inputs.push(d("item", "row"));
      break;
    case "action.runFlow":
      p.inputs.push(d("value", "text"));
      break;
    case "action.log":
      p.inputs.push(d("text", "text"));
      break;
    default:
      break;
  }
  return p;
}

/**
 * The ports the editor shows: `portsOf`, plus on `data.sourceRow` one text output per column (the
 * source's columns, then any other column name a wire still uses).
 */
export function portsWithColumns(node: NodeKind, columns: readonly string[]): Ports {
  const p = portsOf(node);
  if (!p.columnOutputs) return p;
  for (const c of columns) if (isKey(c) && !p.outputs.some((o) => o.name === c)) p.outputs.push(d(c, "text"));
  return p;
}

/** An input port by name. */
export function inputOf(node: NodeKind, name: string): Port | undefined {
  return portsOf(node).inputs.find((p) => p.name === name);
}

/** An output port by name; on `data.sourceRow` any key-shaped name is a text column. */
export function outputOf(node: NodeKind, name: string): Port | undefined {
  const p = portsOf(node);
  const found = p.outputs.find((o) => o.name === name);
  if (found) return found;
  return p.columnOutputs && isKey(name) ? d(name, "text") : undefined;
}

/** Whether a data wire may carry `from` into an input of type `to` (`number`/`bool`/`time` → `text`). */
export function compatible(from: DataType, to: DataType): boolean {
  return from === to || (to === "text" && (from === "number" || from === "bool" || from === "time"));
}

/** Whether a data input of `node` accepts `from`. `setFields` inputs also take `rows` (lists). */
export function accepts(node: NodeKind, port: Port, from: DataType): boolean {
  if (!port.type) return false;
  return compatible(from, port.type) || (node.type === "action.setFields" && from === "rows");
}

export type ConnectCheck = { ok: true; kind: WireKind } | { ok: false; reason: string };

/** Whether a wire from `from` (an output) to `to` (an input) may be added to the graph. */
export function checkConnection(
  graph: { nodes: readonly FlowNode[]; edges: readonly FlowEdge[] },
  from: PortRef,
  to: PortRef,
): ConnectCheck {
  const a = graph.nodes.find((n) => n.id === from.node);
  const b = graph.nodes.find((n) => n.id === to.node);
  if (!a || !b) return { ok: false, reason: "That node no longer exists." };
  const out = outputOf(a, from.port);
  const inp = inputOf(b, to.port);
  if (!out) return { ok: false, reason: `There is no output "${from.port}".` };
  if (!inp) return { ok: false, reason: `There is no input "${to.port}".` };
  if (out.kind !== inp.kind) {
    return {
      ok: false,
      reason: out.kind === "event" ? "An event can't go into a data input." : "A value can't go into an event input.",
    };
  }
  const same = (e: FlowEdge) =>
    e.from.node === from.node && e.from.port === from.port && e.to.node === to.node && e.to.port === to.port;
  if (graph.edges.some(same)) return { ok: false, reason: "These ports are already connected." };
  if (out.kind === "event" && from.node === to.node && a.type === "logic.repeat" && to.port === "in") {
    return { ok: false, reason: "Repeat already loops; wire to stop instead." };
  }
  if (out.kind === "event" && from.node === to.node && !WAITS.has(a.type)) {
    return { ok: false, reason: "Only a Delay, Wait until, Debounce or Throttle can run itself again." };
  }
  if (out.kind === "data") {
    const t = out.type ?? "text";
    if (!accepts(b, inp, t)) return { ok: false, reason: `${t} can't go into a ${inp.type} input.` };
    if (graph.edges.some((e) => e.kind === "data" && e.to.node === to.node && e.to.port === to.port)) {
      return { ok: false, reason: "This input already has a wire." };
    }
    if (from.node === to.node) return { ok: false, reason: "A value can't feed its own node." };
  }
  return { ok: true, kind: out.kind };
}
