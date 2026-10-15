// Linear flows: "When <trigger> then <a>, <b>, …". Plain functions, no Svelte, unit-tested.
//
// A flow is linear when it has exactly one trigger node and one event chain
//   trigger.fired → s1.in, s1.done → s2.in, … (a Delay continues from `out`)
// through action.* and logic.delay nodes only: no data wires, no other nodes, no branches. Control
// edits such flows with the linear editor; anything else is shown as "<n> nodes" (the node editor
// comes later). `toLinear` / `fromLinear` convert both ways; node ids are kept, so saving an
// unchanged flow saves the same graph.

import type { Flow, FlowEdge, FlowNode, LiveScreenView, NodeKind, SourceSummary } from "$lib/types";
import { newNodeId } from "./common";
import { transitionText } from "$lib/components/control/studio/transitions";

export { newNodeId, relTime } from "./common";

export type TriggerConf = Extract<NodeKind, { type: `trigger.${string}` }>;
export type StepConf = Extract<NodeKind, { type: `action.${string}` | "logic.delay" }>;
export type TriggerType = TriggerConf["type"];
export type StepType = StepConf["type"];

export type LinearTrigger = TriggerConf & { id: string };
export type LinearStep = StepConf & { id: string };

export interface LinearFlow {
  trigger: LinearTrigger;
  steps: LinearStep[];
}

/** Horizontal distance between nodes laid out by `fromLinear`. */
export const LINEAR_SPACING = 240;

export const isTriggerType = (t: string): t is TriggerType => t.startsWith("trigger.");
export const isStepType = (t: string): t is StepType => t.startsWith("action.") || t === "logic.delay";

/** The event output a linear chain continues from. */
export function outPort(type: string): string {
  return isTriggerType(type) ? "fired" : type === "logic.delay" ? "out" : "done";
}

const strip = <T extends FlowNode>({ position: _p, ...rest }: T) => rest;

/** The linear form of a flow, or null when it isn't one chain from one trigger. */
export function toLinear(flow: Pick<Flow, "nodes" | "edges">): LinearFlow | null {
  const triggers = flow.nodes.filter((n) => isTriggerType(n.type));
  if (triggers.length !== 1) return null;
  if (flow.nodes.some((n) => !isTriggerType(n.type) && !isStepType(n.type))) return null;
  if (flow.edges.some((e) => e.kind !== "event")) return null;
  if (flow.edges.length !== flow.nodes.length - 1) return null;
  const byId = new Map(flow.nodes.map((n) => [n.id, n]));
  const out = new Map<string, FlowEdge[]>();
  for (const e of flow.edges) out.set(e.from.node, [...(out.get(e.from.node) ?? []), e]);

  const trigger = triggers[0];
  const steps: LinearStep[] = [];
  const seen = new Set([trigger.id]);
  let cur: FlowNode = trigger;
  for (;;) {
    const next = out.get(cur.id) ?? [];
    if (next.length === 0) break;
    if (next.length > 1) return null;
    const e = next[0];
    if (e.from.port !== outPort(cur.type) || e.to.port !== "in") return null;
    const node = byId.get(e.to.node);
    if (!node || seen.has(node.id) || !isStepType(node.type)) return null;
    seen.add(node.id);
    steps.push(strip(node) as LinearStep);
    cur = node;
  }
  if (seen.size !== flow.nodes.length) return null;
  return { trigger: strip(trigger) as LinearTrigger, steps };
}

/**
 * The graph of a linear flow: nodes laid out left to right (x = 240 × i), one event wire per step.
 * Wires between the same nodes as in `prevEdges` keep their ids.
 */
export function fromLinear(
  l: LinearFlow,
  prevEdges: readonly FlowEdge[] = [],
  id: () => string = newNodeId,
): { nodes: FlowNode[]; edges: FlowEdge[] } {
  const chain = [l.trigger, ...l.steps];
  const nodes = chain.map((n, i) => ({ ...n, position: { x: LINEAR_SPACING * i, y: 0 } }) as FlowNode);
  const edges: FlowEdge[] = [];
  for (let i = 1; i < chain.length; i++) {
    const from = { node: chain[i - 1].id, port: outPort(chain[i - 1].type) };
    const to = { node: chain[i].id, port: "in" };
    const prev = prevEdges.find((e) => e.from.node === from.node && e.to.node === to.node);
    edges.push({ id: prev?.id ?? id(), from, to, kind: "event" });
  }
  return { nodes, edges };
}

// ---- Summaries ----------------------------------------------------------------------------------

