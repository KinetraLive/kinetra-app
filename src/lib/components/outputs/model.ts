// Display outputs, the pure part shared by Preferences › Outputs, Control's outputs strip and the
// Output menu: labels, status text, the display map's layout, display / screen choices and the
// prefs edits. No Svelte, no I/O. The core clamps whatever is saved (≤ 8, unique names, a key
// output's fill, one enabled output per display), so these edits only keep the UI sensible.

import {
  CHROMA_GREEN,
  NDI_OUTPUTS_MAX,
  OUTPUTS_MAX,
  type DisplayInfo,
  type NdiOutputSettings,
  type NdiRate,
  type OutputBackground,
  type OutputConfig,
  type OutputStatus,
  type TestPattern,
} from "$lib/types";

export { isOnAir, menuLabel, patchOutput, screenOf, showingText, warningText } from "./labels";
import { isOnAir, patchOutput } from "./labels";

// ---- Labels -----------------------------------------------------------------------------------

/** "3840×2160". */
export const resolution = (d: { width: number; height: number }): string => `${d.width}×${d.height}`;

export const PATTERNS: readonly { value: TestPattern; label: string }[] = [
  { value: "bars", label: "Bars" },
  { value: "grid", label: "Grid" },
  { value: "safe", label: "Safe areas" },
  { value: "name", label: "Name" },
];

/** How a status reads: `program` = on air (a red TallyDot), `preview` = on a receiver's preview
 * (a green TallyDot; NDI tally), `warning` = a problem (amber text and icon, never a tally color),
 * `muted` = off or waiting. */
export type StatusTone = "program" | "preview" | "warning" | "muted";

export interface StatusLine {
  tone: StatusTone;
  text: string;
}

/** The status of an output as one short line ("Showing", "Display disconnected", …). */
export function statusLine(o: OutputConfig, s: OutputStatus | undefined): StatusLine {
  const state = s?.state ?? (o.enabled ? "noProject" : "off");
  if (o.kind === "ndi") return ndiStatusLine(state, s);
  if (s?.pattern && state !== "displayMissing" && state !== "failed") return { tone: "program", text: "Test pattern" };
  switch (state) {
    case "showing":
      return { tone: "program", text: "Showing" };
    case "displayMissing":
      return { tone: "warning", text: "Display disconnected" };
    case "screenMissing":
      return { tone: "warning", text: "Screen not found" };
    case "failed":
      return { tone: "warning", text: s?.message || "Couldn't open the output" };
    case "noProject":
      return { tone: "muted", text: "Waiting for a project" };
    default:
      return { tone: "muted", text: "Off" };
  }
}

/** An NDI output's status: on air only with a receiver; the receiver count itself isn't shown
 * until it is checked against vMix and Studio Monitor (owner, 2026-10-21). */
function ndiStatusLine(state: OutputStatus["state"], s: OutputStatus | undefined): StatusLine {
  switch (state) {
    case "showing": {
      const live = s?.ndi;
      if (live?.dropping) return { tone: "warning", text: "Dropping frames" };
      if (live?.program) return { tone: "program", text: "On program" };
      if (live?.preview) return { tone: "preview", text: "On preview" };
      if (live && live.receivers > 0) return { tone: "program", text: "Sending" };
      return { tone: "muted", text: "Ready · no receiver" };
    }
    case "ndiMissing":
      return { tone: "warning", text: "NDI® Runtime not installed" };
    case "screenMissing":
      return { tone: "warning", text: "Screen not found" };
    case "failed":
      return { tone: "warning", text: s?.message || "Couldn't start" };
    case "noProject":
      return { tone: "muted", text: "Waiting for a project" };
    default:
      return { tone: "muted", text: "Off" };
  }
}

/** Key pairs: "Frame-locked" / "Not frame-locked: displays differ"; null when not a pair or not
 * shown yet. */
export function frameLockText(s: OutputStatus | undefined): string | null {
  if (!s || s.frameLocked === null) return null;
  return s.frameLocked ? "Frame-locked" : "Not frame-locked: displays differ";
}

/** The outputs on air (a screen or a pattern; NDI: with a receiver), in prefs order. */
export const showing = (configs: readonly OutputConfig[], status: readonly OutputStatus[]): OutputConfig[] =>
  configs.filter((o) => isOnAir(status.find((s) => s.id === o.id)));

// ---- Display map --------------------------------------------------------------------------------

