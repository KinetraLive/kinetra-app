// MIDI in / OSC in flow triggers: new-node config, summary text, validation, ports, the palette entry
// and the MIDI fields (device, channel, Note / CC, number, Learn).
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MidiSource } from "$lib/types";

const m = vi.hoisted(() => ({
  handlers: new Map<string, (p: unknown) => void>(),
  midiLearn: vi.fn(async (_on: boolean) => {}),
}));
vi.mock("$lib/api/midi", () => ({
  midiLearn: m.midiLearn,
  listMidiDevices: async () => [{ name: "APC40", input: true, output: true, active: true }, { name: "Out", input: false, output: true, active: false }],
  onMidiLearn: (h: (p: unknown) => void) => {
    m.handlers.set("midi-learn", h);
    return () => m.handlers.delete("midi-learn");
  },
}));
vi.mock("$lib/api", () => ({ toAppError: (e: unknown) => ({ code: "internal", message: String(e) }), on: () => () => {} }));

import MidiTriggerFields from "$lib/components/midi/MidiTriggerFields.svelte";
import { triggerOfType } from "./defaults";
import { TRIGGER_OPTIONS, triggerIssue } from "./edit";
import { portsOf } from "./graph/ports";
import { NODES } from "./graph/nodes";
import { triggerText, type Lookups, type TriggerConf } from "./linear";

const D = { screenId: "s1", layerId: "l1", scheduleId: "", textId: "" };
const K: Lookups = { screens: new Map(), sources: new Map(), flows: new Map() };
const midi = (c: Partial<MidiSource> = {}): TriggerConf => ({ type: "trigger.midi", config: { device: null, channel: null, kind: "note", number: 36, ...c } });
const osc = (address: string): TriggerConf => ({ type: "trigger.osc", config: { address } });

afterEach(cleanup);

describe("MIDI in / OSC in", () => {
  it("start as Note 36 on any device and /kinetra/go", () => {
    expect(triggerOfType("trigger.midi", D)).toMatchObject(midi());
    expect(triggerOfType("trigger.osc", D)).toMatchObject(osc("/kinetra/go"));
  });

  it("read as a sentence", () => {
    expect(triggerText(midi(), K)).toBe("MIDI note 36");
    expect(triggerText(midi({ kind: "cc", number: 7, channel: 2, device: "APC40" }), K)).toBe("MIDI CC 7 on Ch 2 from APC40");
    expect(triggerText(osc("/show/*/go"), K)).toBe("OSC /show/*/go");
  });

  it("are checked", () => {
    expect(triggerIssue(midi(), K)).toBeNull();
    expect(triggerIssue(midi({ number: 128 }), K)?.level).toBe("error");
    expect(triggerIssue(midi({ channel: 17 }), K)?.message).toBe("Channel: 1 to 16.");
    expect(triggerIssue(osc("/kinetra/layer/*/take"), K)).toBeNull();
    for (const bad of ["", "kinetra/go", "/a//b", "/a b", "/a/"]) expect(triggerIssue(osc(bad), K)?.level).toBe("error");
  });

  it("have a value port: a number for MIDI, text for OSC", () => {
    expect(portsOf(midi()).outputs).toEqual([
      { name: "fired", kind: "event" },
      { name: "value", kind: "data", type: "number" },
    ]);
    expect(portsOf(osc("/a")).outputs.at(-1)).toEqual({ name: "value", kind: "data", type: "text" });
  });

  it("are in the palette and the linear editor's trigger list", () => {
    expect(NODES.find((n) => n.type === "trigger.midi")?.label).toBe("MIDI in");
    expect(NODES.find((n) => n.type === "trigger.osc")?.label).toBe("OSC in");
    expect(TRIGGER_OPTIONS.map((o) => o.value)).toEqual(expect.arrayContaining(["trigger.midi", "trigger.osc"]));
  });
});

describe("MidiTriggerFields", () => {
  it("edits device, channel, kind and number; Learn fills all four", async () => {
    const onchange = vi.fn();
    const config: MidiSource = { device: null, channel: null, kind: "note", number: 36 };
    render(MidiTriggerFields, { config, onchange });
    await tick();
    await tick();
    const device = screen.getByRole("combobox", { name: "MIDI device" }) as HTMLSelectElement;
    expect([...device.options].map((o) => o.text)).toEqual(["Any device", "APC40"]);
    await fireEvent.change(device, { target: { value: "APC40" } });
    expect(onchange).toHaveBeenLastCalledWith({ ...config, device: "APC40" });
    await fireEvent.change(screen.getByRole("combobox", { name: "MIDI channel" }), { target: { value: "10" } });
    expect(onchange).toHaveBeenLastCalledWith({ ...config, channel: 10 });
    await fireEvent.change(screen.getByRole("combobox", { name: "Note or CC" }), { target: { value: "cc" } });
    expect(onchange).toHaveBeenLastCalledWith({ ...config, kind: "cc" });

    await fireEvent.click(screen.getByRole("button", { name: "Learn" }));
    expect(screen.getByText("Press a key, pad or fader…")).toBeTruthy();
    await tick();
    expect(m.midiLearn).toHaveBeenCalledWith(true);
    m.handlers.get("midi-learn")?.({ device: "APC40", channel: 3, kind: "cc", number: 21, value: 64 });
    expect(onchange).toHaveBeenLastCalledWith({ device: "APC40", channel: 3, kind: "cc", number: 21 });
    await tick();
    expect(screen.queryByText("Press a key, pad or fader…")).toBeNull();
  });
});
