// Flow templates (lazy chunk): what the "Point it at" form picks from, loaded when the gallery
// opens: the screens (layers, field definitions), sources and flows, and which preset each layer's
// template came from (the screen documents' template ids → the templates' preset), for the slots'
// smart defaults. A part that fails to load stays empty.

import { listFlows } from "$lib/api/flows";
import { listScreens } from "$lib/api/live";
import { getScreen } from "$lib/api/screens";
import { listSources } from "$lib/api/sources";
import { listTemplates } from "$lib/api/templates";
import { TEST_SCREEN_ID } from "$lib/types";
import type { Targets } from "./slots";

const ok = <T>(p: Promise<T>, empty: T): Promise<T> => p.then((v) => v ?? empty, () => empty);

export async function loadTargets(): Promise<Targets> {
  const [all, sources, flows, templates] = await Promise.all([
    ok(listScreens(), []),
    ok(listSources(), []),
    ok(listFlows(), []),
    ok(listTemplates(), []),
  ]);
  const screens = all.filter((v) => v.screen.id !== TEST_SCREEN_ID && !v.screen.id.startsWith("__"));
  const presetOf = new Map(templates.filter((t) => t.preset).map((t) => [t.id, t.preset?.id ?? ""]));
  const docs = await Promise.all(screens.map((v) => ok(getScreen(v.screen.id).then((s) => s.doc), null)));
  const presets = new Map<string, string>();
  for (const doc of docs) {
    for (const l of doc?.layers ?? []) {
      const p = l.kind === "template" ? presetOf.get(l.templateId) : undefined;
      if (p) presets.set(`${doc?.id}/${l.id}`, p);
    }
  }
  return { screens, sources, flows, presets };
}
