// The Polls page's plain logic (no Svelte): status and connection labels, the quota lines, limit
// messages with their numbers and dates, the editor's input rules (the same as core/polls.rs
// `check_input`; the server's message still wins), durations and shares.

import { toAppError } from "$lib/api";
import { cloudMessage } from "$lib/components/license/messages";
import type { PollConn, PollData, PollInput, PollQuota } from "$lib/types";
import {
  POLL_DURATION_MAX_S,
  POLL_DURATION_MIN_S,
  POLL_OPTIONS_MAX,
  POLL_OPTIONS_MIN,
  POLL_OPTION_MAX,
  POLL_QUESTION_MAX,
} from "$lib/types";

export type PollState = "draft" | "open" | "closed" | "full";

/** draft | open | closed | full (open with the vote limit reached). */
export function stateOf(p: Pick<PollData, "status" | "voteLimitReached">): PollState {
  if (p.status === "open" && p.voteLimitReached) return "full";
  return p.status;
}

export const STATE_LABEL: Record<PollState, string> = {
  draft: "Draft",
  open: "Open",
  closed: "Closed",
  full: "Full",
};

/** `2418` → `2,418`. */
export const fmt = (n: number) => n.toLocaleString("en-US");
/** "0 votes", "1 vote", "2,418 votes". */
export const votesText = (n: number) => `${fmt(n)} ${n === 1 ? "vote" : "votes"}`;

/** Whole percentages that add up to 100 (largest remainder; ties keep the earlier answer). */
export function shares(votes: readonly number[]): number[] {
  const total = votes.reduce((a, b) => a + b, 0);
  if (total === 0) return votes.map(() => 0);
  const exact = votes.map((v) => (v * 100) / total);
  const out = exact.map(Math.floor);
  let left = 100 - out.reduce((a, b) => a + b, 0);
  const order = exact.map((p, i) => [p - Math.floor(p), i] as const).sort((a, b) => b[0] - a[0] || a[1] - b[1]);
  for (const [, i] of order) {
    if (left <= 0) break;
    out[i] += 1;
    left -= 1;
  }
  return out;
}

/** The leading answer's index: most votes, null with no votes or a tie for first. */
export function leader(votes: readonly number[]): number | null {
  const max = Math.max(0, ...votes);
  if (max === 0) return null;
  const at = votes.flatMap((v, i) => (v === max ? [i] : []));
  return at.length === 1 ? at[0] : null;
}