export interface MapBox {
  display: DisplayInfo;
  /** Percent of the map's box. */
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface MapLayout {
  /** Width / height of the whole desktop. */
  aspect: number;
  boxes: MapBox[];
}

/** Displays at their desktop positions, as percentages of the desktop's bounding box. */
export function mapLayout(displays: readonly DisplayInfo[]): MapLayout {
  if (displays.length === 0) return { aspect: 16 / 9, boxes: [] };
  const x0 = Math.min(...displays.map((d) => d.x));
  const y0 = Math.min(...displays.map((d) => d.y));
  const x1 = Math.max(...displays.map((d) => d.x + d.width));
  const y1 = Math.max(...displays.map((d) => d.y + d.height));
  const w = Math.max(1, x1 - x0);
  const h = Math.max(1, y1 - y0);
  const pct = (v: number, of: number) => Math.round((v / of) * 10000) / 100;
  return {
    aspect: w / h,
    boxes: displays.map((d) => ({
      display: d,
      left: pct(d.x - x0, w),
      top: pct(d.y - y0, h),
      width: pct(d.width, w),
      height: pct(d.height, h),
    })),
  };
}

/** The display an output is on now (the core's fallback match wins), else its configured one. */
export const displayIdOf = (o: OutputConfig, s: OutputStatus | undefined): string => s?.displayId ?? o.display.id;

// ---- Choices ------------------------------------------------------------------------------------

/** The Display select: connected displays ("DELL U2720Q · 3840×2160"), then the remembered one
 * when it isn't connected ("DELL U2720Q (disconnected)"). */
export function displayOptions(o: OutputConfig, s: OutputStatus | undefined, displays: readonly DisplayInfo[]): { value: string; label: string }[] {
  const out = displays.map((d) => ({ value: d.id, label: `${d.name} · ${resolution(d)}` }));
  const current = displayIdOf(o, s);
  if (!displays.some((d) => d.id === current)) out.push({ value: current, label: `${o.display.name || "Display"} (disconnected)` });
  return out;
}

/** The fill outputs a key output can key for (not itself). */
export const fillOptions = (configs: readonly OutputConfig[], id: string): { value: string; label: string }[] =>
  configs.filter((c) => c.mode === "fill" && c.kind === "display" && c.id !== id).map((c) => ({ value: c.id, label: c.name }));

export type BackgroundKind = OutputBackground["kind"];

export const BACKGROUNDS: readonly { value: BackgroundKind; label: string }[] = [
  { value: "black", label: "Black" },
  { value: "green", label: "Green" },
  { value: "color", label: "Color" },
];

/** The page color behind the screen. */
export function backgroundColor(b: OutputBackground): string {
  return b.kind === "green" ? CHROMA_GREEN : b.kind === "color" ? b.color : "#000000";
}

// ---- Edits ----------------------------------------------------------------------------------------

/** A new output id (10 characters, like the core's nanoid). */
export function outputId(): string {
  const abc = "0123456789abcdefghijklmnopqrstuvwxyz";
  const bytes = new Uint8Array(10);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => abc[b % abc.length]).join("");
}

/** "Output N", the lowest N not taken (ignoring case). */
export function nextName(configs: readonly OutputConfig[]): string {
  const taken = new Set(configs.map((c) => c.name.toLowerCase()));
  for (let n = 1; ; n++) if (!taken.has(`output ${n}`)) return `Output ${n}`;
}

/** The remembered part of a display (for the "disconnected" text and the fallback match). */
export const displayRef = (d: DisplayInfo): OutputConfig["display"] => ({
  id: d.id,
  name: d.name,
  edidKey: d.edidKey,
  width: d.width,
  height: d.height,
});

/**
 * Add output: the first display no output uses (one without Kinetra's window first), else the
 * first display; the first screen; fill on black, always on top. Off until switched on, so a
 * new output never covers a display by surprise. Null at the limit or without displays.
 */
export function newOutput(configs: readonly OutputConfig[], displays: readonly DisplayInfo[], firstScreen: string | null): OutputConfig | null {
  if (configs.length >= OUTPUTS_MAX || displays.length === 0) return null;
  const used = new Set(configs.filter((c) => c.kind === "display").map((c) => c.display.id));
  const free = displays.filter((d) => !used.has(d.id));
  const display = free.find((d) => !d.hasMainWindow) ?? free[0] ?? displays[0];
  return {
    id: outputId(),
    kind: "display",
    name: nextName(configs),
    enabled: false,
    display: displayRef(display),
    screenId: firstScreen,
    mode: "fill",
    background: { kind: "black" },
    keyOf: null,
    alwaysOnTop: true,
  };
}

/**
 * Fill ⇄ Key. A key output keys for the first other fill output (null when there is none: the
 * caller says so instead of saving). Turning an output back to fill releases its keyOf; key
 * outputs that keyed for an output that becomes a key are left to the core (they become fill).
 */
