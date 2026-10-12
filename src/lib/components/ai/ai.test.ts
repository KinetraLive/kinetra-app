import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { AiJobView, LicenseView } from "$lib/types";

const handlers = vi.hoisted(() => new Map<string, (p: unknown) => void>());
const ai = vi.hoisted(() => ({
  aiLimits: vi.fn(() => Promise.resolve({ maxChars: 2000, maxColors: 4, maxFonts: 2, types: ["lower_third", "other"] })),
  aiQuota: vi.fn(() =>
    Promise.resolve({
      enabled: true,
      disabled_for_account: false,
      plan: "pro",
      generate_template: { used: 3, limit: 50, period: "month", day_used: 1, day_limit: 10 },
      tweak_template: { used: 12, limit: 300, period: "month" },
    }),
  ),
  aiGenerate: vi.fn(),
  aiTweak: vi.fn(),
  aiCancel: vi.fn(() => Promise.resolve()),
  aiJobs: vi.fn(() => Promise.resolve([])),
}));
vi.mock("$lib/api", () => ({
  on: (name: string, h: (p: unknown) => void) => {
    handlers.set(name, h);
    return () => handlers.delete(name);
  },
  toAppError: (e: unknown) => (e && typeof e === "object" && "code" in e ? e : { code: "internal", message: String(e) }),
}));
vi.mock("$lib/api/ai", () => ai);
vi.mock("$lib/api/license", () => ({
  licenseView: vi.fn(() => new Promise(() => {})),
  signInStart: vi.fn(),
  cloudLinks: vi.fn(() => Promise.resolve({ account: "a", pricing: "https://kinetra.live/pricing", download: "d" })),
}));
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));

import { project } from "$lib/stores/project.svelte";
import { license } from "$lib/components/license/state.svelte";
import AiDialog from "./AiDialog.svelte";
import AiEntry from "./AiEntry.svelte";
import AiJobsChip from "./AiJobsChip.svelte";
import { jobs } from "./jobs.svelte";

beforeAll(() => {
  // jsdom has no <dialog> methods.
  HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
    this.setAttribute("open", "");
  };
  HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
    this.removeAttribute("open");
  };
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function setView(features: { ai: boolean }) {
  license.view = { features, signedIn: true } as unknown as LicenseView;
}

