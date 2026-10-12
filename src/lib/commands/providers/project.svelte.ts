// The palette's project items (part of its lazy chunk, never in the shell's initial bundle).
//
//   const stop = watchItems();   // the palette, while it is open
//
// The first call registers the providers (they stay registered; they are asked only while the
// palette is open). Each call loads the data they read: screens, templates, sources, flows and the
// schedules' views (one list call each, answered from the backend's in-memory caches), kept current
// by events until the returned stop. They load after the panel has painted (cached items show at
// once) and replace a list only when it changed, so an open never searches twice for nothing.
// `items()` reads only that in-memory data, synchronously.
// Only api modules and tiny helpers are imported (not the stores and helpers Control's route
// chunk holds), so nothing is split out of the routes' chunks for this lazy one.
//
// Items (ids are stable; they never contain display names):
//   screen.edit:<screenId>              Go to {screen}                    @  Project items
//   screen.control:<screenId>           Open {screen} in Control          @  Project items
//   template.edit:<templateId>          Edit {template}                   #  Project items
//   template.update:<templateId>        Update {template} to the latest look   #  Edit (an outdated
//                                       preset template; the template editor registers its own)
//   source.open:<sourceId>              Open {source}                        Project items
//   flow.open:<flowId>                  Open {flow}                          Project items
//   cue.go:<flowId>                     GO {cue}           live, when Control hasn't registered it
//   source.next:<sourceId> / .prev:     Next / Previous slide in {source}   live, auto-advance on
//   schedule.start:<sourceId>/<itemId>  Start {item} now                    live
//   schedule.next:<sourceId>/<itemId>   Set {item} as next                  live
//   settings.section:<id>               Preferences: {section} / Keyboard shortcuts   /  Settings
// Live items rank +10 000 in Control. Every live item calls the same api action Control uses
// (runFlow, stepSource, scheduleAction); none has logic of its own.

import Database from "lucide-svelte/icons/database";
import LayoutTemplate from "lucide-svelte/icons/layout-template";
import Monitor from "lucide-svelte/icons/monitor";
import Play from "lucide-svelte/icons/play";
import RadioTower from "lucide-svelte/icons/radio-tower";
import RefreshCw from "lucide-svelte/icons/refresh-cw";
import Workflow from "lucide-svelte/icons/workflow";
import { goto } from "$app/navigation";
import { on } from "$lib/api";
import { listFlows, runFlow } from "$lib/api/flows";
import { listScreens } from "$lib/api/live";
import { scheduleAction, scheduleView } from "$lib/api/recipes";
import { listSources, stepSource } from "$lib/api/sources";
import { listTemplates } from "$lib/api/templates";
import { cueText } from "$lib/components/flows/cue";
import { showPreferences } from "$lib/components/settings/open";
import { PREFERENCE_ENTRIES } from "$lib/components/settings/sections";
import { t } from "$lib/i18n";
import { isAdvanced } from "$lib/stores/mode.svelte";
import { project, uiState } from "$lib/stores/project.svelte";
import {
  TEST_SCREEN_ID,
  type FlowInfo,
  type LiveScreenView,
  type ScheduleView,
  type SourceSummary,
  type TemplateSummary,
} from "$lib/types";
import { commands, registerProvider, type Command, type CommandContext, type CommandProvider } from "../registry.svelte";

class Items {
  screens = $state.raw<readonly LiveScreenView[]>([]);
  templates = $state.raw<readonly TemplateSummary[]>([]);
  sources = $state.raw<readonly SourceSummary[]>([]);
  flows = $state.raw<readonly FlowInfo[]>([]);
  /** Schedule source id → its view. */
  schedules = $state.raw<ReadonlyMap<string, ScheduleView>>(new Map());
}

export const items = new Items();

/** The project the cached lists belong to (another project: never list the previous one's). */
let loadedFor: string | null = null;
let registered = false;

const isUserScreen = (id: string) => id !== TEST_SCREEN_ID && !id.startsWith("__");
const LIVE_RANK = 10_000;
const liveRank = (ctx: CommandContext) => (ctx.route === "control" ? LIVE_RANK : 0);

/** The cached list when nothing changed: an unchanged reply never makes the palette search its
 * entries again. */
function same<T>(before: readonly T[], after: readonly T[]): readonly T[] {
  return before.length === after.length && JSON.stringify(before) === JSON.stringify(after) ? before : after;
}

async function loadScreens() {
  try {
    items.screens = same(items.screens, (await listScreens()).filter((v) => isUserScreen(v.screen.id)));
  } catch {
    // Keep what is cached.
  }
}

