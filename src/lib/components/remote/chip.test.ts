// The remote control chip and its panel: words, the control link, collaborators, Lock remote
// take, "Move it here?", opening on switch-on and Invite collaborator.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
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
  remoteCollaborators: vi.fn(async () => ({ showId: "show-1", limit: { max: 5, used: 0 }, collaborators: [] })),
}));
vi.mock("$lib/api/remote", () => api);
vi.mock("$lib/api", () => ({
  on: () => () => {},
  openPreferences: vi.fn(async () => {}),
  toAppError: (e: unknown) => ({ code: "internal", message: String(e) }),
}));
vi.mock("$lib/components/license/open", () => ({ openInBrowser: vi.fn() }));

import { openInBrowser } from "$lib/components/license/open";
import RemoteChip from "./RemoteChip.svelte";
import { remote } from "./state.svelte";

// The invite dialog is a lazy chunk (`import()` on click). Loaded here once, so the tests wait on the
// dialog, not on Vite compiling it on first use (over waitFor's 1 s under the full suite's load).
beforeAll(async () => {
  await import("./InviteDialog.svelte");
});

const ALL = ["take_clear", "edit_fields", "edit_live_fields", "run_cues", "studio", "timers", "variants", "rows", "button_panel", "watch_preview"];

const live: RemoteView = {
  state: "live",
  runId: "run-1",
  resumed: false,
  on: true,
  takeLocked: false,
  crew: [
    { id: "a", name: "Kari N.", role: "", permissions: ALL, watching: [], sinceMs: 0 },
    { id: "b", name: "Jonas B.", role: "", permissions: ["edit_fields", "rows", "watch_preview"], watching: [{ screen: "s1", kind: "program" }], sinceMs: 0 },
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
  protocol: 2,
  features: ["permissions", "invites"],
  invites: true,
  screensHidden: 0,
};

beforeEach(() => {
  // jsdom has no popover API.
  HTMLElement.prototype.showPopover ??= function (this: HTMLElement) {
    this.setAttribute("data-open", "");
  };
  HTMLElement.prototype.hidePopover ??= function (this: HTMLElement) {
    this.removeAttribute("data-open");
  };
});
afterEach(() => {
  cleanup();
  remote.view = null;
  remote.justOn = false;
  vi.clearAllMocks();
});

describe("RemoteChip", () => {
  it("shows nothing while off", () => {
    remote.view = { ...live, on: false, state: "off" } as RemoteView;
    const { container } = render(RemoteChip);
    expect(container.textContent?.trim()).toBe("");
  });

  it("says who is connected and lists them with what they may do", () => {
    remote.view = live;
    render(RemoteChip);
    expect(screen.getByRole("button", { name: "Remote control: 2 connected" })).toBeTruthy();
    expect(screen.getByText("2 collaborators connected")).toBeTruthy();
    expect(screen.getByText("Kari N.")).toBeTruthy();
    expect(screen.getByText(/^Full operator · since/)).toBeTruthy();
    expect(screen.getByText(/^Data editor · watching · since/)).toBeTruthy();
    expect(screen.getByText(/42 ms/)).toBeTruthy();
    expect(document.body.textContent).not.toMatch(/viewer|guest/i);
  });

  it("names a protocol 1 crew by what its role allowed", () => {
    remote.view = {
      ...live,
      protocol: 1,
      invites: false,
      features: [],
      crew: [{ id: "c", name: "Ola", role: "data_editor", permissions: null, watching: [], sinceMs: 0 }],
    };
    render(RemoteChip);
    expect(screen.getByText(/^Data editor · since/)).toBeTruthy();
  });

  it("says when collaborators don't see every screen", () => {
    remote.view = { ...live, screensHidden: 3 };
    render(RemoteChip);
    expect(screen.getByText("Collaborators see the first 32 screens.")).toBeTruthy();
  });

  it("shows the control link with Copy, and waits for it while connecting", async () => {
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    remote.view = live;
    render(RemoteChip);
    expect(screen.getByText("https://control.kinetra.live/s/show-1")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Copy link", hidden: true }));
    expect(writeText).toHaveBeenCalledWith("https://control.kinetra.live/s/show-1");
    cleanup();
    remote.view = { ...live, state: "connecting", controllerUrl: null } as RemoteView;
    render(RemoteChip);
    expect(screen.getByText("Getting the link…")).toBeTruthy();
  });

  it("opens the panel each time remote control is switched on", async () => {
    remote.view = live;
    remote.justOn = true;
    render(RemoteChip);
    await waitFor(() => expect(document.querySelector("[popover][data-open]")).toBeTruthy());
    expect(remote.justOn).toBe(false);
  });

  it("Invite collaborator opens the dialog, or the control site while the server has no invites", async () => {
    remote.view = { ...live, invites: false, features: ["permissions"] };
    render(RemoteChip);
    await fireEvent.click(screen.getByRole("button", { name: "Invite collaborator ↗", hidden: true }));
    expect(openInBrowser).toHaveBeenCalledWith("https://control.kinetra.live/s/show-1/crew");
    expect(api.remoteCollaborators).not.toHaveBeenCalled();
    cleanup();
    remote.view = live;
    render(RemoteChip);
    await fireEvent.click(screen.getByRole("button", { name: "Invite collaborator…", hidden: true }));
    await waitFor(() => expect(api.remoteCollaborators).toHaveBeenCalled());
    expect(await screen.findByRole("heading", { name: "Invite collaborator", hidden: true })).toBeTruthy();
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

describe("RemoteState", () => {
  it("marks a switch-on seen as off → on, never a view that was already on or unknown", async () => {
    const { RemoteState } = await import("./state.svelte");
    const s = new RemoteState();
    // Unknown (a page reload while on): not a switch-on.
    s.apply(live);
    expect(s.justOn).toBe(false);
    s.apply({ ...live, on: false, state: "off" } as RemoteView);
    s.apply({ ...live, state: "connecting" } as RemoteView);
    expect(s.justOn).toBe(true);
    // Later changes while on don't set it again once the chip has cleared it.
    s.justOn = false;
    s.apply(live);
    expect(s.justOn).toBe(false);
    // Switched off before the chip saw it: cleared.
    s.apply({ ...live, on: false, state: "off" } as RemoteView);
    s.apply({ ...live, state: "connecting" } as RemoteView);
    s.apply({ ...live, on: false, state: "off" } as RemoteView);
    expect(s.justOn).toBe(false);
  });
});
