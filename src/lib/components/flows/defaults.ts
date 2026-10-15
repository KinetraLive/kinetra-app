// What a new trigger or action starts as, aimed at the given targets (or those of the node it
// replaces). Shared by the linear editor (./edit.ts re-exports it) and the node editor's catalogue.

import type { LinearStep, LinearTrigger, StepConf, StepType, TriggerConf, TriggerType } from "./linear";
import { newNodeId } from "./linear";

/** What new triggers and steps point at: Control's selected screen and its first layer. */
export interface Defaults {
  screenId: string;
  layerId: string;
  /** First schedule source, first text source ("" when none). */
  scheduleId: string;
  textId: string;
  /** First API source ("" or absent when none). */
  apiId?: string;
}

type Targets = { screenId: string; layerId: string; sourceId: string };

/** The targets of `from` (a schedule source only carries over to a schedule kind), else defaults. */
function targetsOf(from: { config: object } | undefined, fromSched: boolean, d: Defaults, sched: boolean): Targets {
  const c = (from?.config ?? {}) as Partial<Targets>;
  return {
    screenId: c.screenId ?? d.screenId,
    layerId: c.layerId ?? d.layerId,
    sourceId: (fromSched === sched ? c.sourceId : undefined) ?? (sched ? d.scheduleId : d.textId),
  };
}

const SCHEDULE_TYPES = new Set(["trigger.itemStatus", "trigger.beforeNext", "action.schedule"]);
const isSched = (type: string | undefined) => type !== undefined && SCHEDULE_TYPES.has(type);

/** A trigger of `type` (id kept), keeping the targets of `from` (or the defaults). */
export function triggerOfType(type: TriggerType, d: Defaults, from?: LinearTrigger): LinearTrigger {
  const id = from?.id ?? newNodeId();
  const t = targetsOf(from, isSched(from?.type), d, isSched(type));
  const make = (c: TriggerConf): LinearTrigger => ({ ...c, id });
  switch (type) {
    case "trigger.cue":
      return make({ type, config: { label: null } });
    case "trigger.itemStatus":
      return make({ type, config: { sourceId: t.sourceId, status: "now", itemId: null } });
    case "trigger.beforeNext":
      return make({ type, config: { sourceId: t.sourceId, minutes: 5 } });
    case "trigger.timeOfDay":
      return make({ type, config: { at: "09:00", days: [] } });
    case "trigger.timerZero":
      return make({ type, config: { screenId: t.screenId, layerId: t.layerId, key: "" } });
    case "trigger.layer":
      return make({ type, config: { screenId: t.screenId, layerId: t.layerId, onAir: true } });
    case "trigger.slideshowWrap":
      return make({ type, config: { sourceId: t.sourceId } });
    case "trigger.sourceRow":
      return make({ type, config: { sourceId: t.sourceId } });
    case "trigger.runByFlow":
    case "trigger.rest":
      return make({ type, config: {} });
    case "trigger.midi":
      return make({ type, config: { device: null, channel: null, kind: "note", number: 36 } });
    case "trigger.osc":
      return make({ type, config: { address: "/kinetra/go" } });
    case "trigger.mediaEnded":
      return make({ type, config: { screenId: t.screenId, layerId: t.layerId } });
    case "trigger.apiChanged":
      return make({
        type,
        config: { sourceId: from?.type === type ? from.config.sourceId : (d.apiId ?? ""), column: null },
      });
  }
}

/** A step of `type` (id kept), keeping the targets of `from` (or the defaults). */
export function stepOfType(type: StepType, d: Defaults, from?: LinearStep): LinearStep {
  const id = from?.id ?? newNodeId();
  const t = targetsOf(from, isSched(from?.type), d, isSched(type));
  const make = (c: StepConf): LinearStep => ({ ...c, id });
  const layer = { screenId: t.screenId, layerId: t.layerId };
  switch (type) {
    case "action.take":
      return make({ type, config: { ...layer, forMs: null } });
    case "action.clear":
      return make({ type, config: layer });
    case "action.clearScreen":
      return make({ type, config: { screenId: t.screenId } });
    case "action.clearAll":
      return make({ type, config: {} });
    case "action.setFields":
      return make({ type, config: { ...layer, fields: {} } });
    case "action.setTimer":
      return make({ type, config: { ...layer, key: "", action: { action: "start" } } });
    case "action.stepSource":
      return make({ type, config: { sourceId: t.sourceId, step: { to: "by", delta: 1 } } });
    case "action.pauseSource":
      return make({ type, config: { sourceId: t.sourceId, mode: "pause" } });
    case "action.schedule":
      return make({ type, config: { sourceId: t.sourceId, item: { pick: "now" }, op: { op: "end" } } });
    case "action.setVariant":
      return make({ type, config: { ...layer, variant: "", transition: "crossfade" } });
    case "action.takePreview":
      return make({ type, config: { screenId: t.screenId, transition: null } });
    case "action.runFlow":
      return make({ type, config: { flowId: "", wait: false } });
    case "action.stopFlow":
      return make({ type, config: { flowId: "" } });
    case "action.log":
      return make({ type, config: { message: "", toast: false } });
    case "action.media":
      return make({ type, config: { ...layer, action: "play", ms: null } });
    case "logic.delay":
      return make({ type, config: { ms: 1000 } });
  }
}
