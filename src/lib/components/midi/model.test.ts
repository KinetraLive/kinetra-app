// MIDI & OSC control, the pure part: source text, conflicts, chips, target names and options, edits.
import { describe, expect, it } from "vitest";
import type { ControlMapping, ControlsDoc, FlowInfo, LiveScreenView, MidiSource, SourceSummary } from "$lib/types";
import {
  addMapping,
  assign,
  chipKey,
  chips,
  command,
  conflicts,
  groupOf,
  hasLights,
  mappingId,
  overlaps,
  relearn,
  removeMapping,
  removeTarget,
  setLights,
  targetOptions,
  targetText,
  targetValue,
  type Lookups,
} from "./model";
import { conflictText, sourceText } from "./source";

const src = (p: Partial<MidiSource> = {}): MidiSource => ({ device: "APC40", channel: 1, kind: "note", number: 36, ...p });
const map = (id: string, source: MidiSource, commandId = `live.toggle:s1/${id}`): ControlMapping => ({
  id,
  source,
  target: command(commandId),
  feedback: true,
});

const screens = [
  {
    screen: { id: "s1", name: "Main", width: 1920, height: 1080, background: "transparent" },
    layers: [
      { id: "l1", name: "Guest", onAir: false, pending: false, fields: {}, fieldDefs: [{ key: "score", label: "Score", type: "number", default: "0" }] },
      { id: "l2", name: "Bug", onAir: true, pending: false, fields: {}, fieldDefs: [] },
    ],
  },
  { screen: { id: "test", name: "Output test", width: 1920, height: 1080, background: "transparent" }, layers: [] },
] as unknown as LiveScreenView[];
const flows = [
  { id: "f1", name: "Opening", nodes: [{ id: "n", type: "trigger.cue", config: { label: "Open" } }] },
  { id: "f2", name: "Rule", nodes: [{ id: "n", type: "trigger.timeOfDay", config: { at: "09:00", days: [] } }] },
] as unknown as FlowInfo[];
const sources = [
  { id: "src1", name: "Sponsors", kind: "text" },
  { id: "sch", name: "Run of show", kind: "schedule" },
] as unknown as SourceSummary[];
const k: Lookups = {
  screens,
  flows,
  sources,
  vars: new Map([["s1/l1", [{ key: "speed", label: "Speed", type: "range", min: 0, max: 10, step: 1, unit: "", default: "5" }]]]),
} as Lookups;

describe("source text", () => {
  it("reads like Note 36 · Ch 1 · APC40, with Any for null", () => {
    expect(sourceText(src())).toBe("Note 36 · Ch 1 · APC40");
    expect(sourceText(src({ kind: "cc", number: 7, channel: null, device: null }))).toBe("CC 7 · Any channel · Any device");
    expect(sourceText(src(), false)).toBe("Note 36 · Ch 1");
  });

  it("says a conflict ran nothing", () => {
    expect(conflictText(src(), 2)).toBe("Note 36 · Ch 1 runs nothing: it is assigned twice");
    expect(conflictText(src(), 3)).toBe("Note 36 · Ch 1 runs nothing: it is assigned 3 times");
  });
});

describe("conflicts", () => {
  it("overlap: same kind and number, device and channel equal or any", () => {
    expect(overlaps(src(), src())).toBe(true);
    expect(overlaps(src(), src({ device: null }))).toBe(true);
    expect(overlaps(src(), src({ channel: null }))).toBe(true);
    expect(overlaps(src(), src({ channel: 2 }))).toBe(false);
    expect(overlaps(src(), src({ device: "Launchpad" }))).toBe(false);
    expect(overlaps(src(), src({ kind: "cc" }))).toBe(false);
    expect(overlaps(src(), src({ number: 37 }))).toBe(false);
  });

  it("groups every mapping that overlaps another", () => {
    const list = [map("a", src()), map("b", src({ device: null })), map("c", src({ number: 40 })), map("d", src({ channel: null }))];
    expect(conflicts(list)).toEqual([{ source: src(), mappingIds: ["a", "b", "d"] }]);
    expect(conflicts([map("a", src()), map("c", src({ number: 40 }))])).toEqual([]);
  });
});

