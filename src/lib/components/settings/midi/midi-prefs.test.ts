// Preferences > MIDI & OSC (devices: rows, outputs, prefs edits; the Devices list following
// `midi-devices`) and Server > OSC (allowlist, addresses, feedback target).
import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MidiDevice, MidiPrefs, Prefs } from "$lib/types";

const midi = vi.hoisted(() => ({
  handlers: new Map<string, (p: unknown) => void>(),
  listMidiDevices: vi.fn(),
  testMidiFeedback: vi.fn(async () => {}),
  watchMidiDevices: vi.fn(async () => {}),
}));
vi.mock("$lib/api/midi", () => ({
  listMidiDevices: midi.listMidiDevices,
  testMidiFeedback: midi.testMidiFeedback,
  watchMidiDevices: midi.watchMidiDevices,
  onMidiDevices: (h: (p: unknown) => void) => {
    midi.handlers.set("midi-devices", h);
    return () => midi.handlers.delete("midi-devices");
  },
}));
const api = vi.hoisted(() => ({ getPrefs: vi.fn(), setPrefs: vi.fn(async (p: { prefs: unknown }) => p) }));
vi.mock("$lib/api", () => ({
  getPrefs: api.getPrefs,
  setPrefs: (p: unknown) => api.setPrefs({ prefs: p }).then((x) => x.prefs),
  toAppError: (e: unknown) => ({ code: "internal", message: String(e) }),
  on: () => () => {},
}));

import { prefs } from "$lib/stores/prefs.svelte";
import Devices from "./Devices.svelte";
import MidiSettings from "./MidiSettings.svelte";
import { deviceRows, outputOptions, withFeedback, withInput } from "./devices";
import { isIp, oscAddresses, parseTarget, targetString, withAllowed } from "./osc";

const dev = (name: string, p: Partial<MidiDevice> = {}): MidiDevice => ({ name, input: true, output: true, active: true, ...p });
const NONE: MidiPrefs = { inputs: {}, feedback: {} };

describe("MIDI on macOS", () => {
  it("works as on Windows: the devices are watched, no notice", async () => {
    const ua = vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/605.1.15 (KHTML, like Gecko)");
    midi.listMidiDevices.mockResolvedValue([]);
    try {
      render(MidiSettings);
      await tick();
      await vi.waitFor(() => expect(midi.watchMidiDevices).toHaveBeenLastCalledWith(true));
      expect(screen.queryByText(/not available on macOS/)).toBeNull();
    } finally {
      ua.mockRestore();
      cleanup();
    }
  });
});

describe("devices model", () => {
  it("lists inputs now, then remembered ones as not connected", () => {
    const rows = deviceRows([dev("APC40"), dev("Busy pad", { active: false }), dev("Out only", { input: false })], {
      inputs: { Old: { enabled: false }, "Busy pad": { enabled: true } },
      feedback: { Gone: { output: null, profile: "apc40" } },
    });
    expect(rows).toEqual([
      { name: "APC40", enabled: true, status: "connected" },
      { name: "Busy pad", enabled: true, status: "busy" },
      { name: "Gone", enabled: true, status: "missing" },
      { name: "Old", enabled: false, status: "missing" },
    ]);
    // Switched off: not opened on purpose, so not "busy".
    expect(deviceRows([dev("X", { active: false })], withInput(NONE, "X", false))[0].status).toBe("connected");
  });

  it("offers no lights, the device itself, other outputs and a missing saved one", () => {
    const devices = [dev("APC40"), dev("Launchpad"), dev("Keys", { output: false })];
    expect(outputOptions(devices, "APC40").map((o) => o.value)).toEqual(["off", "", "Launchpad"]);
    expect(outputOptions(devices, "Keys").map((o) => o.value)).toEqual(["off", "APC40", "Launchpad"]);
    expect(outputOptions(devices, "Keys", "Old out").map((o) => o.label)).toContain("Old out (not connected)");
  });

  it("edits prefs", () => {
    const on = withFeedback(NONE, "APC40", { output: null, profile: "apc40" });
    expect(on.feedback.APC40).toEqual({ output: null, profile: "apc40" });
    expect(withFeedback(on, "APC40", null).feedback).toEqual({});
    expect(withInput(NONE, "APC40", false).inputs).toEqual({ APC40: { enabled: false } });
  });
});

