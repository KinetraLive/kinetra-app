// Loads what MIDI targets are named and picked from: screens (layers, number fields), cues,
// sources, and each layer's template variables (for range variables on faders). Lazy pieces only.

import { listFlows } from "$lib/api/flows";
import { listScreens } from "$lib/api/live";
import { getScreen } from "$lib/api/screens";
import { listSources } from "$lib/api/sources";
import { getTemplate } from "$lib/api/templates";
import type { VariableDef } from "$lib/types";
import type { Lookups } from "./model";

const settle = <T>(p: Promise<T>, fallback: T) => p.catch(() => fallback);

/** Names first (fast); `withVars` also reads each screen document and template. */
export async function loadLookups(withVars = false): Promise<Lookups> {
  const [screens, flows, sources] = await Promise.all([
    settle(listScreens(), []),
    settle(listFlows(), []),
    settle(listSources(), []),
  ]);
  const k: Lookups = { screens, flows, sources };
  if (!withVars) return k;
  const docs = await Promise.all(screens.filter((v) => !v.screen.id.startsWith("__")).map((v) => settle(getScreen(v.screen.id), null)));
  const templateIds = new Set<string>();
  for (const d of docs) for (const l of d?.doc.layers ?? []) if (l.kind !== "media") templateIds.add(l.templateId);
  const templates = new Map<string, readonly VariableDef[]>();
  await Promise.all(
    [...templateIds].map(async (id) => {
      const t = await settle(getTemplate(id), null);
      if (t) templates.set(id, t.variables);
    }),
  );
  const vars = new Map<string, readonly VariableDef[]>();
  for (const d of docs) {
    if (!d) continue;
    for (const l of d.doc.layers) vars.set(`${d.doc.id}/${l.id}`, l.kind !== "media" ? (templates.get(l.templateId) ?? []) : []);
  }
  return { ...k, vars };
}
