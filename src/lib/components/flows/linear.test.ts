import { describe, expect, it } from "vitest";
import type { FlowEdge, FlowNode, LiveScreenView, SourceSummary } from "$lib/types";
import {
  check,
  newLinear,
  stepIssue,
  stepOfType,
  toNewFlow,
  toSaveFlow,
  triggerIssue,
  triggerOfType,
  validLocal,
  type Defaults,
} from "./edit";
import {
  duration,
  fromLinear,
  relTime,
  stepText,
  summary,
  toLinear,
  type LinearFlow,
  type Lookups,
  type StepConf,
} from "./linear";
import { NO_OTHER_FLOWS, apiColumnOptions, apiOptions, runFlowPick, stopFlowPick } from "./pickers";
import { portsOf } from "./graph/ports";
import { nodeSummary } from "./graph/catalog";

// ---- Fixtures -----------------------------------------------------------------------------------

const main: LiveScreenView = {
  screen: { id: "s1", name: "Main", width: 1920, height: 1080, background: "transparent" },
  layers: [
    {
      id: "nn",
      name: "Now & Next",
      onAir: false,
      pending: false,
      fields: {},
      fieldDefs: [
        { key: "title", label: "Title", type: "text", default: "" },
        { key: "left", label: "Left", type: "timer", default: "" },
      ],
    },
    { id: "board", name: "Board", onAir: false, pending: false, fields: {}, fieldDefs: [] },
  ],
};

function source(id: string, name: string, kind: "text" | "schedule"): SourceSummary {
  return {
    id,
    name,
    kind,
    rowCount: 2,
    index: 0,
    current: {},
    columns: [],
    usedBy: [],
    updatedAt: "",
    auto: null,
    nextAt: null,
    nowId: null,
    nextId: null,
  };
}

const k: Lookups = {
  screens: new Map([["s1", main]]),
  sources: new Map([
    ["sched", source("sched", "Run of show", "schedule")],
    ["sp", source("sp", "Sponsors", "text")],
  ]),
  flows: new Map([["f2", { name: "Opener" }]]),
};

const at = (x: number) => ({ x, y: 0 });
const node = (id: string, kind: Omit<FlowNode, "id" | "position">, x = 0): FlowNode =>
  ({ id, position: at(x), ...kind }) as FlowNode;
const wire = (id: string, from: string, fromPort: string, to: string, toPort = "in", kind: FlowEdge["kind"] = "event"): FlowEdge => ({
  id,
  from: { node: from, port: fromPort },
  to: { node: to, port: toPort },
  kind,
});

const trig = node("t", { type: "trigger.itemStatus", config: { sourceId: "sched", status: "now", itemId: null } });
const take = node("a", { type: "action.take", config: { screenId: "s1", layerId: "nn", forMs: 10_000 } }, 240);
const wait = node("d", { type: "logic.delay", config: { ms: 5000 } }, 480);
const clear = node("c", { type: "action.clear", config: { screenId: "s1", layerId: "nn" } }, 720);
const chain = {
  nodes: [trig, take, wait, clear],
  edges: [wire("e1", "t", "fired", "a"), wire("e2", "a", "done", "d"), wire("e3", "d", "out", "c")],
};

// ---- Linear form --------------------------------------------------------------------------------

