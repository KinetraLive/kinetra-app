import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ScheduleDoc, ScheduleItem, ScheduleItemView, ScheduleView } from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Handler>() }));
const api = vi.hoisted(() => ({
  on: vi.fn(),
  scheduleView: vi.fn(),
  scheduleAction: vi.fn(),
  getSchedule: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) => ({ code: "internal", message: e instanceof Error ? e.message : String(e) }),
}));
vi.mock("$lib/api/recipes", () => api);
vi.mock("$lib/api/sources", () => api);

import SchedulePanel from "./SchedulePanel.svelte";
import Menu from "$lib/components/control/Menu.svelte";
import { clock } from "./clock.svelte";
import {
  countdownText,
  hasOverride,
  itemActions,
  lateMinutes,
  scheduleKey,
  startsText,
} from "./schedule";
import { toasts } from "$lib/stores/toasts.svelte";

// ---- Fixtures (local times around 10:30 on 4 Oct 2026) -------------------------------------------

const NOW = new Date(2026, 9, 4, 10, 30, 0).getTime();
const pad = (n: number) => String(n).padStart(2, "0");
const at = (h: number, m: number) => `2026-10-04T${pad(h)}:${pad(m)}`;
const ms = (h: number, m: number) => new Date(2026, 9, 4, h, m).getTime();

function item(id: string, title: string, start: [number, number], end: [number, number], patch: Partial<ScheduleItem> = {}): ScheduleItem {
  return {
    id,
    title,
    subtitle: "",
    location: `Hall ${id.toUpperCase()}`,
    start: at(...start),
    end: at(...end),
    delayMin: 0,
    state: "auto",
    ...patch,
  };
}

const doc: ScheduleDoc = {
  id: "sched",
  name: "Run of show",
  kind: "schedule",
  items: [
    item("a", "Doors", [9, 0], [10, 0]),
    item("b", "Keynote", [10, 0], [10, 50]),
    item("c", "Panel", [10, 32], [11, 30]),
    item("d", "Workshop", [12, 0], [13, 0], { delayMin: 10 }),
    item("e", "Raffle", [13, 0], [13, 15], { state: "cancelled" }),
  ],
  ripple: false,
  nextOverride: null,
  labels: { now: "NOW", next: "UP NEXT", delayed: "DELAYED", previous: "PREVIOUS", cancelled: "CANCELLED", upcoming: "" },
  updatedAt: "2026-10-04T08:00:00Z",
};

function iv(id: string, status: ScheduleItemView["status"], start: [number, number], end: [number, number], moved = false): ScheduleItemView {
  return { id, title: id, location: "", status, start: at(...start), end: at(...end), startMs: ms(...start), endMs: ms(...end), moved };
}

const makeView = (patch: Partial<ScheduleView> = {}): ScheduleView => ({
  sourceId: "sched",
  items: [
    iv("a", "previous", [9, 0], [10, 0]),
    iv("b", "now", [10, 0], [10, 50]),
    iv("c", "next", [10, 32], [11, 30]),
    iv("d", "delayed", [12, 10], [13, 10], true),
    iv("e", "cancelled", [13, 0], [13, 15]),
  ],
  nowId: "b",
  nextId: "c",
  computedAt: Date.now(),
  canUndo: false,
  ...patch,
});

const flush = async () => {
  for (let i = 0; i < 4; i++) {
    await Promise.resolve();
    await tick();
  }
};

async function mount(kind: "schedule" | "countdown" = "schedule", view = makeView()) {
  api.scheduleView.mockResolvedValue(view);
  const r = render(SchedulePanel, { props: { sourceId: "sched", kind, name: "Event", updatedAt: doc.updatedAt, Menu } });
  await flush();
  return r;
}

const row = (id: string) => document.querySelector<HTMLElement>(`[data-item="${id}"]`)!;
const lastAction = () => api.scheduleAction.mock.calls.at(-1);

async function menuAction(itemId: string, label: string | RegExp) {
  const r = row(itemId);
  await fireEvent.click(within(r).getByRole("button", { name: /^Actions for/ }));
  await fireEvent.click(within(r).getByRole("menuitem", { name: label }));
  await flush();
}

