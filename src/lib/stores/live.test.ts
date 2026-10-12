import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FilesChanged, LiveScreenView, Project } from "$lib/types";

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Handler>() }));
const api = vi.hoisted(() => ({
  listScreens: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  on: vi.fn(),
}));
vi.mock("$lib/api", () => api);
vi.mock("$lib/api/live", () => api);

import { live } from "./live.svelte";
import { project } from "./project.svelte";

function view(id: string, onAir = false, name = `Screen ${id}`): LiveScreenView {
  return {
    screen: { id, name, width: 1920, height: 1080, background: "transparent" },
    layers: [{ id: "l", name: "Layer", onAir, fields: {}, fieldDefs: [], pending: false }],
  };
}

const proj = (path: string): Project => ({
  path,
  name: path,
  createdAt: "2026-01-01T00:00:00Z",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 } },
});

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

const emit = (name: string, payload: unknown) => events.handlers.get(name)?.(payload);
const flush = () => new Promise((r) => setTimeout(r, 0));

let offs: (() => void)[] = [];

beforeEach(() => {
  events.handlers.clear();
  api.on.mockImplementation((name: string, h: Handler) => {
    events.handlers.set(name, h);
    return () => events.handlers.delete(name);
  });
  project.current = proj("C:\\a");
});

afterEach(() => {
  for (const off of offs) off();
  offs = [];
  vi.clearAllMocks();
});

describe("live store", () => {
  it("loads the list on the first watch and applies live-changed events", async () => {
    api.listScreens.mockResolvedValue([view("a"), view("b")]);
    offs.push(live.watch());
    await flush();
    expect(live.loaded).toBe(true);
    expect(live.list.map((v) => v.screen.id)).toEqual(["a", "b"]);
    emit("live-changed", view("b", true));
    expect(live.screens.get("b")?.layers[0].onAir).toBe(true);
    // Order kept.
    expect(live.list.map((v) => v.screen.id)).toEqual(["a", "b"]);
  });

  it("a stale list never overwrites a fresher event", async () => {
    const list = deferred<LiveScreenView[]>();
    api.listScreens.mockReturnValue(list.promise);
    offs.push(live.watch());
    emit("live-changed", view("a", true));
    emit("live-changed", view("new", true));
    list.resolve([view("a", false), view("b", false)]);
    await flush();
    expect(live.screens.get("a")?.layers[0].onAir).toBe(true);
    expect(live.screens.get("b")?.layers[0].onAir).toBe(false);
    // A screen created after the request went out is kept.
    expect(live.screens.has("new")).toBe(true);
  });

  it("ignores reserved screens and events after the last unwatch", async () => {
    api.listScreens.mockResolvedValue([view("a")]);
    const off = live.watch();
    await flush();
    emit("live-changed", view("__preview", true));
    expect(live.screens.has("__preview")).toBe(false);
    off();
    expect(events.handlers.has("live-changed")).toBe(false);
  });

  it("is ref-counted: one subscription for many watchers", async () => {
    api.listScreens.mockResolvedValue([view("a")]);
    const a = live.watch();
    const b = live.watch();
    expect(api.on).toHaveBeenCalledTimes(2); // live-changed + project-files-changed, once
    a();
    a(); // idempotent
    expect(events.handlers.has("live-changed")).toBe(true);
    b();
    expect(events.handlers.has("live-changed")).toBe(false);
  });

  it("putLayer replaces one layer; unknown screens are ignored", async () => {
    api.listScreens.mockResolvedValue([view("a")]);
    offs.push(live.watch());
    await flush();
    live.putLayer("a", { id: "l", name: "Layer", onAir: true, fields: { x: "1" }, fieldDefs: [], pending: false });
    expect(live.screens.get("a")?.layers[0].fields).toEqual({ x: "1" });
    live.putLayer("zz", { id: "l", name: "Layer", onAir: true, fields: {}, fieldDefs: [], pending: false });
    expect(live.screens.has("zz")).toBe(false);
  });

  it("reloads when a screen file changes on disk; another project starts empty", async () => {
    api.listScreens.mockResolvedValue([view("a")]);
    const off = live.watch();
    await flush();
    api.listScreens.mockResolvedValue([view("a"), view("c")]);
    emit("project-files-changed", { kind: "screen", id: "c", change: "changed", message: null } satisfies FilesChanged);
    await flush();
    expect(live.list.map((v) => v.screen.id)).toEqual(["a", "c"]);
    off();

    project.current = proj("C:\\b");
    const list = deferred<LiveScreenView[]>();
    api.listScreens.mockReturnValue(list.promise);
    offs.push(live.watch());
    expect(live.screens.size).toBe(0);
    expect(live.loaded).toBe(false);
    list.resolve([view("z")]);
    await flush();
    expect(live.list.map((v) => v.screen.id)).toEqual(["z"]);
  });
});
