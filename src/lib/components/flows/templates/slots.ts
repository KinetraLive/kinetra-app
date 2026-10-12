// Flow templates (lazy chunk): the gallery's groups and search, the arm note, and the "Point it
// at" form's options and smart defaults per slot. Plain functions, no Svelte.
//
// Slot values (AddFlowTemplate.slots): a screen, source or flow id; a layer "screenId/layerId"; a
// timer or field "screenId/layerId/key".

import type { FlowInfo, FlowSlot, FlowTemplateCategory, FlowTemplateInfo, LiveScreenView, SourceSummary } from "$lib/types";

export const CATEGORIES: { id: FlowTemplateCategory; label: string }[] = [
  { id: "show", label: "Show running" },
  { id: "schedule", label: "Event schedule" },
  { id: "sponsors", label: "Sponsors" },
  { id: "sports", label: "Sports & LAN" },
  { id: "blocks", label: "Building blocks" },
];

export type Group = { id: FlowTemplateCategory; label: string; items: FlowTemplateInfo[] };

/** The gallery in category order (gallery order within one), filtered by every word of `query`
 *  (name, description, chain or category). Empty groups are left out. */
export function groupTemplates(list: readonly FlowTemplateInfo[], query: string): Group[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return CATEGORIES.map((c) => ({
    ...c,
    items: list.filter((t) => {
      if (t.category !== c.id) return false;
      const hay = `${t.name} ${t.description} ${t.chain} ${c.label}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    }),
  })).filter((g) => g.items.length > 0);
}

/**
 * A flow is added armed exactly when it has no automatic trigger (Cue and Run by flow are on
 * demand). The chain shows only the first flow, so which templates hold automatic flows is listed
 * here: these have on-demand flows only, these mix both, every other one starts disarmed.
 */
const ON_DEMAND = new Set(["show-open", "next-guest", "stinger-once", "random-fact", "clean-all-log", "reusable-lower-third"]);
const MIXED = new Set(["break-countdown", "manual-hold"]);

export type Arming = "armed" | "mixed" | "disarmed";

export function armingOf(id: string): Arming {
  return ON_DEMAND.has(id) ? "armed" : MIXED.has(id) ? "mixed" : "disarmed";
}

/** The card's hint: null when everything is added armed. */
export function armHint(id: string): string | null {
  const a = armingOf(id);
  return a === "disarmed" ? "Starts disarmed" : a === "mixed" ? "Partly disarmed" : null;
}

/** The form's note on what starts disarmed (`name` is the flow name being added). */
export function armNote(t: Pick<FlowTemplateInfo, "id" | "flows">, name: string): string | null {
  const q = (suffix: string) => `“${name} · ${suffix}”`;
  switch (t.id) {
    case "break-countdown":
      return `${q("start")} is added armed. ${q("end")} starts disarmed: the show graphic doesn't return at zero until you arm it.`;
    case "manual-hold":
      return `${q("take")} starts disarmed. ${q("hold")} and ${q("release")} are armed, but do nothing until ${q("take")} is armed.`;
  }
  const a = armingOf(t.id);
  if (a === "armed") return null;
  if (a === "mixed") return "Flows with an automatic trigger start disarmed; the others are armed.";
  return t.flows > 1
    ? "Its flows start disarmed, so nothing goes on air until you arm them."
    : "It starts disarmed, so nothing goes on air until you arm it.";
}

/** What the form picks from: the project's screens (with layers and field definitions), sources,
 *  flows, and which preset each layer's template came from ("screenId/layerId" → preset id). */
export interface Targets {
  screens: readonly LiveScreenView[];
  sources: readonly SourceSummary[];
  flows: readonly FlowInfo[];
  presets: ReadonlyMap<string, string>;
}

export const NO_TARGETS: Targets = { screens: [], sources: [], flows: [], presets: new Map() };

export type SlotOpt = { value: string; label: string; preferred: boolean };

/** The choices for a slot, in project order (screens in order, layers top first). */
export function slotOptions(slot: FlowSlot, t: Targets): SlotOpt[] {
  const want = slot.prefer?.template ?? null;
  const prefers = (screenId: string, layerId: string) => want !== null && t.presets.get(`${screenId}/${layerId}`) === want;
  const out: SlotOpt[] = [];
  switch (slot.kind) {
    case "screen":
      for (const v of t.screens) {
        const preferred = v.layers.some((l) => prefers(v.screen.id, l.id));
        out.push({ value: v.screen.id, label: v.screen.name, preferred });
      }
      break;
    case "layer":
    case "timer":
    case "field":
      for (const v of t.screens) {
        for (const l of [...v.layers].reverse()) {
          const preferred = prefers(v.screen.id, l.id);
          const base = `${v.screen.name} · ${l.name}`;
          if (slot.kind === "layer") {
            out.push({ value: `${v.screen.id}/${l.id}`, label: base, preferred });
            continue;
          }
          for (const f of l.fieldDefs) {
            if (slot.kind === "timer" && f.type !== "timer") continue;
            out.push({ value: `${v.screen.id}/${l.id}/${f.key}`, label: `${base} · ${f.label || f.key}`, preferred });
          }
        }
      }
      break;
    case "textSource":
    case "scheduleSource":
      for (const s of t.sources) {
        if ((s.kind === "schedule") !== (slot.kind === "scheduleSource")) continue;
        out.push({ value: s.id, label: s.name, preferred: false });
      }
      break;
    case "flow":
      for (const f of t.flows) out.push({ value: f.id, label: f.name, preferred: false });
      break;
  }
  return out;
}

/** The smart default: a preferred choice (the slot's preset template), else the first choice for
 *  a required slot; an optional slot without a preferred choice stays empty. */
export function slotDefault(slot: FlowSlot, t: Targets): string {
  const opts = slotOptions(slot, t);
  const preferred = opts.find((o) => o.preferred);
  if (preferred) return preferred.value;
  return slot.optional ? "" : (opts[0]?.value ?? "");
}

export function slotDefaults(slots: readonly FlowSlot[], t: Targets): Record<string, string> {
  return Object.fromEntries(slots.map((s) => [s.key, slotDefault(s, t)]));
}

/** Required slots without a value. */
export function missingSlots(slots: readonly FlowSlot[], values: Readonly<Record<string, string>>): FlowSlot[] {
  return slots.filter((s) => !s.optional && !values[s.key]);
}

/** What `add_flow_template` gets: filled slots only. */
export function filledSlots(slots: readonly FlowSlot[], values: Readonly<Record<string, string>>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const s of slots) if (values[s.key]) out[s.key] = values[s.key];
  return out;
}

const NOUN: Record<FlowSlot["kind"], string> = {
  screen: "screens",
  layer: "layers",
  timer: "timer fields",
  field: "fields",
  textSource: "text sources",
  scheduleSource: "schedules",
  flow: "flows",
};

/** The empty choice of a slot's picker. */
export function emptyLabel(slot: FlowSlot, count: number): string {
  if (count === 0) return `No ${NOUN[slot.kind]} in this project`;
  return slot.optional ? "None" : "Choose…";
}
