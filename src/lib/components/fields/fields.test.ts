import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { FieldDef, TimerState } from "$lib/types";
import FieldInputs from "./FieldInputs.svelte";
import TimerControls from "./TimerControls.svelte";
import { bindableColumns, bindingText } from "./binding-label";
import { parseRows, rowCount, serializeRows } from "./list";
import {
  applyTimerAction,
  formatSeconds,
  formatTimer,
  nextTickIn,
  parseDuration,
  parseTimer,
  shownMs,
  timerValue,
} from "./timer";

// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (docs/TESTING.md).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/fields/TimerControls.svelte"),
  ]);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const COLS = [
  { key: "name", label: "Name" },
  { key: "score", label: "Score" },
];
const clockDef: FieldDef = { key: "time", label: "Time", type: "clock", default: "HH:mm", format: "HH:mm" };
const timerState = (p: Partial<TimerState> = {}): TimerState => ({
  run: false,
  at: 0,
  base: 0,
  dir: "up",
  end: "stop",
  ...p,
});
const timerDef: FieldDef = { key: "clock", label: "Game clock", type: "timer", default: timerValue(timerState()) };

// ---- list.ts --------------------------------------------------------------------------------------

describe("list values", () => {
  it("parses rows leniently and serializes only the columns' cells", () => {
    expect(parseRows(undefined)).toEqual([]);
    expect(parseRows("nope")).toEqual([]);
    expect(parseRows('{"a":1}')).toEqual([]);
    expect(parseRows('[{"name":"A","score":10,"x":null}, 3, {"name":true}]')).toEqual([
      { name: "A", score: "10" },
      { name: "true" },
    ]);
    expect(serializeRows([{ name: "A", score: "", other: "x" }, {}], COLS)).toBe('[{"name":"A"},{}]');
    expect(rowCount('[{},{}]')).toBe(2);
  });
});

// ---- timer.ts -------------------------------------------------------------------------------------

describe("timer math", () => {
  it("parses with defaults per field", () => {
    expect(parseTimer("bad")).toEqual(timerState());
    expect(parseTimer('{"run":true,"at":5,"base":-3,"dir":"down","end":"x"}')).toEqual(
      timerState({ run: true, at: 5, base: 0, dir: "down" }),
    );
  });

  it("shows base while paused and counts from `at` while running", () => {
    expect(shownMs(timerState({ base: 4000 }), 99_999)).toBe(4000);
    expect(shownMs(timerState({ run: true, at: 1000, base: 500 }), 3000)).toBe(2500);
    expect(shownMs(timerState({ run: true, at: 1000, base: 5000, dir: "down" }), 3000)).toBe(3000);
    expect(shownMs(timerState({ run: true, at: 1000, base: 5000, dir: "down" }), 99_000)).toBe(0);
  });

  it("formats: floor counting up, ceil counting down, hours when needed", () => {
    expect(formatTimer(timerState({ run: true, at: 0, base: 0 }), 61_999)).toBe("01:01");
    expect(formatTimer(timerState({ run: true, at: 0, base: 10_000, dir: "down" }), 100)).toBe("00:10");
    expect(formatTimer(timerState({ run: true, at: 0, base: 10_000, dir: "down" }), 9_999)).toBe("00:01");
    expect(formatSeconds(3723)).toBe("1:02:03");
  });

  it("next tick: aligned to the shown second; none while paused or at zero", () => {
    expect(nextTickIn(timerState(), 0)).toBeNull();
    expect(nextTickIn(timerState({ run: true, at: 0, base: 250 }), 0)).toBe(750);
    expect(nextTickIn(timerState({ run: true, at: 0, base: 2300, dir: "down" }), 0)).toBe(300);
    expect(nextTickIn(timerState({ run: true, at: 0, base: 2000, dir: "down" }), 0)).toBe(1000);
    expect(nextTickIn(timerState({ run: true, at: 0, base: 2000, dir: "down" }), 5000)).toBeNull();
  });

  it("parses durations", () => {
    expect(parseDuration("90")).toBe(90_000);
    expect(parseDuration("1:30")).toBe(90_000);
    expect(parseDuration("01:00:00")).toBe(3_600_000);
    expect(parseDuration("1:75")).toBeNull();
    expect(parseDuration("abc")).toBeNull();
    expect(parseDuration("100:00:00")).toBeNull();
  });

  it("applies actions locally like the server", () => {
    const def: FieldDef = { ...timerDef, default: timerValue(timerState({ base: 60_000, dir: "down" })) };
    let s = parseTimer(def.default);
    s = applyTimerAction(s, { action: "start" }, 1000, def);
    expect(s).toMatchObject({ run: true, at: 1000, base: 60_000 });
    s = applyTimerAction(s, { action: "pause" }, 11_000, def);
    expect(s).toMatchObject({ run: false, base: 50_000 });
    s = applyTimerAction(s, { action: "set", ms: 5000 }, 12_000, def);
    expect(s).toMatchObject({ run: false, base: 5000 });
    s = applyTimerAction(s, { action: "configure", dir: "up", end: "hide" }, 13_000, def);
    expect(s).toMatchObject({ dir: "up", end: "hide", base: 5000 });
    expect(applyTimerAction(s, { action: "reset" }, 0, def)).toEqual(parseTimer(def.default));
  });
});