describe("linear flows", () => {
  it("a trigger wired to a chain of actions and delays is linear", () => {
    const l = toLinear(chain)!;
    expect(l.trigger).toEqual({ id: "t", type: "trigger.itemStatus", config: { sourceId: "sched", status: "now", itemId: null } });
    expect(l.steps.map((s) => s.id)).toEqual(["a", "d", "c"]);
    expect(l.steps[0]).not.toHaveProperty("position");
    // The order of nodes in the file doesn't matter, the wires do.
    expect(toLinear({ ...chain, nodes: [clear, wait, take, trig] })?.steps.map((s) => s.id)).toEqual(["a", "d", "c"]);
    // A trigger alone is linear too.
    expect(toLinear({ nodes: [trig], edges: [] })).toEqual({ trigger: l.trigger, steps: [] });
  });

  it("anything else is not linear", () => {
    const other = node("t2", { type: "trigger.cue", config: { label: null } });
    const branch = node("b", { type: "logic.branch", config: {} }, 240);
    const constant = node("k", { type: "data.constant", config: { value: { kind: "text", value: "x" } } });
    const cases: { nodes: FlowNode[]; edges: FlowEdge[] }[] = [
      { nodes: [...chain.nodes, other], edges: chain.edges }, // two triggers
      { nodes: [take, wait], edges: [wire("e", "a", "done", "d")] }, // no trigger
      { nodes: [trig, branch], edges: [wire("e", "t", "fired", "b")] }, // a logic node other than delay
      { nodes: [...chain.nodes, constant], edges: [...chain.edges, wire("e4", "k", "value", "a", "x", "data")] }, // data wire
      { nodes: [trig, take, clear], edges: [wire("e1", "t", "fired", "a"), wire("e2", "t", "fired", "c")] }, // fan-out
      { nodes: [trig, take, clear], edges: [wire("e1", "t", "fired", "a"), wire("e2", "a", "failed", "c")] }, // failed port
      { nodes: [trig, take, clear], edges: [wire("e1", "t", "fired", "a")] }, // an unwired node
      { nodes: [trig, take, wait], edges: [wire("e1", "t", "fired", "a"), wire("e2", "a", "done", "d"), wire("e3", "d", "out", "a")] }, // cycle
      { nodes: [trig, take], edges: [wire("e1", "t", "fired", "a", "cancel")] }, // wrong input
    ];
    for (const c of cases) expect(toLinear(c)).toBeNull();
  });

  it("builds the graph left to right; wires between the same nodes keep their ids", () => {
    const l = toLinear(chain)!;
    let n = 0;
    const g = fromLinear(l, [], () => `id${n++}`);
    expect(g.nodes.map((x) => x.position)).toEqual([at(0), at(240), at(480), at(720)]);
    expect(g.edges).toEqual([
      wire("id0", "t", "fired", "a"),
      wire("id1", "a", "done", "d"),
      wire("id2", "d", "out", "c"),
    ]);
    expect(toLinear(g)).toEqual(l);
    // Reordered: the take → delay wire keeps its id; the new pairs get new ones.
    const moved: LinearFlow = { trigger: l.trigger, steps: [l.steps[0], l.steps[1], l.steps[2]].reverse() };
    const g2 = fromLinear(moved, chain.edges, () => "new");
    expect(g2.edges.map((e) => e.id)).toEqual(["new", "new", "new"]);
    const same = fromLinear(l, chain.edges, () => "new");
    expect(same.edges.map((e) => e.id)).toEqual(["e1", "e2", "e3"]);
  });
});

// ---- Summaries ----------------------------------------------------------------------------------

