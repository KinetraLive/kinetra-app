// MIDI UI: the Learn session, Preferences > MIDI & OSC > Mappings (grouped rows, Learn, Remove,
// conflicts with Change …), the layer / cue context menu (keyboard: Shift+F10, arrows, Esc) and the
// Assign MIDI popover.
import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ControlsDoc, LiveScreenView, MidiLearned } from "$lib/types";

const m = vi.hoisted(() => ({
  handlers: new Map<string, (p: unknown) => void>(),
  midiLearn: vi.fn(async (_on: boolean) => {}),
  getControls: vi.fn(),
  saveControls: vi.fn(async (d: unknown) => d),
}));
vi.mock("$lib/api/midi", () => ({
  midiLearn: m.midiLearn,
  listMidiDevices: async () => [],
  onMidiLearn: (h: (p: unknown) => void) => {
    m.handlers.set("midi-learn", h);
    return () => m.handlers.delete("midi-learn");
  },
  onControlsChanged: (h: (p: unknown) => void) => {
    m.handlers.set("controls-changed", h);
    return () => m.handlers.delete("controls-changed");
  },
}));
vi.mock("$lib/api/controls", () => ({ getControls: m.getControls, saveControls: m.saveControls }));
vi.mock("$lib/api", () => ({
  toAppError: (e: unknown) => ({ code: "internal", message: String(e) }),
  on: () => () => {},
  setPrefs: async (p: unknown) => p,
}));

const SCREENS = [
  {
    screen: { id: "s1", name: "Main", width: 1920, height: 1080, background: "transparent" },
    layers: [
      { id: "l1", name: "Guest", onAir: false, pending: false, fields: {}, fieldDefs: [] },
      { id: "l2", name: "Bug", onAir: false, pending: false, fields: {}, fieldDefs: [] },
    ],
  },
] as unknown as LiveScreenView[];
vi.mock("$lib/api/live", () => ({ listScreens: async () => SCREENS }));
vi.mock("$lib/api/flows", () => ({ listFlows: async () => [] }));
vi.mock("$lib/api/sources", () => ({ listSources: async () => [] }));
vi.mock("$lib/api/screens", () => ({ getScreen: async () => ({ doc: { id: "s1", layers: [] } }) }));
vi.mock("$lib/api/templates", () => ({ getTemplate: async () => null }));

import Mappings from "$lib/components/settings/midi/Mappings.svelte";
import { controls } from "./controls.svelte";
import { prefs } from "$lib/stores/prefs.svelte";
import { closePopup, openMenu } from "./host.svelte";
import { Learn } from "./learn.svelte";
import { contextMenu } from "./menu";
import { command } from "./model";

const learned = (p: Partial<MidiLearned> = {}): MidiLearned => ({ device: "APC40", channel: 1, kind: "note", number: 36, value: 127, ...p });
const emit = (event: string, payload: unknown) => m.handlers.get(event)?.(payload);
const flush = async () => {
  for (let i = 0; i < 6; i++) await tick();
  await new Promise((r) => setTimeout(r, 0));
  await tick();
};

beforeEach(() => {
  m.handlers.clear();
  m.midiLearn.mockClear();
  m.saveControls.mockClear();
  controls.doc = null;
});
afterEach(() => {
  closePopup(false);
  cleanup();
});

describe("Learn", () => {
  it("opens the session, takes the first message, then closes it", async () => {
    const got = vi.fn();
    const l = new Learn();
    l.start(got);
    expect(l.active).toBe(true);
    await flush();
    expect(m.midiLearn).toHaveBeenLastCalledWith(true);
    emit("midi-learn", learned());
    expect(got).toHaveBeenCalledWith(learned());
    expect(l.active).toBe(false);
    await flush();
    expect(m.midiLearn).toHaveBeenLastCalledWith(false);
    // Later messages go nowhere.
    emit("midi-learn", learned({ number: 1 }));
    expect(got).toHaveBeenCalledTimes(1);
  });

  it("Esc cancels; stopping before the core answered still closes its session", async () => {
    const cancel = vi.fn();
    const l = new Learn();
    l.start(() => {}, cancel);
    await flush();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(cancel).toHaveBeenCalled();
    expect(l.active).toBe(false);
    await flush();
    expect(m.midiLearn.mock.calls.map((c) => c[0])).toEqual([true, false]);

    m.midiLearn.mockClear();
    l.start(() => {});
    l.stop();
    await flush();
    expect(m.midiLearn.mock.calls.map((c) => c[0])).toEqual([true, false]);
  });
});

