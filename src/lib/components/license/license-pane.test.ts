import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CloudLicense, LicenseView } from "$lib/types";

const api = vi.hoisted(() => ({
  licenseView: vi.fn(() => new Promise(() => {})),
  licenseList: vi.fn(() => Promise.resolve([])),
  licenseCheckNow: vi.fn(() => Promise.resolve()),
  licenseDeactivate: vi.fn(() => Promise.resolve()),
  licenseRedeem: vi.fn(() => Promise.resolve({ outcome: "moved", planName: "Pro (yearly)" })),
  signInStart: vi.fn(() => Promise.resolve({ state: "idle" })),
  signInCancel: vi.fn(),
  signOut: vi.fn(() => Promise.resolve()),
  cloudLinks: vi.fn(() => Promise.resolve({ account: "https://account.kinetra.live/account", pricing: "https://kinetra.live/pricing", download: "d" })),
}));
vi.mock("$lib/api", () => ({
  on: () => () => {},
  toAppError: (e: unknown) => (e && typeof e === "object" && "code" in e ? e : { code: "internal", message: String(e) }),
}));
vi.mock("$lib/api/license", () => api);
const opener = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => opener);

import LicensePane from "$lib/components/settings/panes/LicensePane.svelte";
import { agoText, planName } from "./plans";
import { license } from "./state.svelte";

const NOW = Math.floor(Date.now() / 1000);

function lic(over: Partial<CloudLicense> = {}): CloudLicense {
  return {
    id: "l1",
    plan: "pro_yearly",
    kind: "subscription",
    status: "active",
    entitlements: [],
    seats: 2,
    seats_used: 2,
    backup_used: false,
    starts_at: null,
    ends_at: null,
    updates_until: null,
    revoked_at: null,
    beta_program_id: null,
    activations: [
      { id: "own", machine_name: "EDIT-PC", slot: "main", created_at: null, last_refreshed_at: null, this_machine: true },
      { id: "a2", machine_name: "STUDIO-PC", slot: "main", created_at: null, last_refreshed_at: null, this_machine: false },
    ],
    ...over,
  };
}

function view(over: Partial<LicenseView> = {}): LicenseView {
  return {
    signedIn: true,
    account: { email: "ola@example.com", name: "Ola Nordmann" },
    status: "active",
    access: "full",
    plan: "pro_yearly",
    planName: null,
    entitlements: ["core.app", "pro.ai"],
    features: { ai: true, polls: true, control: true, cloud: true },
    pending: null,
    endsAt: NOW + 200 * 86400,
    updatesUntil: null,
    fullUntil: NOW + 18 * 86400,
    reducedUntil: null,
    offlineDaysLeft: 18,
    lastCheckAt: NOW - 180,
    online: true,
    machineName: "EDIT-PC",
    slot: "main",
    licenses: [lic()],
    upgradeBlocked: null,
    beta: null,
    notice: null,
    welcome: false,
    onAir: false,
    signin: { state: "idle" },
    ...over,
  };
}

/** Types like a person: one character at a time at the cursor, each with its input event. */
async function typeKey(input: HTMLInputElement, text: string) {
  input.focus();
  for (const ch of text) {
    const at = input.selectionStart ?? input.value.length;
    input.value = input.value.slice(0, at) + ch + input.value.slice(input.selectionEnd ?? at);
    input.setSelectionRange(at + 1, at + 1);
    await fireEvent(input, new InputEvent("input", { inputType: "insertText", data: ch, bubbles: true }));
  }
}

/** Pastes over the field's whole value. */
async function pasteKey(input: HTMLInputElement, text: string) {
  input.value = text;
  await fireEvent(input, new InputEvent("input", { inputType: "insertFromPaste", bubbles: true }));
}