describe("Devices", () => {
  const base: Prefs = { theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 } };
  beforeEach(() => {
    midi.handlers.clear();
    prefs.set(base);
  });
  afterEach(cleanup);

  it("shows status, follows hot-plug and saves the switch, lights and Test lights", async () => {
    midi.listMidiDevices.mockResolvedValue([dev("APC40 mkII")]);
    const view = render(Devices);
    const row = await screen.findByText("APC40 mkII", { selector: ".title" });
    // Ports open and hot-plug is watched only while the list is shown.
    expect(midi.watchMidiDevices).toHaveBeenLastCalledWith(true);
    const li = row.closest("li") as HTMLElement;
    expect(within(li).getByText("Connected")).toBeTruthy();
    expect(within(li).getByRole("button", { name: "Test lights" }).hasAttribute("disabled")).toBe(true);

    // Lights on: the device's own output, Generic colours.
    const fb = within(li).getByLabelText("LED feedback of APC40 mkII") as HTMLSelectElement;
    await fireEvent.change(fb, { target: { value: "" } });
    await vi.waitFor(() => expect(api.setPrefs).toHaveBeenCalled());
    expect(prefs.value?.midi?.feedback["APC40 mkII"]).toEqual({ output: null, profile: "generic" });
    await tick();
    await fireEvent.click(within(li).getByRole("button", { name: "Test lights" }));
    expect(midi.testMidiFeedback).toHaveBeenCalledWith("APC40 mkII");

    await fireEvent.click(within(li).getByRole("switch", { name: "Use APC40 mkII" }));
    expect(prefs.value?.midi?.inputs["APC40 mkII"]).toEqual({ enabled: false });

    // Unplugged, and a new one plugged in.
    midi.handlers.get("midi-devices")?.([dev("Launchpad X")]);
    await tick();
    expect(screen.getByText("Launchpad X", { selector: ".title" })).toBeTruthy();
    const gone = screen.getByText("APC40 mkII", { selector: ".title" }).closest("li") as HTMLElement;
    expect(within(gone).getByText("Not connected")).toBeTruthy();

    view.unmount();
    expect(midi.watchMidiDevices).toHaveBeenLastCalledWith(false);
  });

  it("without devices: says to connect one", async () => {
    midi.listMidiDevices.mockResolvedValue([]);
    render(Devices);
    expect(await screen.findByText("Connect a MIDI controller. It shows up here at once.")).toBeTruthy();
  });
});

describe("OSC model", () => {
  it("accepts IP literals for the allowlist", () => {
    expect(isIp("192.168.1.20")).toBe(true);
    expect(isIp("::1")).toBe(true);
    expect(isIp("fe80::1")).toBe(true);
    expect(isIp("256.1.1.1")).toBe(false);
    expect(isIp("studio-pc")).toBe(false);
    expect(withAllowed(["10.0.0.1"], " 10.0.0.2 ")).toEqual(["10.0.0.1", "10.0.0.2"]);
    expect(withAllowed(["10.0.0.1"], "10.0.0.1")).toEqual(["10.0.0.1"]);
    expect(withAllowed([], "nope")).toBeNull();
  });

  it("lists the addresses with the screen AND layer id", () => {
    const list = oscAddresses({ screenId: "s1", layerId: "l1", flowId: "f1", sourceId: null });
    expect(list.map((a) => a.address)).toEqual([
      "/kinetra/layer/s1/l1/take",
      "/kinetra/layer/s1/l1/clear",
      "/kinetra/layer/s1/l1/toggle",
      "/kinetra/layer/s1/l1/update",
      "/kinetra/layer/s1/l1/opacity",
      "/kinetra/layer/s1/l1/var/<key>",
      "/kinetra/layer/s1/l1/play",
      "/kinetra/layer/s1/l1/pause",
      "/kinetra/layer/s1/l1/stop",
      "/kinetra/layer/s1/l1/seek",
      "/kinetra/flow/f1/go",
      "/kinetra/source/<sourceId>/next",
      "/kinetra/source/<sourceId>/prev",
      "/kinetra/screen/s1/clear",
      "/kinetra/clear-all",
      "/kinetra/panic",
    ]);
    expect(list[4].arg).toBe("0..1");
  });

  it("parses the feedback target", () => {
    expect(parseTarget("192.168.1.20:9001")).toEqual({ host: "192.168.1.20", port: 9001 });
    expect(parseTarget("[::1]:9001")).toEqual({ host: "::1", port: 9001 });
    expect(parseTarget("mixer.local:70000")).toBeNull();
    expect(parseTarget("nohost")).toBeNull();
    expect(targetString({ host: "::1", port: 9001 })).toBe("[::1]:9001");
  });
});