describe("Generate template", () => {
  it("counts characters at the backend limit, prefills the brand kit and starts a job", async () => {
    project.current = {
      path: "p",
      name: "Show",
      settings: { brand: { accent: "#C8102E", secondary: "#ffffff", fontHeading: "Barlow Condensed", fontBody: "Inter" } },
    } as never;
    ai.aiGenerate.mockResolvedValue({ id: "j1", task: "generate", status: "pending", code: null, message: null, templateId: null, label: "x" });
    const onclose = vi.fn();
    render(AiDialog, { mode: { kind: "generate" }, onclose });
    await vi.waitFor(() => expect(screen.getByText("47 of 50 left this month · 9 of 10 today")).toBeTruthy());
    const text = screen.getByLabelText("Describe the graphic");
    await fireEvent.input(text, { target: { value: "x".repeat(2001) } });
    expect(screen.getByText("2001 / 2000")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Generate" }) as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.input(text, { target: { value: "Lower third for a football match" } });
    await fireEvent.click(screen.getByRole("button", { name: "Generate" }));
    expect(ai.aiGenerate).toHaveBeenCalledWith({
      prompt: "Lower third for a football match",
      templateType: null,
      // The brand kit: colors as given, only curated fonts (Inter isn't one).
      brandColors: ["#C8102E", "#ffffff"],
      fonts: ["Barlow Condensed"],
      width: null,
      height: null,
    });
    await vi.waitFor(() => expect(onclose).toHaveBeenCalled());
  });

  it("explains a used-up quota with its reset date and keeps a too-long text", async () => {
    project.current = null;
    ai.aiGenerate.mockRejectedValueOnce({
      code: "cloud",
      apiCode: "ai_quota_exhausted",
      message: "You've used this month's AI templates.",
      details: { scope: "month", resets_at: Date.UTC(2026, 10, 1) / 1000 },
    });
    render(AiDialog, { mode: { kind: "generate" }, onclose: vi.fn() });
    await fireEvent.input(screen.getByLabelText("Describe the graphic"), { target: { value: "Bug" } });
    await fireEvent.click(screen.getByRole("button", { name: "Generate" }));
    await vi.waitFor(() => expect(screen.getByRole("alert").textContent).toBe("You've used this month's AI templates. It resets Nov 1."));
    ai.aiGenerate.mockRejectedValueOnce({ code: "cloud", apiCode: "ai_input_too_long", message: "The description is too long: at most 2000 characters." });
    await fireEvent.click(screen.getByRole("button", { name: "Generate" }));
    await vi.waitFor(() => expect(screen.getByRole("alert").textContent).toContain("too long"));
    expect((screen.getByLabelText("Describe the graphic") as HTMLTextAreaElement).value).toBe("Bug");
  });

  it("says so up front when AI is switched off, and sends nothing", async () => {
    ai.aiQuota.mockResolvedValueOnce({ enabled: false, disabled_for_account: false, plan: "pro", generate_template: null, tweak_template: null } as never);
    render(AiDialog, { mode: { kind: "generate" }, onclose: vi.fn() });
    await vi.waitFor(() => expect(screen.getByRole("status").textContent).toBe("AI is turned off right now. Everything else in Kinetra works as usual."));
    await fireEvent.input(screen.getByLabelText("Describe the graphic"), { target: { value: "Bug" } });
    expect((screen.getByRole("button", { name: "Generate" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("tweak sends the instruction for the template", async () => {
    ai.aiTweak.mockResolvedValue({ id: "j2", task: "tweak", status: "pending", code: null, message: null, templateId: null, label: "x" });
    render(AiDialog, { mode: { kind: "tweak", templateId: "t1", name: "Score bug" }, onclose: vi.fn() });
    expect(screen.getByRole("heading", { name: "Tweak “Score bug”" })).toBeTruthy();
    await vi.waitFor(() => expect(screen.getByText("288 of 300 left this month")).toBeTruthy());
    await fireEvent.input(screen.getByLabelText("What should change?"), { target: { value: "Make it blue" } });
    await fireEvent.click(screen.getByRole("button", { name: "Tweak" }));
    expect(ai.aiTweak).toHaveBeenCalledWith("t1", "Make it blue");
  });
});

describe("entry and job chip", () => {
  it("locked without Pro, a button with it", async () => {
    setView({ ai: false });
    render(AiEntry, { mode: { kind: "generate" } });
    expect(screen.getByRole("button", { name: "Generate template: needs Kinetra Pro" })).toBeTruthy();
    cleanup();
    setView({ ai: true });
    render(AiEntry, { mode: { kind: "generate" } });
    expect(screen.getByRole("button", { name: "Generate template" })).toBeTruthy();
  });

  it("shows progress, lets the user cancel, then the result or the reason", async () => {
    jobs.list = [];
    render(AiJobsChip);
    const j = (o: Partial<AiJobView>): AiJobView => ({ id: "j1", task: "generate", status: "pending", code: null, message: null, templateId: null, label: "Lower third", ...o });
    jobs.put(j({}));
    await tick();
    expect(screen.getByText("Designing your template…")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(ai.aiCancel).toHaveBeenCalledWith("j1");
    jobs.put(j({ status: "succeeded", templateId: "t9" }));
    await tick();
    expect(screen.getByText("Template ready")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open" })).toBeTruthy();
    jobs.put(j({ id: "j2", status: "failed", code: "ai_declined" }));
    await tick();
    expect(screen.getByText("Couldn't generate this template. Try rephrasing.")).toBeTruthy();
  });
});
