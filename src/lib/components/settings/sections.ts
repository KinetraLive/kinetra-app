// The Preferences window's sections, in order: their ids (`/preferences?section=<id>`), titles and
// lazy bodies. A new section (License) slots in by adding one entry here.
//
// Old `/settings#<id>` anchors still work: `PREFERENCE_ALIASES` maps each to the section that holds
// it, and the section keeps the old id as an anchor on that block, so `backups` opens Advanced
// scrolled to Backups. `updates` opens About and starts a check (Help › Check for updates).

import type { Component } from "svelte";
import type { PreferenceSectionId } from "$lib/types";
import { REMOTE_BUILD } from "$remote";

export interface PreferenceSection {
  id: PreferenceSectionId;
  title: string;
  /** The section body, a lazy chunk of its own. */
  load: () => Promise<{ default: Component }>;
  /** The whole section is project settings: one line instead of it while no project is open. */
  needsProject?: boolean;
}

export const PREFERENCE_SECTIONS: readonly PreferenceSection[] = [
  { id: "general", title: "General", load: () => import("./GeneralSection.svelte") },
  { id: "appearance", title: "Appearance", load: () => import("./panes/AppearancePane.svelte") },
  { id: "outputs", title: "Outputs", load: () => import("./panes/OutputsPane.svelte") },
  { id: "network", title: "Network & API", load: () => import("./panes/NetworkPane.svelte") },
  { id: "license", title: "License", load: () => import("./panes/LicensePane.svelte") },
  { id: "cloud", title: "Cloud backup", load: () => import("./panes/CloudPane.svelte") },
  { id: "shortcuts", title: "Shortcuts", load: () => import("./ShortcutsSection.svelte") },
  { id: "advanced", title: "Advanced", load: () => import("./panes/AdvancedPane.svelte") },
  { id: "about", title: "About", load: () => import("./AboutSection.svelte") },
];

export const DEFAULT_SECTION: PreferenceSectionId = "general";

/** Old anchors (and sub-blocks) → the section that holds them. */
export const PREFERENCE_ALIASES: Readonly<Record<string, PreferenceSectionId>> = {
  general: "general",
  style: "appearance",
  displays: "outputs",
  server: "network",
  midi: "network",
  osc: "network",
  remote: "network",
  "control-api": "network",
  shortcuts: "shortcuts",
  location: "advanced",
  backups: "advanced",
  "settings-file": "advanced",
  reset: "advanced",
  about: "about",
  updates: "about",
  account: "license",
};

/** Asks About to check for updates as it opens. */
export const CHECK_UPDATES = "updates";

export interface SectionTarget {
  section: PreferenceSectionId;
  /** The block to scroll to and focus inside the section (an old anchor id); null = the top. */
  anchor: string | null;
  /** Start an update check (About). */
  check: boolean;
}

const isSection = (id: string): id is PreferenceSectionId => PREFERENCE_SECTIONS.some((s) => s.id === id);

/**
 * Where `raw` (a section id, an old anchor with or without "#", or nothing) lands. Unknown ids
 * open General.
 */
export function resolveSection(raw: string | null | undefined): SectionTarget {
  let id = (raw ?? "").trim().replace(/^#/, "");
  try {
    id = decodeURIComponent(id);
  } catch {
    // keep it as it is
  }
  if (id === CHECK_UPDATES) return { section: "about", anchor: null, check: true };
  if (isSection(id)) return { section: id, anchor: null, check: false };
  const alias = PREFERENCE_ALIASES[id];
  if (alias) return { section: alias, anchor: id, check: false };
  return { section: DEFAULT_SECTION, anchor: null, check: false };
}

/**
 * The palette's "Preferences: …" items: every section plus the blocks people look for by name.
 * `id` is what `openPreferences` gets.
 */
export const PREFERENCE_ENTRIES: readonly { id: string; title: string }[] = [
  { id: "general", title: "General" },
  { id: "appearance", title: "Appearance" },
  { id: "style", title: "Style & brand" },
  { id: "outputs", title: "Outputs" },
  { id: "network", title: "Network & API" },
  { id: "server", title: "Server" },
  ...(REMOTE_BUILD ? [{ id: "remote", title: "Remote control" }] : []),
  { id: "midi", title: "MIDI & OSC" },
  { id: "shortcuts", title: "Keyboard shortcuts" },
  { id: "advanced", title: "Advanced" },
  { id: "location", title: "Project location" },
  { id: "backups", title: "Backups" },
  { id: "cloud", title: "Cloud backup" },
  { id: "settings-file", title: "Settings file" },
  { id: "reset", title: "Reset" },
  { id: "about", title: "About" },
];
