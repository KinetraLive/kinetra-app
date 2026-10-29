// The node palette of the flow editor (the port table is `core/flow_nodes.rs`): categories, the palette
// order and each node type's short description. Split from ./catalog.ts so the editor route's own
// chunk doesn't carry the descriptions: only the lazy palette and inspector import this.

import type { NodeType } from "$lib/types";
import { NODE_LABELS } from "./labels";
import type { Category } from "./ports";

export interface NodeInfo {
  type: NodeType;
  label: string;
  description: string;
}

export const CATEGORIES: { id: Category; label: string }[] = [
  { id: "trigger", label: "Triggers" },
  { id: "data", label: "Data" },
  { id: "logic", label: "Logic" },
  { id: "action", label: "Actions" },
];

/** Palette order within each category (labels: ./labels.ts). */
export const NODES: readonly NodeInfo[] = (
  [
    ["trigger.cue", "A button and hotkey in Control."],
    ["trigger.itemStatus", "An item becomes NOW, UP NEXT, DELAYED…"],
    ["trigger.beforeNext", "N minutes before the next item starts."],
    ["trigger.timeOfDay", "At HH:MM, on chosen days."],
    ["trigger.timerZero", "A layer's countdown ends."],
    ["trigger.layer", "A layer goes on or off air."],
    ["trigger.slideshowWrap", "A slideshow completes a cycle."],
    ["trigger.sourceRow", "The current row of a source changes."],
    ["trigger.runByFlow", "Another flow's Run flow starts this one."],
    ["trigger.rest", "Companion or a script runs this flow."],
    ["trigger.apiChanged", "An API or live data source's data changes."],
    ["trigger.midi", "A key, pad or knob on a MIDI controller."],
    ["trigger.osc", "A message from a control surface or app."],
    ["trigger.mediaEnded", "A media layer reaches its out point."],
    ["data.sourceRow", "A row of a source: its columns as values."],
    ["data.scheduleItem", "The NOW or UP NEXT item's details."],
    ["data.fieldValue", "A layer's current field value."],
    ["data.layerState", "Whether a layer is on air or has staged changes."],
    ["data.constant", "A fixed text, number or yes/no."],
    ["data.timeNow", "The server clock."],
    ["data.format", "A pattern with {a}…{h} placeholders."],
    ["data.math", "Add, subtract, multiply, divide, min, max, round."],
    ["data.compare", "Equal, less than, contains, is empty…"],
    ["logic.branch", "Then or else, by a condition."],
    ["logic.switch", "One output per matching value."],
    ["logic.gate", "Lets events through while open."],
    ["logic.delay", "Waits, then continues."],
    ["logic.waitUntil", "Waits for a time of day or a release."],
    ["logic.debounce", "Continues after a quiet period."],
    ["logic.throttle", "At most once per period."],
    ["logic.sequence", "Runs its outputs in order."],
    ["logic.once", "Only the first time, until reset."],
    ["logic.counter", "Every N events."],
    ["logic.random", "One output at random."],
    ["logic.repeat", "Runs its body again and again."],
    ["action.take", "Takes a layer on air."],
    ["action.clear", "Takes a layer off air."],
    ["action.clearScreen", "Takes every layer of a screen off air."],
    ["action.clearAll", "Takes everything off air (animated)."],
    ["action.setFields", "Sets field values of a layer."],
    ["action.setTimer", "Starts, pauses, resets or sets a timer."],
    ["action.stepSource", "Next, previous or a given row."],
    ["action.pauseSource", "Pauses or resumes auto-advance."],
    ["action.schedule", "Start, end, delay or move an item."],
    ["action.setVariant", "Shows another look of a layer."],
    ["action.media", "Plays, pauses, stops or seeks a media layer."],
    ["action.takePreview", "Takes a screen's Preview to Program (Studio mode)."],
    ["action.runFlow", "Runs another flow."],
    ["action.stopFlow", "Stops a running flow (this one too)."],
    ["action.log", "Writes a message to the flow's log."],
  ] as const
).map(([type, description]) => ({ type, label: NODE_LABELS[type], description }));

const BY_TYPE = new Map(NODES.map((n) => [n.type, n]));

export const catalogInfo = (type: string): NodeInfo | undefined => BY_TYPE.get(type as NodeType);

