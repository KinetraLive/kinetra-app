import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ServerStatus } from "$lib/types";

type Handler = (payload: unknown) => void;

const events = vi.hoisted(() => ({ handlers: [] as Handler[], off: vi.fn() }));
const api = vi.hoisted(() => ({
  serverStatus: vi.fn(),
  on: vi.fn(),
}));
vi.mock("$lib/api", () => api);
// Domain modules share the same mock functions (the split only changes where they are imported from).
vi.mock("$lib/api/live", () => api);

import { lanScreenUrl, screenUrl, server, statusText } from "./server.svelte";

const running: ServerStatus = {
  state: "running",
  port: 7770,
  url: "http://127.0.0.1:7770",
  lanUrl: "http://192.168.1.20:7770",
};

const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  vi.clearAllMocks();
  events.handlers.length = 0;
  server.status = null;
  api.serverStatus.mockResolvedValue({ state: "stopped" });
  api.on.mockImplementation((_name: string, h: Handler) => {
    events.handlers.push(h);
    return events.off;
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("server store", () => {
  it("shares one subscription and one load between watchers, and releases it with the last", async () => {
    const a = server.watch();
    const b = server.watch();
    expect(api.on).toHaveBeenCalledTimes(1);
    expect(api.on).toHaveBeenCalledWith("server-status", expect.any(Function));
    expect(api.serverStatus).toHaveBeenCalledTimes(1);
    await flush();
    expect(server.status).toEqual({ state: "stopped" });

    a();
    a(); // idempotent
    expect(events.off).not.toHaveBeenCalled();
    b();
    expect(events.off).toHaveBeenCalledTimes(1);
  });

  it("updates status from server-status events", async () => {
    const off = server.watch();
    await flush();
    events.handlers[0](running);
    expect(server.status).toEqual(running);
    off();
  });

  it("never lets a late load overwrite a newer event", async () => {
    let resolve!: (s: ServerStatus) => void;
    api.serverStatus.mockReturnValue(new Promise((r) => (resolve = r)));
    const off = server.watch();
    events.handlers[0](running);
    resolve({ state: "stopped" });
    await flush();
    expect(server.status).toEqual(running);
    off();
  });

  it("builds screen URLs only while running", () => {
    expect(screenUrl(running, "test")).toBe("http://127.0.0.1:7770/screen/test");
    expect(lanScreenUrl(running, "test")).toBe("http://192.168.1.20:7770/screen/test");
    expect(screenUrl({ state: "starting", port: 7770 }, "test")).toBeNull();
    expect(lanScreenUrl({ ...running, lanUrl: null }, "test")).toBeNull();
    expect(screenUrl(null, "test")).toBeNull();
    expect(statusText({ state: "error", code: "portInUse", port: 7770, message: "" })).toBe(
      "Port 7770 is in use — choose another port.",
    );
  });
});
