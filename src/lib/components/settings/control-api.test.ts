// Preferences › Network & API › Control API: the switch, the address, the token (Copy, New token
// after a confirm) and the connected programs.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ControlApiView, ServerStatus } from "$lib/types";

const events = vi.hoisted(() => ({ handlers: new Map<string, (p: unknown) => void>() }));
const api = vi.hoisted(() => ({
  controlApiView: vi.fn(),
  controlApiSet: vi.fn(),
  controlApiToken: vi.fn(async () => "kt_secret"),
  controlApiRegenerateToken: vi.fn(async () => "kt_new"),
  controlApiDisconnect: vi.fn(async () => {}),
  onControlApi: vi.fn((h: (v: ControlApiView) => void) => {
    events.handlers.set("control-api", h as (p: unknown) => void);
    return () => events.handlers.delete("control-api");
  }),
}));
const server = vi.hoisted(() => ({ status: null as ServerStatus | null, watch: () => () => {} }));
vi.mock("$lib/api/control-api", () => api);
vi.mock("$lib/api", () => ({
  on: () => () => {},
  toAppError: (e: unknown) => ({ code: "internal", message: String(e) }),
}));
vi.mock("$lib/stores/server.svelte", () => ({ server }));

import ControlApiSection from "./ControlApiSection.svelte";
import { toasts } from "$lib/stores/toasts.svelte";

const off: ControlApiView = { enabled: false, clients: [] };
const on: ControlApiView = { enabled: true, clients: [] };
const running: ServerStatus = {
  state: "running",
  port: 7770,
  url: "http://127.0.0.1:7770",
  lanUrl: "http://192.168.1.20:7770",
};

const writeText = vi.fn(async (_: string) => {});

beforeEach(() => {
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
    this.removeAttribute("open");
  };
  server.status = running;
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  toasts.list.splice(0);
});

const toggle = () => screen.getByRole("switch", { name: "Control API" });

describe("Control API section", () => {
  it("is off by default and shows nothing else", async () => {
    api.controlApiView.mockResolvedValue(off);
    render(ControlApiSection);
    await waitFor(() => expect(api.controlApiView).toHaveBeenCalled());
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(screen.getByText("For Bitfocus Companion, Stream Deck and scripts.")).toBeTruthy();
    expect(screen.queryByText("API token")).toBeNull();
  });

  it("switching on shows the addresses Companion needs and the token row", async () => {
    api.controlApiView.mockResolvedValue(off);
    api.controlApiSet.mockResolvedValue(on);
    render(ControlApiSection);
    await waitFor(() => expect(api.controlApiView).toHaveBeenCalled());
    await fireEvent.click(toggle());
    await waitFor(() => expect(api.controlApiSet).toHaveBeenCalledWith(true));
    await tick();
    expect(screen.getByText("127.0.0.1:7770").classList.contains("mono")).toBe(true);
    expect(screen.getByText("192.168.1.20:7770")).toBeTruthy();
    expect(screen.getByText("Nothing connected. Add the Kinetra module in Companion.")).toBeTruthy();
    // The token is never shown; Copy puts it on the clipboard.
    expect(screen.queryByText("kt_secret")).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Copy token" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("kt_secret"));
    expect(toasts.list.at(-1)?.message).toBe("Copied the API token");
  });

  it("snaps back when switching fails", async () => {
    api.controlApiView.mockResolvedValue(off);
    api.controlApiSet.mockRejectedValue("Keychain unavailable");
    render(ControlApiSection);
    await waitFor(() => expect(api.controlApiView).toHaveBeenCalled());
    await fireEvent.click(toggle());
    await waitFor(() => expect(toasts.list.at(-1)?.message).toBe("Keychain unavailable"));
    expect(toggle().getAttribute("aria-checked")).toBe("false");
  });

  it("without a project there is no address yet", async () => {
    server.status = { state: "stopped" };
    api.controlApiView.mockResolvedValue(on);
    render(ControlApiSection);
    await waitFor(() => expect(screen.getByText("Open a project to start the server.")).toBeTruthy());
  });

  it("a new token needs a confirm", async () => {
    api.controlApiView.mockResolvedValue(on);
    render(ControlApiSection);
    await waitFor(() => expect(screen.getByRole("button", { name: "New token…" })).toBeTruthy());
    await fireEvent.click(screen.getByRole("button", { name: "New token…" }));
    expect(screen.getByText("Connected programs disconnect and need the new token.")).toBeTruthy();
    expect(api.controlApiRegenerateToken).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole("button", { name: "Make new token" }));
    await waitFor(() => expect(api.controlApiRegenerateToken).toHaveBeenCalledOnce());
    await waitFor(() => expect(screen.queryByText("Make a new token?")).toBeNull());
  });

  it("lists connected programs from the event and disconnects one", async () => {
    api.controlApiView.mockResolvedValue(on);
    render(ControlApiSection);
    await waitFor(() => expect(events.handlers.has("control-api")).toBe(true));
    events.handlers.get("control-api")?.({
      enabled: true,
      clients: [{ id: 7, name: "companion-module-kinetra-graphics/1.0.0", addr: "192.168.1.30", since: Date.now() }],
    });
    await tick();
    expect(screen.getByText("companion-module-kinetra-graphics/1.0.0")).toBeTruthy();
    expect(screen.getByText("192.168.1.30").classList.contains("mono")).toBe(true);
    expect(screen.getByText("just now")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));
    expect(api.controlApiDisconnect).toHaveBeenCalledWith(7);
  });
});
