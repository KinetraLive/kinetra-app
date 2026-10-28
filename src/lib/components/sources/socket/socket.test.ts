import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import "../../templates/list/matchers";
import type { SocketStatus, SocketTestResult, SourceSummary } from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Set<(p: unknown) => void>>() }));
const api = vi.hoisted(() => ({
  listSources: vi.fn(),
  getSource: vi.fn(),
  createSource: vi.fn(),
  createSchedule: vi.fn(),
  createApiSource: vi.fn(),
  createSocketSource: vi.fn(),
  saveSource: vi.fn(),
  duplicateSource: vi.fn(),
  deleteSource: vi.fn(),
  testSocketSource: vi.fn(),
  retrySocketSource: vi.fn(),
  setSocketPaused: vi.fn(),
  watchSocketSource: vi.fn(),
  setPrefs: vi.fn(),
  on: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e ? e : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/api-sources", () => api);
vi.mock("$lib/api/socket-sources", () => api);

import SocketEditor from "./SocketEditor.svelte";
import NewSourceDialog from "../new/NewSourceDialog.svelte";
import SourceList from "../SourceList.svelte";
import { localIp, NOT_LOCAL, rateFrom, socketBlocked, socketIssues, urlIssue, withFilter, type SocketDoc } from "./model";
import { socketLong, socketShort } from "./status";
import { prefs } from "$lib/stores/prefs.svelte";
import { toasts } from "$lib/stores/toasts.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

const MESSAGE = { type: "score", teams: [{ name: "Bergen", score: 2 }, { name: "Oslo", score: 1 }] };

function socketDoc(over: Partial<SocketDoc> = {}): SocketDoc {
  return {
    id: "ws1",
    name: "Scores",
    kind: "socket",
    url: "ws://localhost:8080",
    mapping: { rows: "", columns: [], csv: { header: true, delimiter: "auto" } },
    rate: 10,
    paused: false,
    columns: [],
    rows: [],
    index: 0,
    updatedAt: "2026-10-29T10:00:00Z",
    ...over,
  };
}

const status = (over: Partial<SocketStatus> = {}): SocketStatus => ({
  state: "live",
  lastMessageAt: Date.now() - 2000,
  nextAt: null,
  message: null,
  rows: 2,
  dropped: 0,
  ...over,
});

function summary(doc: SocketDoc, st: SocketStatus | null = status()): SourceSummary {
  return {
    id: doc.id,
    name: doc.name,
    kind: "socket",
    rowCount: doc.rows.length,
    index: 0,
    current: {},
    columns: doc.columns,
    usedBy: [],
    updatedAt: doc.updatedAt,
    auto: null,
    nextAt: null,
    nowId: null,
    nextId: null,
    socket: st,
  };
}

const testResult = (over: Partial<SocketTestResult> = {}): SocketTestResult => ({
  messages: [MESSAGE],
  json: MESSAGE,
  rows: [],
  hits: {},
  seen: 3,
  matched: 1,
  error: null,
  ...over,
});

let doc: SocketDoc;

async function advance(ms: number) {
  await vi.advanceTimersByTimeAsync(ms);
  await tick();
}

const lastSaved = (): SocketDoc => api.saveSource.mock.calls.at(-1)?.[0];

beforeAll(async () => {
  // The preview grid is a lazy chunk: compiled once here, not inside a test's wait.
  await import("$lib/components/sheet/SheetEditor.svelte");
});

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  events.handlers.clear();
  toasts.clear();
  doc = socketDoc();
  api.on.mockImplementation((event: string, h: Handler) => {
    let set = events.handlers.get(event);
    if (!set) events.handlers.set(event, (set = new Set()));
    set.add(h);
    return () => set.delete(h);
  });
  api.getSource.mockImplementation(async () => doc);
  api.saveSource.mockImplementation(async (d: SocketDoc) => {
    doc = { ...d, updatedAt: "2026-10-29T10:01:00Z" };
    return doc;
  });
  api.testSocketSource.mockResolvedValue(testResult());
  api.watchSocketSource.mockResolvedValue(undefined);
  api.retrySocketSource.mockImplementation(async () => summary(doc, status({ state: "connecting" })));
  api.setSocketPaused.mockImplementation(async (_id: string, paused: boolean) =>
    summary(doc, status({ state: paused ? "paused" : "connecting" })),
  );
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  prefs.set(null);
});

async function mount(props: Partial<{ socket: SocketStatus | null }> = {}) {
  const all = { id: "ws1", usedBy: [], socket: props.socket ?? null, onsaved: vi.fn(), onduplicated: vi.fn(), ondeleted: vi.fn() };
  const r = render(SocketEditor, all);
  await vi.waitFor(() => expect(screen.getByLabelText("Source name")).toBeTruthy());
  await tick();
  return { ...r, props: all };
}

// ---- Pure helpers -------------------------------------------------------------------------------

describe("live data source checks", () => {
  it("only ws:// and wss:// addresses, IPs on this computer or the network", () => {
    expect(urlIssue("")).toBeNull();
    expect(urlIssue("ws://localhost:49123")).toBeNull();
    expect(urlIssue("wss://studio.local/feed")).toBeNull();
    expect(urlIssue("ws://192.168.1.20:8080")).toBeNull();
    expect(urlIssue("ws://[::1]:9000")).toBeNull();
    expect(urlIssue("http://localhost:1")).toBe("Enter a ws:// or wss:// address.");
    expect(urlIssue("ws://8.8.8.8/x")).toBe(NOT_LOCAL);
    expect(urlIssue("ws://[2001:4860::8888]/")).toBe(NOT_LOCAL);
    expect(urlIssue("not a url")).toBe("Enter a ws:// or wss:// address.");
    // As the backend's is_local_ip.
    for (const ok of ["127.0.0.1", "100.64.0.1", "100.127.1.1", "10.0.0.1", "172.16.0.1", "172.31.0.1", "192.168.0.1", "169.254.1.1", "fd00::1", "fe80::1", "::1"])
      expect(localIp(ok), ok).toBe(true);
    for (const no of ["8.8.8.8", "172.32.0.1", "100.63.0.1", "100.128.0.1", "2001:db8::1", "::ffff:8.8.8.8"]) expect(localIp(no), no).toBe(false);
    expect(localIp("example.local")).toBeNull();
  });

  it("the filter needs a pointer; an empty filter is none", () => {
    expect(socketIssues(withFilter(doc, "/type", "score")).filter).toBeNull();
    expect(socketIssues(withFilter(doc, "type", "score")).filter).toBe("A pointer starts with /, like /type.");
    expect(socketIssues(withFilter(doc, "", "score")).filter).toBe("A pointer starts with /, like /type.");
    expect("filter" in withFilter(withFilter(doc, "/a", "b"), "", "")).toBe(false);
    expect(socketBlocked(socketDoc({ url: "ws://8.8.8.8" }))).toBe(`URL: ${NOT_LOCAL}`);
    expect(rateFrom(0)).toEqual({ rate: 1, clamped: true });
    expect(rateFrom(45)).toEqual({ rate: 30, clamped: true });
    expect(rateFrom(12.4)).toEqual({ rate: 12, clamped: false });
  });

  it("names the connection states", () => {
    expect(socketShort(status())).toEqual({ tally: "preview", text: "Live" });
    expect(socketShort(status({ state: "stale" })).text).toBe("No data");
    expect(socketShort(status({ state: "error" })).tally).toBe("warning");
    expect(socketShort(null).text).toBe("Waiting");
    expect(socketLong(status({ state: "waiting" })).text).toBe("Waiting for the server");
    expect(socketLong(status({ state: "idle" })).text).toBe("Waiting: not in use");
  });
});

// ---- The editor ---------------------------------------------------------------------------------

describe("live data editor", () => {
  it("connects while shown, edits and saves the settings", async () => {
    const { unmount } = await mount({ socket: status() });
    expect(api.watchSocketSource).toHaveBeenCalledWith("ws1", true);
    expect(screen.getByText("This computer or your local network.")).toBeTruthy();
    await fireEvent.input(screen.getByLabelText("URL"), { target: { value: "ws://8.8.8.8:1" } });
    expect(screen.getByText(NOT_LOCAL)).toBeTruthy();
    await advance(600);
    expect(api.saveSource).not.toHaveBeenCalled();
    await fireEvent.input(screen.getByLabelText("URL"), { target: { value: "ws://127.0.0.1:9000/feed" } });
    await fireEvent.input(screen.getByLabelText("Send on connect"), { target: { value: '{"subscribe":"scores"}' } });
    await fireEvent.input(screen.getByLabelText("Only messages where"), { target: { value: "/type" } });
    await fireEvent.input(screen.getByLabelText("equals"), { target: { value: "score" } });
    await advance(600);
    expect(lastSaved()).toMatchObject({
      url: "ws://127.0.0.1:9000/feed",
      sendOnConnect: '{"subscribe":"scores"}',
      filter: { pointer: "/type", equals: "score" },
    });
    // Clearing the text drops it from the document.
    await fireEvent.input(screen.getByLabelText("Send on connect"), { target: { value: "" } });
    await advance(600);
    expect("sendOnConnect" in lastSaved()).toBe(false);
    unmount();
    expect(api.watchSocketSource).toHaveBeenLastCalledWith("ws1", false);
  });

  it("test connection → the message tree; pick rows and columns; the preview shows them", async () => {
    await mount({ socket: status() });
    await fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    await advance(0);
    expect(api.testSocketSource).toHaveBeenCalledWith(expect.objectContaining({ id: "ws1", kind: "socket" }));
    expect(screen.getByText("3 messages")).toBeTruthy();
    const tree = screen.getByRole("tree", { name: "Message" });
    const node = (ptr: string) => tree.querySelector<HTMLElement>(`[data-ptr="${ptr}"]`)!;
    await fireEvent.click(node("/teams"));
    await fireEvent.click(screen.getByRole("button", { name: "Use as rows" }));
    await tick();
    expect((screen.getByLabelText("Rows") as HTMLInputElement).value).toBe("/teams");
    expect((screen.getByLabelText("Column 1 path") as HTMLInputElement).value).toBe("/name");
    await vi.waitFor(() => expect(screen.getByRole("grid", { name: "Preview of Scores" })).toBeTruthy());
    expect(screen.getByText("Bergen")).toBeTruthy();
    expect(screen.getByText("From the test connection")).toBeTruthy();
    await advance(600);
    expect(lastSaved().mapping.rows).toBe("/teams");
    expect(lastSaved().columns.map((c) => c.key)).toEqual(["name", "score"]);
  });

  it("status: state, last message, dropped, the problem; Connect now and Pause", async () => {
    const { props } = await mount({ socket: status({ state: "error", message: "The server answered HTTP 404 instead of a WebSocket", dropped: 4 }) });
    expect(document.querySelector('[data-socket-state="error"]')).toHaveTextContent("Error");
    expect(screen.getByText("The server answered HTTP 404 instead of a WebSocket")).toBeTruthy();
    expect(screen.getByText("4 messages")).toBeTruthy();
    expect(screen.getByText("2 s ago")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Connect now" }));
    await advance(0);
    expect(api.retrySocketSource).toHaveBeenCalledWith("ws1");
    expect(props.onsaved).toHaveBeenCalledWith(expect.objectContaining({ socket: expect.objectContaining({ state: "connecting" }) }));
    await fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    await advance(0);
    expect(api.setSocketPaused).toHaveBeenCalledWith("ws1", true);
    expect(screen.getByRole("button", { name: "Resume" })).toBeTruthy();
  });

  it("new live rows are read when the status changes", async () => {
    const cols = [{ key: "name", label: "Name" }];
    doc = socketDoc({ mapping: { rows: "/teams", columns: [{ key: "name", label: "Name", kind: "text", path: "/name" }], csv: { header: true, delimiter: "auto" } }, columns: cols });
    const { rerender } = await mount({ socket: status({ rows: 0, lastMessageAt: 1 }) });
    expect(screen.getByText("No rows yet: test the connection, or wait for data.")).toBeTruthy();
    doc = { ...doc, rows: [{ name: "Bergen" }] };
    await rerender({ socket: status({ rows: 1, lastMessageAt: 2 }) });
    await advance(0);
    await vi.waitFor(() => expect(screen.getByRole("grid", { name: "Preview of Scores" })).toBeTruthy());
    expect(screen.getByText("Bergen")).toBeTruthy();
    expect(screen.getByText("Live", { selector: ".preview span" })).toBeTruthy();
  });

  it("Simple mode shows the status and data with the Advanced mode hint", async () => {
    prefs.set({ theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 }, uiMode: "simple" } as never);
    await mount({ socket: status() });
    expect(screen.queryByLabelText("Send on connect")).toBeNull();
    expect(screen.queryByRole("button", { name: "Test connection" })).toBeNull();
    expect(screen.getByText("Live", { selector: "[data-socket-state] span" })).toBeTruthy();
  });
});

// ---- Lists --------------------------------------------------------------------------------------

describe("lists", () => {
  it("the Sources list shows the connection state", () => {
    render(SourceList, { sources: [summary(socketDoc(), status({ state: "waiting" }))], selectedId: null, onselect: () => {} });
    expect(document.querySelector('[data-socket-state="waiting"]')).toHaveTextContent("Waiting");
    expect(screen.getByText("Live data")).toBeTruthy();
  });

  it("New source offers live data in Advanced mode only", async () => {
    prefs.set({ theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 }, uiMode: "advanced" } as never);
    api.listSources.mockResolvedValue([]);
    api.createSocketSource.mockResolvedValue(socketDoc());
    const oncreated = vi.fn();
    const { unmount } = render(NewSourceDialog, { open: true, onclose: () => {}, oncreated, names: [] });
    await tick();
    await fireEvent.click(screen.getByLabelText(/Live data \(WebSocket\)/));
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await vi.waitFor(() => expect(api.createSocketSource).toHaveBeenCalledWith("Source 1"));
    expect(oncreated).toHaveBeenCalledWith(expect.objectContaining({ kind: "socket" }));
    unmount();
    prefs.set({ theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 }, uiMode: "simple" } as never);
    render(NewSourceDialog, { open: true, onclose: () => {}, names: [] });
    await tick();
    expect(screen.queryByLabelText(/Live data/)).toBeNull();
  });
});
