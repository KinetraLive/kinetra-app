// Linear editor model: trigger and step kinds with their defaults, client-side checks and the
// requests Save sends. Plain functions, no Svelte, unit-tested. The server validates again
// (`create_flow` / `save_flow`); its message is shown in the editor.
//
// Errors block Save (no name, a missing target, a bad time). Warnings don't: a target that no
// longer exists is saved and skipped when the flow runs.

import {
  FLOW_DELAY_MAX_MS,
  FLOW_NAME_MAX,
  FLOW_NODES_MAX,
  type FlowEdge,
  type FlowInfo,
  type NewFlow,
  type Retrigger,
  type SaveFlow,
} from "$lib/types";
import { stepOfType, triggerOfType, type Defaults } from "./defaults";
import { nameError } from "./names";
import { TAKE_MS } from "$lib/components/control/studio/transitions";
import { BEFORE_MINUTES, TAKE_FOR_MS } from "./pickers";
import {
  fromLinear,
  type LinearFlow,
  type LinearStep,
  type LinearTrigger,
  type Lookups,
  type StepConf,
  type StepType,
  type TriggerConf,
  type TriggerType,
} from "./linear";


export const TRIGGER_OPTIONS: { value: TriggerType; label: string }[] = [
  { value: "trigger.cue", label: "Cue button or hotkey" },
  { value: "trigger.itemStatus", label: "A schedule item becomes…" },
  { value: "trigger.beforeNext", label: "Minutes before the next item" },
  { value: "trigger.timeOfDay", label: "Time of day" },
  { value: "trigger.timerZero", label: "A timer reaches zero" },
  { value: "trigger.layer", label: "A layer goes on or off air" },
  { value: "trigger.slideshowWrap", label: "A slideshow wraps" },
  { value: "trigger.sourceRow", label: "A source's row changes" },
  { value: "trigger.apiChanged", label: "API value changed" },
  { value: "trigger.runByFlow", label: "Run by another flow" },
  { value: "trigger.rest", label: "An API call (Companion, scripts)" },
  { value: "trigger.midi", label: "MIDI in" },
  { value: "trigger.osc", label: "OSC in" },
  { value: "trigger.mediaEnded", label: "Media ends" },
];

export const STEP_OPTIONS: { value: StepType; label: string }[] = [
  { value: "action.take", label: "Take" },
  { value: "action.clear", label: "Clear layer" },
  { value: "action.clearScreen", label: "Clear screen" },
  { value: "action.clearAll", label: "Clear all screens" },
  { value: "action.setFields", label: "Set fields" },
  { value: "action.setTimer", label: "Set timer" },
  { value: "action.stepSource", label: "Step source" },
  { value: "action.pauseSource", label: "Pause / resume source" },
  { value: "action.schedule", label: "Schedule action" },
  { value: "action.setVariant", label: "Set variant" },
  { value: "action.media", label: "Media" },
  { value: "action.takePreview", label: "Take preview" },
  { value: "logic.delay", label: "Wait" },
  { value: "action.runFlow", label: "Run flow" },
  { value: "action.stopFlow", label: "Stop flow" },
  { value: "action.log", label: "Log" },
];

export const RETRIGGER_OPTIONS: { value: Retrigger; label: string }[] = [
  { value: "restart", label: "Running again restarts it" },
  { value: "ignore", label: "Ignored while it runs" },
];

/** A new cue (one Take of the selected layer) or rule (`rule`: its trigger, else a default). */
export function newLinear(kind: "cue" | "rule", d: Defaults, rule?: TriggerType): LinearFlow {
  const trigger: TriggerType =
    kind === "cue" ? "trigger.cue" : (rule ?? (d.scheduleId ? "trigger.itemStatus" : "trigger.layer"));
  return { trigger: triggerOfType(trigger, d), steps: [stepOfType("action.take", d)] };
}

export { stepOfType, triggerOfType, type Defaults } from "./defaults";
export { BEFORE_MINUTES, DELAY_MINUTES, STATUS_OPTIONS, TAKE_FOR_MS } from "./pickers";
export { defaultName, nameError } from "./names";

// ---- Checks -------------------------------------------------------------------------------------

export interface Issue {
  level: "error" | "warning";
  message: string;
}

const err = (message: string): Issue => ({ level: "error", message });
const warn = (message: string): Issue => ({ level: "warning", message });

