import { readFileSync } from "node:fs";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LicenseView } from "$lib/types";

const handlers = vi.hoisted(() => new Map<string, (p: unknown) => void>());
const api = vi.hoisted(() => ({
  licenseView: vi.fn(),
  signInStart: vi.fn(),
  signInCancel: vi.fn(),
  cloudLinks: vi.fn(),
  licenseActivate: vi.fn(),
  licenseDeactivate: vi.fn(),
  licenseMoveHere: vi.fn(),
  licenseRedeem: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  on: (name: string, h: (p: unknown) => void) => {
    handlers.set(name, h);
    return () => handlers.delete(name);
  },
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e ? e : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/license", () => api);
vi.mock("$lib/api/ai", () => ({ aiJobs: vi.fn(() => Promise.resolve([])), aiCancel: vi.fn() }));
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => opener);

import { API_MESSAGES, cloudMessage, formatKey, keyComplete } from "./messages";
import LicenseHost from "./LicenseHost.svelte";
import { license } from "./state.svelte";

function view(over: Partial<LicenseView> = {}): LicenseView {
  return {
    signedIn: false,
    account: null,
    status: "none",
    access: "free",
    plan: null,
    planName: null,
    entitlements: ["core.app"],
    features: { ai: false, polls: false },
    pending: null,
    endsAt: null,
    updatesUntil: null,
    fullUntil: null,
    reducedUntil: null,
    offlineDaysLeft: 0,
    lastCheckAt: null,
    online: null,
    machineName: "EDIT-PC",
    slot: null,
    licenses: [],
    upgradeBlocked: null,
    beta: null,
    notice: null,
    welcome: false,
    onAir: false,
    signin: { state: "idle" },
    ...over,
  };
}

async function show(v: LicenseView) {
  license.view = v;
  await tick();
}

beforeEach(() => {
  license.dismissed = { movePro: "", seats: "", beta: false };
  license.pendingKey = null;
  render(LicenseHost);
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("messages", () => {
  it("has a message for every error code in docs/API.md", () => {
    const doc = readFileSync("docs/API.md", "utf8");
    const codes = new Set([...doc.matchAll(/^\| \d{3}(?:, \d{3})? \| `([a-z_]+)` \|/gm)].map((m) => m[1]));
    codes.add("ai_job_gone");
    for (const m of doc.matchAll(/`(ai_[a-z_]+)`/g)) codes.add(m[1]);
    expect(codes.size).toBeGreaterThan(30);
    for (const c of codes) expect(Object.keys(API_MESSAGES), c).toContain(c);
  });

  it("maps cloud errors to human copy, or shows the server's message where API.md says so", () => {
    expect(cloudMessage({ code: "cloud", apiCode: "access_denied", message: "x" })).toBe("Sign-in was denied.");
    expect(cloudMessage({ code: "cloud", apiCode: "invalid_license_key", message: "This key was already used." })).toBe(
      "This key was already used.",
    );
    expect(cloudMessage({ code: "offline", message: "Kinetra can't reach the server." })).toBe("Kinetra can't reach the server.");
  });

  it("formats keys as typed, forgiving any shape", () => {
    expect(formatKey("kntr4b7")).toBe("KNTR-4B7");
    expect(formatKey("kntr 4b7x 9q2z hhhh")).toBe("KNTR-4B7X-9Q2Z-HHHH");
    expect(formatKey("4B7X-9Q2Z-HHHH-EXTRA")).toBe("KNTR-4B7X-9Q2Z-HHHH");
    expect(formatKey("o0l1i")).toBe(""); // not in the alphabet
    expect(keyComplete("KNTR-4B7X-9Q2Z-HHHH")).toBe(true);
    expect(keyComplete("KNTR-4B7X")).toBe(false);
  });
});

describe("license host", () => {
  it("first launch: sign in (opens the browser) or a key; no way around it", async () => {
    api.signInStart.mockResolvedValue({ state: "waiting", userCode: "RKPH-XHVM", url: "https://account.kinetra.live/device?code=RKPH-XHVM", expiresAt: 0 });
    await show(view({ welcome: true }));
    await fireEvent.click(screen.getByRole("button", { name: "Sign in to get your free license" }));
    await vi.waitFor(() => expect(opener.openUrl).toHaveBeenCalledWith("https://account.kinetra.live/device?code=RKPH-XHVM"));
    // The core reports progress: the code shows.
    await show(view({ welcome: true, signin: { state: "waiting", userCode: "RKPH-XHVM", url: "u", expiresAt: 0 } }));
    expect(screen.getByText("RKPH-XHVM")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(api.signInCancel).toHaveBeenCalled();
    await show(view({ welcome: true }));
    expect(screen.queryByRole("button", { name: /continue/i })).toBeNull();
  });

  it("a license key is formatted, then redeemed after sign-in", async () => {
    api.signInStart.mockResolvedValue({ state: "idle" });
    await show(view({ welcome: true }));
    await fireEvent.click(screen.getByRole("button", { name: "I have a license key" }));
    const input = screen.getByLabelText("License key") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "kntr 4b7x9q2zhhhh" } });
    expect(input.value).toBe("KNTR-4B7X-9Q2Z-HHHH");
    await fireEvent.click(screen.getByRole("button", { name: "Sign in and add key" }));
    expect(api.signInStart).toHaveBeenCalled();
    expect(api.licenseRedeem).not.toHaveBeenCalled();
    // Signed in: the key goes to the core.
    api.licenseView.mockResolvedValue(view({ signedIn: true }));
    license.start();
    handlers.get("license-changed")!(view({ signedIn: true }));
    expect(api.licenseRedeem).toHaveBeenCalledWith("KNTR-4B7X-9Q2Z-HHHH");
  });

  it("losing access on air: a quiet notice, nothing blocks", async () => {
    await show(view({ status: "expired", entitlements: ["core.app", "pro.ai"], pending: ["core.app"] }));
    expect(screen.getByRole("status").textContent).toBe("Your Pro subscription ended. Kinetra keeps running until you end the show.");
    expect(screen.queryByRole("dialog")).toBeNull();
    await show(view({ status: "reduced", entitlements: ["core.app", "pro.ai"] }));
    expect(screen.getByRole("status").textContent).toContain("Pro features are limited until it does");
  });

  it("Pro on another computer: Move Pro here", async () => {
    await show(
      view({
        upgradeBlocked: { license_id: "l", plan: "pro_yearly", reason: "seats_full", activations: [{ id: "a1", machine_name: "STUDIO-PC", slot: "main", created_at: null, last_refreshed_at: null, this_machine: false }] },
      }),
    );
    expect(screen.getByRole("heading", { name: "Pro is active on STUDIO-PC. Move Pro here?" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Move Pro here" }));
    expect(api.licenseMoveHere).toHaveBeenCalledWith("a1");
  });

  it("the move offer names the blocked license's plan", async () => {
    const activations = [{ id: "a1", machine_name: "STUDIO-PC", slot: "main", created_at: null, last_refreshed_at: null, this_machine: false }];
    const team = { id: "l-team", plan: "business_monthly", plan_name: "Kinetra Team", kind: "subscription", status: "active", entitlements: [], seats: 5, seats_used: 5, backup_used: false, starts_at: null, ends_at: null, updates_until: null, revoked_at: null, beta_program_id: null, activations };
    await show(view({ licenses: [team], upgradeBlocked: { license_id: "l-team", plan: "business_monthly", reason: "seats_full", activations } }));
    expect(screen.getByRole("heading", { name: "Kinetra Team is active on STUDIO-PC. Move Kinetra Team here?" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Move Kinetra Team here" })).toBeTruthy();
  });

  it("seats full: deactivate one and use this computer, or use as backup", async () => {
    await show(
      view({
        notice: { kind: "seatsFull", code: "seats_full", backup: true, activations: [{ id: "a2", machine_name: "TRUCK", slot: "main", created_at: null, last_refreshed_at: null, this_machine: false }] },
      }),
    );
    await fireEvent.click(screen.getByRole("button", { name: "Deactivate and use this computer" }));
    await vi.waitFor(() => expect(api.licenseActivate).toHaveBeenCalledWith("main"));
    expect(api.licenseDeactivate).toHaveBeenCalledWith("a2");
    await fireEvent.click(screen.getByRole("button", { name: "Use as backup" }));
    expect(api.licenseActivate).toHaveBeenCalledWith("backup");
  });

  it("a plan without a backup computer offers no backup, only freeing a seat", async () => {
    const activations = [{ id: "a2", machine_name: "TRUCK", slot: "main", created_at: null, last_refreshed_at: null, this_machine: false }];
    await show(view({ notice: { kind: "seatsFull", code: "seats_full", backup: false, activations } }));
    expect(screen.queryByRole("button", { name: "Use as backup" })).toBeNull();
    await show(view({ notice: { kind: "seatsFull", code: "backup_not_included", backup: false, activations } }));
    expect(screen.getByRole("heading", { name: "This plan doesn't include a backup computer" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Use as backup" })).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Deactivate and use this computer" }));
    await vi.waitFor(() => expect(api.licenseActivate).toHaveBeenCalledWith("main"));
  });

  it("Beta Ended waits for the show to end and shows the signed lines", async () => {
    const beta = { name: "Spring", state: "ended" as const, endsAt: 0, endMessage: ["Beta Ended", "Thanks for testing!", "Watch out for a new beta soon."], ended: true, blocking: false };
    await show(view({ beta, onAir: true }));
    expect(screen.queryByText("Beta Ended")).toBeNull();
    await show(view({ beta, onAir: false }));
    expect(screen.getByRole("heading", { name: "Beta Ended" })).toBeTruthy();
    expect(screen.getByText("Watch out for a new beta soon.")).toBeTruthy();
    // Stable: Continue (Free). Beta / canary: blocked, with the download link.
    expect(screen.getByRole("button", { name: "Continue" })).toBeTruthy();
    api.cloudLinks.mockResolvedValue({ account: "a", pricing: "p", download: "https://kinetra.live/download" });
    await show(view({ beta: { ...beta, blocking: true } }));
    expect(screen.queryByRole("button", { name: "Continue" })).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Download the stable version" }));
    await vi.waitFor(() => expect(opener.openUrl).toHaveBeenCalledWith("https://kinetra.live/download"));
  });
});
