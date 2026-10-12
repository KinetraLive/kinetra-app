// MIDI & OSC control, the pure part: what a mapping's target is called, the targets a new mapping
// can pick, conflicts (two mappings on overlapping sources run nothing), the MIDI chips Control
// shows, and the edits Learn / Remove / Lights make to the document. No Svelte, no I/O.

import type {
  ControlMapping,
  ControlTarget,
  ControlsDoc,
  FlowInfo,
  LiveScreenView,
  MidiLearned,
  MidiSource,
  SourceSummary,
  VariableDef,
} from "$lib/types";
import { TEST_SCREEN_ID } from "$lib/types";
import { cueText } from "$lib/components/flows/cue";
import { sourceText } from "./source";

export { sourceText };

/** What target names are looked up in. `vars`: layer key "<screen>/<layer>" → its template's
 * variables (only range variables are offered to faders); absent = not loaded. */
export interface Lookups {
  screens: readonly LiveScreenView[];
  flows: readonly FlowInfo[];
  sources: readonly SourceSummary[];
  vars?: ReadonlyMap<string, readonly VariableDef[]>;
}

export type TargetGroup = "layers" | "cues" | "sources" | "show" | "faders";

export const GROUP_ORDER: readonly TargetGroup[] = ["layers", "cues", "sources", "show", "faders"];
export const GROUP_LABEL: Record<TargetGroup, string> = {
  layers: "Layers",
  cues: "Cues",
  sources: "Slides and rows",
  show: "Show",
  faders: "Faders and knobs",
};

export interface TargetOption {
  /** `targetValue(target)`: unique per target. */
  value: string;
  label: string;
  group: TargetGroup;
  target: ControlTarget;
}

const isUserScreen = (id: string) => id !== TEST_SCREEN_ID && !id.startsWith("__");

/** A stable string per target (Select values, "already assigned" checks). */
export function targetValue(t: ControlTarget): string {
  if (t.kind === "command") return t.commandId;
  const what = t.target.kind === "opacity" ? "opacity" : `${t.target.kind}/${t.target.key}`;
  return `value:${t.screenId}/${t.layerId}/${what}`;
}

export const command = (commandId: string): ControlTarget => ({ kind: "command", commandId });

/** The group a target is listed under. */
export function groupOf(t: ControlTarget): TargetGroup {
  if (t.kind === "value") return "faders";
  const id = t.commandId;
  if (id.startsWith("cue.go:")) return "cues";
  if (id.startsWith("source.")) return "sources";
  if (/^(live\.(toggle|take|clear|update)|media\.(play|pause|stop)):/.test(id)) return "layers";
  return "show";
}

/** The layer ("<screen>/<layer>") or cue ("flow/<id>") a target acts on, for Control's chips. */
export function chipKey(t: ControlTarget): string | null {
  if (t.kind === "value") return null;
  const m = /^(?:live\.(?:toggle|take|clear|update)|media\.(?:play|pause|stop)):(.+\/.+)$/.exec(t.commandId);
  if (m) return m[1];
  return t.commandId.startsWith("cue.go:") ? `flow/${t.commandId.slice(7)}` : null;
}

/** Lights (LED feedback) only mean something on layers and cues. */
export const hasLights = (t: ControlTarget): boolean => chipKey(t) !== null;

function names(k: Lookups) {
  const screen = (s: string) => k.screens.find((v) => v.screen.id === s);
  return {
    screen: (s: string) => screen(s)?.screen.name ?? "a removed screen",
    layer: (s: string, l: string) => {
      const v = screen(s);
      const layer = v?.layers.find((x) => x.id === l);
      return layer && v ? `${layer.name} (${v.screen.name})` : "a removed layer";
    },
    cue: (f: string) => {
      const flow = k.flows.find((x) => x.id === f);
      return flow ? (cueText(flow) ?? flow.name) : "a removed cue";
    },
    source: (id: string) => k.sources.find((x) => x.id === id)?.name ?? "a removed source",
  };
}