beforeEach(() => {
  vi.clearAllMocks();
  events.handlers.clear();
  vi.useFakeTimers({ now: NOW, toFake: ["setTimeout", "clearTimeout", "Date"] });
  api.on.mockImplementation((name: string, h: Handler) => {
    events.handlers.set(name, h);
    return () => events.handlers.delete(name);
  });
  api.getSchedule.mockResolvedValue(doc);
  api.scheduleAction.mockImplementation(async () => makeView({ canUndo: true, computedAt: Date.now() + 1 }));
});

afterEach(() => {
  cleanup();
  toasts.clear();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

// ---- Model --------------------------------------------------------------------------------------

describe("schedule model", () => {
  it("formats countdowns and start texts", () => {
    expect(countdownText(125_000)).toBe("02:05");
    expect(countdownText(3_725_000)).toBe("1:02:05");
    expect(countdownText(-5)).toBe("00:00");
    const v = iv("c", "next", [10, 32], [11, 30]);
    expect(startsText(v, NOW, "2026-10-04")).toBe("starts in 02:00");
    expect(startsText(iv("x", "next", [12, 0], [13, 0]), NOW, "2026-10-04")).toBe("at 12:00");
    expect(startsText(v, ms(10, 33), "2026-10-04")).toBe("starting");
  });

  it("maps shortcuts and ignores typing, repeats and other modifiers", () => {
    const k = (key: string, init: KeyboardEventInit = {}) => new KeyboardEvent("keydown", { key, ...init });
    expect(scheduleKey(k("n"), false)).toBe("startNext");
    expect(scheduleKey(k("D"), false)).toBe("delayNext");
    expect(scheduleKey(k("z", { ctrlKey: true }), false)).toBe("undo");
    expect(scheduleKey(k("z", { ctrlKey: true, shiftKey: true }), false)).toBeNull();
    expect(scheduleKey(k("n"), true)).toBeNull();
    expect(scheduleKey(k("n", { repeat: true }), false)).toBeNull();
    expect(scheduleKey(k("n", { altKey: true }), false)).toBeNull();
  });

  it("offers the actions that fit an item's status", () => {
    const ids = (v: ScheduleItemView, it?: ScheduleItem) => itemActions(v, it, null).map((a) => a.id);
    expect(ids(iv("b", "now", [10, 0], [10, 50]), doc.items[1])).toContain("end");
    expect(ids(iv("b", "now", [10, 0], [10, 50]), doc.items[1])).not.toContain("start");
    expect(ids(iv("e", "cancelled", [13, 0], [13, 15]), doc.items[4])).toEqual(["restore", "auto"]);
    expect(ids(iv("c", "next", [10, 32], [11, 30]), doc.items[2])).not.toContain("auto");
    expect(hasOverride(iv("c", "next", [10, 32], [11, 30]), doc.items[2], "c")).toBe(true);
    expect(lateMinutes(iv("d", "delayed", [12, 10], [13, 10], true), doc.items[3])).toBe(10);
  });
});

// ---- Panel --------------------------------------------------------------------------------------

describe("schedule panel", () => {
  it("shows status badges with text, highlights NOW and strikes CANCELLED", async () => {
    await mount();
    const badge = (id: string) => [...row(id).querySelectorAll("[data-badge]")].map((b) => b.textContent?.replace(/\s+/g, " ").trim());
    expect(badge("a")).toEqual(["PREVIOUS"]);
    expect(badge("b")).toEqual(["NOW"]);
    expect(badge("c")).toEqual(["UP NEXT"]);
    expect(badge("d")).toEqual(["DELAYED +10 min"]);
    expect(badge("e")).toEqual(["CANCELLED"]);
    expect(row("b").classList.contains("is-now")).toBe(true);
    expect(row("e").querySelector(".title")?.classList.contains("struck")).toBe(true);
    // The planned time of a moved item is struck through next to the new one.
    expect(row("d").querySelector(".time")?.textContent?.replace(/\s+/g, " ").trim()).toBe("12:00 12:10");
    expect(within(row("c")).getByText("Panel")).toBeTruthy();
    expect(within(row("c")).getByText("Hall C")).toBeTruthy();
  });

  it("the NEXT card counts down with the clock and starts or delays the next item", async () => {
    await mount();
    const card = document.querySelector<HTMLElement>("[data-next]")!;
    expect(card.dataset.next).toBe("c");
    expect(within(card).getByText("Panel")).toBeTruthy();
    expect(card.querySelector("[data-starts]")?.textContent).toBe("starts in 02:00");
    await vi.advanceTimersByTimeAsync(1010);
    expect(card.querySelector("[data-starts]")?.textContent).toBe("starts in 01:59");
    expect(screen.getByLabelText("Current time").textContent).toBe("10:30:01");

    await fireEvent.click(within(card).getByRole("button", { name: /Start now/ }));
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "startNow", itemId: "c" }]);
    await fireEvent.click(within(card).getByRole("button", { name: /\+5 min/ }));
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "delay", itemId: "c", minutes: 5 }]);
  });

  it("corrects the countdown by the server clock drift", async () => {
    // The server is 30 s ahead of this machine.
    await mount("schedule", makeView({ computedAt: Date.now() + 30_000 }));
    expect(document.querySelector("[data-starts]")?.textContent).toBe("starts in 01:30");
  });

  it("each item action sends the right payload", async () => {
    await mount();
    await fireEvent.click(within(row("b")).getByRole("button", { name: "End Keynote" }));
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "end", itemId: "b" }]);

    await fireEvent.click(within(row("d")).getByRole("button", { name: "Start Workshop now" }));
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "startNow", itemId: "d" }]);

    await menuAction("d", "Delay +10 min");
    expect(lastAction()).toEqual(["sched", { do: "delay", itemId: "d", minutes: 10 }]);
    await menuAction("d", "Delay +15 min");
    expect(lastAction()).toEqual(["sched", { do: "delay", itemId: "d", minutes: 15 }]);
    await menuAction("d", "Cancel");
    expect(lastAction()).toEqual(["sched", { do: "cancel", itemId: "d" }]);
    await menuAction("d", "Back to automatic");
    expect(lastAction()).toEqual(["sched", { do: "auto", itemId: "d" }]);
    await menuAction("e", "Restore");
    expect(lastAction()).toEqual(["sched", { do: "restore", itemId: "e" }]);
  });

  it("inline forms: set next (with or without a time), delay by N, set start time", async () => {
    await mount();
    await menuAction("d", "Set as next…");
    const form = () => row("d").querySelector<HTMLFormElement>("[data-form]")!;
    expect(form().dataset.form).toBe("next");
    await fireEvent.submit(form());
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "setNext", itemId: "d", start: null }]);

    await menuAction("d", "Set as next…");
    // A forgiving time: "1510" is 15:10 (committed on Enter, before the form submits).
    const at = within(form()).getByLabelText("Starts at (optional)");
    await fireEvent.input(at, { target: { value: "1510" } });
    await fireEvent.keyDown(at, { key: "Enter" });
    await fireEvent.submit(form());
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "setNext", itemId: "d", start: "2026-10-04T15:10" }]);

    // An invalid time stays in the box, marked, and blocks Set.
    await menuAction("d", "Set as next…");
    const bad = within(form()).getByLabelText("Starts at (optional)");
    await fireEvent.input(bad, { target: { value: "24:00" } });
    await fireEvent.blur(bad);
    expect((bad as HTMLInputElement).value).toBe("24:00");
    expect(bad.getAttribute("aria-invalid")).toBe("true");
    expect(within(form()).getByText("Hours go from 00 to 23.")).toBeTruthy();
    expect(within(form()).getByRole("button", { name: "Set as next" }).hasAttribute("disabled")).toBe(true);
    const calls = api.scheduleAction.mock.calls.length;
    await fireEvent.submit(form());
    await flush();
    expect(api.scheduleAction.mock.calls.length).toBe(calls);

    await menuAction("d", "Delay by…");
    const minutes = within(form()).getByLabelText(/Delay Workshop by/) as HTMLInputElement;
    minutes.value = "20";
    await fireEvent.change(minutes);
    await fireEvent.submit(form());
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "delay", itemId: "d", minutes: 20 }]);

    await menuAction("d", "Set start time…");
    const start = within(form()).getByLabelText("Start time");
    await fireEvent.input(start, { target: { value: "12:45" } });
    await fireEvent.keyDown(start, { key: "Enter" });
    await fireEvent.submit(form());
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "setStart", itemId: "d", start: "2026-10-04T12:45" }]);

    // Esc closes a form without sending.
    await menuAction("d", "Set start time…");
    const n = api.scheduleAction.mock.calls.length;
    await fireEvent.keyDown(within(form()).getByLabelText("Start time"), { key: "Escape" });
    expect(row("d").querySelector("[data-form]")).toBeNull();
    expect(api.scheduleAction.mock.calls.length).toBe(n);
  });

  it("Undo follows canUndo; the reply replaces the view", async () => {
    await mount();
    const undo = screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement;
    expect(undo.disabled).toBe(true);
    await menuAction("d", "Cancel");
    expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(false);
    await fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "undo" }]);
  });

  it("shortcuts N / D / Ctrl+Z work only with focus in the panel and never while typing", async () => {
    await mount("schedule", makeView({ canUndo: true }));
    const inside = within(row("c")).getByRole("button", { name: /^Actions for/ });
    await fireEvent.keyDown(inside, { key: "n" });
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "startNow", itemId: "c" }]);
    await fireEvent.keyDown(inside, { key: "d" });
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "delay", itemId: "c", minutes: 5 }]);
    await fireEvent.keyDown(inside, { key: "z", ctrlKey: true });
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "undo" }]);

    const n = api.scheduleAction.mock.calls.length;
    await fireEvent.keyDown(window, { key: "n" });
    await menuAction("d", "Set start time…");
    await fireEvent.keyDown(within(row("d")).getByLabelText("Start time"), { key: "n" });
    await fireEvent.keyDown(within(row("d")).getByLabelText("Start time"), { key: "z", ctrlKey: true });
    await flush();
    expect(api.scheduleAction.mock.calls.length).toBe(n);
  });

  it("schedule-changed for this source replaces the view; older or other views are ignored", async () => {
    await mount();
    const push = events.handlers.get("schedule-changed")!;
    push(makeView({ sourceId: "other", nextId: "d" }));
    push(makeView({ nextId: "d", computedAt: Date.now() - 10_000 }));
    await flush();
    expect(document.querySelector<HTMLElement>("[data-next]")!.dataset.next).toBe("c");
    push(makeView({ nextId: "d", computedAt: Date.now() + 5 }));
    await flush();
    expect(document.querySelector<HTMLElement>("[data-next]")!.dataset.next).toBe("d");
  });

  it("a failed action shows a toast", async () => {
    await mount();
    api.scheduleAction.mockRejectedValueOnce(new Error("busy"));
    await menuAction("d", "Cancel");
    expect(toasts.list.at(-1)?.message).toBe("Couldn't change the schedule: busy");
  });

  it("the clock ticks only while the panel is on screen, and stops when it unmounts", async () => {
    const io = { cb: null as IntersectionObserverCallback | null };
    vi.stubGlobal(
      "IntersectionObserver",
      class {
        constructor(cb: IntersectionObserverCallback) {
          io.cb = cb;
        }
        observe() {}
        disconnect() {}
      },
    );
    const r = await mount();
    expect(clock.watching).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    const fire = (on: boolean) =>
      io.cb!([{ isIntersecting: on } as IntersectionObserverEntry], {} as IntersectionObserver);
    fire(true);
    await flush();
    expect(clock.watching).toBe(true);
    expect(vi.getTimerCount()).toBe(1);
    fire(false);
    await flush();
    expect(clock.watching).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
    fire(true);
    await flush();
    r.unmount();
    expect(clock.watching).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("countdown kind: a big countdown to the target, set target time, start, reset", async () => {
    await mount("countdown");
    const card = document.querySelector<HTMLElement>("[data-countdown-target]")!;
    expect(card.dataset.countdownTarget).toBe("c");
    expect(card.querySelector("[data-countdown]")?.textContent?.trim()).toBe("02:00");
    await vi.advanceTimersByTimeAsync(2010);
    expect(card.querySelector("[data-countdown]")?.textContent?.trim()).toBe("01:58");

    await fireEvent.click(within(card).getByRole("button", { name: "Start now" }));
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "startNow", itemId: "c" }]);
    await fireEvent.click(within(card).getByRole("button", { name: "Reset" }));
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "auto", itemId: "c" }]);
    await fireEvent.click(within(card).getByRole("button", { name: "Set target time" }));
    const time = within(card).getByLabelText("Start time");
    await fireEvent.input(time, { target: { value: "11.00" } });
    await fireEvent.blur(time);
    expect((time as HTMLInputElement).value).toBe("11:00");
    await fireEvent.submit(card.querySelector("form")!);
    await flush();
    expect(lastAction()).toEqual(["sched", { do: "setStart", itemId: "c", start: "2026-10-04T11:00" }]);
  });
});