function layerIssue(c: { screenId: string; layerId: string }, k: Lookups): Issue | null {
  if (!c.screenId) return err("Choose a screen.");
  const screen = k.screens.get(c.screenId);
  if (!screen) return warn("This screen no longer exists; skipped.");
  if (!c.layerId) return err("Choose a layer.");
  if (!screen.layers.some((l) => l.id === c.layerId)) return warn("This layer no longer exists; skipped.");
  return null;
}

function sourceIssue(sourceId: string, k: Lookups, kind?: "text" | "schedule" | "api"): Issue | null {
  if (!sourceId) return err(kind === "schedule" ? "Choose a schedule." : kind === "api" ? "Choose an API source." : "Choose a source.");
  const s = k.sources.get(sourceId);
  if (!s) return warn("This source no longer exists; skipped.");
  if (kind === "api") return s.kind === "api" ? null : err("Choose an API source.");
  if (kind && (s.kind === "schedule") !== (kind === "schedule"))
    return err(kind === "schedule" ? "Choose a schedule." : "Choose a text source.");
  return null;
}

export function validClock(t: string): boolean {
  return /^([01]\d|2[0-3]):[0-5]\d$/.test(t);
}

/** A local date-time `YYYY-MM-DDTHH:MM` (schedule items' format) on a real calendar day. */
export function validLocal(t: string): boolean {
  const m = /^(\d{4})-(\d{2})-(\d{2})T([01]\d|2[0-3]):[0-5]\d$/.exec(t);
  if (!m) return false;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const date = new Date(Date.UTC(y, mo - 1, d));
  return date.getUTCFullYear() === y && date.getUTCMonth() === mo - 1 && date.getUTCDate() === d;
}

/** The first problem of `base` that is an error, else `extra`, else `base`'s warning. */
function then(base: Issue | null, extra: Issue | null): Issue | null {
  return base?.level === "error" ? base : (extra ?? base);
}

