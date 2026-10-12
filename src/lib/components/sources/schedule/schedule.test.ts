import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../templates/list/matchers";
import { SCHEDULE_ITEMS_MAX, type ScheduleDoc, type ScheduleItem, type ScheduleView } from "$lib/types";
import { parseDelimited } from "../paste";
import {
  addItem,
  applySchedulePaste,
  dayLabel,
  duplicateItem,
  fromMinutes,
  groupByDay,
  parseWhen,
  scheduleBlocked,
  scheduleIssues,
  setDuration,
  setEndTime,
  setStart,
  shiftAll,
  toMinutes,
} from "./schedule";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Set<(p: unknown) => void>>() }));

const api = vi.hoisted(() => ({
  getSchedule: vi.fn(),
  saveSchedule: vi.fn(),
  duplicateSource: vi.fn(),
  deleteSource: vi.fn(),
  scheduleView: vi.fn(),
  listScreenDocs: vi.fn(),
  on: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/recipes", () => api);
vi.mock("$lib/api/screens", () => api);

import ScheduleEditor from "./ScheduleEditor.svelte";
import { toasts } from "$lib/stores/toasts.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

const item = (id: string, title: string, start: string, end: string, over: Partial<ScheduleItem> = {}): ScheduleItem => ({
  id,
  title,
  subtitle: "",
  location: "",
  start,
  end,
  delayMin: 0,
  state: "auto",
  ...over,
});

function makeDoc(over: Partial<ScheduleDoc> = {}): ScheduleDoc {
  return {
    id: "r1",
    name: "Run of show",
    kind: "schedule",
    items: [
      item("i1", "Doors", "2026-10-05T09:00", "2026-10-05T09:30", { location: "Lobby" }),
      item("i2", "Keynote", "2026-10-05T09:30", "2026-10-05T10:30", { location: "Main hall", delayMin: 10 }),
      item("i3", "Workshops", "2026-10-05T10:30", "2026-10-05T12:00"),
      item("i4", "Day two", "2026-10-06T09:00", "2026-10-06T09:45"),
    ],
    ripple: true,
    nextOverride: null,
    labels: { now: "NOW", next: "UP NEXT", delayed: "DELAYED", previous: "PREVIOUS", cancelled: "CANCELLED", upcoming: "" },
    updatedAt: "2026-10-04T10:00:00Z",
    ...over,
  };
}

const view = (over: Partial<ScheduleView> = {}): ScheduleView => ({
  sourceId: "r1",
  items: [
    { id: "i1", title: "i1", location: "", status: "now", start: "2026-10-05T09:00", end: "2026-10-05T09:30", startMs: 0, endMs: 0, moved: false },
    { id: "i2", title: "i2", location: "", status: "delayed", start: "2026-10-05T09:40", end: "2026-10-05T10:40", startMs: 0, endMs: 0, moved: true },
    { id: "i3", title: "i3", location: "", status: "next", start: "2026-10-05T10:40", end: "2026-10-05T12:10", startMs: 0, endMs: 0, moved: true },
    { id: "i4", title: "i4", location: "", status: "cancelled", start: "2026-10-06T09:00", end: "2026-10-06T09:45", startMs: 0, endMs: 0, moved: false },
  ],
  nowId: "i1",
  nextId: "i3",
  computedAt: 0,
  canUndo: false,
  ...over,
});

let disk: ScheduleDoc;

function emit(event: string, payload: unknown) {
  for (const h of events.handlers.get(event) ?? []) h(payload);
}

async function advance(ms: number) {
  await vi.advanceTimersByTimeAsync(ms);
  await tick();
}

const lastSaved = (): ScheduleDoc => api.saveSchedule.mock.calls.at(-1)?.[0];
const input = (itemId: string, key: string) =>
  document.querySelector<HTMLInputElement>(`input[data-item="${itemId}"][data-key="${key}"]`)!;
const titles = () => [...document.querySelectorAll<HTMLInputElement>('input[data-key="title"]')].map((i) => i.value);
const badge = (itemId: string) => document.querySelector<HTMLElement>(`[data-row="${itemId}"] .badge`);

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 9, 5, 8, 7));
  vi.clearAllMocks();
  events.handlers.clear();
  toasts.clear();
  disk = makeDoc();
  api.on.mockImplementation((event: string, h: Handler) => {
    let set = events.handlers.get(event);
    if (!set) events.handlers.set(event, (set = new Set()));
    set.add(h);
    return () => set.delete(h);
  });
  api.getSchedule.mockImplementation(async () => disk);
  api.saveSchedule.mockImplementation(async (d: ScheduleDoc) => ({ ...d, updatedAt: "2026-10-05T08:10:00Z" }));
  api.scheduleView.mockResolvedValue(view());
  api.duplicateSource.mockImplementation(async (_id: string, name: string) => ({ ...makeDoc(), id: "r2", name }));
  api.deleteSource.mockResolvedValue(undefined);
  api.listScreenDocs.mockResolvedValue([{ id: "m1", name: "Main program" }]);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

async function mountEditor(usedBy: string[] = []) {
  const props = { id: "r1", usedBy, onsaved: vi.fn(), onduplicated: vi.fn(), ondeleted: vi.fn() };
  const r = render(ScheduleEditor, props);
  await vi.waitFor(() => expect(input("i1", "title")).toBeTruthy());
  await advance(0);
  return { ...r, props };
}

// ---- Model --------------------------------------------------------------------------------------

describe("schedule model", () => {
  it("local date-time arithmetic", () => {
    expect(fromMinutes(toMinutes("2026-10-05T23:50") + 20)).toBe("2026-10-06T00:10");
    expect(toMinutes("2026-02-31T10:00")).toBeNaN();
    expect(toMinutes("2026-10-05 10:00")).toBeNaN();
    expect(dayLabel("2026-10-05")).toBe("Mon 5 Oct 2026");
  });

  it("add: after the last item with the same length; the first at the next quarter hour", () => {
    const empty = makeDoc({ items: [] });
    const a = addItem(empty, new Date(2026, 9, 5, 14, 7), "n1");
    expect(a.doc.items).toEqual([item("n1", "", "2026-10-05T14:15", "2026-10-05T14:45")]);
    const b = addItem(makeDoc(), new Date(), "n2");
    expect(b.doc.items.at(-1)).toMatchObject({ id: "n2", start: "2026-10-06T09:45", end: "2026-10-06T10:30" });
    const full = makeDoc({ items: Array.from({ length: SCHEDULE_ITEMS_MAX }, (_, i) => item(`x${i}`, "x", "2026-10-05T09:00", "2026-10-05T10:00")) });
    expect(addItem(full).doc).toBe(full);
  });

  it("duplicate starts when the original ends; start keeps the length and re-sorts", () => {
    const d = duplicateItem(makeDoc(), "i2", "c1").doc;
    expect(d.items.map((x) => x.id)).toEqual(["i1", "i2", "c1", "i3", "i4"]);
    expect(d.items[2]).toMatchObject({ title: "Keynote", start: "2026-10-05T10:30", end: "2026-10-05T11:30", delayMin: 0 });
    const moved = setStart(makeDoc(), "i1", "2026-10-05T13:00");
    expect(moved.items.map((x) => x.id)).toEqual(["i2", "i3", "i1", "i4"]);
    expect(moved.items[2]).toMatchObject({ start: "2026-10-05T13:00", end: "2026-10-05T13:30" });
  });

  it("end time rolls to the next day when not after the start; length moves the end", () => {
    expect(setEndTime(makeDoc(), "i1", "01:00").items[0].end).toBe("2026-10-06T01:00");
    expect(setEndTime(makeDoc(), "i1", "09:15").items[0].end).toBe("2026-10-05T09:15");
    expect(setDuration(makeDoc(), "i1", 90).items[0].end).toBe("2026-10-05T10:30");
  });

  it("shift all moves every start and end", () => {
    const d = shiftAll(makeDoc(), -30);
    expect(d.items.map((x) => x.start)).toEqual(["2026-10-05T08:30", "2026-10-05T09:00", "2026-10-05T10:00", "2026-10-06T08:30"]);
    expect(shiftAll(makeDoc(), 0).items).toEqual(makeDoc().items);
  });

  it("groups by day", () => {
    expect(groupByDay(makeDoc().items).map((g) => [g.day, g.items.length])).toEqual([
      ["2026-10-05", 3],
      ["2026-10-06", 1],
    ]);
  });

  it("parses pasted times", () => {
    expect(parseWhen("9:05", "2026-10-05")).toBe("2026-10-05T09:05");
    expect(parseWhen("14.30", "2026-10-05")).toBe("2026-10-05T14:30");
    expect(parseWhen("2026-10-07 8:00", "2026-10-05")).toBe("2026-10-07T08:00");
    expect(parseWhen("2026-10-07T08:00:00", "2026-10-05")).toBe("2026-10-07T08:00");
    expect(parseWhen("7.10.2026 18:00", "2026-10-05")).toBe("2026-10-07T18:00");
    expect(parseWhen("25:00", "2026-10-05")).toBeNull();
    expect(parseWhen("soon", "2026-10-05")).toBeNull();
  });

  it("paste: header or title/start/end/location; times roll past midnight; bad rows are skipped", () => {
    let n = 0;
    const id = () => `p${++n}`;
    const empty = makeDoc({ items: [] });
    const withHeader = applySchedulePaste(
      empty,
      parseDelimited("Location\tTitle\tStart\nHall A\tOpening\t22:00\nHall B\tLate set\t23:30\n\tNight\t00:30\n\tNo time\t"),
      "2026-10-05",
      id,
    );
    expect(withHeader.header).toBe(true);
    expect(withHeader.added).toBe(3);
    expect(withHeader.skipped).toBe(1);
    expect(withHeader.doc.items.map((x) => [x.title, x.location, x.start, x.end])).toEqual([
      ["Opening", "Hall A", "2026-10-05T22:00", "2026-10-05T22:30"],
      ["Late set", "Hall B", "2026-10-05T23:30", "2026-10-06T00:00"],
      ["Night", "", "2026-10-06T00:30", "2026-10-06T01:00"],
    ]);
    const plain = applySchedulePaste(makeDoc(), parseDelimited("Lunch,12:00,13:15,Cafe\nTalk,2026-10-07 10:00,11:00"), "2026-10-01", id);
    expect(plain.header).toBe(false);
    const lunch = plain.doc.items.find((x) => x.title === "Lunch")!;
    // HH:MM lands on the day of the last item (6 Oct).
    expect(lunch).toMatchObject({ start: "2026-10-06T12:00", end: "2026-10-06T13:15", location: "Cafe" });
    expect(plain.doc.items.find((x) => x.title === "Talk")).toMatchObject({ start: "2026-10-07T10:00", end: "2026-10-07T11:00" });
  });

  it("validation: title, times, end after start, labels", () => {
    const d = makeDoc();
    d.items = [item("a", " ", "2026-10-05T10:00", "2026-10-05T09:00"), item("b", "B", "nope", "2026-10-05T09:00")];
    d.labels = { ...d.labels, now: "", next: "x".repeat(33) };
    const issues = scheduleIssues(d);
    expect(issues.items.get("a")).toMatchObject({ title: "Enter a title.", end: "The end must be after the start." });
    expect(issues.items.get("b")).toMatchObject({ start: "Enter a date and time." });
    // Empty is allowed (no badge for that status); only the length is checked.
    expect(issues.labels).toEqual({ next: "Up to 32 characters." });
    expect(scheduleBlocked(d)).toBe("An item: Enter a title.");
    expect(scheduleBlocked(makeDoc())).toBeNull();
  });
});

// ---- Editor -------------------------------------------------------------------------------------

describe("schedule editor", () => {
  it("items grouped by day with mono times and status badges with text from the view", async () => {
    await mountEditor();
    const days = [...document.querySelectorAll(".day [role=rowheader]")].map((d) => d.textContent);
    expect(days).toEqual(["Mon 5 Oct 2026", "Tue 6 Oct 2026"]);
    // Day labels are words ("Mon 5 Oct"): body text; times stay mono.
    expect(document.querySelector(".day [role=rowheader]")?.classList.contains("mono")).toBe(false);
    expect(titles()).toEqual(["Doors", "Keynote", "Workshops", "Day two"]);
    expect(input("i1", "time").value).toBe("09:00");
    expect(input("i1", "time").classList.contains("mono")).toBe(true);
    expect(input("i1", "length").value).toBe("30");
    expect(badge("i1")).toHaveTextContent("NOW");
    expect(badge("i1")?.querySelector(".tally.program")).toBeTruthy();
    expect(badge("i2")).toHaveTextContent("DELAYED");
    expect(badge("i2")?.querySelector(".tally.warning")).toBeTruthy();
    expect(badge("i3")).toHaveTextContent("UP NEXT");
    expect(badge("i3")?.querySelector(".tally.preview")).toBeTruthy();
    expect(badge("i4")).toHaveTextContent("CANCELLED");
    expect(badge("i4")?.classList.contains("strike")).toBe(true);
    // Delay is read-only info: +10 and the new start.
    expect(document.querySelector('[data-row="i2"] .delay')).toHaveTextContent("+10 → 09:40");
    expect(document.querySelectorAll("[placeholder]")).toHaveLength(0);
  });

  it("statuses follow schedule-changed for this source only", async () => {
    await mountEditor();
    const next = view();
    next.items = next.items.map((v) => (v.id === "i1" ? { ...v, status: "previous" } : v.id === "i3" ? { ...v, status: "now" } : v));
    emit("schedule-changed", { ...next, sourceId: "other" });
    await tick();
    expect(badge("i1")).toHaveTextContent("NOW");
    emit("schedule-changed", next);
    await tick();
    expect(badge("i1")).toHaveTextContent("PREVIOUS");
    expect(badge("i3")).toHaveTextContent("NOW");
  });

  it("add item: after the last, same length, title focused; saved once the title is filled", async () => {
    const { props } = await mountEditor();
    await fireEvent.click(screen.getByRole("button", { name: "Add item" }));
    await tick();
    const added = document.activeElement as HTMLInputElement;
    expect(added.dataset.key).toBe("title");
    const id = added.dataset.item!;
    expect(input(id, "time").value).toBe("09:45");
    expect(input(id, "date").value).toBe("2026-10-06");
    expect(input(id, "length").value).toBe("45");
    expect(screen.getByRole("status")).toHaveTextContent("Not saved: An item: Enter a title.");
    expect(added).toHaveAttribute("aria-invalid", "true");
    await advance(1000);
    expect(api.saveSchedule).not.toHaveBeenCalled();
    await fireEvent.input(added, { target: { value: "Closing" } });
    await advance(500);
    expect(api.saveSchedule).toHaveBeenCalledTimes(1);
    expect(lastSaved().items.at(-1)).toMatchObject({ title: "Closing", start: "2026-10-06T09:45", end: "2026-10-06T10:30" });
    expect(props.onsaved).toHaveBeenCalledWith(expect.objectContaining({ id: "r1", kind: "schedule", rowCount: 5, nowId: "i1" }));
  });

  it("editing times re-sorts by start and keeps the length; end and length edit the end", async () => {
    await mountEditor();
    // Times commit on blur (forgiving: "11" is 11:00), so typing never re-sorts mid-way.
    await fireEvent.input(input("i1", "time"), { target: { value: "11" } });
    expect(titles()[0]).toBe("Doors");
    await fireEvent.blur(input("i1", "time"));
    expect(titles()).toEqual(["Keynote", "Workshops", "Doors", "Day two"]);
    expect(input("i1", "length").value).toBe("30");
    await fireEvent.change(input("i3", "date"), { target: { value: "2026-10-06" } });
    expect(titles()).toEqual(["Keynote", "Doors", "Day two", "Workshops"]);
    await fireEvent.input(input("i2", "end"), { target: { value: "11.15" } });
    await fireEvent.blur(input("i2", "end"));
    expect(input("i2", "length").value).toBe("105");
    await fireEvent.change(input("i2", "length"), { target: { value: "20" } });
    expect(input("i2", "end").value).toBe("09:50");
    await advance(500);
    expect(api.saveSchedule).toHaveBeenCalledTimes(1);
    expect(lastSaved().items.find((x) => x.id === "i2")).toMatchObject({ start: "2026-10-05T09:30", end: "2026-10-05T09:50" });
  });

  it("a time that isn't one stays in the box, is marked and blocks saving until fixed", async () => {
    await mountEditor();
    const t = input("i1", "time");
    await fireEvent.input(t, { target: { value: "12:60" } });
    await fireEvent.blur(t);
    expect(t.value).toBe("12:60");
    expect(t).toHaveAttribute("aria-invalid", "true");
    expect(t.title).toBe("Minutes go from 00 to 59.");
    expect(screen.getByRole("status")).toHaveTextContent("Not saved: A time: Minutes go from 00 to 59.");
    await fireEvent.input(input("i1", "title"), { target: { value: "Doors open" } });
    await advance(1000);
    expect(api.saveSchedule).not.toHaveBeenCalled();
    await fireEvent.input(t, { target: { value: "0915" } });
    await fireEvent.blur(t);
    expect(input("i1", "time").value).toBe("09:15");
    await advance(500);
    expect(api.saveSchedule).toHaveBeenCalledTimes(1);
    expect(lastSaved().items.find((x) => x.id === "i1")).toMatchObject({ title: "Doors open", start: "2026-10-05T09:15" });
  });

  it("an end before the start (from disk) is marked and blocks saving", async () => {
    disk = makeDoc({ items: [item("i1", "Doors", "2026-10-05T09:00", "2026-10-05T08:00")] });
    await mountEditor();
    expect(input("i1", "end")).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("The end must be after the start.")).toBeTruthy();
    await fireEvent.input(input("i1", "title"), { target: { value: "Doors open" } });
    await advance(1000);
    expect(api.saveSchedule).not.toHaveBeenCalled();
    expect(screen.getByRole("status")).toHaveTextContent("Not saved: Doors open: The end must be after the start.");
  });

  it("duplicate and delete per item", async () => {
    await mountEditor();
    await fireEvent.click(screen.getByRole("button", { name: "Duplicate Keynote" }));
    expect(titles()).toEqual(["Doors", "Keynote", "Keynote", "Workshops", "Day two"]);
    await fireEvent.click(screen.getByRole("button", { name: "Delete Doors" }));
    expect(titles()).toEqual(["Keynote", "Keynote", "Workshops", "Day two"]);
    await advance(500);
    expect(lastSaved().items).toHaveLength(4);
    expect(lastSaved().items[1]).toMatchObject({ title: "Keynote", start: "2026-10-05T10:30", end: "2026-10-05T11:30" });
  });

  it("shift all earlier / later by the minutes given", async () => {
    await mountEditor();
    const minutes = screen.getByRole("spinbutton", { name: "Minutes to shift" });
    await fireEvent.change(minutes, { target: { value: "30" } });
    await fireEvent.blur(minutes);
    await fireEvent.click(screen.getByRole("button", { name: "Earlier" }));
    expect(input("i1", "time").value).toBe("08:30");
    expect(toasts.list.at(-1)?.message).toBe("Moved 4 items 30 min earlier.");
    await fireEvent.click(screen.getByRole("button", { name: "Later" }));
    await fireEvent.click(screen.getByRole("button", { name: "Later" }));
    await advance(500);
    expect(lastSaved().items.map((x) => x.start)).toEqual([
      "2026-10-05T09:30",
      "2026-10-05T10:00",
      "2026-10-05T11:00",
      "2026-10-06T09:30",
    ]);
  });

  it("pasting rows from a spreadsheet adds items; a single value pastes into the input", async () => {
    await mountEditor();
    const paste = (target: Element, text: string) => {
      const e = new Event("paste", { bubbles: true, cancelable: true }) as ClipboardEvent;
      Object.defineProperty(e, "clipboardData", { value: { getData: () => text } });
      target.dispatchEvent(e);
      return e;
    };
    expect(paste(input("i1", "title"), "Hello").defaultPrevented).toBe(false);
    const e = paste(input("i1", "title"), "Title\tStart\tEnd\tLocation\r\nLunch\t12:00\t13:00\tCafe\r\nBad row\t\t\t\r\n");
    expect(e.defaultPrevented).toBe(true);
    await tick();
    expect(titles()).toContain("Lunch");
    expect(toasts.list.at(-1)?.message).toBe(
      "Added 1 item, first line used as column headers. 1 row without a title or start time left out.",
    );
    await advance(500);
    // HH:MM on the day of the last item.
    expect(lastSaved().items.find((x) => x.title === "Lunch")).toMatchObject({
      start: "2026-10-06T12:00",
      end: "2026-10-06T13:00",
      location: "Cafe",
    });
  });

  it("Import CSV… reads a file", async () => {
    await mountEditor();
    const file = new File(["title,start,end\nAfterparty,2026-10-06 20:00,23:00\n"], "run.csv", { type: "text/csv" });
    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]')!;
    Object.defineProperty(fileInput, "files", { value: [file], configurable: true });
    await fireEvent.change(fileInput);
    await vi.waitFor(() => expect(titles()).toContain("Afterparty"));
    await advance(500);
    expect(lastSaved().items.find((x) => x.title === "Afterparty")).toMatchObject({
      start: "2026-10-06T20:00",
      end: "2026-10-06T23:00",
    });
  });

  it("ripple toggle and status labels (≤ 32 characters) are saved; badges use the labels", async () => {
    await mountEditor();
    await fireEvent.click(screen.getByRole("switch", { name: "Ripple delays" }));
    const toggle = screen.getByRole("button", { name: "Status labels" });
    await fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    const group = screen.getByRole("group", { name: "Status labels" });
    const nowInput = within(group).getByLabelText("Now") as HTMLInputElement;
    expect(nowInput.value).toBe("NOW");
    await fireEvent.input(nowInput, { target: { value: "LIVE" } });
    expect(badge("i1")).toHaveTextContent("LIVE");
    await fireEvent.input(within(group).getByLabelText("Up next"), { target: { value: "x".repeat(33) } });
    expect(within(group).getByText("Up to 32 characters.")).toBeTruthy();
    expect(screen.getByRole("status")).toHaveTextContent("Not saved: Up next label: Up to 32 characters.");
    await advance(1000);
    expect(api.saveSchedule).not.toHaveBeenCalled();
    await fireEvent.input(within(group).getByLabelText("Up next"), { target: { value: " NEXT " } });
    await advance(500);
    expect(api.saveSchedule).toHaveBeenCalledTimes(1);
    expect(lastSaved().ripple).toBe(false);
    expect(lastSaved().labels).toMatchObject({ now: "LIVE", next: "NEXT" });
  });

  it("Ctrl+S saves at once; unmount flushes", async () => {
    const { unmount } = await mountEditor();
    await fireEvent.input(input("i1", "title"), { target: { value: "Doors open" } });
    await fireEvent.keyDown(window, { key: "s", ctrlKey: true });
    await advance(0);
    expect(api.saveSchedule).toHaveBeenCalledTimes(1);
    await fireEvent.input(input("i1", "location"), { target: { value: "Foyer" } });
    unmount();
    expect(api.saveSchedule).toHaveBeenCalledTimes(2);
    expect(lastSaved().items[0]).toMatchObject({ title: "Doors open", location: "Foyer" });
  });

  it("changed on disk: reload without edits, banner with edits", async () => {
    await mountEditor();
    disk = makeDoc({ items: [item("i9", "Only", "2026-10-05T09:00", "2026-10-05T10:00")] });
    emit("project-files-changed", { kind: "source", id: "r1", change: "changed", message: null });
    await advance(0);
    expect(titles()).toEqual(["Only"]);
    await fireEvent.input(input("i9", "title"), { target: { value: "Mine" } });
    emit("project-files-changed", { kind: "source", id: "r1", change: "changed", message: null });
    await tick();
    const banner = screen.getByRole("alert");
    expect(banner).toHaveTextContent("This source was edited outside Kinetra");
    await advance(2000);
    expect(api.saveSchedule).not.toHaveBeenCalled();
    await fireEvent.click(within(banner).getByRole("button", { name: "Keep mine" }));
    await advance(0);
    expect(lastSaved().items[0].title).toBe("Mine");
  });

  it("delete asks first", async () => {
    const { props } = await mountEditor(["m1"]);
    await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await advance(0);
    const dialog = await vi.waitFor(() => screen.getByRole("dialog"));
    expect(dialog).toHaveTextContent("“Main program”");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await advance(0);
    expect(api.deleteSource).toHaveBeenCalledWith("r1");
    expect(props.ondeleted).toHaveBeenCalled();
  });

  it("an empty schedule offers Add item and Paste rows", async () => {
    disk = makeDoc({ items: [] });
    render(ScheduleEditor, { id: "r1", usedBy: [], onsaved: vi.fn(), onduplicated: vi.fn(), ondeleted: vi.fn() });
    await vi.waitFor(() => expect(screen.getByText(/No items yet/)).toBeTruthy());
    expect(screen.getAllByRole("button", { name: "Add item" })).toHaveLength(2);
    await fireEvent.click(screen.getAllByRole("button", { name: "Add item" })[1]);
    await tick();
    // 08:07 → the next quarter hour.
    const id = (document.activeElement as HTMLInputElement).dataset.item!;
    expect(input(id, "time").value).toBe("08:15");
  });
});
