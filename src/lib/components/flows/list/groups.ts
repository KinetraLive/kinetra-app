// The /flows list's model: flows grouped by origin (the project's own first, then one group per
// recipe instance, by its name). Plain TS, unit-tested. Row summaries are in ./summary.ts.

import type { FlowInfo, FlowIssue } from "$lib/types";

export interface FlowGroup {
  /** "" for the project's own flows, else the recipe instance id. */
  id: string;
  name: string;
  recipe: boolean;
  flows: FlowInfo[];
}

const byName = (a: { name: string }, b: { name: string }) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });

/** Own flows first, then one group per recipe (by recipe name); rows by name within a group. */
export function groupFlows(list: readonly FlowInfo[], recipeNames: ReadonlyMap<string, string>): FlowGroup[] {
  const own: FlowInfo[] = [];
  const byOrigin = new Map<string, FlowInfo[]>();
  for (const f of list) {
    if (!f.origin) own.push(f);
    else byOrigin.set(f.origin, [...(byOrigin.get(f.origin) ?? []), f]);
  }
  const out: FlowGroup[] = own.length > 0 ? [{ id: "", name: "Project", recipe: false, flows: own.sort(byName) }] : [];
  const recipes = [...byOrigin].map(([id, flows]) => ({
    id,
    name: recipeNames.get(id) ?? "Removed recipe",
    recipe: true,
    flows: flows.sort(byName),
  }));
  return [...out, ...recipes.sort(byName)];
}

/** The server's "Not valid, so it doesn't run: …" warning of a file that failed to load. */
export const invalidOf = (f: FlowInfo): FlowIssue | undefined =>
  f.warnings.find((w) => w.node === null && w.message.startsWith("Not valid"));