describe("targets", () => {
  it("names commands and values", () => {
    expect(targetText(command("live.toggle:s1/l1"), k)).toBe("Take or clear Guest (Main)");
    expect(targetText(command("live.take:s1/l2"), k)).toBe("Take Bug (Main)");
    expect(targetText(command("cue.go:f1"), k)).toBe("GO Open");
    expect(targetText(command("source.next:src1"), k)).toBe("Next in Sponsors");
    expect(targetText(command("live.clearScreen:s1"), k)).toBe("Clear Main");
    expect(targetText(command("live.panic"), k)).toBe("Panic");
    expect(targetText(command("live.toggle:s1/gone"), k)).toBe("Take or clear a removed layer");
    expect(targetText({ kind: "value", screenId: "s1", layerId: "l1", target: { kind: "opacity" } }, k)).toBe("Opacity of Guest (Main)");
    expect(targetText({ kind: "value", screenId: "s1", layerId: "l1", target: { kind: "var", key: "speed" } }, k)).toBe("Speed of Guest (Main)");
    expect(targetText({ kind: "value", screenId: "s1", layerId: "l1", target: { kind: "field", key: "score" } }, k)).toBe("Score of Guest (Main)");
  });

  it("offers layers, cues, sources, show and faders in that order", () => {
    const opts = targetOptions(k);
    const groups = [...new Set(opts.map((o) => o.group))];
    expect(groups).toEqual(["layers", "cues", "sources", "show", "faders"]);
    const values = opts.map((o) => o.value);
    // Front layer first; the test screen and rules (no Cue) and schedules are left out.
    expect(values.slice(0, 3)).toEqual(["live.toggle:s1/l2", "live.take:s1/l2", "live.clear:s1/l2"]);
    expect(values).toContain("cue.go:f1");
    expect(values).not.toContain("cue.go:f2");
    expect(values).toContain("source.prev:src1");
    expect(values.some((v) => v.includes("sch"))).toBe(false);
    expect(values).toContain("live.clearAll");
    expect(values).toContain("value:s1/l1/opacity");
    expect(values).toContain("value:s1/l1/var/speed");
    expect(values).toContain("value:s1/l1/field/score");
    expect(values.some((v) => v.includes("test"))).toBe(false);
  });

  it("groups, chips and lights by target", () => {
    expect(groupOf(command("live.update:s1/l1"))).toBe("layers");
    expect(groupOf(command("live.clearAll"))).toBe("show");
    expect(chipKey(command("live.take:s1/l1"))).toBe("s1/l1");
    expect(chipKey(command("cue.go:f1"))).toBe("flow/f1");
    expect(chipKey(command("live.panic"))).toBeNull();
    expect(hasLights(command("live.panic"))).toBe(false);
    expect(hasLights(command("cue.go:f1"))).toBe(true);
    expect(chips([map("a", src(), "live.toggle:s1/l1"), map("b", src({ number: 1 }), "cue.go:f1")])).toEqual(
      new Map([
        ["s1/l1", ["Note 36 · Ch 1 · APC40"]],
        ["flow/f1", ["Note 1 · Ch 1 · APC40"]],
      ]),
    );
  });
});

describe("edits", () => {
  const doc: ControlsDoc = { mappings: [map("a", src(), "live.toggle:s1/l1"), map("b", src({ number: 2 }), "cue.go:f1")] };

  it("assign replaces the target's source in place; lights on for layers and cues", () => {
    const next = assign(doc, command("live.toggle:s1/l1"), src({ number: 50 }));
    expect(next.mappings.map((m) => [m.id, m.source.number])).toEqual([
      ["a", 50],
      ["b", 2],
    ]);
    const added = assign(doc, command("live.panic"), src({ number: 9 }));
    expect(added.mappings).toHaveLength(3);
    expect(added.mappings[2]).toMatchObject({ target: command("live.panic"), feedback: false });
    expect(added.mappings[2].id).toMatch(/^[0-9a-z]{10}$/);
  });

  it("adds, relearns, removes and switches lights", () => {
    const fader = { kind: "value" as const, screenId: "s1", layerId: "l1", target: { kind: "opacity" as const } };
    const added = addMapping(doc, fader, src({ kind: "cc", number: 7 }));
    expect(added.mappings[2]).toMatchObject({ target: fader, feedback: false });
    expect(relearn(doc, "b", src({ number: 3 })).mappings[1].source.number).toBe(3);
    expect(removeMapping(doc, "a").mappings.map((m) => m.id)).toEqual(["b"]);
    expect(removeTarget(doc, command("cue.go:f1")).mappings.map((m) => m.id)).toEqual(["a"]);
    expect(setLights(doc, "a", false).mappings[0].feedback).toBe(false);
    expect(targetValue(fader)).toBe("value:s1/l1/opacity");
    expect(mappingId()).not.toBe(mappingId());
  });
});