/** `https://poll.kinetra.live/K7P2QX` → `poll.kinetra.live/K7P2QX`. */
export const shortUrl = (url: string) => url.replace(/^https?:\/\//, "").replace(/\/$/, "");

// ---- Live status -----------------------------------------------------------------------------

export type ConnTone = "preview" | "warning" | "idle";

/** The page's quiet status line for a poll's results (never alarming, never modal). */
export function connLabel(conn: PollConn): { text: string; tone: ConnTone } | null {
  switch (conn) {
    case "live":
      return { text: "Live", tone: "preview" };
    case "connecting":
      return { text: "Connecting…", tone: "idle" };
    case "reconnecting":
      return { text: "Reconnecting…", tone: "warning" };
    case "polling":
      return { text: "Updating every 5 s", tone: "warning" };
    case "signedOut":
      return { text: "Sign in for live results", tone: "warning" };
    case "gone":
      return { text: "Deleted on the server", tone: "idle" };
    case "idle":
      return null;
  }
}

// ---- Quota and limits ---------------------------------------------------------------------------

export interface QuotaLine {
  label: string;
  value: string;
  /** The limit is reached. */
  full: boolean;
}

/** "Open now 1 of 3 · This month 4 of 30 · Up to 500 votes per poll". Empty without limits. */
export function quotaLines(q: PollQuota | null): QuotaLine[] {
  const l = q?.limits;
  if (!q || !l) return [];
  const month = l.pollsPerMonth;
  return [
    { label: "Open now", value: `${q.usage.openPolls} of ${l.openPolls}`, full: q.usage.openPolls >= l.openPolls },
    {
      label: "This month",
      value: month === null ? fmt(q.usage.pollsThisMonth) : `${q.usage.pollsThisMonth} of ${month}`,
      full: month !== null && q.usage.pollsThisMonth >= month,
    },
    { label: "Votes per poll", value: `Up to ${fmt(l.votesPerPoll)}`, full: false },
  ];
}

/** "1 November" from an RFC 3339 date or a unix time in seconds; "" when unreadable. */
export function dayMonth(at: string | number | null | undefined): string {
  if (at === null || at === undefined || at === "") return "";
  const d = typeof at === "number" ? new Date(at * 1000) : new Date(at);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleDateString("en-US", { day: "numeric", month: "long", timeZone: "UTC" }).replace(/^(\w+) (\d+)$/, "$2 $1");
}

/** Why a new poll can't be created now (the month's limit), or null. */
export function createBlocked(q: PollQuota | null): string | null {
  const l = q?.limits;
  if (!q || !l || l.pollsPerMonth === null || q.usage.pollsThisMonth < l.pollsPerMonth) return null;
  const from = dayMonth(q.monthResetsAt);
  return `You've created ${fmt(l.pollsPerMonth)} ${l.pollsPerMonth === 1 ? "poll" : "polls"} this month.${from ? ` New ones from ${from}.` : ""}`;
}

/** Why this poll can't be opened now (open polls at the plan's limit), or null. */
export function openBlocked(q: PollQuota | null, p: PollData): string | null {
  const l = q?.limits;
  if (!q || !l || p.status === "open") return null;
  if (q.usage.openPolls < l.openPolls) return null;
  return `${l.openPolls} ${l.openPolls === 1 ? "poll is" : "polls are"} open, the most your plan allows. Close one to open this.`;
}

/** A poll command's failure in plain words, with the numbers and dates from `details`. */
export function pollError(e: unknown): string {
  const err = toAppError(e);
  if (err.code === "proRequired") return "Creating and opening polls is part of Kinetra Pro. Your existing polls still work.";
  if (err.code === "proLimited") return "Your Pro access is ending: existing polls keep running, but new ones can't start.";
  const d = err.details ?? {};
  const max = typeof d.max === "number" ? d.max : null;
  if (err.code === "cloud" && err.apiCode === "poll_limit_reached" && max !== null) {
    return `${max} ${max === 1 ? "poll is" : "polls are"} open, the most your plan allows. Close one to open this.`;
  }
  if (err.code === "cloud" && err.apiCode === "poll_monthly_limit_reached") {
    const from = dayMonth(typeof d.resets_at === "number" ? d.resets_at : null);
    const n = max !== null ? `${fmt(max)} ${max === 1 ? "poll" : "polls"}` : "as many polls as your plan allows";
    return `You've created ${n} this month.${from ? ` New ones from ${from}.` : ""}`;
  }
  return cloudMessage(e);
}

// ---- Editing ---------------------------------------------------------------------------------------

export interface Draft {
  question: string;
  options: string[];
  /** Seconds, or null for no time limit. */
  duration: number | null;
  showResults: boolean;
}

export const emptyDraft = (): Draft => ({ question: "", options: ["", ""], duration: null, showResults: true });

export const draftOf = (p: PollData): Draft => ({
  question: p.question,
  options: p.options.map((o) => o.label),
  duration: p.durationSeconds,
  showResults: p.showResults,
});

/** What's wrong with a draft (the core's rules), or null. `maxOptions` from the plan. */
export function checkDraft(d: Draft, maxOptions = POLL_OPTIONS_MAX): string | null {
  const q = d.question.trim();
  if (!q) return "Add a question.";
  if ([...q].length > POLL_QUESTION_MAX) return `The question is too long: at most ${POLL_QUESTION_MAX} characters.`;
  if (/[\u0000-\u001f\u007f]/.test(q)) return "The question can't contain line breaks.";
  const max = Math.min(Math.max(maxOptions, POLL_OPTIONS_MIN), POLL_OPTIONS_MAX);
  if (d.options.length < POLL_OPTIONS_MIN || d.options.length > max) {
    return `A poll needs ${POLL_OPTIONS_MIN} to ${max} options on your plan.`;
  }
  const seen = new Set<string>();
  for (const [i, raw] of d.options.entries()) {
    const o = raw.trim();
    if (!o) return `Answer ${i + 1} is empty.`;
    if ([...o].length > POLL_OPTION_MAX) return `Answer ${i + 1} is too long: at most ${POLL_OPTION_MAX} characters.`;
    if (/[\u0000-\u001f\u007f]/.test(o)) return "Answers can't contain line breaks.";
    if (seen.has(o.toLowerCase())) return `Two answers are the same: “${o}”.`;
    seen.add(o.toLowerCase());
  }
  if (d.duration !== null && (d.duration < POLL_DURATION_MIN_S || d.duration > POLL_DURATION_MAX_S)) {
    return "A time limit must be between 30 seconds and 24 hours.";
  }
  return null;
}

/** A new poll from a draft. */
export function createInput(d: Draft): PollInput {
  const input: PollInput = {
    question: d.question.trim(),
    options: d.options.map((o) => o.trim()),
    showResults: d.showResults,
  };
  if (d.duration !== null) input.durationSeconds = d.duration;
  return input;
}

/** Only what changed (the server refuses text changes while open or with votes). Null: nothing. */
export function updateInput(d: Draft, p: PollData): PollInput | null {
  const input: PollInput = {};
  if (d.question.trim() !== p.question) input.question = d.question.trim();
  const opts = d.options.map((o) => o.trim());
  if (opts.length !== p.options.length || opts.some((o, i) => o !== p.options[i]?.label)) input.options = opts;
  if (d.duration !== p.durationSeconds) {
    if (d.duration === null) input.clearDuration = true;
    else input.durationSeconds = d.duration;
  }
  if (d.showResults !== p.showResults) input.showResults = d.showResults;
  return Object.keys(input).length ? input : null;
}

/** Why the question and answers can't be edited, or null. */
export function textLocked(p: PollData): string | null {
  if (p.status === "open") return "Close the poll to change the question or answers.";
  if (p.totalVotes > 0) return "Reset the votes to change the question or answers.";
  return null;
}

// ---- Durations -------------------------------------------------------------------------------------

/** 120 → "2:00", 5400 → "1:30:00". */
export function formatDuration(s: number): string {
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
}

/** "2:00", "1:30:00", "90" (seconds), "5m", "1h" → seconds; null when unreadable. */
export function parseDuration(text: string): number | null {
  const t = text.trim().toLowerCase();
  if (!t) return null;
  let m = /^(\d+)\s*([smh])$/.exec(t);
  if (m) return Number(m[1]) * { s: 1, m: 60, h: 3600 }[m[2] as "s" | "m" | "h"];
  if (/^\d+$/.test(t)) return Number(t);
  m = /^(\d+):([0-5]\d)(?::([0-5]\d))?$/.exec(t);
  if (!m) return null;
  return m[3] === undefined ? Number(m[1]) * 60 + Number(m[2]) : Number(m[1]) * 3600 + Number(m[2]) * 60 + Number(m[3]);
}

/** "Deleted in 11 h" for a closed poll awaiting deletion, else "". */
export function deletesIn(p: Pick<PollData, "status" | "deleteAfter">, nowMs: number): string {
  if (p.status !== "closed" || !p.deleteAfter) return "";
  const ms = Date.parse(p.deleteAfter) - nowMs;
  if (!Number.isFinite(ms)) return "";
  if (ms <= 60_000) return "Deleted soon";
  const h = Math.floor(ms / 3_600_000);
  return h >= 1 ? `Deleted in ${h} h` : `Deleted in ${Math.ceil(ms / 60_000)} min`;
}