describe("summaries", () => {
  it("When <trigger> then <a>, <b>; a non-linear flow is its node count", () => {
    expect(summary(chain, k)).toBe(
      "When item becomes NOW in Run of show then take Now & Next for 10 s, wait 5 s, clear Now & Next",
    );
    const cue = node("t", { type: "trigger.cue", config: { label: "Go" } });
    expect(summary({ nodes: [cue], edges: [] }, k)).toBe("When cued");
    expect(summary({ nodes: [trig, take, clear], edges: [] }, k)).toBe("3 nodes");
    expect(summary({ nodes: [], edges: [] }, k)).toBe("0 nodes");
  });

  it("triggers name their targets; missing ones say so", () => {
    const s = (config: object, type: string) => summary({ nodes: [node("t", { type, config } as never)], edges: [] }, k);
    expect(s({ sourceId: "sched", minutes: 5 }, "trigger.beforeNext")).toBe("When 5 min before the next item in Run of show");
    expect(s({ at: "18:30", days: [6, 5] }, "trigger.timeOfDay")).toBe("When at 18:30 on Sat, Sun");
    expect(s({ at: "09:00", days: [] }, "trigger.timeOfDay")).toBe("When at 09:00 every day");
    expect(s({ screenId: "s1", layerId: "nn", key: "left" }, "trigger.timerZero")).toBe("When a timer of Now & Next reaches zero");
    expect(s({ screenId: "s1", layerId: "board", onAir: false }, "trigger.layer")).toBe("When Board goes off air");
    expect(s({ sourceId: "sp" }, "trigger.slideshowWrap")).toBe("When Sponsors wraps");
    expect(s({ sourceId: "sp" }, "trigger.sourceRow")).toBe("When the row of Sponsors changes");
    expect(s({}, "trigger.runByFlow")).toBe("When run by another flow");
    expect(s({}, "trigger.rest")).toBe("When an API call arrives");
    expect(s({ sourceId: "gone", status: "next", itemId: "i1" }, "trigger.itemStatus")).toBe(
      "When a chosen item becomes UP NEXT in a missing source",
    );
  });

  it("every step kind", () => {
    const t = (s: StepConf) => stepText(s, k);
    const L = { screenId: "s1", layerId: "nn" };
    expect(t({ type: "action.take", config: { ...L, forMs: null } })).toBe("Take Now & Next");
    expect(t({ type: "action.take", config: { ...L, forMs: 60_000 } })).toBe("Take Now & Next for 1 min");
    expect(t({ type: "action.clear", config: { screenId: "s1", layerId: "x" } })).toBe("Clear a missing layer");
    expect(t({ type: "action.clearScreen", config: { screenId: "s1" } })).toBe("Clear Main");
    expect(t({ type: "action.clearAll", config: {} })).toBe("Clear all screens");
    expect(t({ type: "action.setFields", config: { ...L, fields: { title: "Hi" } } })).toBe("Set title of Now & Next");
    expect(t({ type: "action.setTimer", config: { ...L, key: "left", action: { action: "start" } } })).toBe(
      "Start the timer of Now & Next",
    );
    expect(t({ type: "action.setTimer", config: { ...L, key: "left", action: { action: "set", ms: 90_000 } } })).toBe(
      "Set the timer of Now & Next to 90 s",
    );
    expect(t({ type: "action.stepSource", config: { sourceId: "sp", step: { to: "by", delta: 1 } } })).toBe("Next row of Sponsors");
    expect(t({ type: "action.stepSource", config: { sourceId: "sp", step: { to: "by", delta: -1 } } })).toBe(
      "Previous row of Sponsors",
    );
    expect(t({ type: "action.stepSource", config: { sourceId: "sp", step: { to: "index", index: 2 } } })).toBe(
      "Go to row 3 of Sponsors",
    );
    expect(t({ type: "action.pauseSource", config: { sourceId: "sp", mode: "resume" } })).toBe("Resume Sponsors");
    expect(t({ type: "action.schedule", config: { sourceId: "sched", item: { pick: "now" }, op: { op: "end" } } })).toBe(
      "End the NOW item of Run of show",
    );
    expect(
      t({ type: "action.schedule", config: { sourceId: "sched", item: { pick: "next" }, op: { op: "delay", minutes: 5 } } }),
    ).toBe("Delay the UP NEXT item of Run of show by 5 min");
    expect(t({ type: "action.runFlow", config: { flowId: "f2", wait: true } })).toBe("Run Opener and wait");
    expect(t({ type: "action.log", config: { message: "hi", toast: false } })).toBe('Log "hi"');
    expect(t({ type: "logic.delay", config: { ms: 1500 } })).toBe("Wait 1.5 s");
    expect(t({ type: "action.setVariant", config: { ...L, variant: "compact", transition: "outIn" } })).toBe(
      "Show compact on Now & Next",
    );
    expect(t({ type: "action.setVariant", config: { ...L, variant: "", transition: "crossfade" } })).toBe(
      "Show a variant on Now & Next",
    );
  });

  it("a Take preview step: its text, defaults and checks", () => {
    const tp = (screenId: string, transition: { kind: "cut" | "crossfade" | "outIn"; ms: number } | null) =>
      ({ type: "action.takePreview", config: { screenId, transition } }) as const;
    expect(stepText(tp("s1", null), k)).toBe("Take the Preview of Main");
    expect(stepText(tp("s1", { kind: "crossfade", ms: 800 }), k)).toBe("Take the Preview of Main (Crossfade 800 ms)");
    expect(stepText(tp("s1", { kind: "outIn", ms: 500 }), k)).toBe("Take the Preview of Main (Play out & in)");
    expect(stepText(tp("gone", { kind: "cut", ms: 500 }), k)).toBe("Take the Preview of a missing screen (Cut)");
    expect(stepIssue(tp("s1", null), k, null)).toBeNull();
    expect(stepIssue(tp("", null), k, null)?.message).toBe("Choose a screen.");
    expect(stepIssue(tp("gone", null), k, null)?.level).toBe("warning");
    expect(stepIssue(tp("s1", { kind: "crossfade", ms: 50 }), k, null)?.message).toBe("Duration: 100 to 5000 ms.");
    // A new one keeps the screen of the step it replaces and starts with the project default.
    const d: Defaults = { screenId: "s1", layerId: "nn", scheduleId: "sched", textId: "sp" };
    expect(stepOfType("action.takePreview", d, { id: "a", type: "action.take", config: { screenId: "s1", layerId: "board", forMs: null } })).toEqual({
      id: "a",
      type: "action.takePreview",
      config: { screenId: "s1", transition: null },
    });
    const flow = { nodes: [trig, node("p", tp("s1", null), 240)], edges: [wire("e1", "t", "fired", "p")] };
    expect(summary(flow, k)).toBe("When item becomes NOW in Run of show then take the Preview of Main");
  });

  it("a Set variant step: linear, summarised, built back the same", () => {
    const sv = node("v", { type: "action.setVariant", config: { screenId: "s1", layerId: "nn", variant: "compact", transition: "crossfade" } }, 240);
    const flow = { nodes: [trig, sv], edges: [wire("e1", "t", "fired", "v")] };
    const l = toLinear(flow)!;
    expect(l.steps).toEqual([{ id: "v", type: "action.setVariant", config: sv.config }]);
    expect(summary(flow, k)).toBe("When item becomes NOW in Run of show then show compact on Now & Next");
    expect(fromLinear(l, flow.edges)).toEqual(flow);
  });

  it("durations and relative times", () => {
    expect(duration(10_000)).toBe("10 s");
    expect(duration(120_000)).toBe("2 min");
    expect(duration(7_200_000)).toBe("2 h");
    const now = Date.parse("2026-10-04T12:00:00Z");
    expect(relTime("2026-10-04T11:59:48Z", now)).toBe("12 s ago");
    expect(relTime("2026-10-04T11:57:00Z", now)).toBe("3 min ago");
    expect(relTime("2026-10-04T09:00:00Z", now)).toBe("3 h ago");
    expect(relTime("2026-10-01T12:00:00Z", now)).toBe("3 d ago");
  });
});

