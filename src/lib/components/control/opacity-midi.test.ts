// Control's opacity (slider + number, saved to the screen ~150 ms after the last move) and its MIDI
// part (the side panel's context menu: layer rows and cues, never inside a text field).
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ScreenDoc } from "$lib/types";

const api = vi.hoisted(() => ({
  getScreen: vi.fn(),
  saveScreen: vi.fn(async (doc: unknown) => ({ doc })),
  openControlMenu: vi.fn(),
}));
vi.mock("$lib/api/screens", () => ({ getScreen: api.getScreen, saveScreen: api.saveScreen }));
vi.mock("$lib/api", () => ({ toAppError: (e: unknown) => ({ code: "internal", message: String(e) }), on: () => () => {} }));
vi.mock("$lib/components/midi/lazy", () => ({ host: async () => ({ openControlMenu: api.openControlMenu }) }));

import OpacityInput from "$lib/components/inputs/OpacityInput.svelte";
import { sideMenu } from "$lib/components/midi/eager.svelte";
import type { ControlCtl } from "./control.svelte";
import { LayerOpacity } from "./opacity.svelte";

const DOC = { id: "s1", name: "Main", layers: [{ id: "l1", name: "Guest", opacity: 40 }, { id: "l2", name: "Bug" }] } as unknown as ScreenDoc;

beforeEach(() => {
  vi.useFakeTimers();
  api.getScreen.mockResolvedValue({ doc: DOC });
  api.saveScreen.mockClear();
  api.openControlMenu.mockClear();
});
afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("OpacityInput", () => {
  it("reports slider moves and typed values, clamped to 0–100", async () => {
    const onvalue = vi.fn();
    render(OpacityInput, { value: 100, onvalue });
    const slider = screen.getByRole("slider", { name: "Opacity slider" }) as HTMLInputElement;
    expect(slider.value).toBe("100");
    await fireEvent.input(slider, { target: { value: "35" } });
    expect(onvalue).toHaveBeenLastCalledWith(35);
    const box = screen.getByRole("spinbutton", { name: "Opacity" }) as HTMLInputElement;
    await fireEvent.change(box, { target: { value: "55" } });
    expect(onvalue).toHaveBeenLastCalledWith(55);
    await fireEvent.change(box, { target: { value: "-5" } });
    expect(onvalue).toHaveBeenLastCalledWith(0);
  });
});

describe("LayerOpacity", () => {
  it("shows the saved value (absent = 100), a draft at once, and saves once after the last move", async () => {
    const ctl = { doc: DOC } as unknown as ControlCtl;
    const o = new LayerOpacity(() => ctl);
    expect(o.value("s1", "l1")).toBe(40);
    expect(o.value("s1", "l2")).toBe(100);
    expect(o.value("other", "l1")).toBe(100);
    o.set("s1", "l2", 80);
    o.set("s1", "l2", 60);
    expect(o.value("s1", "l2")).toBe(60);
    await vi.advanceTimersByTimeAsync(149);
    expect(api.saveScreen).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(10);
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
    const saved = api.saveScreen.mock.calls[0][0] as ScreenDoc;
    expect(saved.layers.map((l) => l.opacity)).toEqual([40, 60]);
    // The saved document replaces Control's copy; the draft goes.
    expect((ctl.doc as ScreenDoc).layers[1].opacity).toBe(60);
    expect(o.value("s1", "l2")).toBe(60);
  });

  it("flush saves what waits", async () => {
    const o = new LayerOpacity(() => ({ doc: DOC }) as unknown as ControlCtl);
    o.set("s1", "l1", 10);
    await o.flush();
    expect(api.saveScreen).toHaveBeenCalledTimes(1);
  });
});

describe("Control's side panel menu", () => {
  function panel() {
    const aside = document.createElement("aside");
    aside.innerHTML = `<ul><li data-layer="l1"><span class="name">Guest</span><input type="text" /></li></ul><ul><li data-cue="f1"><span>Open</span></li></ul><p>other</p>`;
    document.body.append(aside);
    const ctl = {} as ControlCtl;
    aside.addEventListener("contextmenu", (e) => sideMenu(e, ctl));
    aside.addEventListener("keydown", (e) => sideMenu(e, ctl));
    return { aside, ctl };
  }

  it("opens for layer rows and cues (pointer, Shift+F10, menu key), not in text fields or elsewhere", async () => {
    vi.useRealTimers();
    const { aside, ctl } = panel();
    const layer = aside.querySelector<HTMLElement>("[data-layer]") as HTMLElement;
    const cue = aside.querySelector<HTMLElement>("[data-cue]") as HTMLElement;

    const right = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 30, clientY: 40 });
    layer.querySelector("span")?.dispatchEvent(right);
    expect(right.defaultPrevented).toBe(true);
    await vi.waitFor(() => expect(api.openControlMenu).toHaveBeenCalledWith(ctl, layer, { x: 30, y: 40 }));

    cue.dispatchEvent(new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(api.openControlMenu).toHaveBeenLastCalledWith(ctl, cue, null));
    cue.dispatchEvent(new KeyboardEvent("keydown", { key: "ContextMenu", bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(api.openControlMenu).toHaveBeenCalledTimes(3));

    // A text field keeps its own menu; other keys and other places do nothing.
    const inField = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 1, clientY: 1 });
    layer.querySelector("input")?.dispatchEvent(inField);
    expect(inField.defaultPrevented).toBe(false);
    aside.querySelector("p")?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    cue.dispatchEvent(new KeyboardEvent("keydown", { key: "F10", bubbles: true }));
    await new Promise((r) => setTimeout(r, 10));
    expect(api.openControlMenu).toHaveBeenCalledTimes(3);
    aside.remove();
  });
});