describe("Mappings", () => {
  const doc: ControlsDoc = {
    mappings: [
      { id: "a", source: { device: "APC40", channel: 1, kind: "note", number: 36 }, target: command("live.toggle:s1/l1"), feedback: true },
      { id: "b", source: { device: null, channel: 1, kind: "note", number: 36 }, target: command("live.panic"), feedback: false },
      { id: "c", source: { device: "APC40", channel: 1, kind: "cc", number: 7 }, target: { kind: "value", screenId: "s1", layerId: "l2", target: { kind: "opacity" } }, feedback: false },
    ],
  };

  it("groups rows, shows the conflict and fixes it with Change …", async () => {
    m.getControls.mockResolvedValue(doc);
    render(Mappings);
    await flush();
    expect(screen.getByRole("heading", { name: "Layers" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Show" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Faders and knobs" })).toBeTruthy();
    expect(screen.getByText("Opacity of Bug (Main)", { selector: ".what" })).toBeTruthy();
    expect(screen.getByText("CC 7 · Ch 1 · APC40")).toBeTruthy();

    const panel = screen.getByRole("region", { name: "1 conflict" });
    expect(panel.textContent?.replace(/\s+/g, " ")).toContain("Note 36 · Ch 1 runs nothing: it is assigned to Take or clear Guest (Main), Panic.");
    // Lights only for layers and cues.
    expect(screen.getAllByRole("switch", { name: "Lights" })).toHaveLength(1);

    await fireEvent.click(within(panel).getByRole("button", { name: "Change Panic" }));
    expect(screen.getByText("Press a key, pad or fader…")).toBeTruthy();
    await flush();
    emit("midi-learn", learned({ number: 40, device: "Launchpad" }));
    await flush();
    expect(m.saveControls).toHaveBeenCalledTimes(1);
    const saved = m.saveControls.mock.calls[0][0] as ControlsDoc;
    expect(saved.mappings.find((x) => x.id === "b")?.source).toEqual({ device: "Launchpad", channel: 1, kind: "note", number: 40 });
    expect(screen.queryByRole("region", { name: "1 conflict" })).toBeNull();
  });

  it("adds a mapping: pick an action, Learn, press; Esc cancels; Remove", async () => {
    m.getControls.mockResolvedValue({ mappings: [] });
    render(Mappings);
    await flush();
    const learn = screen.getByRole("button", { name: "Learn" });
    expect(learn.hasAttribute("disabled")).toBe(true);
    const select = screen.getByRole("combobox", { name: "Add" }) as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: "live.take:s1/l2" } });
    await fireEvent.click(screen.getByRole("button", { name: "Learn" }));
    await flush();
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByText("Press a key, pad or fader…")).toBeNull();
    expect(m.saveControls).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole("button", { name: "Learn" }));
    await flush();
    emit("midi-learn", learned({ kind: "cc", number: 20 }));
    await flush();
    expect(m.saveControls).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Take Bug (Main)", { selector: ".what" })).toBeTruthy();
    expect(screen.getByText("CC 20 · Ch 1 · APC40")).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Remove CC 20 · Ch 1 · APC40 from Take Bug (Main)" }));
    await flush();
    expect((m.saveControls.mock.calls[1][0] as ControlsDoc).mappings).toEqual([]);
  });
});