async function loadTemplates() {
  try {
    items.templates = same(items.templates, await listTemplates());
  } catch {
    // No backend or a transient failure: keep what is cached.
  }
}

async function loadSources() {
  try {
    items.sources = same(items.sources, await listSources());
  } catch {
    return;
  }
  const ids = items.sources.filter((s) => s.kind === "schedule").map((s) => s.id);
  const views = new Map<string, ScheduleView>();
  await Promise.all(
    ids.map((id) =>
      scheduleView(id).then(
        (v) => void views.set(id, v),
        () => {},
      ),
    ),
  );
  const before = items.schedules;
  if (views.size !== before.size || [...views].some(([id, v]) => JSON.stringify(v) !== JSON.stringify(before.get(id)))) {
    items.schedules = views;
  }
}

async function loadFlows() {
  try {
    items.flows = same(items.flows, await listFlows());
  } catch {
    // Keep what is cached.
  }
}

/** Registers the providers (once) and loads their data; returns the function that stops following
 * changes (the palette closed). */
export function watchItems(): () => void {
  if (!registered) {
    registered = true;
    for (const p of PROVIDERS) registerProvider(p);
  }
  const path = project.current?.path ?? null;
  if (path !== loadedFor) {
    loadedFor = path;
    items.screens = [];
    items.templates = [];
    items.sources = [];
    items.flows = [];
    items.schedules = new Map();
  }
  // The lists load after the panel has painted (the cached ones show at once).
  let alive = true;
  const load = () => {
    if (!alive) return;
    void loadScreens();
    void loadTemplates();
    void loadSources();
    void loadFlows();
  };
  const frame = typeof requestAnimationFrame === "function" ? requestAnimationFrame(() => setTimeout(load, 0)) : setTimeout(load, 0);
  const offFiles = on("project-files-changed", (c) => {
    if (c.kind === "screen") void loadScreens();
    else if (c.kind === "template") void loadTemplates();
    else if (c.kind === "source") void loadSources();
    else if (c.kind === "flow") void loadFlows();
  });
  const offSource = on("source-changed", (s) => {
    items.sources = items.sources.map((x) => (x.id === s.id ? s : x));
  });
  const offSchedule = on("schedule-changed", (v) => {
    if (items.schedules.has(v.sourceId)) items.schedules = new Map(items.schedules).set(v.sourceId, v);
  });
  return () => {
    alive = false;
    if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(frame as number);
    offSchedule();
    offSource();
    offFiles();
  };
}

/** Keeps a schedule's view current after an action from the palette (its reply is the new view). */
function putSchedule(v: ScheduleView) {
  items.schedules = new Map(items.schedules).set(v.sourceId, v);
}

// ---- Providers -------------------------------------------------------------------------------------

const screens: CommandProvider = {
  id: "items.screens",
  items(ctx) {
    if (!ctx.project) return [];
    const out: Command[] = [];
    for (const v of items.screens) {
      const id = v.screen.id;
      const params = () => ({ screen: v.screen.name });
      out.push(
        {
          id: `screen.edit:${id}`,
          title: "Go to {screen}",
          params,
          section: "items",
          scope: "screen",
          icon: Monitor,
          keywords: [t("Screen"), t("Layout")],
          run: () => goto(`/screens/${id}`),
        },
        {
          id: `screen.control:${id}`,
          title: "Open {screen} in Control",
          params,
          section: "items",
          scope: "screen",
          icon: RadioTower,
          run: () => openInControl(id),
        },
      );
    }
    return out;
  },
};

/** Control with this screen selected (the same ui-state write as Control's screen list). */
async function openInControl(screenId: string) {
  const v = uiState.value;
  if (v && v.control.selectedScreenId !== screenId) {
    await uiState.update({ ...v, control: { ...v.control, selectedScreenId: screenId } });
  }
  await goto("/control");
}

const templates: CommandProvider = {
  id: "items.templates",
  items(ctx) {
    if (!ctx.project) return [];
    const out = items.templates.map(
      (tpl): Command => ({
        id: `template.edit:${tpl.id}`,
        title: "Edit {template}",
        params: () => ({ template: tpl.name }),
        section: "items",
        scope: "template",
        icon: LayoutTemplate,
        keywords: tpl.description ? [tpl.description] : undefined,
        run: () => goto(`/templates/${tpl.id}`),
      }),
    );
    // Outdated preset templates. Not in the template editor: it registers its own (it saves first).
    if (ctx.route !== "templateEditor") {
      for (const tpl of items.templates) {
        if (!tpl.presetUpdate) continue;
        out.push({
          id: `template.update:${tpl.id}`,
          title: "Update {template} to the latest look",
          params: () => ({ template: tpl.name }),
          section: "edit",
          scope: "template",
          icon: RefreshCw,
          keywords: ["newer look"],
          run: async () => {
            const { updateLook } = await import("$lib/components/templates/update");
            await updateLook(tpl, { after: () => void loadTemplates() });
          },
        });
      }
    }
    return out;
  },
};