// ---- Editor model -------------------------------------------------------------------------------

describe("editor model", () => {
  const d: Defaults = { screenId: "s1", layerId: "nn", scheduleId: "sched", textId: "sp" };

  it("a new cue is armed, a new rule disarmed; names are trimmed", () => {
    const cue = newLinear("cue", d);
    expect(cue.trigger.type).toBe("trigger.cue");
    expect(cue.steps).toMatchObject([{ type: "action.take", config: { screenId: "s1", layerId: "nn", forMs: null } }]);
    const req = toNewFlow("  Opener  ", "ignore", cue);
    expect(req).toMatchObject({ name: "Opener", armed: true, retrigger: "ignore" });
    expect(toLinear(req)).toEqual(cue);

    const rule = newLinear("rule", d);
    expect(rule.trigger).toMatchObject({ type: "trigger.itemStatus", config: { sourceId: "sched", status: "now" } });
    expect(toNewFlow("Rule 1", "restart", rule).armed).toBe(false);
    expect(newLinear("rule", { ...d, scheduleId: "" }).trigger.type).toBe("trigger.layer");
    expect(newLinear("rule", d, "trigger.slideshowWrap").trigger).toMatchObject({ config: { sourceId: "sp" } });
    expect(toNewFlow("Rule 1", "restart", rule, "rcp1").origin).toBe("rcp1");
    expect("origin" in toNewFlow("Rule 1", "restart", rule)).toBe(false);
  });

  it("schedule start times are local date-times on real days", () => {
    expect(validLocal("2026-10-04T09:30")).toBe(true);
    expect(validLocal("2028-02-29T23:59")).toBe(true);
    for (const bad of ["2026-02-30T10:00", "2026-10-04 09:30", "2026-10-04T24:00", "2026-13-01T10:00", "2026-10-04T9:30", ""])
      expect(validLocal(bad), bad).toBe(false);
    const sched = (op: object) => stepIssue({ type: "action.schedule", config: { sourceId: "sched", item: { pick: "now" }, op } } as never, k, null);
    expect(sched({ op: "setStart", start: "" })?.message).toBe("Enter the new start date and time.");
    expect(sched({ op: "setStart", start: "2026-02-30T10:00" })?.message).toBe("Enter a date and time like 2026-10-04 09:30.");
    expect(sched({ op: "setStart", start: "2026-10-04T10:00" })).toBeNull();
    expect(sched({ op: "setNext", start: null })).toBeNull();
    expect(sched({ op: "setNext", start: "x" })?.level).toBe("error");
  });

  it("saving keeps the flow's id, node ids and wire ids", () => {
    const l = toLinear(chain)!;
    const req = toSaveFlow({ id: "fl1", edges: chain.edges }, " Strap ", "restart", l);
    expect(req.id).toBe("fl1");
    expect(req.name).toBe("Strap");
    expect(req.nodes.map((n) => n.id)).toEqual(["t", "a", "d", "c"]);
    expect(req.edges.map((e) => e.id)).toEqual(["e1", "e2", "e3"]);
  });

  it("changing a kind keeps the node id and the targets that still apply", () => {
    const t = triggerOfType("trigger.layer", d, { id: "t", type: "trigger.timerZero", config: { screenId: "s1", layerId: "board", key: "left" } });
    expect(t).toEqual({ id: "t", type: "trigger.layer", config: { screenId: "s1", layerId: "board", onAir: true } });
    // A schedule doesn't carry over to a slideshow trigger (and back).
    const w = triggerOfType("trigger.slideshowWrap", d, { id: "t", type: "trigger.itemStatus", config: { sourceId: "sched", status: "now" } });
    expect(w.config).toEqual({ sourceId: "sp" });
    const s = stepOfType("action.clear", d, { id: "a", type: "action.take", config: { screenId: "s1", layerId: "board", forMs: 5000 } });
    expect(s).toEqual({ id: "a", type: "action.clear", config: { screenId: "s1", layerId: "board" } });
    // Set variant keeps the layer; the variant is chosen anew.
    expect(stepOfType("action.setVariant", d, { id: "a", type: "action.take", config: { screenId: "s1", layerId: "board", forMs: null } })).toEqual({
      id: "a",
      type: "action.setVariant",
      config: { screenId: "s1", layerId: "board", variant: "", transition: "crossfade" },
    });
    for (const type of ["action.clearAll", "action.setTimer", "action.schedule", "action.runFlow", "action.log", "logic.delay"] as const) {
      expect(stepOfType(type, d).type).toBe(type);
    }
  });

  it("missing targets are errors, gone ones warnings; Save counts them", () => {
    expect(triggerIssue({ type: "trigger.itemStatus", config: { sourceId: "", status: "now" } }, k)?.level).toBe("error");
    expect(triggerIssue({ type: "trigger.itemStatus", config: { sourceId: "sp", status: "now" } }, k)?.message).toBe("Choose a schedule.");
    expect(triggerIssue({ type: "trigger.timeOfDay", config: { at: "25:00", days: [] } }, k)?.level).toBe("error");
    expect(triggerIssue({ type: "trigger.timerZero", config: { screenId: "s1", layerId: "nn", key: "" } }, k)?.message).toBe(
      "Choose a timer field.",
    );
    expect(stepIssue({ type: "action.take", config: { screenId: "s1", layerId: "gone" } }, k, null)?.level).toBe("warning");
    expect(stepIssue({ type: "action.take", config: { screenId: "s1", layerId: "nn", forMs: 500 } }, k, null)?.level).toBe("error");
    expect(stepIssue({ type: "action.runFlow", config: { flowId: "fl1", wait: false } }, k, "fl1")?.message).toBe(
      "A flow can't run itself; use Repeat.",
    );
    expect(stepIssue({ type: "action.log", config: { message: " ", toast: false } }, k, null)?.level).toBe("error");
    const sv = (layerId: string, variant: string) =>
      stepIssue({ type: "action.setVariant", config: { screenId: "s1", layerId, variant, transition: "crossfade" } }, k, null);
    expect(sv("nn", "")?.message).toBe("Choose a variant.");
    expect(sv("nn", "compact")).toBeNull();
    expect(sv("gone", "compact")?.level).toBe("warning");
    expect(sv("", "compact")?.message).toBe("Choose a layer.");
    const l = toLinear(chain)!;
    expect(check("Strap", l, k, null)).toEqual({ errors: 0, warnings: 0 });
    expect(check(" ", l, k, null).errors).toBe(1);
    const gone = { ...l, steps: [{ ...l.steps[0], config: { screenId: "s1", layerId: "gone", forMs: null } } as typeof l.steps[0]] };
    expect(check("Strap", gone, k, null)).toEqual({ errors: 0, warnings: 1 });
  });
});