/** What a target does, e.g. "Take or clear Guest (Main program)", "Opacity of Bug (Main)". */
export function targetText(t: ControlTarget, k: Lookups): string {
  const n = names(k);
  if (t.kind === "value") {
    const layer = n.layer(t.screenId, t.layerId);
    if (t.target.kind === "opacity") return `Opacity of ${layer}`;
    const key = t.target.key;
    let label = key;
    if (t.target.kind === "var") label = k.vars?.get(`${t.screenId}/${t.layerId}`)?.find((v) => v.key === key)?.label || key;
    else {
      const v = k.screens.find((x) => x.screen.id === t.screenId);
      label = v?.layers.find((x) => x.id === t.layerId)?.fieldDefs.find((f) => f.key === key)?.label || key;
    }
    return `${label} of ${layer}`;
  }
  const id = t.commandId;
  const colon = id.indexOf(":");
  const verb = colon < 0 ? id : id.slice(0, colon);
  const arg = colon < 0 ? "" : id.slice(colon + 1);
  const [s, l] = arg.split("/");
  switch (verb) {
    case "live.toggle":
      return `Take or clear ${n.layer(s, l)}`;
    case "live.take":
      return `Take ${n.layer(s, l)}`;
    case "live.clear":
      return `Clear ${n.layer(s, l)}`;
    case "live.update":
      return `Update ${n.layer(s, l)}`;
    case "media.play":
      return `Play ${n.layer(s, l)}`;
    case "media.pause":
      return `Pause ${n.layer(s, l)}`;
    case "media.stop":
      return `Stop ${n.layer(s, l)}`;
    case "cue.go":
      return `GO ${n.cue(arg)}`;
    case "source.next":
      return `Next in ${n.source(arg)}`;
    case "source.prev":
      return `Previous in ${n.source(arg)}`;
    case "live.clearScreen":
      return `Clear ${n.screen(arg)}`;
    case "live.clearAll":
      return "Clear all";
    case "live.panic":
      return "Panic";
    default:
      return id;
  }
}

/** Every target a new mapping can pick, grouped (GROUP_ORDER) and named. */
export function targetOptions(k: Lookups): TargetOption[] {
  const out: TargetOption[] = [];
  const add = (target: ControlTarget) => out.push({ value: targetValue(target), label: targetText(target, k), group: groupOf(target), target });
  const screens = k.screens.filter((v) => isUserScreen(v.screen.id));
  for (const v of screens) {
    for (const l of [...v.layers].reverse()) {
      const key = `${v.screen.id}/${l.id}`;
      for (const verb of ["toggle", "take", "clear"]) add(command(`live.${verb}:${key}`));
      if (l.media) for (const verb of ["play", "pause", "stop"]) add(command(`media.${verb}:${key}`));
    }
  }
  for (const f of k.flows) if (cueText(f) !== null) add(command(`cue.go:${f.id}`));
  for (const s of k.sources) {
    if (s.kind === "schedule") continue;
    add(command(`source.next:${s.id}`));
    add(command(`source.prev:${s.id}`));
  }
  for (const v of screens) add(command(`live.clearScreen:${v.screen.id}`));
  add(command("live.clearAll"));
  add(command("live.panic"));
  for (const v of screens) {
    for (const l of [...v.layers].reverse()) {
      const base = { kind: "value" as const, screenId: v.screen.id, layerId: l.id };
      add({ ...base, target: { kind: "opacity" } });
      for (const d of k.vars?.get(`${v.screen.id}/${l.id}`) ?? []) {
        if (d.type === "range") add({ ...base, target: { kind: "var", key: d.key } });
      }
      for (const f of l.fieldDefs) if (f.type === "number") add({ ...base, target: { kind: "field", key: f.key } });
    }
  }
  return GROUP_ORDER.flatMap((g) => out.filter((o) => o.group === g));
}

// ---- Sources and conflicts ----------------------------------------------------------------------

/** A captured message as a mapping source (this device, this channel). */
export const sourceOf = (l: MidiLearned): MidiSource => ({ device: l.device, channel: l.channel, kind: l.kind, number: l.number });

const anyOrSame = <T>(a: T | null, b: T | null) => a === null || b === null || a === b;

/** Two sources one message can match (null device / channel = any). */
export function overlaps(a: MidiSource, b: MidiSource): boolean {
  return a.kind === b.kind && a.number === b.number && anyOrSame(a.device, b.device) && anyOrSame(a.channel, b.channel);
}

