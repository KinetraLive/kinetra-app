// Preferences > Server > OSC, the pure part: defaults, allowlist edits, the listener status line
// and the address list. The address scheme (exactly the core's, src-tauri/src/core/osc.rs):
//   /kinetra/layer/<screenId>/<layerId>/take | clear | toggle | update
//   /kinetra/layer/<screenId>/<layerId>/opacity <0..1>
//   /kinetra/layer/<screenId>/<layerId>/var/<key> <0..1>   (a range variable, min..max)
//   /kinetra/layer/<screenId>/<layerId>/play | pause | stop   (media layers)
//   /kinetra/layer/<screenId>/<layerId>/seek <seconds>         (media layers)
//   /kinetra/flow/<flowId>/go
//   /kinetra/source/<sourceId>/next | prev
//   /kinetra/screen/<screenId>/clear
//   /kinetra/clear-all
//   /kinetra/panic
// Layer addresses carry the screen id too: layer ids are unique per screen only.

import { OSC_DEFAULT_PORT, type OscSettings, type OscStatus } from "$lib/types";

export const OSC_DEFAULTS: OscSettings = { enabled: false, port: OSC_DEFAULT_PORT, allow: [], feedback: null };
export const OSC_ALLOW_MAX = 32;

const IPV4 = /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/;
const IPV6 = /^[0-9a-f:]+$/i;

/** An IPv4 or IPv6 literal (the core checks it again). */
export function isIp(s: string): boolean {
  return IPV4.test(s) || (s.includes(":") && IPV6.test(s) && s.split("::").length <= 2);
}

/** The allowlist with `ip` added (trimmed, lowercase, no duplicates); null when it isn't valid. */
export function withAllowed(list: readonly string[], ip: string): string[] | null {
  const v = ip.trim().toLowerCase();
  if (!isIp(v) || list.length >= OSC_ALLOW_MAX) return null;
  return list.includes(v) ? [...list] : [...list, v];
}

export interface AddressPick {
  screenId: string | null;
  layerId: string | null;
  flowId: string | null;
  sourceId: string | null;
  /** A variable key of the layer, for the var address ("<key>" when none). */
  varKey?: string | null;
}

export interface OscAddress {
  address: string;
  /** The argument, when it takes one ("0..1", "<value>"). */
  arg?: string;
  /** What it does. */
  label: string;
}

/** The addresses with the picked ids filled in (placeholders where nothing is picked). */
export function oscAddresses(p: AddressPick): OscAddress[] {
  const screen = p.screenId ?? "<screenId>";
  const layer = `/kinetra/layer/${screen}/${p.layerId ?? "<layerId>"}`;
  const flow = `/kinetra/flow/${p.flowId ?? "<flowId>"}`;
  const source = `/kinetra/source/${p.sourceId ?? "<sourceId>"}`;
  return [
    { address: `${layer}/take`, label: "Take" },
    { address: `${layer}/clear`, label: "Clear" },
    { address: `${layer}/toggle`, label: "Take or clear" },
    { address: `${layer}/update`, label: "Update" },
    { address: `${layer}/opacity`, arg: "0..1", label: "Opacity" },
    { address: `${layer}/var/${p.varKey || "<key>"}`, arg: "0..1", label: "Variable" },
    { address: `${layer}/play`, label: "Play media" },
    { address: `${layer}/pause`, label: "Pause media" },
    { address: `${layer}/stop`, label: "Stop media" },
    { address: `${layer}/seek`, arg: "seconds", label: "Go to time in media" },
    { address: `${flow}/go`, label: "GO a cue" },
    { address: `${source}/next`, label: "Next row" },
    { address: `${source}/prev`, label: "Previous row" },
    { address: `/kinetra/screen/${screen}/clear`, label: "Clear screen" },
    { address: "/kinetra/clear-all", label: "Clear all" },
    { address: "/kinetra/panic", label: "Panic" },
  ];
}

/** "host:port" → feedback, or null when it doesn't parse. */
export function parseTarget(s: string): { host: string; port: number } | null {
  const m = /^\s*(\[[^\]]+\]|[^:\s]+):(\d{1,5})\s*$/.exec(s);
  if (!m) return null;
  const port = Number(m[2]);
  if (port < 1 || port > 65535) return null;
  return { host: m[1].replace(/^\[|\]$/g, ""), port };
}

export const targetString = (f: { host: string; port: number }) => (f.host.includes(":") ? `[${f.host}]:${f.port}` : `${f.host}:${f.port}`);

/** The OSC status line: nothing when off; ready with the port (and LAN) when listening; a warning
 * when the port is taken; the core's message for any other bind failure. */
export type OscStatusLine =
  | { kind: "listening"; port: number; lan: boolean }
  | { kind: "portInUse"; port: number }
  | { kind: "failed"; message: string };

export function oscStatusLine(s: OscStatus | null): OscStatusLine | null {
  switch (s?.state) {
    case "listening":
      return { kind: "listening", port: s.port, lan: s.lan };
    case "error":
      return s.code === "portInUse" ? { kind: "portInUse", port: s.port } : { kind: "failed", message: s.message };
    default:
      return null;
  }
}
