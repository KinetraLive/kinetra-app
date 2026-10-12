// Display outputs, the pure part shared by Preferences › Outputs, Control's outputs strip and the
// Output menu: labels, status text, the display map's layout, display / screen choices and the
// prefs edits. No Svelte, no I/O. The core clamps whatever is saved (≤ 8, unique names, a key
// output's fill, one enabled output per display), so these edits only keep the UI sensible.

import {
  CHROMA_GREEN,
  OUTPUTS_MAX,
  type DisplayInfo,
  type OutputBackground,
  type OutputConfig,
  type OutputStatus,
  type TestPattern,
} from "$lib/types";

export { menuLabel, patchOutput, screenOf, showingText, warningText } from "./labels";
import { patchOutput } from "./labels";

// ---- Labels -----------------------------------------------------------------------------------

/** "3840×2160". */
export const resolution = (d: { width: number; height: number }): string => `${d.width}×${d.height}`;

export const PATTERNS: readonly { value: TestPattern; label: string }[] = [
  { value: "bars", label: "Bars" },
  { value: "grid", label: "Grid" },
  { value: "safe", label: "Safe areas" },
  { value: "name", label: "Name" },
];

/** How a status reads: `program` = on air (a red TallyDot), `warning` = a problem (amber text and
 * icon, never a tally color), `muted` = off or waiting. */
export type StatusTone = "program" | "warning" | "muted";

export interface StatusLine {
  tone: StatusTone;
  text: string;
}

/** The status of an output as one short line ("Showing", "Display disconnected", …). */
export function statusLine(o: OutputConfig, s: OutputStatus | undefined): StatusLine {
  const state = s?.state ?? (o.enabled ? "noProject" : "off");
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

/** Key pairs: "Frame-locked" / "Not frame-locked: displays differ"; null when not a pair or not
 * shown yet. */
export function frameLockText(s: OutputStatus | undefined): string | null {
  if (!s || s.frameLocked === null) return null;
  return s.frameLocked ? "Frame-locked" : "Not frame-locked: displays differ";
}

/** The outputs showing (a screen or a pattern), in prefs order. */
export const showing = (configs: readonly OutputConfig[], status: readonly OutputStatus[]): OutputConfig[] =>
  configs.filter((o) => status.find((s) => s.id === o.id)?.state === "showing");

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
  configs.filter((c) => c.mode === "fill" && c.id !== id).map((c) => ({ value: c.id, label: c.name }));

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
  const used = new Set(configs.map((c) => c.display.id));
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
  const fill = configs.find((c) => c.mode === "fill" && c.id !== id);
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
