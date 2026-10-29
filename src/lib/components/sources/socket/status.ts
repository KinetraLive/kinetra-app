// A live data source's connection status in words: the short form for lists and Control's sources
// strip (Live / Connected / No data / Waiting / Error / Paused, with the TallyDot state) and the
// editor's longer one. Tiny and pure: the Sources list and the lazy strip import it.

import type { SocketState, SocketStatus } from "$lib/types";

export type SocketTally = "preview" | "warning" | "idle";

export interface SocketStatusText {
  tally: SocketTally;
  text: string;
}

/** Live (green) / Waiting / Error (amber) / Paused …: lists and Control's sources strip. */
export function socketShort(s: SocketStatus | null | undefined): SocketStatusText {
  switch (s?.state) {
    case "live":
      return { tally: "preview", text: "Live" };
    case "connected":
      return { tally: "idle", text: "Connected" };
    case "stale":
      return { tally: "idle", text: "No data" };
    case "error":
      return { tally: "warning", text: "Error" };
    case "paused":
      return { tally: "idle", text: "Paused" };
    case "connecting":
      return { tally: "idle", text: "Connecting" };
    default:
      return { tally: "idle", text: "Waiting" };
  }
}

const LONG: Record<SocketState, string> = {
  idle: "Waiting: not in use",
  connecting: "Connecting",
  waiting: "Waiting for the server",
  connected: "Connected, no data yet",
  live: "Live",
  stale: "Connected, no data for 5 s",
  error: "Error",
  paused: "Paused",
};

/** The editor's Status line. */
export function socketLong(s: SocketStatus | null | undefined): SocketStatusText {
  return { tally: socketShort(s).tally, text: s ? LONG[s.state] : "Waiting" };
}
