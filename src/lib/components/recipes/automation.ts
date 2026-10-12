// Control's AUTOMATION section for the selected screen, as plain data (unit-tested; the lazy
// Automation strip renders it). Operator terms: an installed recipe is "an automation", its flows
// are its rules.
//
//   - A recipe belongs to a screen when it was installed there (`screenId`) or any of its flows
//     aims at the screen (a node config with that `screenId`).
//   - A recipe is broken when its source or one of its layers is gone (its screen gone counts as
//     every layer gone). Broken recipes are not listed: the attention banner offers Fix / Delete.
//     A broken recipe whose screen is gone shows on every screen (nowhere else would show it).
//   - Every rule shows exactly once: under its recipe (its flows: `origin` or `flowIds`), else
//     under "Other rules" when it aims at the screen. Cues never show here (they are in CUES).
//   - Warnings are aggregated per row: each message once (verbatim from the backend), with the
//     names of the rules it affects.

import type { Flow, FlowInfo, LiveScreenView, RecipeInstance, RecipePanel, SourceSummary } from "$lib/types";

export interface Warning {
  message: string;
  /** Names of the rules it affects, in list order, each once. */
  rules: string[];
}

export interface Qualifier {
  text: string;
  /** A time: shown in mono. */
  mono: boolean;
}

export interface RecipeRow {
  recipe: RecipeInstance;
  /** Set when another listed automation has the same name. */
  qualifier: Qualifier | null;
  rules: FlowInfo[];
  armed: number;
  warnings: Warning[];
}

export interface Broken {
  recipe: RecipeInstance;
  qualifier: Qualifier | null;
  sourceMissing: boolean;
  /** Its layers that are gone (all of them when its screen is gone). */
  missingLayers: string[];
  screenMissing: boolean;
}

export interface OtherRules {
  rules: FlowInfo[];
  armed: number;
  warnings: Warning[];
}

export interface AutomationView {
  broken: Broken[];
  recipes: RecipeRow[];
  other: OtherRules | null;
}

export interface AutomationInput {
  screenId: string;
  recipes: readonly RecipeInstance[];
  flows: readonly FlowInfo[];
  sources: ReadonlyMap<string, SourceSummary>;
  screens: ReadonlyMap<string, LiveScreenView>;
  /** Sources and screens are loaded: before that nothing counts as missing. */
  ready: boolean;
}

/** The flows a recipe owns (created or added to it), in the flows' order. */
export function flowsOf(r: RecipeInstance, flows: readonly FlowInfo[]): FlowInfo[] {
  return flows.filter((f) => f.origin === r.id || r.flowIds.includes(f.id));
}

/** What is missing of a recipe (null when nothing is). */
export function brokenOf(
  r: RecipeInstance,
  sources: ReadonlyMap<string, SourceSummary>,
  screens: ReadonlyMap<string, LiveScreenView>,
): Omit<Broken, "qualifier"> | null {
  const sourceMissing = r.sourceId !== null && !sources.has(r.sourceId);
  const screen = screens.get(r.screenId);
  const missingLayers = screen ? r.layerIds.filter((id) => !screen.layers.some((l) => l.id === id)) : [...r.layerIds];
  if (!sourceMissing && missingLayers.length === 0) return null;
  return { recipe: r, sourceMissing, missingLayers, screenMissing: !screen };
}

