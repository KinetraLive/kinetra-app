import { describe, expect, it } from "vitest";
import type { PollData, PollQuota } from "$lib/types";
import {
  checkDraft,
  connLabel,
  createBlocked,
  createInput,
  dayMonth,
  deletesIn,
  draftOf,
  emptyDraft,
  formatDuration,
  leader,
  openBlocked,
  parseDuration,
  pollError,
  quotaLines,
  shares,
  stateOf,
  testData,
  votesText,
  textLocked,
  updateInput,
} from "./model";

const poll = (over: Partial<PollData> = {}): PollData => ({
  id: "p1",
  code: "K7P2QX",
  url: "https://poll.kinetra.live/K7P2QX",
  question: "Who wins tonight?",
  options: [
    { position: 0, label: "Home", votes: 12 },
    { position: 1, label: "Away", votes: 7 },
  ],
  status: "draft",
  showResults: true,
  durationSeconds: null,
  totalVotes: 19,
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

const quota = (open: number, month: number, limitMonth: number | null = 30): PollQuota => ({
  plan: "pro",
  limits: {
    openPolls: 3,
    pollsPerMonth: limitMonth,
    votesPerPoll: 2000,
    optionsPerPoll: 8,
    maxDurationSeconds: 86400,
    lifetimeSeconds: 604800,
    deleteAfterCloseSeconds: 43200,
  },
  usage: { openPolls: open, pollsThisMonth: month, polls: month },
  monthResetsAt: "2026-11-01T00:00:00Z",
});

describe("counts", () => {
  it("one vote, otherwise votes (0 too), grouped", () => {
    expect([0, 1, 2, 2418].map(votesText)).toEqual(["0 votes", "1 vote", "2 votes", "2,418 votes"]);
  });

  it("a monthly limit of one poll reads in the singular", () => {
    const q = {
      plan: "pro",
      limits: { openPolls: 1, pollsPerMonth: 1, votesPerPoll: 2000, optionsPerPoll: 8, maxDurationSeconds: 86400, lifetimeSeconds: 604800, deleteAfterCloseSeconds: 43200 },
      usage: { openPolls: 0, pollsThisMonth: 1, polls: 1 },
      monthResetsAt: null,
    } as PollQuota;
    expect(createBlocked(q)).toBe("You've created 1 poll this month.");
  });
});

describe("quota display", () => {
  it("shows usage against the plan's limits", () => {
    expect(quotaLines(quota(1, 4))).toEqual([
      { label: "Open now", value: "1 of 3", full: false },
      { label: "This month", value: "4 of 30", full: false },
      { label: "Votes per poll", value: "Up to 2,000", full: false },
    ]);
    expect(quotaLines(quota(3, 30)).map((l) => l.full)).toEqual([true, true, false]);
    // Unlimited months (Enterprise) show only the count.
    expect(quotaLines(quota(0, 41, null))[1]).toEqual({ label: "This month", value: "41", full: false });
    // Without pro.polls (or no request made): nothing to show.
    expect(quotaLines(null)).toEqual([]);
    expect(quotaLines({ ...quota(0, 0), plan: null, limits: null })).toEqual([]);
  });

  it("explains a reached limit before the call, with the numbers and the date", () => {
    expect(createBlocked(quota(0, 29))).toBeNull();
    expect(createBlocked(quota(0, 30))).toBe("You've created 30 polls this month. New ones from 1 November.");
    expect(createBlocked(quota(0, 999, null))).toBeNull();
    expect(openBlocked(quota(2, 0), poll())).toBeNull();
    expect(openBlocked(quota(3, 0), poll())).toBe("3 polls are open, the most your plan allows. Close one to open this.");
    expect(openBlocked(quota(3, 0), poll({ status: "open" }))).toBeNull();
    expect(dayMonth(1_793_491_200)).toBe("1 November");
    expect(dayMonth("nonsense")).toBe("");
  });

  it("maps the server's limit errors with their details", () => {
    const err = (apiCode: string, details?: Record<string, unknown>) => ({ code: "cloud", apiCode, message: "server says", details });
    expect(pollError(err("poll_limit_reached", { max: 15 }))).toBe(
      "15 polls are open, the most your plan allows. Close one to open this.",
    );
    expect(pollError(err("poll_monthly_limit_reached", { max: 30, resets_at: 1_793_491_200 }))).toBe(
      "You've created 30 polls this month. New ones from 1 November.",
    );
    expect(pollError(err("poll_has_votes"))).toBe("Reset the votes to change the question or answers.");
    expect(pollError(err("bad_request"))).toBe("server says");
    expect(pollError({ code: "proRequired", message: "x" })).toMatch(/part of Kinetra Pro. Your existing polls still work/);
    expect(pollError({ code: "offline", message: "Kinetra can't reach the server." })).toBe("Kinetra can't reach the server.");
  });
});

describe("polls", () => {
  it("states, shares and the leader", () => {
    expect(stateOf(poll({ status: "open", voteLimitReached: true }))).toBe("full");
    expect(stateOf(poll({ status: "closed", voteLimitReached: true }))).toBe("closed");
    expect(shares([1, 1, 1])).toEqual([34, 33, 33]);
    expect(shares([0, 0])).toEqual([0, 0]);
    expect(shares([1112, 750, 556]).reduce((a, b) => a + b)).toBe(100);
    expect(leader([5, 5, 1])).toBeNull();
    expect(leader([0, 0])).toBeNull();
    expect(leader([1, 4])).toBe(1);
  });

  it("a test poll reads as poll data with the harmless test link", () => {
    const d = testData({ question: "Snack?", options: ["Popcorn", "Nachos"], votes: [3, 4], status: "open" });
    expect(d.totalVotes).toBe(7);
    expect(d.options[1]).toEqual({ position: 1, label: "Nachos", votes: 4 });
    expect(d.url).toBe("https://poll.kinetra.live/TEST00");
  });

  it("connection labels stay calm", () => {
    expect(connLabel("live")).toEqual({ text: "Live", tone: "preview" });
    expect(connLabel("polling")?.text).toBe("Updating every 5 s");
    expect(connLabel("idle")).toBeNull();
  });

  it("deletion countdown for closed polls", () => {
    const now = Date.parse("2026-10-12T10:00:00Z");
    expect(deletesIn({ status: "closed", deleteAfter: "2026-10-12T21:30:00Z" }, now)).toBe("Deleted in 11 h");
    expect(deletesIn({ status: "closed", deleteAfter: "2026-10-12T10:20:00Z" }, now)).toBe("Deleted in 20 min");
    expect(deletesIn({ status: "open", deleteAfter: "2026-10-12T21:30:00Z" }, now)).toBe("");
  });
});

describe("editing", () => {
  it("checks a draft with the core's rules", () => {
    const ok = { question: "Who wins?", options: ["Home", "Away"], duration: null, showResults: true };
    expect(checkDraft(ok)).toBeNull();
    expect(checkDraft(emptyDraft())).toBe("Add a question.");
    expect(checkDraft({ ...ok, options: ["Only"] })).toBe("A poll needs 2 to 8 options on your plan.");
    expect(checkDraft({ ...ok, options: ["A", "b", "c", "d"] }, 3)).toBe("A poll needs 2 to 3 options on your plan.");
    expect(checkDraft({ ...ok, options: ["A", " a"] })).toMatch(/the same/);
    expect(checkDraft({ ...ok, options: ["A", " "] })).toBe("Answer 2 is empty.");
    expect(checkDraft({ ...ok, question: "x".repeat(201) })).toMatch(/at most 200/);
    expect(checkDraft({ ...ok, duration: 10 })).toMatch(/30 seconds/);
  });

  it("sends only what changed", () => {
    const p = poll({ durationSeconds: 120 });
    expect(updateInput(draftOf(p), p)).toBeNull();
    expect(updateInput({ ...draftOf(p), showResults: false }, p)).toEqual({ showResults: false });
    expect(updateInput({ ...draftOf(p), duration: null }, p)).toEqual({ clearDuration: true });
    expect(updateInput({ ...draftOf(p), options: ["Home", "Away", "Draw"] }, p)).toEqual({
      options: ["Home", "Away", "Draw"],
    });
    expect(createInput({ question: " Q ", options: [" A", "B "], duration: 90, showResults: false })).toEqual({
      question: "Q",
      options: ["A", "B"],
      showResults: false,
      durationSeconds: 90,
    });
    expect(textLocked(poll({ status: "open" }))).toMatch(/Close the poll/);
    expect(textLocked(poll({ status: "closed" }))).toMatch(/Reset the votes/);
    expect(textLocked(poll({ totalVotes: 0 }))).toBeNull();
  });

  it("reads and writes durations", () => {
    expect(formatDuration(120)).toBe("2:00");
    expect(formatDuration(5400)).toBe("1:30:00");
    expect(parseDuration("2:00")).toBe(120);
    expect(parseDuration("1:30:00")).toBe(5400);
    expect(parseDuration("90")).toBe(90);
    expect(parseDuration("5m")).toBe(300);
    expect(parseDuration("1h")).toBe(3600);
    expect(parseDuration("2:75")).toBeNull();
    expect(parseDuration("")).toBeNull();
  });
});
