// Control's resizable right panel: width math, the divider (pointer drag coalesced per frame,
// keyboard, one commit on release) and persistence through ControlCtl.setPanelWidth.

import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PANEL_WIDTH, clampPanel, dragWidth, keyWidth } from "./panel";
import PanelDivider from "./PanelDivider.svelte";

describe("panel width math", () => {
  it("clamps to 280–640 and rounds", () => {
    expect(clampPanel(0)).toBe(280);
    expect(clampPanel(280)).toBe(280);
    expect(clampPanel(400.6)).toBe(401);
    expect(clampPanel(9999)).toBe(640);
    expect(PANEL_WIDTH).toMatchObject({ min: 280, max: 640, step: 16 });
  });

  it("keys: ← wider, → narrower by 16, Home / End the limits, others ignored", () => {
    expect(keyWidth("ArrowLeft", 400)).toBe(416);
    expect(keyWidth("ArrowRight", 400)).toBe(384);
    expect(keyWidth("ArrowLeft", 630)).toBe(640);
    expect(keyWidth("ArrowRight", 290)).toBe(280);
    expect(keyWidth("Home", 400)).toBe(280);
    expect(keyWidth("End", 400)).toBe(640);
    expect(keyWidth("ArrowUp", 400)).toBeNull();
  });

  it("dragging left widens, right narrows, clamped", () => {
    expect(dragWidth(400, 1000, 900)).toBe(500);
    expect(dragWidth(400, 1000, 1050)).toBe(350);
    expect(dragWidth(400, 1000, 0)).toBe(640);
    expect(dragWidth(400, 1000, 2000)).toBe(280);
  });
});

describe("PanelDivider", () => {
  let frames = new Map<number, FrameRequestCallback>();
  let nextId = 1;
  const flushFrames = () => {
    const f = [...frames.values()];
    frames = new Map();
    for (const cb of f) cb(0);
  };

  beforeEach(() => {
    frames = new Map();
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
      frames.set(nextId, cb);
      return nextId++;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  function setup(width: number | null = null) {
    const target = document.createElement("div");
    const panel = document.createElement("aside");
    panel.getBoundingClientRect = () => ({ width: 448 }) as DOMRect;
    const oncommit = vi.fn();
    render(PanelDivider, { props: { target, panel, width, controls: "side", oncommit } });
    const sep = screen.getByRole("separator", { name: "Resize the side panel" });
    return { target, sep, oncommit };
  }

  it("is a focusable vertical separator with its range; the default width is measured", () => {
    const { sep } = setup();
    expect(sep.getAttribute("aria-orientation")).toBe("vertical");
    expect(sep.getAttribute("aria-valuemin")).toBe("280");
    expect(sep.getAttribute("aria-valuemax")).toBe("640");
    expect(sep.getAttribute("aria-valuenow")).toBe("448");
    expect(sep.getAttribute("aria-controls")).toBe("side");
    expect(sep.tabIndex).toBe(0);
  });

  it("shows the saved width", () => {
    const { sep } = setup(360);
    expect(sep.getAttribute("aria-valuenow")).toBe("360");
  });

  it("a drag sets only --panel-w, once per frame, and commits once on release", async () => {
    const { target, sep, oncommit } = setup();
    await fireEvent.pointerDown(sep, { button: 0, clientX: 1000, pointerId: 1 });
    await fireEvent.pointerMove(sep, { clientX: 990, pointerId: 1 });
    await fireEvent.pointerMove(sep, { clientX: 980, pointerId: 1 });
    await fireEvent.pointerMove(sep, { clientX: 950, pointerId: 1 });
    // Three moves, one frame.
    expect(frames.size).toBe(1);
    expect(target.style.getPropertyValue("--panel-w")).toBe("");
    flushFrames();
    expect(target.style.getPropertyValue("--panel-w")).toBe("498px");
    expect(sep.getAttribute("aria-valuenow")).toBe("498");
    expect(oncommit).not.toHaveBeenCalled();
    await fireEvent.pointerMove(sep, { clientX: 100, pointerId: 1 });
    await fireEvent.pointerUp(sep, { clientX: 100, pointerId: 1 });
    expect(target.style.getPropertyValue("--panel-w")).toBe("640px");
    expect(oncommit).toHaveBeenCalledExactlyOnceWith(640);
    // Moves after release do nothing.
    await fireEvent.pointerMove(sep, { clientX: 500, pointerId: 1 });
    expect(frames.size).toBe(0);
  });

  it("keys resize at once and save 400 ms after the last one (or on blur)", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const { target, sep, oncommit } = setup(400);
    sep.focus();
    await fireEvent.keyDown(sep, { key: "ArrowLeft" });
    await fireEvent.keyDown(sep, { key: "ArrowLeft" });
    flushFrames();
    expect(target.style.getPropertyValue("--panel-w")).toBe("432px");
    vi.advanceTimersByTime(399);
    expect(oncommit).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(oncommit).toHaveBeenCalledExactlyOnceWith(432);

    await fireEvent.keyDown(sep, { key: "Home" });
    await fireEvent.blur(sep);
    expect(oncommit).toHaveBeenLastCalledWith(280);
    await fireEvent.keyDown(sep, { key: "End" });
    vi.advanceTimersByTime(400);
    expect(oncommit).toHaveBeenLastCalledWith(640);
    // Other keys are left alone.
    const ev = new KeyboardEvent("keydown", { key: "ArrowUp", cancelable: true, bubbles: true });
    sep.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
  });

  it("no commit when the width didn't change", async () => {
    const { sep, oncommit } = setup(400);
    await fireEvent.pointerDown(sep, { button: 0, clientX: 500, pointerId: 1 });
    await fireEvent.pointerMove(sep, { clientX: 500, pointerId: 1 });
    await fireEvent.pointerUp(sep, { clientX: 500, pointerId: 1 });
    expect(oncommit).not.toHaveBeenCalled();
  });
});
