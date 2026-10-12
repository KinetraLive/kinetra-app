// Palette search (part of the palette's lazy chunk). Pure: commands + context + query → sections.
//
// Two steps so a keystroke stays far below one frame with ~2000 entries:
//   prepare(cmds, ctx)  once per open (and when the command list or provider data changes):
//                       drops commands whose `when` is false, translates titles (`t(title,
//                       params())`), folds them and precomputes word starts, evaluates `rank(ctx)`.
//   searchSections(…)   per keystroke: scores every entry into a reused typed array, keeps the top
//                       rows of each section by insertion (no full sort, no per-entry objects), and
//                       builds `Match` objects and highlight ranges only for the rows shown.
//
// Rules
// - Prefixes: ">" commands, "@" screens, "#" templates, "/" settings (scope filters; the rest of the
//   text is the query).
// - Title first; keywords match at half weight (a keyword substring ranks below a title word-start
//   match) and never highlight. `rank(ctx)` is added to the score; one tier (substring / word
//   starts / consecutive / scattered) is `TIER` = 10 000 points, so `rank: () => 1000` reorders
//   within a tier and `rank: () => 10_000` lifts a whole tier.
// - Noise: once anything matches by word starts or better, scattered-only matches are dropped.
// - Sections, in order: Recent, then in Control Live · Go to · Create · Edit · Project items ·
//   Settings; elsewhere Go to · Create · Edit · Project items · Live · Settings. `hidden` commands
//   are never listed. Empty query: Recent (newest first)
//   then each section in rank, then registration order. With a query: Recent keeps its recency
//   order; each section is best first, ties in registration order (stable). A command in Recent
//   isn't repeated in its own section.
// - Caps per section (the palette shows a "Show N more" row for the rest): 5 with an empty query,
//   8 with a query, 50 inside a prefix scope; `limits` raises a section's cap ("Show more" adds
//   50 each time). Recent is at most 5 (it is the last 5 commands).
// - `disabled` commands stay listed (the palette greys them out with the reason).

import { t as translate } from "$lib/i18n";
import type { Command, CommandContext, Scope, Section } from "../registry.svelte";
import { TIER, boundaries, fold, foldQuery, lastRanges, scoreFolded } from "./score";

/** One search result (the contract's `Match`). */
export interface Match {
  command: Command;
  /** The translated title. */
  title: string;
  section: Section;
  score: number;
  /** [start, end) ranges in `title` to highlight. */
  ranges: readonly (readonly [number, number])[];
}

export const CAP_EMPTY = 5;
export const CAP_QUERY = 8;
export const CAP_SCOPED = 50;
export const MORE_STEP = 50;
export const RECENT_MAX = 5;

/** A command ready for searching (built once per open). */
export interface Entry {
  command: Command;
  /** Registration order. */
  index: number;
  /** Translated title, its folded form and word starts. */
  title: string;
  ft: string;
  fb: Uint8Array;
  /** Folded keywords (joined), or "" for none. */
  kw: string;
  kb: Uint8Array;
  section: Exclude<Section, "recent">;
  scope: Scope;
  rank: number;
}

export interface SectionResult {
  section: Section;
  /** The rows shown (capped). */
  matches: Match[];
  /** Every match in this section (shown + the "Show N more" rest). */
  total: number;
}

const PREFIX: Record<string, Scope> = { ">": "command", "@": "screen", "#": "template", "/": "setting" };

/** Splits a leading scope prefix off the raw input. */
export function parseQuery(raw: string): { scope: Scope | null; text: string } {
  const s = raw.trimStart();
  const scope = PREFIX[s[0] ?? ""];
  return scope ? { scope, text: s.slice(1).trim() } : { scope: null, text: s.trim() };
}

export function sectionOrder(ctx: CommandContext): Exclude<Section, "recent">[] {
  return ctx.route === "control"
    ? ["live", "goto", "create", "edit", "items", "settings"]
    : ["goto", "create", "edit", "items", "live", "settings"];
}

type Translator = (msg: string, params?: Readonly<Record<string, string | number>>) => string;

/** Commands allowed in `ctx`, translated and indexed for searching. */
export function prepare(cmds: readonly Command[], ctx: CommandContext, tr: Translator = translate): Entry[] {
  const out: Entry[] = [];
  for (let i = 0; i < cmds.length; i++) {
    const command = cmds[i];
    if (command.hidden) continue;
    let rank = 0;
    let title: string;
    try {
      if (command.when && !command.when(ctx)) continue;
      title = tr(command.title, command.params?.());
      rank = command.rank?.(ctx) ?? 0;
    } catch {
      continue; // a broken command never breaks the palette
    }
    const kwText = command.keywords && command.keywords.length > 0 ? command.keywords.join(" ") : "";
    out.push({
      command,
      index: i,
      title,
      ft: fold(title),
      fb: boundaries(title),
      kw: kwText ? fold(kwText) : "",
      kb: kwText ? boundaries(kwText) : EMPTY,
      section: command.section,
      scope: command.scope ?? "command",
      rank: Number.isFinite(rank) ? rank : 0,
    });
  }
  return out;
}

const EMPTY = new Uint8Array(0);

// Reused per keystroke (grown on demand).
let base = new Float64Array(0);

function ensure(n: number) {
  if (base.length < n) base = new Float64Array(Math.max(n, base.length * 2, 256));
}