/** Names the summaries resolve ids to. */
export interface Lookups {
  screens: ReadonlyMap<string, LiveScreenView>;
  sources: ReadonlyMap<string, SourceSummary>;
  flows: ReadonlyMap<string, { name: string }>;
  /** The flow being shown or edited: Stop flow names it "this flow". */
  self?: string | null;
}

export const STATUS_LABEL: Record<string, string> = {
  now: "NOW",
  next: "UP NEXT",
  delayed: "DELAYED",
  previous: "PREVIOUS",
  cancelled: "CANCELLED",
  upcoming: "upcoming",
};

/** 0 = Monday … 6 = Sunday. */
export const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] as const;

/** "10 s", "1 min", "1.5 s", "2 h". */
export function duration(ms: number): string {
  if (ms >= 3_600_000 && ms % 3_600_000 === 0) return `${ms / 3_600_000} h`;
  if (ms >= 60_000 && ms % 60_000 === 0) return `${ms / 60_000} min`;
  return `${Math.round(ms / 100) / 10} s`;
}

function names(k: Lookups) {
  return {
    src: (id: string) => k.sources.get(id)?.name ?? "a missing source",
    screen: (id: string) => k.screens.get(id)?.screen.name ?? "a missing screen",
    layer: (s: string, l: string) =>
      k.screens.get(s)?.layers.find((x) => x.id === l)?.name ?? "a missing layer",
    flow: (id: string) => k.flows.get(id)?.name ?? "a missing flow",
  };
}

/** What sets a flow off, e.g. "item becomes NOW in Run of show". */
export function triggerText(t: TriggerConf, k: Lookups): string {
  const n = names(k);
  switch (t.type) {
    case "trigger.cue":
      return "cued";
    case "trigger.itemStatus":
      return `${t.config.itemId ? "a chosen item" : "item"} becomes ${STATUS_LABEL[t.config.status] ?? t.config.status} in ${n.src(t.config.sourceId)}`;
    case "trigger.beforeNext":
      return `${t.config.minutes} min before the next item in ${n.src(t.config.sourceId)}`;
    case "trigger.timeOfDay": {
      const d = t.config.days;
      const days = d.length > 0 && d.length < 7 ? ` on ${[...d].sort().map((x) => DAYS[x]).join(", ")}` : " every day";
      return `at ${t.config.at}${days}`;
    }
    case "trigger.timerZero":
      return `a timer of ${n.layer(t.config.screenId, t.config.layerId)} reaches zero`;
    case "trigger.layer":
      return `${n.layer(t.config.screenId, t.config.layerId)} goes ${t.config.onAir ? "on air" : "off air"}`;
    case "trigger.slideshowWrap":
      return `${n.src(t.config.sourceId)} wraps`;
    case "trigger.sourceRow":
      return `the row of ${n.src(t.config.sourceId)} changes`;
    case "trigger.runByFlow":
      return "run by another flow";
    case "trigger.rest":
      return "an API call arrives";
    case "trigger.midi": {
      const c = t.config;
      return `MIDI ${c.kind === "cc" ? "CC" : "note"} ${c.number}${c.channel === null ? "" : ` on Ch ${c.channel}`}${c.device ? ` from ${c.device}` : ""}`;
    }
    case "trigger.osc":
      return `OSC ${t.config.address}`;
    case "trigger.mediaEnded":
      return `${n.layer(t.config.screenId, t.config.layerId)} reaches its end`;
    case "trigger.apiChanged": {
      const c = t.config.column;
      const col = c ? (k.sources.get(t.config.sourceId)?.columns.find((x) => x.key === c)?.label ?? c) : null;
      return col ? `${col} of ${n.src(t.config.sourceId)} changes` : `${n.src(t.config.sourceId)} changes`;
    }
  }
}

const TIMER_VERB: Record<string, string> = { start: "Start", pause: "Pause", reset: "Reset", set: "Set", configure: "Set up" };
const PAUSE_VERB: Record<string, string> = { pause: "Pause", resume: "Resume", toggle: "Pause or resume" };
const ITEM_TEXT: Record<string, string> = {
  now: "the NOW item",
  next: "the UP NEXT item",
  item: "a chosen item",
  input: "the given item",
};