// ---- FieldInputs: clock and timer ---------------------------------------------------------------

describe("FieldInputs: clock and timer fields", () => {
  it("a clock shows its format read-only (no input)", () => {
    render(FieldInputs, { defs: [clockDef], values: {}, onfield: vi.fn() });
    const group = screen.getByRole("group", { name: "Time" });
    expect(group.textContent).toContain("HH:mm");
    expect(within(group).queryByRole("textbox")).toBeNull();
    expect(group.querySelector(".mono")?.textContent).toBe("HH:mm");
  });

  it("with ontimer, each control sends one action", async () => {
    const ontimer = vi.fn();
    const onfield = vi.fn();
    render(FieldInputs, {
      defs: [timerDef],
      values: { clock: timerValue(timerState({ base: 65_000, dir: "down" })) },
      onfield,
      ontimer,
    });
    expect((await screen.findByRole("timer", { name: "Game clock time" })).textContent).toBe("01:05");
    await fireEvent.click(screen.getByRole("button", { name: "Start Game clock" }));
    expect(ontimer).toHaveBeenLastCalledWith("clock", { action: "start" });
    await fireEvent.click(screen.getByRole("button", { name: "Reset Game clock" }));
    expect(ontimer).toHaveBeenLastCalledWith("clock", { action: "reset" });

    const set = screen.getByRole("textbox", { name: "Set Game clock to (mm:ss)" });
    await fireEvent.input(set, { target: { value: "2:30" } });
    await fireEvent.click(screen.getByRole("button", { name: "Set Game clock" }));
    expect(ontimer).toHaveBeenLastCalledWith("clock", { action: "set", ms: 150_000 });

    await fireEvent.change(screen.getByRole("combobox", { name: "Game clock at zero" }), { target: { value: "hide" } });
    expect(ontimer).toHaveBeenLastCalledWith("clock", { action: "configure", dir: "down", end: "hide" });
    await fireEvent.change(screen.getByRole("combobox", { name: "Game clock direction" }), { target: { value: "up" } });
    expect(ontimer).toHaveBeenLastCalledWith("clock", { action: "configure", dir: "up", end: "stop" });
    expect(onfield).not.toHaveBeenCalled();
  });

  it("without ontimer the state is edited locally and sent as JSON", async () => {
    const onfield = vi.fn();
    render(FieldInputs, { defs: [timerDef], values: {}, onfield });
    await fireEvent.click(await screen.findByRole("button", { name: "Start Game clock" }));
    const [key, value] = onfield.mock.calls.at(-1)!;
    expect(key).toBe("clock");
    expect(JSON.parse(value as string)).toMatchObject({ run: true, base: 0, dir: "up" });
  });
});