export interface Conflict {
  /** The first mapping's source (what the panel names). */
  source: MidiSource;
  mappingIds: string[];
}

/** Groups of mappings whose sources overlap (each group: every mapping that overlaps another in it). */
export function conflicts(mappings: readonly ControlMapping[]): Conflict[] {
  const out: Conflict[] = [];
  const done = new Set<string>();
  for (const m of mappings) {
    if (done.has(m.id)) continue;
    const group = [m];
    for (let i = 0; i < group.length; i++) {
      for (const o of mappings) {
        if (!group.includes(o) && !done.has(o.id) && overlaps(group[i].source, o.source)) group.push(o);
      }
    }
    if (group.length < 2) continue;
    for (const g of group) done.add(g.id);
    out.push({ source: m.source, mappingIds: group.map((g) => g.id) });
  }
  return out;
}

/** Mapping id → its conflict. */
export function conflictById(list: readonly Conflict[]): Map<string, Conflict> {
  const out = new Map<string, Conflict>();
  for (const c of list) for (const id of c.mappingIds) out.set(id, c);
  return out;
}

/** Control's chips: layer key / "flow/<id>" → the sources assigned to it ("Note 36 · Ch 1 · APC40"). */
export function chips(mappings: readonly ControlMapping[]): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const m of mappings) {
    const k = chipKey(m.target);
    if (!k) continue;
    const text = sourceText(m.source);
    const at = out.get(k);
    if (at) at.push(text);
    else out.set(k, [text]);
  }
  return out;
}

// ---- Edits -------------------------------------------------------------------------------------

/** A new mapping id (10 characters, like the core's nanoid). */
export function mappingId(): string {
  const abc = "0123456789abcdefghijklmnopqrstuvwxyz";
  const bytes = new Uint8Array(10);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => abc[b % abc.length]).join("");
}

/** Assigns `source` to `target`: mappings of the same target are replaced (one source each from
 * Assign MIDI…); lights on for layers and cues. */
export function assign(doc: ControlsDoc, target: ControlTarget, source: MidiSource): ControlsDoc {
  const v = targetValue(target);
  const prev = doc.mappings.find((m) => targetValue(m.target) === v);
  const mapping: ControlMapping = { id: prev?.id ?? mappingId(), source, target, feedback: prev?.feedback ?? hasLights(target) };
  if (!prev) return { ...doc, mappings: [...doc.mappings, mapping] };
  // In the first one's place; the others of the target go.
  return { ...doc, mappings: doc.mappings.flatMap((m) => (m === prev ? [mapping] : targetValue(m.target) === v ? [] : [m])) };
}

/** A new mapping for `target` (another source for the same action is allowed here). */
export function addMapping(doc: ControlsDoc, target: ControlTarget, source: MidiSource): ControlsDoc {
  return { ...doc, mappings: [...doc.mappings, { id: mappingId(), source, target, feedback: hasLights(target) }] };
}

/** A mapping with a new source (Learn on its row). */
export function relearn(doc: ControlsDoc, id: string, source: MidiSource): ControlsDoc {
  return { ...doc, mappings: doc.mappings.map((m) => (m.id === id ? { ...m, source } : m)) };
}

export function removeMapping(doc: ControlsDoc, id: string): ControlsDoc {
  return { ...doc, mappings: doc.mappings.filter((m) => m.id !== id) };
}

/** Removes every mapping of a target (Remove MIDI from a context menu). */
export function removeTarget(doc: ControlsDoc, target: ControlTarget): ControlsDoc {
  const v = targetValue(target);
  return { ...doc, mappings: doc.mappings.filter((m) => targetValue(m.target) !== v) };
}

export function setLights(doc: ControlsDoc, id: string, feedback: boolean): ControlsDoc {
  return { ...doc, mappings: doc.mappings.map((m) => (m.id === id ? { ...m, feedback } : m)) };
}

/** The mappings of a target. */
export const mappingsOf = (doc: ControlsDoc | null, target: ControlTarget): ControlMapping[] => {
  const v = targetValue(target);
  return doc?.mappings.filter((m) => targetValue(m.target) === v) ?? [];
};
