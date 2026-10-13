import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../templates/list/matchers";
import type { LicenseView, PollData, PollQuota, SourceSummary } from "$lib/types";

const handlers = vi.hoisted(() => new Map<string, (p: unknown) => void>());
vi.mock("$lib/api", () => ({
  call: vi.fn(() => Promise.resolve([])),
  on: (name: string, f: (p: unknown) => void) => {
    handlers.set(name, f);
    return () => handlers.delete(name);
  },
  toAppError: (e: unknown) => (e && typeof e === "object" && "code" in e ? e : { code: "internal", message: String(e) }),
}));
const polls = vi.hoisted(() => ({
  listPolls: vi.fn(),
  pollsQuota: vi.fn(),
  createPoll: vi.fn(),
  updatePoll: vi.fn(),
  getPoll: vi.fn(),
  openPoll: vi.fn(),
  closePoll: vi.fn(),
  resetPoll: vi.fn(),
  deletePoll: vi.fn(),
  watchPoll: vi.fn(() => Promise.resolve()),
  pollQr: vi.fn(() => Promise.resolve("data:image/svg+xml,x")),
  createPollSource: vi.fn(),
  getPollSource: vi.fn(),
  savePollSource: vi.fn(),
  pollSourceFor: vi.fn(() => Promise.resolve(null)),
}));
vi.mock("$lib/api/polls", () => polls);
const sources = vi.hoisted(() => ({
  listSources: vi.fn(),
  deleteSource: vi.fn(),
  renameSource: vi.fn(),
}));
vi.mock("$lib/api/sources", () => sources);
vi.mock("$lib/api/license", () => ({
  licenseView: vi.fn(() => new Promise(() => {})),
  cloudLinks: vi.fn(() => Promise.resolve({ pricing: "https://kinetra.live/pricing" })),
  signInStart: vi.fn(),
}));
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));

import PollsPage from "../../../routes/(app)/polls/+page.svelte";
import { license } from "$lib/components/license/state.svelte";
import { toasts } from "$lib/stores/toasts.svelte";

const poll = (over: Partial<PollData> = {}): PollData => ({
  id: "p1",
  code: "K7P2QX",
  url: "https://poll.kinetra.live/K7P2QX",
  question: "Who wins tonight?",
  options: [
    { position: 0, label: "Home", votes: 0 },
    { position: 1, label: "Away", votes: 0 },
  ],
  status: "draft",
  showResults: true,
  durationSeconds: null,
  totalVotes: 0,
  maxVotes: 2000,
  voteLimitReached: false,
  round: 1,
  createdAt: null,
  openedAt: null,
  closesAt: null,
  closedAt: null,
  deleteAfter: null,
  expiresAt: null,
  ...over,
});

const quota = (open: number): PollQuota => ({
  plan: "pro",
  planName: "Kinetra Pro",
  limits: {
    openPolls: 3,
    pollsPerMonth: 30,
    votesPerPoll: 2000,
    optionsPerPoll: 8,
    maxDurationSeconds: 86400,
    lifetimeSeconds: 604800,
    deleteAfterCloseSeconds: 43200,
  },
  usage: { openPolls: open, pollsThisMonth: 4, polls: 6 },
  monthResetsAt: "2026-11-01T00:00:00Z",
});

function view(pro: boolean, over: Partial<LicenseView> = {}): LicenseView {
  return {
    signedIn: true,
    account: { email: "ola@example.com", name: "Ola" },
    status: "active",
    access: pro ? "full" : "free",
    plan: pro ? "pro_yearly" : "free",
    planName: null,
    entitlements: pro ? ["core.app", "pro.polls"] : ["core.app"],
    features: { ai: pro, polls: pro },
    pending: null,
    endsAt: null,
    updatesUntil: null,
    fullUntil: null,
    reducedUntil: null,
    offlineDaysLeft: null,
    lastCheckAt: null,
    online: true,
    machineName: "PC",
    slot: "main",
    licenses: [],
    upgradeBlocked: null,
    beta: null,
    notice: null,
    welcome: false,
    onAir: false,
    signin: { state: "idle" },
    ...over,
  } as LicenseView;
}

async function mount(pro: boolean, list: PollData[], q: PollQuota | null, srcs: SourceSummary[] = []) {
  license.view = view(pro);
  polls.listPolls.mockResolvedValue(list);
  polls.pollsQuota.mockResolvedValue(q);
  sources.listSources.mockResolvedValue(srcs);
  render(PollsPage);
  await waitFor(() => expect(sources.listSources).toHaveBeenCalled());
  // The page shows once both lists arrived.
  await waitFor(() => expect(document.querySelector(".split, .empty")).not.toBeNull());
  await tick();
}

beforeEach(() => {
  vi.clearAllMocks();
  toasts.list = [];
});
afterEach(cleanup);

