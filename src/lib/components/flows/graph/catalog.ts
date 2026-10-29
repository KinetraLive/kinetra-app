// The node catalogue of the flow editor (the port table is `core/flow_nodes.rs`): per node type the config a
// new node starts with, plus the one-line summary a node shows on the canvas. The palette order and
// descriptions are ./nodes.ts (only the lazy palette and inspector need them). Plain TS, unit-tested.

import type { FlowNode, LiveScreenView, NodeKind, NodeType } from "$lib/types";
import { stepOfType, triggerOfType, type Defaults } from "../defaults";
import {
  duration,
  newNodeId,
  stepText,
  triggerText,
  type Lookups,
  type StepConf,
  type TriggerConf,
} from "../linear";
import { nodeLabel } from "./labels";
import { categoryOf } from "./ports";

export { nodeLabel };

/** A new node of `type` at `position`, aimed at the defaults (selected screen, first sources). */
export function newNode(type: NodeType, d: Defaults, position: { x: number; y: number }): FlowNode {
  const id = newNodeId();
  const kind = defaultKind(type, d);
  return { ...kind, id, position } as FlowNode;
}

function defaultKind(type: NodeType, d: Defaults): NodeKind {
  const strip = <T extends { id: string }>({ id: _id, ...rest }: T) => rest as unknown as NodeKind;
  const cat = categoryOf(type);
  if (cat === "trigger") return strip(triggerOfType(type as TriggerConf["type"], d));
  if (cat === "action" || type === "logic.delay") return strip(stepOfType(type as StepConf["type"], d));
  const layer = { screenId: d.screenId, layerId: d.layerId };
  switch (type) {
    case "data.sourceRow":
      return { type, config: { sourceId: d.textId, row: { pick: "current" } } };
    case "data.scheduleItem":
      return { type, config: { sourceId: d.scheduleId, which: "now" } };
    case "data.fieldValue":
      return { type, config: { ...layer, key: "" } };
    case "data.layerState":
      return { type, config: layer };
    case "data.constant":
      return { type, config: { value: { kind: "text", value: "" } } };
    case "data.timeNow":
      return { type, config: {} };
    case "data.format":
      return { type, config: { pattern: "{a}" } };
    case "data.math":
      return { type, config: { op: "add", a: 0, b: 0 } };
    case "data.compare":
      return { type, config: { op: "eq", a: "", b: "" } };
    case "logic.branch":
      return { type, config: {} };
    case "logic.switch":
      return { type, config: { cases: [""] } };
    case "logic.gate":
      return { type, config: { open: true } };
    case "logic.waitUntil":
      return { type, config: { at: null, timeoutMs: null } };
    case "logic.debounce":
      return { type, config: { ms: 500 } };
    case "logic.throttle":
      return { type, config: { ms: 1000 } };
    case "logic.sequence":
      return { type, config: { count: 2 } };
    case "logic.once":
      return { type, config: {} };
    case "logic.counter":
      return { type, config: { every: 2 } };
    case "logic.random":
      return { type, config: { count: 2, noRepeat: false } };
    case "logic.repeat":
      return { type, config: { count: 3, delayMs: 0 } };
    default:
      throw new Error(`Unknown node type: ${type}`);
  }
}

/** Defaults for new nodes: a screen's last layer (the top one), the first schedule, text and API source. */
export function defaultsFrom(screens: readonly LiveScreenView[], sources: Lookups["sources"]): Defaults {
  const view = screens[0];
  const src = [...sources.values()];
  return {
    screenId: view?.screen.id ?? "",
    layerId: view?.layers.at(-1)?.id ?? "",
    scheduleId: src.find((s) => s.kind === "schedule")?.id ?? "",
    textId: src.find((s) => s.kind !== "schedule")?.id ?? "",
    apiId: src.find((s) => s.kind === "api" || s.kind === "socket")?.id ?? "",
  };
}