/** One step, e.g. "Take Now & Next for 10 s", "Wait 5 s". */
export function stepText(s: StepConf, k: Lookups): string {
  const n = names(k);
  switch (s.type) {
    case "action.take": {
      const f = s.config.forMs;
      return `Take ${n.layer(s.config.screenId, s.config.layerId)}${f ? ` for ${duration(f)}` : ""}`;
    }
    case "action.clear":
      return `Clear ${n.layer(s.config.screenId, s.config.layerId)}`;
    case "action.clearScreen":
      return `Clear ${n.screen(s.config.screenId)}`;
    case "action.clearAll":
      return "Clear all screens";
    case "action.setFields":
      return `Set ${Object.keys(s.config.fields).join(", ") || "fields"} of ${n.layer(s.config.screenId, s.config.layerId)}`;
    case "action.setTimer": {
      const a = s.config.action;
      const what = `the timer of ${n.layer(s.config.screenId, s.config.layerId)}`;
      return a.action === "set" ? `Set ${what} to ${duration(a.ms)}` : `${TIMER_VERB[a.action]} ${what}`;
    }
    case "action.stepSource": {
      const st = s.config.step;
      const src = n.src(s.config.sourceId);
      if (st.to === "index") return `Go to row ${st.index + 1} of ${src}`;
      if (st.delta === 1) return `Next row of ${src}`;
      if (st.delta === -1) return `Previous row of ${src}`;
      return `Step ${src} by ${st.delta}`;
    }
    case "action.pauseSource":
      return `${PAUSE_VERB[s.config.mode]} ${n.src(s.config.sourceId)}`;
    case "action.schedule": {
      const item = `${ITEM_TEXT[s.config.item.pick]} of ${n.src(s.config.sourceId)}`;
      const op = s.config.op;
      switch (op.op) {
        case "startNow":
          return `Start ${item} now`;
        case "end":
          return `End ${item}`;
        case "setNext":
          return `Make ${item} next${op.start ? ` at ${op.start.replace("T", " ")}` : ""}`;
        case "delay":
          return `Delay ${item} by ${op.minutes} min`;
        case "setStart":
          return `Move ${item} to ${op.start.replace("T", " ")}`;
        case "cancel":
          return `Cancel ${item}`;
        case "restore":
          return `Restore ${item}`;
        case "auto":
          return `Set ${item} back to automatic`;
      }
    }
    case "action.setVariant":
      return `Show ${s.config.variant || "a variant"} on ${n.layer(s.config.screenId, s.config.layerId)}`;
    case "action.takePreview": {
      const t = s.config.transition;
      return `Take the Preview of ${n.screen(s.config.screenId)}${t ? ` (${transitionText(t)})` : ""}`;
    }
    case "action.runFlow":
      return `Run ${n.flow(s.config.flowId)}${s.config.wait ? " and wait" : ""}`;
    case "action.stopFlow":
      return s.config.flowId && s.config.flowId === k.self ? "Stop this flow" : `Stop ${n.flow(s.config.flowId)}`;
    case "action.log":
      return `Log "${s.config.message}"`;
    case "action.media": {
      const l = n.layer(s.config.screenId, s.config.layerId);
      const a = s.config.action;
      if (a === "seek") return `Go to ${mediaTime(s.config.ms ?? 0)} in ${l}`;
      return `${MEDIA_VERB[a]} ${l}`;
    }
    case "logic.delay":
      return `Wait ${duration(s.config.ms)}`;
  }
}

export const MEDIA_VERB: Record<string, string> = { play: "Play", pause: "Pause", stop: "Stop", restart: "Restart", seek: "Go to" };

/** "1:05" (m:ss), "1:05.5" with tenths. */
export function mediaTime(ms: number): string {
  const t = Math.max(0, Math.round(ms / 100)) / 10;
  const m = Math.floor(t / 60);
  const s = t - m * 60;
  const sec = Number.isInteger(s) ? String(s).padStart(2, "0") : s.toFixed(1).padStart(4, "0");
  return `${m}:${sec}`;
}

const lower = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);

/**
 * The one-line summary: "When <trigger> then <a>, <b>", or "<n> nodes" for a non-linear flow. With
 * the flow's id, a Stop flow of itself reads "stop this flow".
 */
export function summary(flow: Pick<Flow, "nodes" | "edges"> & { id?: string }, k: Lookups): string {
  if (flow.id && k.self === undefined) k = { ...k, self: flow.id };
  const l = toLinear(flow);
  if (!l) return `${flow.nodes.length} ${flow.nodes.length === 1 ? "node" : "nodes"}`;
  const when = `When ${triggerText(l.trigger, k)}`;
  return l.steps.length === 0 ? when : `${when} then ${l.steps.map((s) => lower(stepText(s, k))).join(", ")}`;
}