export function withMode(configs: readonly OutputConfig[], id: string, mode: OutputConfig["mode"]): OutputConfig[] | null {
  if (mode === "fill") return patchOutput(configs, id, { mode, keyOf: null });
  const fill = configs.find((c) => c.mode === "fill" && c.kind === "display" && c.id !== id);
  if (!fill) return null;
  return patchOutput(configs, id, { mode, keyOf: fill.id });
}

/** Removes an output; key outputs that keyed for it become fill outputs (as the core does). */
export function removeOutput(configs: readonly OutputConfig[], id: string): OutputConfig[] {
  return configs.filter((c) => c.id !== id).map((c) => (c.keyOf === id ? { ...c, mode: "fill" as const, keyOf: null } : c));
}

/** A background from the segmented choice (Color keeps the last custom color, else a mid gray). */
export function withBackground(o: OutputConfig, kind: BackgroundKind): OutputBackground {
  if (kind === "color") return { kind, color: o.background.kind === "color" ? o.background.color : "#808080" };
  return { kind };
}

// ---- NDI outputs ------------------------------------------------------------------------------------

/** A free source name: "Kinetra <screen>" (or "Kinetra"), then " 2", " 3"… (outputs' names are
 * unique ignoring case; the NDI source is named after its output). ≤ 40 characters. */
export function ndiName(configs: readonly OutputConfig[], screenName: string | null): string {
  const taken = new Set(configs.map((c) => c.name.toLowerCase()));
  const base = (screenName ? `Kinetra ${screenName}` : "Kinetra").slice(0, 40).trim();
  if (!taken.has(base.toLowerCase())) return base;
  for (let n = 2; ; n++) {
    const suffix = ` ${n}`;
    const name = base.slice(0, 40 - suffix.length).trimEnd() + suffix;
    if (!taken.has(name.toLowerCase())) return name;
  }
}

/** NDI outputs configured. */
export const ndiCount = (configs: readonly OutputConfig[]): number => configs.filter((c) => c.kind === "ndi").length;

/** Whether another NDI output fits (≤ 8 outputs, ≤ 4 of them NDI). */
export const canAddNdi = (configs: readonly OutputConfig[]): boolean =>
  configs.length < OUTPUTS_MAX && ndiCount(configs) < NDI_OUTPUTS_MAX;

/** Add NDI output: the screen, the region's frame rate, transparency on, straight alpha, Program.
 * Off unless `enabled` (Preferences adds it off like a display output; "Send over NDI®" turns it
 * on). Null at the limits. */
export function newNdiOutput(
  configs: readonly OutputConfig[],
  screen: { id: string; name: string } | null,
  rate: NdiRate,
  enabled = false,
): OutputConfig | null {
  if (!canAddNdi(configs)) return null;
  return {
    id: outputId(),
    kind: "ndi",
    name: ndiName(configs, screen?.name ?? null),
    enabled,
    display: { id: "", name: "", edidKey: "", width: 0, height: 0 },
    screenId: screen?.id ?? null,
    mode: "fill",
    background: { kind: "black" },
    keyOf: null,
    alwaysOnTop: false,
    ndi: { rate, alpha: true, premultiplied: false, groups: null, bus: "program" },
  };
}

/** The NDI output that sends a screen (the first one, enabled first). */
export function ndiOutputOf(configs: readonly OutputConfig[], screenId: string): OutputConfig | undefined {
  const all = configs.filter((c) => c.kind === "ndi" && c.screenId === screenId);
  return all.find((c) => c.enabled) ?? all[0];
}

/**
 * "Send {screen} over NDI®" / "Stop sending…": turns that screen's NDI output on (making one when
 * there is none) or off. Null when a new one doesn't fit.
 */
export function withScreenNdi(
  configs: readonly OutputConfig[],
  screen: { id: string; name: string },
  on: boolean,
  rate: NdiRate,
): OutputConfig[] | null {
  const existing = ndiOutputOf(configs, screen.id);
  if (existing) return patchOutput(configs, existing.id, { enabled: on });
  if (!on) return [...configs];
  const o = newNdiOutput(configs, screen, rate, true);
  return o ? [...configs, o] : null;
}

/** Patches an NDI output's settings. */
export function withNdi(o: OutputConfig, patch: Partial<NdiOutputSettings>): Partial<OutputConfig> {
  const base: NdiOutputSettings = o.ndi ?? { rate: "50", alpha: true, premultiplied: false, groups: null, bus: "program" };
  return { ndi: { ...base, ...patch } };
}
