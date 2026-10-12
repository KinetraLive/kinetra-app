// Simple mode in the shell: the sidebar and the palette hide Flows, /flows shows the Advanced mode
// hint instead of its pages, and switching mode shows them again.
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { createRawSnippet, tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Prefs } from "$lib/types";

const api = vi.hoisted(() => ({
  getPrefs: vi.fn(),
  setPrefs: vi.fn(async (p: unknown) => p),
  getUiState: vi.fn(),
  setUiState: vi.fn(async (u: unknown) => u),
  on: vi.fn(() => () => {}),
  openPreferences: vi.fn(async () => {}),
}));
vi.mock("$lib/api", () => ({ ...api, toAppError: (e: unknown) => ({ code: "internal", message: String(e) }) }));
vi.mock("$app/navigation", () => ({ goto: vi.fn(async () => {}) }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/dashboard") } }));

import { prefs } from "$lib/stores/prefs.svelte";
import { navCommands } from "./nav";
import Sidebar from "./Sidebar.svelte";
import SimpleGate from "./SimpleGate.svelte";

const withMode = (uiMode: Prefs["uiMode"]): Prefs => ({ theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 }, uiMode });
const ctx = { route: "dashboard", project: true, screenId: null, layerIds: [], studio: false } as const;

beforeEach(() => prefs.set(withMode("simple")));
afterEach(() => {
  cleanup();
  prefs.set(null);
});

describe("Simple mode", () => {
  it("the sidebar hides Flows; Advanced shows it", async () => {
    render(Sidebar);
    expect(screen.queryByRole("link", { name: "Flows" })).toBeNull();
    expect(screen.getByRole("link", { name: "Control" })).toBeTruthy();
    prefs.set(withMode("advanced"));
    await tick();
    expect(screen.getByRole("link", { name: "Flows" })).toBeTruthy();
  });

  it("Go to Flows and New flow aren't available (their keys neither)", () => {
    const cmds = navCommands(() => {});
    const flows = cmds.find((c) => c.id === "nav.flows")!;
    const create = cmds.find((c) => c.id === "flows.new")!;
    expect(flows.when?.(ctx)).toBe(false);
    expect(create.when?.(ctx)).toBe(false);
    expect(cmds.find((c) => c.id === "nav.control")!.when).toBeUndefined();
    prefs.set(withMode("advanced"));
    expect(flows.when?.(ctx)).toBe(true);
  });

  it("/flows shows the hint instead of the page; Switch to Advanced shows the page", async () => {
    const children = createRawSnippet(() => ({ render: () => "<p>flow page</p>" }));
    render(SimpleGate, { path: "/flows/abc", children });
    expect(screen.queryByText("flow page")).toBeNull();
    expect(await screen.findByRole("heading", { name: "Flows" })).toBeTruthy();
    expect(screen.getByText(/Available in Advanced mode/)).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Switch to Advanced" }));
    await vi.waitFor(() => expect(screen.getByText("flow page")).toBeTruthy());
    expect(api.setPrefs).toHaveBeenCalledWith(expect.objectContaining({ uiMode: "advanced" }));
  });

  it("other pages show as usual in Simple mode", () => {
    const children = createRawSnippet(() => ({ render: () => "<p>other page</p>" }));
    render(SimpleGate, { path: "/control", children });
    expect(screen.getByText("other page")).toBeTruthy();
    cleanup();
    render(SimpleGate, { path: "/flowsheet", children });
    expect(screen.getByText("other page")).toBeTruthy();
  });
});