describe("TimerControls: ticking", () => {
  const props = (s: TimerState) => ({ label: "T", def: timerDef, value: timerValue(s), ontimer: vi.fn() });

  it("no timers at rest (paused)", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "Date"] });
    vi.setSystemTime(10_000);
    render(TimerControls, props(timerState({ base: 5000 })));
    await tick();
    expect(screen.getByRole("timer").textContent).toBe("00:05");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("ticks once per shown second while running, aligned to the second", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "Date"] });
    vi.setSystemTime(10_000);
    // Started 400 ms ago at 0: shows 00:00, next change in 600 ms.
    render(TimerControls, props(timerState({ run: true, at: 9_600 })));
    await tick();
    const t = screen.getByRole("timer");
    expect(t.textContent).toBe("00:00");
    expect(vi.getTimerCount()).toBe(1);
    await vi.advanceTimersByTimeAsync(599);
    expect(t.textContent).toBe("00:00");
    await vi.advanceTimersByTimeAsync(1);
    expect(t.textContent).toBe("00:01");
    await vi.advanceTimersByTimeAsync(1000);
    expect(t.textContent).toBe("00:02");
    expect(vi.getTimerCount()).toBe(1);
  });

  it("a countdown stops ticking at zero; pausing stops the timer", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "Date"] });
    vi.setSystemTime(0);
    const r = render(TimerControls, props(timerState({ run: true, at: 0, base: 2000, dir: "down" })));
    await tick();
    const t = screen.getByRole("timer");
    expect(t.textContent).toBe("00:02");
    await vi.advanceTimersByTimeAsync(2000);
    expect(t.textContent).toBe("00:00");
    expect(vi.getTimerCount()).toBe(0);

    await r.rerender(props(timerState({ run: true, at: 2000, base: 0 })));
    await tick();
    expect(vi.getTimerCount()).toBe(1);
    await r.rerender(props(timerState({ base: 1500 })));
    await tick();
    expect(vi.getTimerCount()).toBe(0);
    expect(t.textContent).toBe("00:01");
  });

  it("does not tick while the document is hidden", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "Date"] });
    vi.setSystemTime(0);
    render(TimerControls, props(timerState({ run: true, at: 0 })));
    await tick();
    expect(vi.getTimerCount()).toBe(1);
    const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(true);
    document.dispatchEvent(new Event("visibilitychange"));
    await tick();
    expect(vi.getTimerCount()).toBe(0);
    hidden.mockReturnValue(false);
    document.dispatchEvent(new Event("visibilitychange"));
    await tick();
    expect(vi.getTimerCount()).toBe(1);
    hidden.mockRestore();
  });
});

// ---- binding-label.ts ---------------------------------------------------------------------------

describe("binding labels", () => {
  const guests = { name: "Guests", kind: "text" as const, columns: [{ key: "name", label: "Name" }] };
  const schedule = { name: "Schedule", kind: "schedule" as const, columns: [] };

  it("text sources: Source · Column; lists: all rows", () => {
    expect(bindingText(guests, { sourceId: "g", column: "name" })).toBe("Guests · Name");
    expect(bindingText(guests, { sourceId: "g", column: "gone" })).toBe("Guests · gone (missing)");
    expect(bindingText(guests, { sourceId: "g", column: "", limit: 3 })).toBe("Guests · all rows, top 3");
    // Older callers pass no kind: a text source.
    expect(bindingText({ name: "Guests", columns: guests.columns }, { sourceId: "g", column: "name", row: "current" })).toBe(
      "Guests · Name",
    );
    expect(bindingText(null, { sourceId: "g", column: "name" })).toBe("Missing source · name");
  });

  it("schedules: the row (Now / Up next) and the schedule's columns; lists: all items", () => {
    expect(bindingText(schedule, { sourceId: "s", column: "title", row: "next" })).toBe("Schedule · Up next · Title");
    expect(bindingText(schedule, { sourceId: "s", column: "new_start", row: "now" })).toBe("Schedule · Now · New start");
    expect(bindingText(schedule, { sourceId: "s", column: "countdown", row: "next" })).toBe("Schedule · Up next · Countdown");
    expect(bindingText(schedule, { sourceId: "s", column: "" })).toBe("Schedule · all items");
    expect(bindingText(schedule, { sourceId: "s", column: "", sort: { column: "start", numeric: false, desc: true } })).toBe(
      "Schedule · all items, sorted by Start ↓",
    );
    expect(bindingText(null, { sourceId: "s", column: "title", row: "now" })).toBe("Missing source · Now · title");
    expect(bindableColumns(schedule).map((c) => c.key)).toContain("countdown");
    expect(bindableColumns(guests)).toBe(guests.columns);
  });
});