async function mount(v: LicenseView) {
  license.view = v;
  render(LicensePane);
  await tick();
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Preferences › License", () => {
  it("Pro, signed in: plan, status, dates, offline days, account and computers", async () => {
    await mount(view());
    expect(screen.getByText("Pro (yearly)")).toBeTruthy();
    expect(screen.getByText("Active")).toBeTruthy();
    expect(screen.getByText(/Works for 18 more days without a connection/)).toBeTruthy();
    expect(screen.getByText("3 min ago")).toBeTruthy();
    expect(screen.getByText("Ola Nordmann · ola@example.com")).toBeTruthy();
    expect(screen.getByText("EDIT-PC · Main")).toBeTruthy();
    // Pro: no Upgrade, the feature unlocked (no lock).
    expect(screen.queryByRole("button", { name: "Upgrade to Pro" })).toBeNull();
    expect(screen.queryByRole("button", { name: /needs Kinetra Pro/ })).toBeNull();
    // Another computer: Deactivate.
    await fireEvent.click(screen.getByRole("button", { name: "Deactivate" }));
    expect(api.licenseDeactivate).toHaveBeenCalledWith("a2");
    // This computer asks first.
    await fireEvent.click(screen.getByRole("button", { name: "Deactivate this computer" }));
    await fireEvent.click(screen.getByRole("button", { name: "Deactivate now" }));
    expect(api.licenseDeactivate).toHaveBeenCalledWith("own");
    await fireEvent.click(screen.getByRole("button", { name: "Manage account" }));
    await vi.waitFor(() => expect(opener.openUrl).toHaveBeenCalledWith("https://account.kinetra.live/account"));
    await fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
    expect(api.signOut).toHaveBeenCalled();
  });

  it("Free: Upgrade to Pro, and AI shown locked with what Pro unlocks", async () => {
    await mount(view({ plan: "free", features: { ai: false, polls: false, control: false, cloud: false }, entitlements: ["core.app"], endsAt: null, licenses: [lic({ plan: "free", activations: [] })] }));
    await fireEvent.click(screen.getAllByRole("button", { name: "Upgrade to Pro" })[0]);
    await vi.waitFor(() => expect(opener.openUrl).toHaveBeenCalledWith("https://kinetra.live/pricing"));
    const lock = screen.getByRole("button", { name: "AI templates: needs Kinetra Pro" });
    expect(lock.textContent).toContain("Pro");
    // The explanation is a popover (opened by the lock button).
    expect(screen.getByRole("heading", { name: "AI templates is part of Kinetra Pro", hidden: true })).toBeTruthy();
  });

  it("signed out: Sign in, and the license still works offline", async () => {
    await mount(view({ signedIn: false, account: null }));
    expect(screen.getByText("Your license keeps working offline.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
    expect(api.signInStart).toHaveBeenCalled();
  });

  it("a failed action shows the API message", async () => {
    api.licenseDeactivate.mockRejectedValueOnce({ code: "cloud", apiCode: "not_found", message: "x" } as never);
    await mount(view());
    await fireEvent.click(screen.getByRole("button", { name: "Deactivate" }));
    await vi.waitFor(() => expect(screen.getByRole("alert").textContent).toBe("Not found. It may have been removed already."));
  });

  it("adds a license key here too: at once when signed in", async () => {
    await mount(view());
    const input = screen.getByLabelText("License key") as HTMLInputElement;
    // Typed by hand in lowercase without dashes: never rewritten while typing.
    let so = "";
    for (const ch of "kntr4b7x9q2zhhhh") {
      await typeKey(input, ch);
      so += ch;
      expect(input.value).toBe(so);
    }
    await fireEvent.click(screen.getByRole("button", { name: "Add key" }));
    expect(api.licenseRedeem).toHaveBeenCalledWith("KNTR-4B7X-9Q2Z-HHHH");
    await vi.waitFor(() => expect(screen.getByRole("status").textContent).toBe("License key redeemed."));
  });

  it("a pasted key is tidied at once; a typed one on blur", async () => {
    await mount(view());
    const input = screen.getByLabelText("License key") as HTMLInputElement;
    await pasteKey(input, " 4b7x 9q2z hhhh ");
    expect(input.value).toBe("KNTR-4B7X-9Q2Z-HHHH");
    await pasteKey(input, "");
    await typeKey(input, "KNTR 4B7X 9Q2Z HHHH");
    expect(input.value).toBe("KNTR 4B7X 9Q2Z HHHH");
    await fireEvent.blur(input);
    expect(input.value).toBe("KNTR-4B7X-9Q2Z-HHHH");
  });

  it("a redeemed key says what it did on this computer", async () => {
    api.licenseRedeem.mockResolvedValueOnce({ outcome: "kept", planName: "Kinetra Team" });
    await mount(view());
    const input = screen.getByLabelText("License key") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "KNTR-4B7X-9Q2Z-HHHH" } });
    await fireEvent.click(screen.getByRole("button", { name: "Add key" }));
    await vi.waitFor(() => expect(screen.getByRole("status").textContent).toBe("Redeemed. This computer already uses Kinetra Team."));
  });

  it("a license that couldn't be saved says why and where", async () => {
    await mount(view({ notice: { kind: "saveFailed", reason: "Access is denied. (os error 5)", folder: "C:\\Users\\Ola\\AppData\\Roaming\\kinetra" } }));
    expect(screen.getByRole("alert").textContent).toBe(
      "Couldn't save the license on this computer: Access is denied. (os error 5). Check that Kinetra can write to C:\\Users\\Ola\\AppData\\Roaming\\kinetra.",
    );
  });

  it("Check now waits for the check, then shows its error", async () => {
    let fail!: (e: unknown) => void;
    api.licenseCheckNow.mockReturnValueOnce(new Promise((_, reject) => (fail = reject)));
    await mount(view());
    await fireEvent.click(screen.getByRole("button", { name: "Check now" }));
    expect(screen.getByRole("button", { name: "Checking…" })).toBeTruthy();
    fail({ code: "offline", message: "You're offline. Check your connection and try again." });
    await vi.waitFor(() => expect(screen.getByRole("alert").textContent).toBe("You're offline. Check your connection and try again."));
    expect(screen.getByRole("button", { name: "Check now" })).toBeTruthy();
  });

  it("a license key while signed out signs in first, then redeems", async () => {
    await mount(view({ signedIn: false, account: null }));
    const input = screen.getByLabelText("License key") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "KNTR-4B7X-9Q2Z-HHHH" } });
    await fireEvent.click(screen.getByRole("button", { name: "Sign in and add key" }));
    expect(api.signInStart).toHaveBeenCalled();
    expect(api.licenseRedeem).not.toHaveBeenCalled();
    expect(screen.getByText("Sign in to finish adding the key.")).toBeTruthy();
    license.view = view();
    await vi.waitFor(() => expect(api.licenseRedeem).toHaveBeenCalledWith("KNTR-4B7X-9Q2Z-HHHH"));
  });

  it("shows the plan's name from the server, never its code", async () => {
    const team = lic({ plan: "business_monthly", plan_name: "Kinetra Team" });
    await mount(view({ plan: "business_monthly", planName: "Kinetra Team", licenses: [team] }));
    expect(screen.getByText("Kinetra Team")).toBeTruthy();
    expect(screen.getByText("Kinetra Team · Main")).toBeTruthy();
    expect(screen.queryByText(/business/i)).toBeNull();
  });

  it("names plans and relative times", () => {
    expect(planName("business_monthly", "Kinetra Team")).toBe("Kinetra Team");
    expect(planName("pro_yearly", "Kinetra Pro (yearly)")).toBe("Kinetra Pro (yearly)");
    expect(planName("pro_yearly", " ")).toBe("Pro (yearly)");
    expect(planName("business_monthly")).toBe("Kinetra Team");
    expect(planName("pro_perpetual")).toBe("Pro (perpetual)");
    expect(planName(null)).toBe("No license");
    expect(planName("some_new_plan")).toBe("Some new plan");
    expect(agoText(1000, 1030)).toBe("Just now");
    expect(agoText(0, 7200)).toBe("2 hours ago");
    expect(agoText(0, 3700)).toBe("1 hour ago");
  });
});