const MATH: Record<string, string> = { add: "+", sub: "−", mul: "×", div: "÷", min: "min", max: "max", round: "round" };
const COMPARE: Record<string, string> = {
  eq: "=",
  ne: "≠",
  lt: "<",
  le: "≤",
  gt: ">",
  ge: "≥",
  contains: "contains",
  empty: "is empty",
};
const quote = (s: string) => `“${s}”`;
const upper = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** The one-line config summary under a node's title. */
export function nodeSummary(n: NodeKind, k: Lookups): string {
  const cat = categoryOf(n.type);
  if (cat === "trigger") {
    if (n.type === "trigger.cue") return n.config.label?.trim() ? `Button “${n.config.label.trim()}”` : "Button: the flow's name";
    return upper(triggerText(n as TriggerConf, k));
  }
  if (cat === "action" || n.type === "logic.delay") return stepText(n as StepConf, k);
  const src = (id: string) => k.sources.get(id)?.name ?? "a missing source";
  const layer = (s: string, l: string) =>
    k.screens.get(s)?.layers.find((x) => x.id === l)?.name ?? "a missing layer";
  switch (n.type) {
    case "data.sourceRow": {
      const r = n.config.row;
      const which = r.pick === "index" ? `row ${r.index + 1}` : `${r.pick} row`;
      return `${upper(which)} of ${src(n.config.sourceId)}`;
    }
    case "data.scheduleItem":
      return `${n.config.which === "now" ? "NOW" : "UP NEXT"} in ${src(n.config.sourceId)}`;
    case "data.fieldValue":
      return `${n.config.key || "a field"} of ${layer(n.config.screenId, n.config.layerId)}`;
    case "data.layerState":
      return layer(n.config.screenId, n.config.layerId);
    case "data.constant": {
      const v = n.config.value;
      return v.kind === "text" ? quote(v.value) : v.kind === "bool" ? (v.value ? "Yes" : "No") : String(v.value);
    }
    case "data.timeNow":
      return "Server clock";
    case "data.format":
      return quote(n.config.pattern);
    case "data.math":
      return n.config.op === "round" ? "round a" : `a ${MATH[n.config.op]} b`;
    case "data.compare":
      return n.config.op === "empty" ? "a is empty" : `a ${COMPARE[n.config.op]} b`;
    case "logic.branch":
      return "If the condition is true";
    case "logic.switch":
      return `${n.config.cases.length} ${n.config.cases.length === 1 ? "case" : "cases"}`;
    case "logic.gate":
      return n.config.open ? "Starts open" : "Starts closed";
    case "logic.waitUntil": {
      const t = n.config.timeoutMs;
      const at = n.config.at ? `Until ${n.config.at}` : "Until released";
      return t ? `${at}, timeout ${duration(t)}` : at;
    }
    case "logic.debounce":
      return `Quiet for ${duration(n.config.ms)}`;
    case "logic.throttle":
      return `Once per ${duration(n.config.ms)}`;
    case "logic.sequence":
      return `${n.config.count} steps in order`;
    case "logic.once":
      return "First event only";
    case "logic.counter":
      return n.config.every === 1 ? "Every event" : `Every ${n.config.every} events`;
    case "logic.random":
      return `1 of ${n.config.count}${n.config.noRepeat ? ", never twice" : ""}`;
    case "logic.repeat": {
      const c = n.config;
      const times = c.count === null ? "Forever" : c.count === 1 ? "Once" : `${c.count} times`;
      return c.delayMs > 0 ? `${times}, ${duration(c.delayMs)} apart` : times;
    }
    default:
      return "";
  }
}

/** "14:02:31" (local time) of an RFC 3339 instant, "" when unreadable. */
export function clockOf(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

/** A port's label as shown on the node: case outputs show their value, sequence steps their number. */
export function portLabel(n: NodeKind, name: string): string {
  if (n.type === "logic.switch" && /^case\d+$/.test(name)) {
    const v = n.config.cases[Number(name.slice(4))];
    return v === undefined ? name : v === "" ? "(empty)" : quote(v);
  }
  if (n.type === "logic.sequence" && /^s\d+$/.test(name)) return String(Number(name.slice(1)) + 1);
  if (n.type === "logic.random" && /^r\d+$/.test(name)) return String(Number(name.slice(1)) + 1);
  return name;
}