/** Base score (title, else keywords at half weight) or -1. */
function scoreEntry(q: string, e: Entry): number {
  const s = scoreFolded(q, e.ft, e.fb);
  const k = e.kw ? scoreFolded(q, e.kw, e.kb) : -1;
  if (k < 0) return s;
  return Math.max(s, k * 0.5);
}

function toMatch(q: string, e: Entry, section: Section, final: number): Match {
  let ranges: [number, number][] = [];
  if (q !== "" && scoreFolded(q, e.ft, e.fb) >= 0) ranges = lastRanges(q);
  return { command: e.command, title: e.title, section, score: final, ranges };
}

/**
 * The palette's search: sections in display order, each capped (see the header). `limits` raises
 * a section's cap (the palette's "Show more").
 */
export function searchSections(
  query: string,
  entries: readonly Entry[],
  ctx: CommandContext,
  recent: readonly string[],
  limits?: Partial<Record<Section, number>>,
): SectionResult[] {
  const { scope, text } = parseQuery(query);
  const q = foldQuery(text);
  const n = entries.length;
  ensure(n);

  // Recent: resolvable ids among the allowed entries, newest first.
  const recentPos = new Map<string, number>();
  for (let i = 0; i < recent.length && recentPos.size < RECENT_MAX; i++) {
    if (!recentPos.has(recent[i])) recentPos.set(recent[i], recentPos.size);
  }

  // Pass 1: base scores (-1 = filtered out).
  let best = 0;
  for (let i = 0; i < n; i++) {
    const e = entries[i];
    if (scope && e.scope !== scope) {
      base[i] = -1;
      continue;
    }
    const s = q === "" ? 0 : scoreEntry(q, e);
    base[i] = s;
    if (s > best) best = s;
  }
  const floor = q !== "" && best >= 2 * TIER ? TIER : 0;

  // Pass 2: top rows per section by insertion; totals.
  const cap = scope ? CAP_SCOPED : q === "" ? CAP_EMPTY : CAP_QUERY;
  const order = sectionOrder(ctx);
  const tops = new Map<Section, number[]>();
  const totals = new Map<Section, number>();
  const recentRows: number[] = [];
  for (const s of order) {
    tops.set(s, []);
    totals.set(s, 0);
  }
  for (let i = 0; i < n; i++) {
    const b = base[i];
    if (b < 0 || b < floor) continue;
    const e = entries[i];
    if (recentPos.has(e.command.id)) {
      recentRows.push(i);
      continue;
    }
    const sec = e.section;
    const top = tops.get(sec);
    if (!top) continue;
    totals.set(sec, (totals.get(sec) ?? 0) + 1);
    const limit = limits?.[sec] ?? cap;
    const f = b + e.rank;
    // Insert after every row with a score >= f (stable: earlier registration first on ties).
    let p = top.length;
    while (p > 0 && base[top[p - 1]] + entries[top[p - 1]].rank < f) p--;
    if (p < limit) {
      top.splice(p, 0, i);
      if (top.length > limit) top.pop();
    }
  }


  const out: SectionResult[] = [];
  if (recentRows.length > 0) {
    recentRows.sort((a, b) => (recentPos.get(entries[a].command.id) ?? 0) - (recentPos.get(entries[b].command.id) ?? 0));
    out.push({
      section: "recent",
      matches: recentRows.map((i) => toMatch(q, entries[i], "recent", base[i] + entries[i].rank)),
      total: recentRows.length,
    });
  }
  for (const sec of order) {
    const top = tops.get(sec) ?? [];
    if (top.length === 0) continue;
    out.push({
      section: sec,
      matches: top.map((i) => toMatch(q, entries[i], sec, base[i] + entries[i].rank)),
      total: totals.get(sec) ?? top.length,
    });
  }
  return out;
}

/** The contract's search: every match (uncapped), grouped by section in display order. */
export function search(
  query: string,
  cmds: readonly Command[],
  ctx: CommandContext,
  recent: readonly string[],
): Match[] {
  const all: Partial<Record<Section, number>> = {};
  for (const s of ["live", "goto", "create", "edit", "items", "settings"] as const) all[s] = Infinity;
  return searchSections(query, prepare(cmds, ctx), ctx, recent, all).flatMap((r) => r.matches);
}

// ---- Follow-up pick lists -----------------------------------------------------------------------

export interface PickOption {
  id: string;
  title: string;
  keywords?: readonly string[];
  detail?: string;
}

export interface PickMatch {
  option: PickOption;
  ranges: [number, number][];
}

/** Options for a follow-up "pick" step, best first (stable); all of them for an empty query. */
export function searchOptions(query: string, options: readonly PickOption[], cap = 200): PickMatch[] {
  const q = foldQuery(query);
  if (q === "") return options.slice(0, cap).map((option) => ({ option, ranges: [] }));
  const scored: { i: number; s: number }[] = [];
  for (let i = 0; i < options.length; i++) {
    const o = options[i];
    let s = scoreFolded(q, fold(o.title), boundaries(o.title));
    if (o.keywords && o.keywords.length > 0) {
      const kw = o.keywords.join(" ");
      const k = scoreFolded(q, fold(kw), boundaries(kw));
      if (k >= 0) s = Math.max(s, k * 0.5);
    }
    if (s >= 0) scored.push({ i, s });
  }
  scored.sort((a, b) => b.s - a.s || a.i - b.i);
  return scored.slice(0, cap).map(({ i }) => {
    const option = options[i];
    const ranges = scoreFolded(q, fold(option.title), boundaries(option.title)) >= 0 ? lastRanges(q) : [];
    return { option, ranges };
  });
}
