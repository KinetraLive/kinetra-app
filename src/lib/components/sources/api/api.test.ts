import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import "../../templates/list/matchers";
import type { ApiStatus, ApiTestResult, SourceDoc, SourceSummary } from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Set<(p: unknown) => void>>() }));
const api = vi.hoisted(() => ({
  listSources: vi.fn(),
  getSource: vi.fn(),
  createSource: vi.fn(),
  createSchedule: vi.fn(),
  createApiSource: vi.fn(),
  saveSource: vi.fn(),
  duplicateSource: vi.fn(),
  deleteSource: vi.fn(),
  testApiSource: vi.fn(),
  setSourceSecret: vi.fn(),
  clearSourceSecret: vi.fn(),
  retryApiSource: vi.fn(),
  setApiPaused: vi.fn(),
  watchApiSource: vi.fn(),
  serverStatus: vi.fn(),
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

import ApiEditor from "./ApiEditor.svelte";
import NewSourceDialog from "../new/NewSourceDialog.svelte";
import SourceList from "../SourceList.svelte";
import { ago, apiLong, apiShort, until } from "./status";
import {
  apiIssues,
  cellText,
  csvPathOptions,
  intervalFrom,
  jsonPathOptions,
  mapJson,
  markOf,
  mergeSecrets,
  missingInTest,
  pointerOf,
  resolve,
  tokensOf,
  withinRow,
  type ApiDoc,
} from "./model";
import { prefs } from "$lib/stores/prefs.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

const SAMPLE = {
  updated: "2026-10-06T12:00:00Z",
  data: {
    standings: [
      { team: { name: "Bergen", code: "BRG" }, points: 13, logo: "https://example.com/brg.png" },
      { team: { name: "Oslo", code: "OSL" }, points: 11, logo: null },
      { team: { name: "Tromsø", code: "TOS" }, points: 10 },
    ],
  },
};

function apiDoc(over: Partial<ApiDoc> = {}): ApiDoc {
  return {
    id: "api1",
    name: "League table",
    kind: "api",
    request: { url: "https://example.com/table.json", method: "GET", headers: [], body: null },
    format: "json",
    mapping: { rows: "", columns: [], csv: { header: true, delimiter: "auto" } },
    poll: { intervalMs: 10000, onlyWhenUsed: true, paused: false },
    columns: [],
    rows: [],
    index: 0,
    fetchedAt: null,
    updatedAt: "2026-10-06T10:00:00Z",
    ...over,
  };
}

const status = (over: Partial<ApiStatus> = {}): ApiStatus => ({
  state: "ok",
  lastOkAt: Date.now() - 4000,
  lastAttemptAt: Date.now() - 4000,
  nextAt: Date.now() + 6000,
  httpStatus: 200,
  message: null,
  rows: 3,
  ...over,
});

function summary(doc: ApiDoc, st: ApiStatus | null = status()): SourceSummary {
  return {
    id: doc.id,
    name: doc.name,
    kind: "api",
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
    api: st,
    format: doc.format,
  };
}

const testResult = (over: Partial<ApiTestResult> = {}): ApiTestResult => ({
  httpStatus: 200,
  headers: [
    ["content-type", "application/json"],
    ["etag", '"abc"'],
  ],
  preview: JSON.stringify(SAMPLE, null, 2),
  json: SAMPLE,
  csvColumns: null,
  rows: [],
  hits: {},
  error: null,
  ...over,
});

let doc: ApiDoc;

async function advance(ms: number) {
  await vi.advanceTimersByTimeAsync(ms);
  await tick();
}

const lastSaved = (): ApiDoc => api.saveSource.mock.calls.at(-1)?.[0];

// The preview grid is a lazy chunk (`import()` of SheetEditor). Loaded here once, so the tests
// wait on the editor, not on Vite compiling the chunk on first use (over vi.waitFor's 1 s on the
// slow macOS runner).
beforeAll(async () => {
  await import("$lib/components/sheet/SheetEditor.svelte");
});

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  events.handlers.clear();
  toasts.clear();
  doc = apiDoc();
  api.on.mockImplementation((event: string, h: Handler) => {
    let set = events.handlers.get(event);
    if (!set) events.handlers.set(event, (set = new Set()));
    set.add(h);
    return () => set.delete(h);
  });
  api.getSource.mockImplementation(async () => doc);
  api.saveSource.mockImplementation(async (d: ApiDoc) => {
    doc = { ...d, updatedAt: "2026-10-06T10:01:00Z" };
    return doc;
  });
  api.testApiSource.mockResolvedValue(testResult());
  api.watchApiSource.mockResolvedValue(undefined);
  api.retryApiSource.mockImplementation(async () => summary(doc, status({ state: "fetching" })));
  api.setApiPaused.mockImplementation(async (_id: string, paused: boolean) =>
    summary(doc, status({ state: paused ? "paused" : "ok" })),
  );
  api.setSourceSecret.mockImplementation(async (_id: string, header: string) => {
    doc = {
      ...doc,
      request: {
        ...doc.request,
        headers: doc.request.headers.map((h) => (h.name === header ? { name: h.name, value: null, secret: "sec123", stored: true } : h)),
      },
    };
    return doc;
  });
  api.clearSourceSecret.mockImplementation(async (_id: string, header: string) => {
    doc = {
      ...doc,
      request: {
        ...doc.request,
        headers: doc.request.headers.map((h) => (h.name === header ? { name: h.name, value: null, secret: null } : h)),
      },
    };
    return doc;
  });
  server.status = null;
  api.serverStatus.mockResolvedValue({ state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

async function mount(props: Partial<{ api: ApiStatus | null; usedBy: string[] }> = {}) {
  const all = {
    id: "api1",
    usedBy: props.usedBy ?? [],
    api: props.api ?? null,
    onsaved: vi.fn(),
    onduplicated: vi.fn(),
    ondeleted: vi.fn(),
  };
  const r = render(ApiEditor, all);
  await vi.waitFor(() => expect(screen.getByLabelText("URL")).toBeTruthy());
  await tick();
  return { ...r, props: all };
}

// ---- Pure helpers -------------------------------------------------------------------------------

describe("JSON Pointer and mapping", () => {
  it("pointers escape ~ and /, resolve objects and arrays, and fail cleanly", () => {
    expect(pointerOf(["a/b", "c~d", 0])).toBe("/a~1b/c~0d/0");
    expect(tokensOf("/a~1b/c~0d/0")).toEqual(["a/b", "c~d", "0"]);
    expect(tokensOf("")).toEqual([]);
    expect(tokensOf("nope")).toBeNull();
    expect(resolve(SAMPLE, "/data/standings/1/team/code")).toEqual({ found: true, value: "OSL" });
    expect(resolve(SAMPLE, "/data/standings/9").found).toBe(false);
    expect(resolve(SAMPLE, "/data/standings/01").found).toBe(false);
    expect(resolve(SAMPLE, "").value).toBe(SAMPLE);
  });

  it("cells: strings as is, numbers and booleans written out, null empty, objects as JSON", () => {
    expect([cellText("x"), cellText(13), cellText(false), cellText(null), cellText({ a: 1 })]).toEqual(["x", "13", "false", "", '{"a":1}']);
  });

  it("maps rows and counts the rows that have a value per column", () => {
    const m = mapJson(SAMPLE, {
      rows: "/data/standings",
      columns: [
        { key: "name", label: "Name", kind: "text", path: "/team/name" },
        { key: "points", label: "Points", kind: "text", path: "/points" },
        { key: "logo", label: "Logo", kind: "image", path: "/logo" },
      ],
    });
    expect(m.error).toBeNull();
    expect(m.rows).toEqual([
      { name: "Bergen", points: "13", logo: "https://example.com/brg.png" },
      { name: "Oslo", points: "11", logo: "" },
      { name: "Tromsø", points: "10", logo: "" },
    ]);
    expect(m.hits).toEqual({ name: 3, points: 3, logo: 1 });
    // One object is one row; a scalar or a missing pointer is an error.
    expect(mapJson(SAMPLE, { rows: "/data/standings/0/team", columns: [{ key: "c", label: "C", kind: "text", path: "/code" }] }).rows).toEqual([{ c: "BRG" }]);
    expect(mapJson(SAMPLE, { rows: "/updated", columns: [] }).error).toMatch(/list or an object/);
    expect(mapJson(SAMPLE, { rows: "/nope", columns: [] }).error).toBe("Nothing at /nope.");
  });

  it("a node's pointer within a row, and what the mapping marks", () => {
    expect(withinRow("/data/standings/2/team/name", "/data/standings", true)).toBe("/team/name");
    expect(withinRow("/data/standings/2", "/data/standings", true)).toBe("");
    expect(withinRow("/data/other", "/data/standings", true)).toBeNull();
    expect(withinRow("/data/standings/0/team/code", "/data/standings/0", false)).toBe("/team/code");
    const mapping = { rows: "/data/standings", columns: [{ key: "name", label: "Name", kind: "text" as const, path: "/team/name" }] };
    expect(markOf("/data/standings", mapping, true)).toEqual({ rows: true, keys: [] });
    expect(markOf("/data/standings/1/team/name", mapping, true)).toEqual({ rows: false, keys: ["name"] });
    expect(markOf("/data/standings/1/points", mapping, true)).toBeNull();
  });

  it("the interval clamps to one second", () => {
    expect(intervalFrom(10)).toEqual({ ms: 10000, clamped: false });
    expect(intervalFrom(3600)).toEqual({ ms: 3_600_000, clamped: false });
    expect(intervalFrom(0.5)).toEqual({ ms: 1000, clamped: "min" });
    expect(intervalFrom(Number.NaN)).toEqual({ ms: 1000, clamped: "min" });
    expect(intervalFrom(3601)).toEqual({ ms: 3_600_000, clamped: "max" });
  });

  it("checks the URL, header names and columns", () => {
    expect(apiIssues(apiDoc({ request: { url: "", method: "GET", headers: [] } })).url).toBeNull();
    expect(apiIssues(apiDoc({ request: { url: "ftp://x", method: "GET", headers: [] } })).url).toMatch(/http/);
    const i = apiIssues(
      apiDoc({
        request: { url: "https://a.b", method: "GET", headers: [{ name: "X Key", value: "1" }] },
        mapping: {
          rows: "",
          columns: [
            { key: "a", label: "A", kind: "text", path: "name" },
            { key: "b", label: "B", kind: "text", path: "/b" },
            { key: "b", label: "C", kind: "text", path: "/c" },
          ],
          csv: { header: true, delimiter: "auto" },
        },
      }),
    );
    expect(i.headers[0]).toMatch(/Letters/);
    expect(i.columns[0]).toMatch(/starts with \//);
    expect(i.columns[1]).toMatch(/Another column/);
    expect(i.columns[2]).toMatch(/Another column/);
  });

  it("secret replies merge into the working headers without values", () => {
    const merged = mergeSecrets(
      [
        { name: "X-Key", value: "typed" },
        { name: "Accept", value: "application/json" },
      ],
      [{ name: "X-Key", secret: "s1", stored: true }],
    );
    expect(merged).toEqual([
      { name: "X-Key", value: null, secret: "s1", stored: true },
      { name: "Accept", value: "application/json" },
    ]);
  });

  it("status words and relative times", () => {
    expect(apiShort(status()).text).toBe("Live");
    expect(apiShort(status({ state: "backoff" }))).toEqual({ tally: "warning", text: "Error" });
    expect(apiShort(status({ state: "paused" })).text).toBe("Paused");
    expect(apiShort(status({ state: "fetching", lastOkAt: null })).text).toBe("Waiting");
    expect(apiShort(null).text).toBe("Waiting");
    expect(apiLong(status({ state: "secretMissing" })).text).toBe("Secret missing");
    expect(ago(1000, 5000)).toBe("4 s ago");
    expect(ago(0, 180_000)).toBe("3 min ago");
    expect(until(9000, 1000)).toBe("in 8 s");
    expect(until(1000, 1200)).toBe("now");
  });
});

// ---- The editor ---------------------------------------------------------------------------------

describe("column suggestions and checks against the last test", () => {
  const col = (path: string) => ({ key: "k", label: "K", kind: "text" as const, path });
  const csv = (header: boolean) => ({ rows: "", csv: { header, delimiter: "auto" as const } });

  it("CSV: header names, or column numbers without a header row", () => {
    expect(csvPathOptions(["Team", "Points"], true)).toEqual([
      { value: "Team", label: "Column 1" },
      { value: "Points", label: "Column 2" },
    ]);
    expect(csvPathOptions(["0", "1", "2"], false)).toEqual([
      { value: "0", label: "#1" },
      { value: "1", label: "#2" },
      { value: "2", label: "#3" },
    ]);
  });

  it("JSON: the paths of the plain values in the first rows, with a sample", () => {
    const opts = jsonPathOptions(SAMPLE, "/data/standings");
    expect(opts.map((o) => o.value)).toEqual(["/team/name", "/team/code", "/points", "/logo"]);
    expect(opts[0].label).toBe("Bergen");
    // One object as the only row.
    expect(jsonPathOptions(SAMPLE, "").map((o) => o.value)).toContain("/updated");
    expect(jsonPathOptions(SAMPLE, "/nowhere")).toEqual([]);
  });

  it("warns about columns the last test doesn't have", () => {
    const res = (over: object) => ({ json: null, error: null, csvColumns: null, ...over });
    // CSV with a header row: by name.
    expect(missingInTest(col("Points"), res({ csvColumns: ["Team", "Points"] }), csv(true), "csv")).toBeNull();
    expect(missingInTest(col("Score"), res({ csvColumns: ["Team", "Points"] }), csv(true), "csv")).toBe(
      "The last test has no column “Score”.",
    );
    // Without one: by number.
    expect(missingInTest(col("1"), res({ csvColumns: ["0", "1"] }), csv(false), "csv")).toBeNull();
    expect(missingInTest(col("5"), res({ csvColumns: ["0", "1"] }), csv(false), "csv")).toBe(
      "The last test has 2 columns (0–1).",
    );
    // JSON: in any row.
    const j = { rows: "/data/standings", csv: csv(true).csv };
    expect(missingInTest(col("/logo"), res({ json: SAMPLE }), j, "json")).toBeNull();
    expect(missingInTest(col("/score"), res({ json: SAMPLE }), j, "json")).toBe("/score isn't in any row of the last test.");
    // Nothing to check against: no test, a failed one, no tree.
    expect(missingInTest(col("/score"), null, j, "json")).toBeNull();
    expect(missingInTest(col("/score"), res({ json: SAMPLE, error: "HTTP 500" }), j, "json")).toBeNull();
    expect(missingInTest(col("/score"), res({ json: null }), j, "json")).toBeNull();
  });
});

describe("API editor", () => {
  it("counts as in use while shown: watch on mount, off when hidden and on unmount", async () => {
    const r = await mount();
    expect(api.watchApiSource).toHaveBeenLastCalledWith("api1", true);
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(api.watchApiSource).toHaveBeenLastCalledWith("api1", false);
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(api.watchApiSource).toHaveBeenLastCalledWith("api1", true);
    r.unmount();
    expect(api.watchApiSource).toHaveBeenLastCalledWith("api1", false);
  });

  it("edits the request (URL, method, plain headers, POST body) and autosaves", async () => {
    await mount();
    await fireEvent.input(screen.getByLabelText("URL"), { target: { value: "https://api.example.com/v1/scores" } });
    await fireEvent.change(screen.getByLabelText("Method"), { target: { value: "POST" } });
    await fireEvent.click(screen.getByRole("button", { name: "Add header" }));
    await fireEvent.input(screen.getByLabelText("Header 1 name"), { target: { value: "Accept" } });
    await fireEvent.input(screen.getByLabelText("Header 1 value"), { target: { value: "application/json" } });
    await fireEvent.input(screen.getByLabelText("Body"), { target: { value: '{"round":3}' } });
    await advance(600);
    expect(lastSaved().request).toEqual({
      url: "https://api.example.com/v1/scores",
      method: "POST",
      headers: [{ name: "Accept", value: "application/json" }],
      body: '{"round":3}',
    });
  });

  it("a bad URL blocks saving and says why", async () => {
    await mount();
    await fireEvent.input(screen.getByLabelText("URL"), { target: { value: "example.com" } });
    await advance(600);
    expect(api.saveSource).not.toHaveBeenCalled();
    expect(document.body).toHaveTextContent("Not saved: URL: Enter an http:// or https:// address.");
  });

  it("secret headers: store (after saving), stored text, replace, remove; the value never in the doc", async () => {
    doc = apiDoc({ request: { url: "https://a.example/x", method: "GET", headers: [{ name: "X-Api-Key", value: "abc123" }] } });
    await mount();
    await fireEvent.click(screen.getByRole("button", { name: /Secret: keep X-Api-Key/ }));
    const pw = screen.getByLabelText("Secret value of X-Api-Key") as HTMLInputElement;
    expect(pw.type).toBe("password");
    // The typed plain value moved into the write-only box.
    expect(pw.value).toBe("abc123");
    await fireEvent.input(pw, { target: { value: "s3cr3t" } });
    await fireEvent.click(screen.getByRole("button", { name: "Store" }));
    await advance(0);
    // Pending edits saved first (the header without its value), then the keychain.
    expect(lastSaved().request.headers[0]).toEqual({ name: "X-Api-Key", value: null });
    expect(api.setSourceSecret).toHaveBeenCalledWith("api1", "X-Api-Key", "s3cr3t");
    expect(screen.getByText("Stored in Windows Credential Manager")).toBeTruthy();
    expect(screen.queryByLabelText("Secret value of X-Api-Key")).toBeNull();
    for (const call of api.saveSource.mock.calls) expect(JSON.stringify(call[0])).not.toContain("s3cr3t");

    await fireEvent.click(screen.getByRole("button", { name: "Replace" }));
    expect((screen.getByLabelText("Secret value of X-Api-Key") as HTMLInputElement).value).toBe("");
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    await advance(0);
    expect(api.clearSourceSecret).toHaveBeenCalledWith("api1", "X-Api-Key");
    expect(screen.queryByText("Stored in Windows Credential Manager")).toBeNull();
    expect(screen.getByLabelText("Header 1 value")).toBeTruthy();
  });

  it("a secret missing on this machine asks for it again", async () => {
    doc = apiDoc({ request: { url: "https://a.example/x", method: "GET", headers: [{ name: "Authorization", secret: "s9", stored: false }] } });
    await mount();
    expect(screen.getByText("Secret missing on this machine — enter it again")).toBeTruthy();
    expect(screen.getByLabelText("Secret value of Authorization")).toBeTruthy();
  });

  it("format: CSV shows the header row and delimiter options", async () => {
    await mount();
    expect(screen.queryByLabelText("Delimiter")).toBeNull();
    await fireEvent.change(screen.getByRole("combobox", { name: "Format" }), { target: { value: "csv" } });
    await fireEvent.click(screen.getByRole("switch", { name: "First row is the header" }));
    await fireEvent.change(screen.getByLabelText("Delimiter"), { target: { value: ";" } });
    await advance(600);
    expect(lastSaved().format).toBe("csv");
    expect(lastSaved().mapping.csv).toEqual({ header: false, delimiter: ";" });
  });

  it("test request → tree; pick the rows and a column; pointers fill in, mapped values highlight, preview and hits", async () => {
    await mount();
    expect(screen.queryByRole("tree")).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Test request" }));
    await advance(0);
    expect(api.testApiSource).toHaveBeenCalledWith(expect.objectContaining({ id: "api1", kind: "api" }));
    expect(document.body).toHaveTextContent("HTTP 200");
    const tree = screen.getByRole("tree", { name: "Response" });
    // Expand data, then select standings and use it as rows.
    const node = (ptr: string) => tree.querySelector<HTMLElement>(`[data-ptr="${ptr}"]`)!;
    await fireEvent.click(node("/data"));
    await fireEvent.keyDown(tree, { key: "ArrowRight" });
    await tick();
    await fireEvent.click(node("/data/standings"));
    await fireEvent.click(screen.getByRole("button", { name: "Use as rows" }));
    await tick();
    expect((screen.getByLabelText("Rows") as HTMLInputElement).value).toBe("/data/standings");
    // No columns yet: one per plain value of the first row.
    expect((screen.getByLabelText("Column 1 path") as HTMLInputElement).value).toBe("/points");
    expect((screen.getByLabelText("Column 2 key") as HTMLInputElement).value).toBe("logo");
    expect((screen.getByLabelText("Column 2 kind") as HTMLSelectElement).value).toBe("image");
    expect(node("/data/standings").classList.contains("rows")).toBe(true);
    // The first row is revealed: pick its team name as a column (a relative pointer).
    await fireEvent.click(node("/data/standings/0/team"));
    await fireEvent.keyDown(tree, { key: "ArrowRight" });
    await tick();
    await fireEvent.click(node("/data/standings/0/team/name"));
    await fireEvent.click(screen.getByRole("button", { name: "Add as column" }));
    await tick();
    expect((screen.getByLabelText("Column 3 path") as HTMLInputElement).value).toBe("/team/name");
    expect(node("/data/standings/0/team/name").classList.contains("mapped")).toBe(true);
    expect(node("/data/standings/0/team/name")).toHaveTextContent("name");
    // Per-column rows with a value; the preview shows the mapped rows read-only.
    expect(document.querySelector('[data-hits="logo"]')).toHaveTextContent("1 / 3");
    expect(document.querySelector('[data-hits="name"]')).toHaveTextContent("3 / 3");
    await vi.waitFor(() => expect(screen.getByRole("grid", { name: "Preview of League table" })).toBeTruthy());
    const grid = screen.getByRole("grid", { name: "Preview of League table" });
    expect(grid.getAttribute("aria-readonly")).toBe("true");
    expect(within(grid).getByText("Tromsø")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Add row" })).toBeNull();
    await advance(600);
    expect(lastSaved().mapping.rows).toBe("/data/standings");
    expect(lastSaved().mapping.columns.map((c) => c.path)).toEqual(["/points", "/logo", "/team/name"]);
    expect(lastSaved().columns.map((c) => c.key)).toEqual(["points", "logo", "name"]);
  });

  it("the tree is keyboard navigable: arrows move and expand, Enter picks", async () => {
    await mount();
    await fireEvent.click(screen.getByRole("button", { name: "Test request" }));
    await advance(0);
    const tree = screen.getByRole("tree", { name: "Response" });
    const focused = () => tree.querySelector<HTMLElement>('[tabindex="0"]')!.dataset.ptr;
    expect(focused()).toBe("");
    await fireEvent.keyDown(tree, { key: "ArrowDown" });
    await tick();
    expect(focused()).toBe("/updated");
    await fireEvent.keyDown(tree, { key: "ArrowDown" });
    await fireEvent.keyDown(tree, { key: "ArrowRight" });
    await tick();
    expect(tree.querySelector('[data-ptr="/data"]')!.getAttribute("aria-expanded")).toBe("true");
    await fireEvent.keyDown(tree, { key: "ArrowRight" });
    await tick();
    expect(focused()).toBe("/data/standings");
    await fireEvent.keyDown(tree, { key: "Enter" });
    await tick();
    expect((screen.getByLabelText("Rows") as HTMLInputElement).value).toBe("/data/standings");
    await fireEvent.keyDown(tree, { key: "ArrowLeft" });
    await tick();
    expect(tree.querySelector('[data-ptr="/data/standings"]')!.getAttribute("aria-expanded")).toBe("false");
  });

  it("CSV: header names as buttons add columns", async () => {
    doc = apiDoc({ format: "csv" });
    api.testApiSource.mockResolvedValue(
      testResult({ json: null, csvColumns: ["Team", "Points"], preview: "Team,Points\nBergen,13\n", rows: [{}, {}], hits: {} }),
    );
    await mount();
    await fireEvent.click(screen.getByRole("button", { name: "Test request" }));
    await advance(0);
    await fireEvent.click(screen.getByRole("button", { name: "Add column Points" }));
    await tick();
    expect((screen.getByLabelText("Column 1 path") as HTMLInputElement).value).toBe("Points");
    expect(screen.getByRole("button", { name: "Add column Points" }).classList.contains("mapped")).toBe(true);
    expect(document.body).toHaveTextContent("The mapping changed: test the request again to see it.");
  });

  it("CSV: after a test the column field suggests the headers, still takes typing, and warns", async () => {
    doc = apiDoc({ format: "csv" });
    api.testApiSource.mockResolvedValue(
      testResult({ json: null, csvColumns: ["Team", "Points"], preview: "Team,Points\nBergen,13\n", rows: [{}, {}], hits: {} }),
    );
    await mount();
    await fireEvent.click(screen.getByRole("button", { name: "Test request" }));
    await advance(0);
    await fireEvent.click(screen.getByRole("button", { name: "Add column Team" }));
    await tick();
    const path = screen.getByLabelText("Column 1 path") as HTMLInputElement;
    const list = document.getElementById(path.getAttribute("list")!) as HTMLDataListElement;
    expect([...list.options].map((o) => o.value)).toEqual(["Team", "Points"]);
    // Typed by hand: a name the response doesn't have gets a warning.
    await fireEvent.input(path, { target: { value: "Score" } });
    await tick();
    expect(document.querySelector('[data-missing="team"]')).toHaveTextContent("The last test has no column “Score”.");
    await fireEvent.input(path, { target: { value: "Points" } });
    await tick();
    expect(document.querySelector("[data-missing]")).toBeNull();
  });

  it("CSV without a header row: column numbers, and a warning past the last one", async () => {
    doc = apiDoc({ format: "csv", mapping: { rows: "", columns: [], csv: { header: false, delimiter: "auto" } } });
    api.testApiSource.mockResolvedValue(
      testResult({ json: null, csvColumns: ["0", "1"], preview: "Bergen,13\n", rows: [{}], hits: {} }),
    );
    await mount();
    await fireEvent.click(screen.getByRole("button", { name: "Test request" }));
    await advance(0);
    await fireEvent.click(screen.getByRole("button", { name: "Add column 2" }));
    await tick();
    const path = screen.getByLabelText("Column 1 path") as HTMLInputElement;
    expect(path.value).toBe("1");
    const list = document.getElementById(path.getAttribute("list")!) as HTMLDataListElement;
    expect([...list.options].map((o) => [o.value, o.label])).toEqual([
      ["0", "#1"],
      ["1", "#2"],
    ]);
    await fireEvent.input(path, { target: { value: "4" } });
    await tick();
    expect(document.querySelector("[data-missing]")).toHaveTextContent("The last test has 2 columns (0–1).");
  });

  it("JSON: the path field suggests the rows' values; a path in no row is flagged", async () => {
    doc = apiDoc({
      mapping: {
        rows: "/data/standings",
        columns: [{ key: "points", label: "Points", kind: "text", path: "/points" }],
        csv: { header: true, delimiter: "auto" },
      },
    });
    api.testApiSource.mockResolvedValue(testResult());
    await mount();
    const path = screen.getByLabelText("Column 1 path") as HTMLInputElement;
    // No suggestions or warnings before a test.
    expect(path.hasAttribute("list")).toBe(false);
    await fireEvent.click(screen.getByRole("button", { name: "Test request" }));
    await advance(0);
    const list = document.getElementById(path.getAttribute("list")!) as HTMLDataListElement;
    expect([...list.options].map((o) => o.value)).toEqual(["/team/name", "/team/code", "/points", "/logo"]);
    expect(document.querySelector("[data-missing]")).toBeNull();
    await fireEvent.input(path, { target: { value: "/score" } });
    await tick();
    expect(document.querySelector('[data-missing="points"]')).toHaveTextContent("/score isn't in any row of the last test.");
  });

  it("polling: interval in seconds, clamped to 1 with a message; only when used; the hint", async () => {
    await mount();
    expect(document.body).toHaveTextContent(
      "Once a second at most. For live data, use a source that pushes updates.",
    );
    const every = screen.getByLabelText("Every") as HTMLInputElement;
    expect(every.value).toBe("10");
    await fireEvent.change(every, { target: { value: "0.2" } });
    await tick();
    expect(document.body).toHaveTextContent("The shortest interval is 1 second; set to 1.");
    expect((screen.getByLabelText("Every") as HTMLInputElement).value).toBe("1");
    await fireEvent.click(screen.getByRole("switch", { name: "Poll only when used" }));
    await advance(600);
    expect(lastSaved().poll).toEqual({ intervalMs: 1000, onlyWhenUsed: false, paused: false });
  });

  it("status: state, last and next fetch, error; Retry now and Pause", async () => {
    const { props } = await mount({
      api: status({ state: "backoff", httpStatus: 503, message: "Service unavailable", lastOkAt: Date.now() - 12_000, nextAt: Date.now() + 20_000 }),
    });
    const state = document.querySelector('[data-api-state="backoff"]')!;
    expect(state).toHaveTextContent("Error, retrying");
    expect(state).toHaveTextContent("HTTP 503");
    expect(document.body).toHaveTextContent("12 s ago");
    expect(document.body).toHaveTextContent("in 20 s");
    expect(screen.getByRole("alert")).toHaveTextContent("Service unavailable");
    await advance(2000);
    expect(document.body).toHaveTextContent(/1[34] s ago/);
    await fireEvent.click(screen.getByRole("button", { name: "Retry now" }));
    await advance(0);
    expect(api.retryApiSource).toHaveBeenCalledWith("api1");
    expect(props.onsaved).toHaveBeenLastCalledWith(expect.objectContaining({ api: expect.objectContaining({ state: "fetching" }) }));
    await fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    await advance(0);
    expect(api.setApiPaused).toHaveBeenCalledWith("api1", true);
    expect(screen.getByRole("button", { name: "Resume" })).toBeTruthy();
  });

  it("shows the last good fetch's rows before any test", async () => {
    doc = apiDoc({
      columns: [{ key: "team", label: "Team" }],
      mapping: { rows: "/data", columns: [{ key: "team", label: "Team", kind: "text", path: "/team" }], csv: { header: true, delimiter: "auto" } },
      rows: [{ team: "Bergen" }, { team: "Oslo" }],
      fetchedAt: new Date(Date.now() - 30_000).toISOString(),
    });
    await mount();
    await vi.waitFor(() => expect(screen.getByRole("grid", { name: "Preview of League table" })).toBeTruthy());
    expect(document.body).toHaveTextContent("Last fetch, 30 s ago");
    expect(within(screen.getByRole("grid")).getByText("Oslo")).toBeTruthy();
  });
});

// ---- New source, list ---------------------------------------------------------------------------

describe("API sources in the list and the New source dialog", () => {
  it("New source: API calls createApiSource", async () => {
    api.createApiSource.mockImplementation(async (name: string) => apiDoc({ id: "n9", name }));
    const oncreated = vi.fn();
    render(NewSourceDialog, { open: true, onclose: vi.fn(), oncreated, names: [] });
    await tick();
    await fireEvent.click(screen.getByRole("radio", { name: /^API/ }));
    await fireEvent.input(screen.getByLabelText("Name"), { target: { value: "Scores" } });
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await advance(0);
    expect(api.createApiSource).toHaveBeenCalledWith("Scores");
    expect(api.createSource).not.toHaveBeenCalled();
    expect(oncreated).toHaveBeenCalledWith(expect.objectContaining({ id: "n9", kind: "api" }));
  });

  it("the list shows an API source's kind and status with text", () => {
    render(SourceList, {
      sources: [summary(apiDoc(), status({ state: "error", message: "401" })), { ...summary(apiDoc({ id: "b", name: "Feed" }), status()) }],
      selectedId: "api1",
      onselect: vi.fn(),
    });
    const [first, second] = screen.getAllByRole("option");
    expect(first).toHaveTextContent("League table");
    expect(first).toHaveTextContent("Error");
    expect(first).toHaveTextContent("API · 0 rows");
    expect(second).toHaveTextContent("Live");
  });

  it("the list calls a CSV source a CSV link", () => {
    render(SourceList, {
      sources: [summary(apiDoc({ format: "csv" }))],
      selectedId: "api1",
      onselect: vi.fn(),
    });
    expect(screen.getByRole("option")).toHaveTextContent("CSV link · 0 rows");
    expect(screen.getByRole("option")).not.toHaveTextContent("API");
  });
});

// ---- Simple mode --------------------------------------------------------------------------------

describe("Simple mode", () => {
  beforeEach(() => {
    prefs.set({ theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 }, uiMode: "simple" });
    api.setPrefs.mockImplementation(async (p: unknown) => p);
  });
  afterEach(() => prefs.set(null));

  const editor = () =>
    render(ApiEditor, { id: "api1", usedBy: [], api: status(), onsaved: vi.fn(), onduplicated: vi.fn(), ondeleted: vi.fn() });

  it("New source offers CSV link, not API; CSV link is an API source set to CSV", async () => {
    api.createApiSource.mockImplementation(async (name: string) => apiDoc({ id: "n9", name }));
    const oncreated = vi.fn();
    render(NewSourceDialog, { open: true, onclose: vi.fn(), oncreated, names: [] });
    await tick();
    expect(screen.getAllByRole("radio").map((r) => r.closest("label")?.querySelector(".kind-label")?.textContent)).toEqual([
      "Data table",
      "Schedule",
      "CSV link",
    ]);
    await fireEvent.click(screen.getByRole("radio", { name: /^CSV link/ }));
    await fireEvent.input(screen.getByLabelText("Name"), { target: { value: "Sheet" } });
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await advance(0);
    expect(api.createApiSource).toHaveBeenCalledWith("Sheet");
    expect(lastSaved()).toMatchObject({ id: "n9", format: "csv" });
    expect(oncreated).toHaveBeenCalledWith(expect.objectContaining({ id: "n9", format: "csv" }));
  });

  it("Advanced mode offers both", async () => {
    prefs.set({ theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 }, uiMode: "advanced" });
    render(NewSourceDialog, { open: true, onclose: vi.fn(), names: [] });
    await tick();
    expect(screen.getByRole("radio", { name: /^CSV link/ })).toBeTruthy();
    expect(screen.getByRole("radio", { name: /^API/ })).toBeTruthy();
  });

  it("a JSON source shows its status and data with the hint; Switch to Advanced shows the editor", async () => {
    editor();
    await vi.waitFor(() => expect(screen.getByText(/The request and mapping: available in Advanced mode/)).toBeTruthy());
    expect(screen.queryByLabelText("URL")).toBeNull();
    expect(screen.queryByText("Mapping")).toBeNull();
    expect(screen.getByText("Preview")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Switch to Advanced" }));
    await advance(0);
    expect(screen.getByLabelText("URL")).toBeTruthy();
    expect(screen.queryByText(/available in Advanced mode/)).toBeNull();
  });

  it("a CSV source is a CSV link: the URL, header row and delimiter, no method, headers or format", async () => {
    doc = apiDoc({ format: "csv", request: { url: "https://docs.google.com/x.csv", method: "GET", headers: [], body: null } });
    editor();
    await vi.waitFor(() => expect(screen.getByLabelText("URL")).toBeTruthy());
    expect(screen.getByText("CSV link")).toBeTruthy();
    expect(screen.getByText("Link")).toBeTruthy();
    expect(screen.queryByLabelText("Method")).toBeNull();
    expect(screen.queryByText("Headers")).toBeNull();
    // Only the section is labelled Format (no JSON / CSV select).
    expect(screen.getAllByLabelText("Format").map((e) => e.tagName)).toEqual(["SECTION"]);
    expect(screen.getByLabelText("Delimiter")).toBeTruthy();
    expect(screen.queryByText(/available in Advanced mode/)).toBeNull();
    // The Mapping speaks of the link, not a request.
    expect(screen.getByRole("button", { name: "Test link" })).toBeTruthy();
    expect(screen.getByText("Test the link, then pick the columns to use.")).toBeTruthy();
    expect(screen.queryByText(/request/i)).toBeNull();
  });

  it("a CSV link without a URL asks for the link", async () => {
    doc = apiDoc({ format: "csv", request: { url: "", method: "GET", headers: [], body: null } });
    editor();
    await vi.waitFor(() => expect(screen.getByLabelText("URL")).toBeTruthy());
    expect(screen.getByText("Paste the link, then test it.")).toBeTruthy();
  });
});
