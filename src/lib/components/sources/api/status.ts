// An API source's fetch status in words: the short form for lists and Control's sources strip
// (Live / Waiting / Error / Paused, with the TallyDot state) and the editor's longer one, plus
// relative times ("4 s ago", "in 8 s"). Tiny and pure: the Sources list and the lazy strip import it.

import type { ApiState, ApiStatus } from "$lib/types";

export type ApiTally = "preview" | "warning" | "idle";

export interface ApiStatusText {
  tally: ApiTally;
  text: string;
}

/** Live (green) / Waiting / Error (amber) / Paused: lists and the Control strip. */
export function apiShort(s: ApiStatus | null | undefined): ApiStatusText {
  switch (s?.state) {
    case "ok":
      return { tally: "preview", text: "Live" };
    case "fetching":
      return s.lastOkAt !== null ? { tally: "preview", text: "Live" } : { tally: "idle", text: "Waiting" };
    case "error":
    case "backoff":
    case "secretMissing":
      return { tally: "warning", text: "Error" };
    case "paused":
      return { tally: "idle", text: "Paused" };
    default:
      return { tally: "idle", text: "Waiting" };
  }
}

const LONG: Record<ApiState, string> = {
  ok: "Live",
  fetching: "Fetching",
  idle: "Waiting: not in use",
  error: "Error",
  backoff: "Error, retrying",
  secretMissing: "Secret missing",
  paused: "Paused",
};

/** The editor's Status line. */
export function apiLong(s: ApiStatus | null | undefined): ApiStatusText {
  const short = apiShort(s);
  return { tally: short.tally, text: s ? LONG[s.state] : "Waiting" };
}

function span(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min`;
  if (s < 86400) return `${Math.floor(s / 3600)} h`;
  return `${Math.floor(s / 86400)} d`;
}

/** "4 s ago", "3 min ago" (server epoch ms; formatted on render). */
export const ago = (at: number, now: number): string => `${span(now - at)} ago`;

/** "in 8 s", or "now" once due. */
export const until = (at: number, now: number): string => (at - now < 500 ? "now" : `in ${span(at - now)}`);