describe("context menu", () => {
  function row() {
    const li = document.createElement("li");
    li.tabIndex = 0;
    li.textContent = "Guest";
    document.body.append(li);
    return li;
  }

  it("opens with Shift+F10, moves with arrows, Esc closes and refocuses the row", async () => {
    m.getControls.mockResolvedValue({ mappings: [] });
    const li = row();
    const setHotkey = vi.fn();
    li.addEventListener("keydown", (e) => contextMenu(e, { target: { kind: "layer", screenId: "s1", layerId: "l1" }, name: "Guest", setHotkey }));
    li.focus();
    await fireEvent.keyDown(li, { key: "F10" }); // no Shift: nothing
    await flush();
    expect(screen.queryByRole("menu")).toBeNull();
    await fireEvent.keyDown(li, { key: "F10", shiftKey: true });
    const menu = await screen.findByRole("menu", { name: "Actions for Guest" });
    const items = within(menu).getAllByRole("menuitem");
    expect(items.map((x) => x.textContent?.trim())).toEqual(["Assign MIDI…", "Set hotkey"]);
    await vi.waitFor(() => expect(document.activeElement).toBe(items[0]));
    await fireEvent.keyDown(items[0], { key: "ArrowDown" });
    expect(document.activeElement).toBe(items[1]);
    await fireEvent.keyDown(items[1], { key: "Escape" });
    await flush();
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(li);

    // Set hotkey runs the page's action.
    await fireEvent.keyDown(li, { key: "ContextMenu" });
    await fireEvent.click(await screen.findByRole("menuitem", { name: "Set hotkey" }));
    expect(setHotkey).toHaveBeenCalled();
    li.remove();
  });

  it("Assign MIDI… learns the first message for Take or clear; Remove MIDI when assigned", async () => {
    m.getControls.mockResolvedValue({
      mappings: [{ id: "x", source: { device: "APC40", channel: 1, kind: "note", number: 36 }, target: command("live.panic"), feedback: false }],
    });
    const li = row();
    openMenu({ target: { kind: "layer", screenId: "s1", layerId: "l1" }, name: "Guest" }, li, { x: 10, y: 10 });
    await flush();
    await fireEvent.click(screen.getByRole("menuitem", { name: "Assign MIDI…" }));
    await flush();
    const dialog = await screen.findByRole("dialog", { name: "Assign MIDI to Guest" });
    expect(within(dialog).getByText("Press a key, pad or fader…")).toBeTruthy();
    expect(m.midiLearn).toHaveBeenLastCalledWith(true);
    emit("midi-learn", learned());
    await flush();
    const saved = m.saveControls.mock.calls[0][0] as ControlsDoc;
    expect(saved.mappings.at(-1)).toMatchObject({ target: command("live.toggle:s1/l1"), feedback: true, source: { number: 36 } });
    expect(within(dialog).getByText("Note 36 · Ch 1 · APC40")).toBeTruthy();
    // Panic has the same source: neither runs.
    expect(within(dialog).getByRole("alert").textContent?.replace(/\s+/g, " ")).toContain("Also assigned to Panic. Neither runs until one changes.");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Done" }));
    await flush();
    expect(screen.queryByRole("dialog")).toBeNull();

    m.getControls.mockResolvedValue(saved);
    openMenu({ target: { kind: "layer", screenId: "s1", layerId: "l1" }, name: "Guest" }, li, null);
    await flush();
    const assign = screen.getByRole("menuitem", { name: /Assign MIDI…/ });
    expect(assign.textContent).toContain("Note 36 · Ch 1 · APC40");
    await fireEvent.click(screen.getByRole("menuitem", { name: "Remove MIDI" }));
    await flush();
    const after = m.saveControls.mock.calls.at(-1)?.[0] as ControlsDoc;
    expect(after.mappings.map((x) => x.id)).toEqual(["x"]);
    li.remove();
  });
});

describe("context menu in Simple mode", () => {
  afterEach(() => prefs.set(null));

  it("hides Assign and Remove MIDI behind the Advanced mode hint; Set hotkey stays", async () => {
    prefs.set({ theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 }, uiMode: "simple" });
    m.getControls.mockResolvedValue({
      mappings: [{ id: "x", source: { device: "APC40", channel: 1, kind: "note", number: 36 }, target: command("live.toggle:s1/l1"), feedback: true }],
    });
    const li = document.createElement("li");
    document.body.append(li);
    openMenu({ target: { kind: "layer", screenId: "s1", layerId: "l1" }, name: "Guest", setHotkey: vi.fn() }, li, null);
    await flush();
    const menu = screen.getByRole("menu", { name: "Actions for Guest" });
    expect(within(menu).getAllByRole("menuitem").map((x) => x.textContent?.trim())).toEqual(["Set hotkey"]);
    expect(await within(menu).findByText(/MIDI: available in Advanced mode/)).toBeTruthy();
    // The arrow keys reach the hint's switch.
    const items = within(menu).getAllByRole("menuitem");
    await vi.waitFor(() => expect(document.activeElement).toBe(items[0]));
    await fireEvent.keyDown(items[0], { key: "ArrowDown" });
    expect(document.activeElement).toBe(within(menu).getByRole("button", { name: "Switch to Advanced" }));
    await fireEvent.click(within(menu).getByRole("button", { name: "Switch to Advanced" }));
    await flush();
    expect(within(menu).getAllByRole("menuitem").map((x) => x.textContent?.trim().replace(/\s+/g, " "))).toEqual([
      "Assign MIDI… Note 36 · Ch 1 · APC40",
      "Remove MIDI",
      "Set hotkey",
    ]);
    li.remove();
  });
});