/** "/a/b", with * as a whole segment; no spaces, #, or empty segments. */
const oscAddressOk = (a: string) => /^(\/([^/\s#*,?[\]{}]+|\*))+$/.test(a);

export function triggerIssue(t: TriggerConf, k: Lookups): Issue | null {
  switch (t.type) {
    case "trigger.cue":
    case "trigger.runByFlow":
    case "trigger.rest":
      return null;
    case "trigger.midi": {
      const c = t.config;
      if (!Number.isInteger(c.number) || c.number < 0 || c.number > 127) return err("Number: 0 to 127.");
      return c.channel !== null && (!Number.isInteger(c.channel) || c.channel < 1 || c.channel > 16) ? err("Channel: 1 to 16.") : null;
    }
    case "trigger.osc":
      return oscAddressOk(t.config.address) ? null : err("Enter an address that starts with /, like /kinetra/go.");
    case "trigger.itemStatus":
      return sourceIssue(t.config.sourceId, k, "schedule");
    case "trigger.beforeNext": {
      const m = t.config.minutes;
      if (!Number.isInteger(m) || m < BEFORE_MINUTES.min || m > BEFORE_MINUTES.max)
        return err(`Minutes: ${BEFORE_MINUTES.min} to ${BEFORE_MINUTES.max}.`);
      return sourceIssue(t.config.sourceId, k, "schedule");
    }
    case "trigger.timeOfDay":
      if (!validClock(t.config.at)) return err("Enter a time as HH:MM.");
      return null;
    case "trigger.timerZero":
      return then(layerIssue(t.config, k), t.config.key ? null : err("Choose a timer field."));
    case "trigger.layer":
    case "trigger.mediaEnded":
      return layerIssue(t.config, k);
    case "trigger.slideshowWrap":
    case "trigger.sourceRow":
      return sourceIssue(t.config.sourceId, k, "text");
    case "trigger.apiChanged": {
      const base = sourceIssue(t.config.sourceId, k, "api");
      const c = t.config.column;
      const s = k.sources.get(t.config.sourceId);
      return then(base, c && s && !s.columns.some((x) => x.key === c) ? warn("This column no longer exists; it never changes.") : null);
    }
  }
}

export function stepIssue(s: StepConf, k: Lookups, selfId: string | null): Issue | null {
  switch (s.type) {
    case "action.take": {
      const f = s.config.forMs;
      if (f != null && (!Number.isInteger(f) || f < TAKE_FOR_MS.min || f > TAKE_FOR_MS.max))
        return err(`"For" is ${TAKE_FOR_MS.min / 1000} to ${TAKE_FOR_MS.max / 1000} seconds.`);
      return layerIssue(s.config, k);
    }
    case "action.clear":
      return layerIssue(s.config, k);
    case "action.clearScreen":
      if (!s.config.screenId) return err("Choose a screen.");
      return k.screens.has(s.config.screenId) ? null : warn("This screen no longer exists; skipped.");
    case "action.clearAll":
      return null;
    case "action.setFields":
      return then(
        layerIssue(s.config, k),
        Object.keys(s.config.fields).length === 0 ? err("Add at least one field.") : null,
      );
    case "action.setTimer":
      return then(layerIssue(s.config, k), s.config.key ? null : err("Choose a timer field."));
    case "action.stepSource": {
      const st = s.config.step;
      if (st.to === "by" ? !Number.isInteger(st.delta) || st.delta === 0 : !Number.isInteger(st.index) || st.index < 0)
        return err("Choose a valid step.");
      return sourceIssue(s.config.sourceId, k);
    }
    case "action.pauseSource":
      return sourceIssue(s.config.sourceId, k, "text");
    case "action.schedule": {
      const base = sourceIssue(s.config.sourceId, k, "schedule");
      if (s.config.item.pick === "item" && !s.config.item.itemId) return then(base, err("Choose an item."));
      const op = s.config.op;
      const start = op.op === "setStart" ? op.start : op.op === "setNext" ? (op.start ?? null) : null;
      if (op.op === "setStart" && !start) return then(base, err("Enter the new start date and time."));
      if (start !== null && !validLocal(start)) return then(base, err("Enter a date and time like 2026-10-04 09:30."));
      return base;
    }
    case "action.setVariant":
      return then(layerIssue(s.config, k), s.config.variant ? null : err("Choose a variant."));
    case "action.takePreview": {
      const t = s.config.transition;
      if (t && (!Number.isInteger(t.ms) || t.ms < TAKE_MS.min || t.ms > TAKE_MS.max))
        return err(`Duration: ${TAKE_MS.min} to ${TAKE_MS.max} ms.`);
      if (!s.config.screenId) return err("Choose a screen.");
      return k.screens.has(s.config.screenId) ? null : warn("This screen no longer exists; skipped.");
    }
    case "action.runFlow":
      if (!s.config.flowId) return err("Choose a flow.");
      if (s.config.flowId === selfId) return err("A flow can't run itself; use Repeat.");
      return k.flows.has(s.config.flowId) ? null : warn("This flow no longer exists; skipped.");
    case "action.stopFlow":
      if (!s.config.flowId) return err("Choose a flow.");
      if (s.config.flowId === selfId) return null;
      return k.flows.has(s.config.flowId) ? null : warn("This flow no longer exists; skipped.");
    case "action.log":
      return s.config.message.trim() ? null : err("Write a message.");
    case "action.media": {
      const ms = s.config.ms;
      if (s.config.action === "seek" && (ms == null || !Number.isFinite(ms) || ms < 0)) return then(layerIssue(s.config, k), err("Enter a time to go to."));
      return layerIssue(s.config, k);
    }
    case "logic.delay": {
      const ms = s.config.ms;
      return Number.isInteger(ms) && ms >= 0 && ms <= FLOW_DELAY_MAX_MS ? null : err("Wait up to 24 hours.");
    }
  }
}

export interface Check {
  errors: number;
  warnings: number;
}

export function check(name: string, l: LinearFlow, k: Lookups, selfId: string | null): Check {
  let errors = nameError(name) ? 1 : 0;
  let warnings = 0;
  if (l.steps.length + 1 > FLOW_NODES_MAX) errors++;
  for (const i of [triggerIssue(l.trigger, k), ...l.steps.map((s) => stepIssue(s, k, selfId))]) {
    if (i?.level === "error") errors++;
    else if (i) warnings++;
  }
  return { errors, warnings };
}

// ---- Requests -----------------------------------------------------------------------------------

/**
 * A new flow: cues are armed at once; automatic flows start disarmed (like recipes install them).
 * `origin`: the recipe instance it belongs to.
 */
export function toNewFlow(name: string, retrigger: Retrigger, l: LinearFlow, origin?: string): NewFlow {
  const flow: NewFlow = { name: name.trim(), armed: l.trigger.type === "trigger.cue", retrigger, ...fromLinear(l) };
  if (origin) flow.origin = origin;
  return flow;
}

/** The saved form of an existing flow (its wires keep their ids). */
export function toSaveFlow(
  flow: Pick<FlowInfo, "id" | "edges">,
  name: string,
  retrigger: Retrigger,
  l: LinearFlow,
): SaveFlow {
  return { id: flow.id, name: name.trim(), retrigger, ...fromLinear(l, flow.edges as FlowEdge[]) };
}
