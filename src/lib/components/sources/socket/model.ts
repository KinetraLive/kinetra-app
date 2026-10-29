// Pure logic of the live data (WebSocket) source editor: checks against the backend's rules
// (core/socket_source.rs: ws:// or wss://, IP addresses on this computer or the local network, the
// filter's pointer, the rate). No Svelte, no I/O.

import { SOCKET_RATE_MAX, SOCKET_RATE_MIN, SOCKET_SEND_MAX, type SourceDoc } from "$lib/types";
import { columnIssues, nameIssue } from "../api/model";

export type SocketDoc = Extract<SourceDoc, { kind: "socket" }>;

/** The backend's words for an address outside the network. */
export const NOT_LOCAL = "Live data comes from this computer or your local network.";

// ---- Addresses ----------------------------------------------------------------------------------

function v4Local(parts: number[]): boolean {
  const [a, b] = parts;
  return (
    a === 127 ||
    a === 10 ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 169 && b === 254) ||
    // Shared address space: Tailscale and similar VPNs.
    (a === 100 && b >= 64 && b <= 127)
  );
}

/** Whether an IP literal is on this computer or the local network (as `is_local_ip`); null when
 * `host` isn't an IP address (names are checked when connecting). */
export function localIp(host: string): boolean | null {
  const v4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(host);
  if (v4) {
    const parts = v4.slice(1).map(Number);
    return parts.every((p) => p <= 255) ? v4Local(parts) : null;
  }
  if (!host.includes(":")) return null;
  const h = host.toLowerCase();
  const mapped = /^::ffff:(\d{1,3}(?:\.\d{1,3}){3})$/.exec(h);
  if (mapped) return localIp(mapped[1]);
  if (h === "::1") return true;
  const first = parseInt(h.split(":")[0] || "0", 16);
  return (first & 0xfe00) === 0xfc00 || (first & 0xffc0) === 0xfe80;
}

/** What's wrong with a URL, or null (empty: not configured yet). */
export function urlIssue(raw: string): string | null {
  const u = raw.trim();
  if (u === "") return null;
  let url: URL;
  try {
    url = new URL(u);
  } catch {
    return "Enter a ws:// or wss:// address.";
  }
  if (url.protocol !== "ws:" && url.protocol !== "wss:") return "Enter a ws:// or wss:// address.";
  const host = url.hostname.replace(/^\[|\]$/g, "");
  if (host === "") return "Enter a ws:// or wss:// address.";
  return localIp(host) === false ? NOT_LOCAL : null;
}

// ---- Checks -------------------------------------------------------------------------------------

export interface SocketIssues {
  name: string | null;
  url: string | null;
  filter: string | null;
  send: string | null;
  columns: (string | null)[];
  table: string | null;
}

export function socketIssues(doc: SocketDoc): SocketIssues {
  const f = doc.filter;
  const ptr = f?.pointer.trim() ?? "";
  const filter =
    f && (ptr !== "" || f.equals !== "") && (ptr === "" || !ptr.startsWith("/")) ? "A pointer starts with /, like /type." : null;
  const send =
    new TextEncoder().encode(doc.sendOnConnect ?? "").length > SOCKET_SEND_MAX ? `Up to ${SOCKET_SEND_MAX / 1024} KB.` : null;
  return { name: nameIssue(doc.name), url: urlIssue(doc.url), filter, send, ...columnIssues(doc.mapping.columns, true) };
}

/** Why the working copy can't be saved, or null. */
export function socketBlocked(doc: SocketDoc, issues: SocketIssues = socketIssues(doc)): string | null {
  if (issues.name) return `Name: ${issues.name}`;
  if (issues.url) return `URL: ${issues.url}`;
  if (issues.filter) return `Filter: ${issues.filter}`;
  if (issues.send) return `Send on connect: ${issues.send}`;
  const ci = issues.columns.findIndex((e) => e !== null);
  if (ci >= 0) return `${doc.mapping.columns[ci].label.trim() || `Column ${ci + 1}`}: ${issues.columns[ci]}`;
  return issues.table;
}

/** Updates a second typed → clamped to SOCKET_RATE_MIN…SOCKET_RATE_MAX, whole numbers. */
export function rateFrom(n: number): { rate: number; clamped: boolean } {
  const r = Math.round(n);
  if (!Number.isFinite(r) || r < SOCKET_RATE_MIN) return { rate: SOCKET_RATE_MIN, clamped: true };
  if (r > SOCKET_RATE_MAX) return { rate: SOCKET_RATE_MAX, clamped: true };
  return { rate: r, clamped: false };
}

/** The filter as the document stores it: absent when both parts are empty. */
export function withFilter(doc: SocketDoc, pointer: string, equals: string): SocketDoc {
  if (pointer.trim() !== "" || equals !== "") return { ...doc, filter: { pointer, equals } };
  const next: SocketDoc = { ...doc };
  delete next.filter;
  return next;
}