describe("Polls page", () => {
  it("Free: the lock with Pro", async () => {
    await mount(false, [], null);
    // The usual lock + "Pro" label, explaining what Pro unlocks.
    expect(await screen.findAllByRole("button", { name: "New poll: needs Kinetra Pro" })).not.toHaveLength(0);
    expect(screen.queryByText(/Open now/)).toBeNull();
    // Someone whose Pro ended can list their polls once, on demand.
    expect(screen.getByRole("button", { name: "Show my existing polls" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "New test poll" })).toBeNull();
    // Nothing that needs Pro was asked for.
    expect(polls.createPoll).not.toHaveBeenCalled();
    expect(polls.openPoll).not.toHaveBeenCalled();
  });

  it("Show my existing polls asks the server once, on demand", async () => {
    await mount(false, [], null);
    polls.listPolls.mockResolvedValueOnce([poll({ status: "open", totalVotes: 3 })]);
    await fireEvent.click(screen.getByRole("button", { name: "Show my existing polls" }));
    expect(polls.listPolls).toHaveBeenLastCalledWith(true);
    expect(await screen.findByRole("option", { name: /Who wins tonight/ })).toBeInTheDocument();
  });

  it("Pro: the quota in the header, and a full plan says why before opening", async () => {
    await mount(true, [poll()], quota(3));
    expect(screen.getByLabelText("Your plan's poll limits")).toHaveTextContent("Open now 3 of 3");
    expect(screen.getByLabelText("Your plan's poll limits")).toHaveTextContent("This month 4 of 30");
    expect(screen.getByLabelText("Your plan's poll limits")).toHaveTextContent("Votes per poll Up to 2,000");
    expect(screen.getByRole("button", { name: "Open poll" })).toBeDisabled();
    expect(screen.getByText("3 polls are open, the most your plan allows. Close one to open this.")).toBeInTheDocument();
    // The voting link and code.
    expect(screen.getByText("poll.kinetra.live/K7P2QX")).toBeInTheDocument();
    expect(polls.watchPoll).toHaveBeenCalledWith("p1", true);
  });

  it("Pro ended: close, reset and delete stay; opening and editing say why", async () => {
    polls.closePoll.mockResolvedValue(poll({ status: "closed", totalVotes: 4 }));
    await mount(false, [poll({ status: "open", totalVotes: 4, options: [{ position: 0, label: "Home", votes: 3 }, { position: 1, label: "Away", votes: 1 }] })], null);
    expect(screen.getByText(/Opening and editing polls is part of Kinetra Pro\. You can still close/)).toBeInTheDocument();
    await fireEvent.click(screen.getByRole("button", { name: "Close poll" }));
    expect(polls.closePoll).toHaveBeenCalledWith("p1");
    expect(await screen.findByRole("button", { name: "Open again" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Reset votes" })).not.toBeDisabled();
    expect(screen.getByRole("button", { name: "Delete" })).not.toBeDisabled();
  });

  it("limit errors from the server show their numbers", async () => {
    polls.openPoll.mockRejectedValue({ code: "cloud", apiCode: "poll_limit_reached", message: "-", details: { max: 15 } });
    await mount(true, [poll()], quota(0));
    await fireEvent.click(screen.getByRole("button", { name: "Open poll" }));
    await waitFor(() =>
      expect(toasts.list.at(-1)?.message).toBe(
        "Couldn't open the poll: 15 polls are open, the most your plan allows. Close one to open this.",
      ),
    );
  });

  it("live results arrive through poll-live, with a calm status", async () => {
    await mount(true, [poll({ status: "open" })], quota(1));
    handlers.get("poll-live")!({
      id: "p1",
      conn: "live",
      poll: poll({ status: "open", totalVotes: 8, options: [{ position: 0, label: "Home", votes: 6 }, { position: 1, label: "Away", votes: 2 }] }),
    });
    await tick();
    const results = screen.getByRole("region", { name: "Results" });
    expect(within(results).getByText("8 votes")).toBeInTheDocument();
    expect(within(results).getByText("75%")).toBeInTheDocument();
    expect(within(results).getByText("Leading")).toBeInTheDocument();
    expect(screen.getAllByText("Live").length).toBeGreaterThan(0);
    handlers.get("poll-live")!({ id: "p1", conn: "reconnecting", poll: null });
    await tick();
    expect(screen.getAllByText("Reconnecting…").length).toBeGreaterThan(0);
    expect(within(screen.getByRole("region", { name: "Results" })).getByText("8 votes"), "the last results stay").toBeInTheDocument();
  });

  it("creates a poll from the editor with the core's rules", async () => {
    polls.createPoll.mockResolvedValue(poll({ id: "p2", question: "Best goal?" }));
    await mount(true, [], quota(0));
    await fireEvent.click(screen.getAllByRole("button", { name: "New poll" })[0]);
    await fireEvent.click(await screen.findByRole("button", { name: "Create poll" }));
    expect(screen.getByRole("alert")).toHaveTextContent("Add a question.");
    expect(polls.createPoll).not.toHaveBeenCalled();
    await fireEvent.input(screen.getByLabelText("Question"), { target: { value: "Best goal?" } });
    await fireEvent.input(screen.getByLabelText("Answer 1"), { target: { value: "First" } });
    await fireEvent.input(screen.getByLabelText("Answer 2"), { target: { value: "Second" } });
    await fireEvent.click(screen.getByRole("button", { name: "Create poll" }));
    expect(polls.createPoll).toHaveBeenCalledWith({ question: "Best goal?", options: ["First", "Second"], showResults: true });
  });
});