const sources: CommandProvider = {
  id: "items.sources",
  items(ctx) {
    if (!ctx.project) return [];
    const out: Command[] = [];
    for (const s of items.sources) {
      const params = () => ({ source: s.name });
      out.push({
        id: `source.open:${s.id}`,
        title: "Open {source}",
        params,
        section: "items",
        icon: Database,
        run: () => goto(`/sources?source=${encodeURIComponent(s.id)}`),
      });
      // Slideshows: sources with auto-advance; the same step Control's Prev / Next sends.
      if (s.auto && s.rowCount > 1) {
        out.push(
          {
            id: `source.next:${s.id}`,
            title: "Next slide in {source}",
            params,
            section: "live",
            live: true,
            rank: liveRank,
            run: async () => putSource(await stepSource(s.id, { to: "by", delta: 1 })),
          },
          {
            id: `source.prev:${s.id}`,
            title: "Previous slide in {source}",
            params,
            section: "live",
            live: true,
            rank: liveRank,
            run: async () => putSource(await stepSource(s.id, { to: "by", delta: -1 })),
          },
        );
      }
    }
    return out;
  },
};

function putSource(s: SourceSummary) {
  items.sources = items.sources.map((x) => (x.id === s.id ? s : x));
}

const flows: CommandProvider = {
  id: "items.flows",
  items(ctx) {
    if (!ctx.project) return [];
    // Control registers its cues as commands (with their keys); never list a cue twice.
    const registered = new Set(commands().map((c) => c.id));
    const out: Command[] = [];
    // Simple mode hides the flow editor; cues still run.
    const editable = isAdvanced();
    for (const f of items.flows) {
      if (editable) out.push({
        id: `flow.open:${f.id}`,
        title: "Open {flow}",
        params: () => ({ flow: f.name }),
        section: "items",
        icon: Workflow,
        run: () => goto(`/flows/${f.id}`),
      });
      const id = `cue.go:${f.id}`;
      const label = cueText(f);
      if (label === null || registered.has(id)) continue;
      out.push({
        id,
        title: "GO {cue}",
        params: () => ({ cue: label }),
        section: "live",
        icon: Play,
        live: true,
        rank: liveRank,
        disabled: () => (f.armed ? null : t("Disarmed")),
        run: () => runFlow(f.id),
      });
    }
    return out;
  },
};

const schedules: CommandProvider = {
  id: "items.schedules",
  items(ctx) {
    if (!ctx.project || items.schedules.size === 0) return [];
    const out: Command[] = [];
    for (const s of items.sources) {
      const view = items.schedules.get(s.id);
      if (!view) continue;
      for (const it of view.items) {
        if (it.status === "cancelled" || it.status === "now") continue;
        const params = () => ({ item: it.title || t("Untitled") });
        const keywords = [s.name, ...(it.location ? [it.location] : [])];
        const key = `${s.id}/${it.id}`;
        out.push({
          id: `schedule.start:${key}`,
          title: "Start {item} now",
          params,
          keywords,
          section: "live",
          live: true,
          rank: liveRank,
          run: async () => putSchedule(await scheduleAction(s.id, { do: "startNow", itemId: it.id })),
        });
        if (it.status !== "previous" && it.id !== view.nextId) {
          out.push({
            id: `schedule.next:${key}`,
            title: "Set {item} as next",
            params,
            keywords,
            section: "live",
            live: true,
            rank: liveRank,
            run: async () => putSchedule(await scheduleAction(s.id, { do: "setNext", itemId: it.id, start: null })),
          });
        }
      }
    }
    return out;
  },
};

const preferences: CommandProvider = {
  id: "items.preferences",
  items(ctx) {
    if (!ctx.project) return [];
    return PREFERENCE_ENTRIES.map(
      (s): Command => ({
        id: `settings.section:${s.id}`,
        title: s.id === "shortcuts" ? s.title : "Preferences: {section}",
        params: () => ({ section: t(s.title) }),
        section: "settings",
        scope: "setting",
        keywords: [t("Preferences"), t("Settings")],
        run: () => showPreferences(s.id),
      }),
    );
  },
};

export const PROVIDERS: readonly CommandProvider[] = [screens, templates, sources, flows, schedules, preferences];