// ---- Stop flow, Run flow pickers ------------------------------------------------------------------

describe("Stop flow step and the flow pickers", () => {
  const stopOf = (flowId: string): StepConf => ({ type: "action.stopFlow", config: { flowId } });

  it("summaries: this flow, another flow, a missing one", () => {
    expect(stepText(stopOf("f2"), k)).toBe("Stop Opener");
    expect(stepText(stopOf("me"), { ...k, self: "me" })).toBe("Stop this flow");
    expect(stepText(stopOf("gone"), k)).toBe("Stop a missing flow");
    // A flow's own summary names itself "this flow" (its id).
    const cue = node("t", { type: "trigger.cue", config: { label: null } });
    const stop = node("s", { type: "action.stopFlow", config: { flowId: "loop" } }, 240);
    const flow = { id: "loop", nodes: [cue, stop], edges: [wire("e1", "t", "fired", "s")] };
    expect(summary(flow, k)).toBe("When cued then stop this flow");
    expect(summary({ ...flow, id: "other" }, k)).toBe("When cued then stop a missing flow");
  });

  it("a cue with Stop flow is linear and round-trips (two nodes, one wire)", () => {
    const cue = node("t", { type: "trigger.cue", config: { label: "Stop loop" } });
    const stop = node("s", { type: "action.stopFlow", config: { flowId: "f2" } }, 240);
    const flow = { nodes: [cue, stop], edges: [wire("e1", "t", "fired", "s")] };
    const l = toLinear(flow)!;
    expect(l.steps).toEqual([{ id: "s", type: "action.stopFlow", config: { flowId: "f2" } }]);
    expect(fromLinear(l, flow.edges)).toEqual(flow);
    const d: Defaults = { screenId: "s1", layerId: "nn", scheduleId: "sched", textId: "sp" };
    expect(stepOfType("action.stopFlow", d)).toMatchObject({ type: "action.stopFlow", config: { flowId: "" } });
  });

  it("checks: a flow must be chosen; itself is fine; a gone flow is a warning", () => {
    expect(stepIssue(stopOf(""), k, "me")?.message).toBe("Choose a flow.");
    expect(stepIssue(stopOf("me"), k, "me")).toBeNull();
    expect(stepIssue(stopOf("f2"), k, "me")).toBeNull();
    expect(stepIssue(stopOf("gone"), k, "me")?.level).toBe("warning");
  });

  it("Run flow hides this flow (with none left, the placeholder points at Repeat); Stop flow lists it first", () => {
    const only: Lookups = { ...k, flows: new Map([["me", { name: "Loop" }]]) };
    expect(runFlowPick(only, "me", "")).toEqual([{ value: "", label: NO_OTHER_FLOWS, disabled: true }]);
    expect(NO_OTHER_FLOWS).toBe("No other flows — use the Repeat node to loop this flow.");
    const two: Lookups = { ...k, flows: new Map([["me", { name: "Loop" }], ["f2", { name: "Opener" }]]) };
    expect(runFlowPick(two, "me", "f2")).toEqual([{ value: "f2", label: "Opener" }]);
    expect(runFlowPick(two, "me", "")[0]).toEqual({ value: "", label: "Choose a flow", disabled: true });
    expect(stopFlowPick(two, "me", "me")).toEqual([
      { value: "me", label: "This flow" },
      { value: "f2", label: "Opener" },
    ]);
    // A flow not saved yet has no id: only the others.
    expect(stopFlowPick(two, null, "")).toEqual([
      { value: "", label: "Choose a flow", disabled: true },
      { value: "me", label: "Loop" },
      { value: "f2", label: "Opener" },
    ]);
  });
});