/** Each warning message once, with the rules it affects. */
export function aggregate(rules: readonly FlowInfo[]): Warning[] {
  const out = new Map<string, string[]>();
  for (const f of rules) {
    for (const w of f.warnings) {
      const names = out.get(w.message) ?? [];
      if (!names.includes(f.name)) names.push(f.name);
      out.set(w.message, names);
    }
  }
  return [...out].map(([message, names]) => ({ message, rules: names }));
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** "2 rules · 1 armed · 1 warning" (armed and warnings only when there are any). */
export function summaryText(rules: number, armed: number, warnings: number): string {
  const parts = [plural(rules, "rule", "rules")];
  if (armed > 0) parts.push(`${armed} armed`);
  if (warnings > 0) parts.push(plural(warnings, "warning", "warnings"));
  return parts.join(" · ");
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "4 Oct 15:12" (local). */
export function addedAt(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/**
 * Qualifiers for automations that share a name: the source's name when it tells them apart, else
 * the screen's name, else when it was added.
 */
export function qualifiers(
  list: readonly RecipeInstance[],
  sources: ReadonlyMap<string, SourceSummary>,
  screens: ReadonlyMap<string, LiveScreenView>,
): Map<string, Qualifier> {
  const out = new Map<string, Qualifier>();
  const byName = new Map<string, RecipeInstance[]>();
  for (const r of list) {
    const k = r.name.trim().toLowerCase();
    byName.set(k, [...(byName.get(k) ?? []), r]);
  }
  for (const same of byName.values()) {
    if (same.length < 2) continue;
    const sourceOf = (r: RecipeInstance) => (r.sourceId ? sources.get(r.sourceId)?.name : undefined);
    const screenOf = (r: RecipeInstance) => screens.get(r.screenId)?.screen.name;
    const unique = (f: (r: RecipeInstance) => string | undefined) => {
      const vals = same.map(f);
      return vals.every((v) => v !== undefined) && new Set(vals).size === vals.length;
    };
    for (const r of same) {
      if (unique(sourceOf)) out.set(r.id, { text: `${sourceOf(r)} (source)`, mono: false });
      else if (unique(screenOf)) out.set(r.id, { text: screenOf(r) ?? "", mono: false });
      else out.set(r.id, { text: `added ${addedAt(r.createdAt)}`, mono: true });
    }
  }
  return out;
}

// The same tests as flows.svelte's aimsAt / cueOf: not imported from there, so that module stays
// inside the Control route's chunk instead of being split out for this lazy chunk.

/** A flow aims at a screen when any node (trigger, data or action) names it. */
export function aimsAt(flow: Pick<Flow, "nodes">, screenId: string): boolean {
  return flow.nodes.some((n) => "screenId" in n.config && n.config.screenId === screenId);
}

const isRule = (f: FlowInfo) => !f.nodes.some((n) => n.type === "trigger.cue");
const armedOf = (rules: readonly FlowInfo[]) => rules.filter((f) => f.armed).length;

/** The selected screen's AUTOMATION: broken recipes, recipe rows, other rules. */
export function automationOf(i: AutomationInput): AutomationView {
  const owned = new Set<string>();
  for (const r of i.recipes) for (const f of flowsOf(r, i.flows)) owned.add(f.id);

  const broken: Omit<Broken, "qualifier">[] = [];
  const rows: { recipe: RecipeInstance; rules: FlowInfo[] }[] = [];
  for (const r of i.recipes) {
    const all = flowsOf(r, i.flows);
    const bad = i.ready ? brokenOf(r, i.sources, i.screens) : null;
    const here = r.screenId === i.screenId || all.some((f) => aimsAt(f, i.screenId));
    if (bad) {
      if (here || bad.screenMissing) broken.push(bad);
    } else if (here) {
      rows.push({ recipe: r, rules: all.filter(isRule) });
    }
  }

  const q = qualifiers([...broken.map((b) => b.recipe), ...rows.map((r) => r.recipe)], i.sources, i.screens);
  const otherRules = i.flows.filter((f) => !owned.has(f.id) && isRule(f) && aimsAt(f, i.screenId));
  return {
    broken: broken.map((b) => ({ ...b, qualifier: q.get(b.recipe.id) ?? null })),
    recipes: rows.map(({ recipe, rules }) => ({
      recipe,
      qualifier: q.get(recipe.id) ?? null,
      rules,
      armed: armedOf(rules),
      warnings: aggregate(rules),
    })),
    other: otherRules.length > 0 ? { rules: otherRules, armed: armedOf(otherRules), warnings: aggregate(otherRules) } : null,
  };
}

/** Source kind a recipe panel runs on (a Fix picks a new one of this kind). */
export function sourceKindOf(panel: RecipePanel): "text" | "schedule" {
  return panel === "slideshow" ? "text" : "schedule";
}
