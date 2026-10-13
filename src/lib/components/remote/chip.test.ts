// Control's remote control chip and its panel: words, people, Lock remote take, "Move it here?".
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RemoteView } from "$lib/types";

const api = vi.hoisted(() => ({
  remoteSet: vi.fn(async () => ({})),
  remoteLock: vi.fn(async () => ({})),
  remoteTakeover: vi.fn(async () => {}),
  remoteNewShow: vi.fn(async () => ({})),
  remoteEnd: vi.fn(async () => ({})),
  remoteStartAgain: vi.fn(async () => ({})),
  remoteActivity: vi.fn(async () => [{ at: 0, name: "Kari N.", role: "operator", text: "Kari N. took Lower third" }]),
  remoteView: vi.fn(async () => null),
  cuePosition: vi.fn(async () => ({ current: null, next: null })),
}));
vi.mock("$lib/api/remote", () => api);
vi.mock("$lib/api", () => ({
  on: () => () => {},
  openPreferences: vi.fn(async () => {}),
  toAppError: (e: unknown) => ({ code: "internal", message: String(e) }),
}));
vi.mock("$lib/components/license/open", () => ({ openInBrowser: vi.fn() }));

import RemoteChip from "./RemoteChip.svelte";
import { remote } from "./state.svelte";

const live: RemoteView = {
  state: "live",
  runId: "run-1",
  resumed: false,
  on: true,
  takeLocked: false,
  crew: [
    { id: "a", name: "Kari N.", role: "operator", anonymous: false, sinceMs: 0 },
    { id: null, name: "Guest 3", role: "viewer", anonymous: true, sinceMs: 0 },
  ],
  showId: "show-1",
  crewUrl: "https://control.kinetra.live/s/show-1/crew",
  controllerUrl: "https://control.kinetra.live/s/show-1",
  serverVersion: "1",
  rttMs: 42,
  licenseWarning: null,
  lastError: null,
  hasShowId: true,
  available: true,
  screensHidden: 0,
};

beforeEach(() => {
  // jsdom has no popover API.
  HTMLElement.prototype.showPopover ??= function (this: HTMLElement) {
    this.setAttribute("data-open", "");
  };
});
afterEach(() => {
  cleanup();
  remote.view = null;
  vi.clearAllMocks();
});

describe("RemoteChip", () => {
  it("shows nothing while off", () => {
    remote.view = { ...live, on: false, state: "off" } as RemoteView;
    const { container } = render(RemoteChip);
    expect(container.textContent?.trim()).toBe("");
  });

  it("says who is connected and lists them with their roles", async () => {
    remote.view = live;
    render(RemoteChip);
    expect(screen.getByRole("button", { name: "Remote control: 2 connected" })).toBeTruthy();
    expect(screen.getByText("2 remote users connected")).toBeTruthy();
    expect(screen.getByText("Kari N.")).toBeTruthy();
    expect(screen.getByText(/Viewer · since/)).toBeTruthy();
    expect(screen.getByText(/42 ms/)).toBeTruthy();
  });

  it("says when remote users don't see every screen", () => {
    remote.view = { ...live, screensHidden: 3 };
    render(RemoteChip);
    expect(screen.getByText("Remote users see the first 32 screens.")).toBeTruthy();
  });

  it("locks remote take", async () => {
    remote.view = live;
    render(RemoteChip);
    await fireEvent.click(screen.getByRole("switch", { name: "Lock remote take", hidden: true }));
    expect(api.remoteLock).toHaveBeenCalledWith(true);
  });

  it("asks Move it here? and offers a separate show", async () => {
    remote.view = { ...live, state: "showInUse", machine: "Studio PC" } as RemoteView;
    render(RemoteChip);
    expect(screen.getByText("Remote control is running on Studio PC.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Move it here", hidden: true }));
    await waitFor(() => expect(api.remoteTakeover).toHaveBeenCalled());
    await fireEvent.click(screen.getByRole("button", { name: "Make this a separate show", hidden: true }));
    await waitFor(() => expect(api.remoteNewShow).toHaveBeenCalled());
  });

  it("offers Start again after the run ended", async () => {
    remote.view = { ...live, state: "ended" } as RemoteView;
    render(RemoteChip);
    expect(screen.getByRole("button", { name: "Remote control ended" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Start again", hidden: true }));
    await waitFor(() => expect(api.remoteStartAgain).toHaveBeenCalled());
  });
});