describe("API value changed trigger", () => {
  const feed: SourceSummary = {
    ...source("feed", "Scores", "text"),
    kind: "api",
    columns: [
      { key: "home", label: "Home team" },
      { key: "score", label: "Score" },
    ],
  };
  const ka: Lookups = { ...k, sources: new Map([...k.sources, ["feed", feed]]) };
  const t = (column: string | null, sourceId = "feed") =>
    ({ id: "t", type: "trigger.apiChanged", config: { sourceId, column } }) as const;

  it("summaries: any change, or one column (its label)", () => {
    expect(summary({ nodes: [{ ...t(null), position: at(0) }], edges: [] }, ka)).toBe("When Scores changes");
    expect(summary({ nodes: [{ ...t("score"), position: at(0) }], edges: [] }, ka)).toBe("When Score of Scores changes");
    expect(nodeSummary(t("home"), ka)).toBe("Home team of Scores changes");
    expect(summary({ nodes: [{ ...t(null, "gone"), position: at(0) }], edges: [] }, ka)).toBe("When a missing source changes");
  });

  it("checks: an API source is required; a gone column is a warning", () => {
    expect(triggerIssue(t(null, ""), ka)?.message).toBe("Choose an API or live data source.");
    expect(triggerIssue(t(null, "sp"), ka)).toEqual({ level: "error", message: "Choose an API or live data source." });
    expect(triggerIssue(t(null), ka)).toBeNull();
    expect(triggerIssue(t("score"), ka)).toBeNull();
    expect(triggerIssue(t("away"), ka)?.level).toBe("warning");
    expect(triggerIssue(t(null, "gone"), ka)?.level).toBe("warning");
  });

  it("defaults: the first API source, no column; switching keeps the API source", () => {
    const d: Defaults = { screenId: "s1", layerId: "nn", scheduleId: "sched", textId: "sp", apiId: "feed" };
    const a = triggerOfType("trigger.apiChanged", d);
    expect(a).toMatchObject({ type: "trigger.apiChanged", config: { sourceId: "feed", column: null } });
    const other = triggerOfType("trigger.apiChanged", { ...d, apiId: "x" }, { ...a, config: { sourceId: "feed", column: "score" } } as typeof a);
    expect(other.config).toEqual({ sourceId: "feed", column: null });
    expect(triggerOfType("trigger.apiChanged", { screenId: "", layerId: "", scheduleId: "", textId: "" }).config).toEqual({
      sourceId: "",
      column: null,
    });
  });

  it("pickers: API sources only; the columns with Any row changes first", () => {
    expect(apiOptions(ka)).toEqual([{ value: "feed", label: "Scores" }]);
    expect(apiColumnOptions(ka, "feed", null)).toEqual([
      { value: "", label: "Any row changes" },
      { value: "home", label: "Home team" },
      { value: "score", label: "Score" },
    ]);
    expect(apiColumnOptions(ka, "feed", "away").at(-1)).toEqual({ value: "away", label: "Missing (away)" });
  });

  it("ports: fired, value (text), row (row)", () => {
    const p = portsOf(t("score"));
    expect(p.inputs).toEqual([]);
    expect(p.outputs).toEqual([
      { name: "fired", kind: "event" },
      { name: "value", kind: "data", type: "text" },
      { name: "row", kind: "data", type: "row" },
    ]);
  });
});
