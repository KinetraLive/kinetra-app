// Shell navigation: the sidebar sections, keyboard shortcuts and the sidebar pin toggle.
// Kept out of the components so the shortcut logic is plain, testable TypeScript.

import House from "lucide-svelte/icons/house";
import LayoutTemplate from "lucide-svelte/icons/layout-template";
import Database from "lucide-svelte/icons/database";
import Monitor from "lucide-svelte/icons/monitor";
import Workflow from "lucide-svelte/icons/workflow";
import RadioTower from "lucide-svelte/icons/radio-tower";
import ChartBar from "lucide-svelte/icons/chart-bar";
import { toAppError } from "$lib/api";
import { isTyping } from "$lib/commands/keys";
import type { Command } from "$lib/commands/registry.svelte";
import { showPreferences } from "$lib/components/settings/open";
import { t } from "$lib/i18n";
import { isAdvanced } from "$lib/stores/mode.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";

export type NavIcon = typeof House;

export interface NavItem {
  /** The command id ("nav.templates"): stable when routes are renamed. */
  id: string;
  href: string;
  label: string;
  icon: NavIcon;
  /** Default keyboard shortcut, e.g. "Ctrl+2" (the binding table may override it). */
  shortcut: string;
  /** Hidden in Simple mode (sidebar, palette, its key). */
  advanced?: boolean;
}

/** Main sections, in Ctrl+1…Ctrl+7 order. Preferences is rendered separately at the bottom. */
export const NAV_ITEMS: readonly NavItem[] = [
  { id: "nav.dashboard", href: "/dashboard", label: "Dashboard", icon: House, shortcut: "Ctrl+1" },
  { id: "nav.templates", href: "/templates", label: "Templates", icon: LayoutTemplate, shortcut: "Ctrl+2" },
  { id: "nav.sources", href: "/sources", label: "Sources", icon: Database, shortcut: "Ctrl+3" },
  { id: "nav.screens", href: "/screens", label: "Screens", icon: Monitor, shortcut: "Ctrl+4" },
  { id: "nav.flows", href: "/flows", label: "Flows", icon: Workflow, shortcut: "Ctrl+5", advanced: true },
  { id: "nav.polls", href: "/polls", label: "Polls", icon: ChartBar, shortcut: "Ctrl+6" },
  { id: "nav.control", href: "/control", label: "Control", icon: RadioTower, shortcut: "Ctrl+7" },
];

/** The old Preferences page: now a redirect that opens the Preferences window (kept for links). */
export const SETTINGS_HREF = "/settings";

/** True when `pathname` is `href` or below it (e.g. /templates/abc is inside /templates). */
export function isActive(pathname: string, href: string): boolean {
  return pathname === href || pathname.startsWith(href + "/");
}

/**
 * The section root for `pathname`, used when switching projects: ids under /templates/<id> belong
 * to the old project, so we land on /templates. Unknown routes fall back to /dashboard.
 */
export function sectionRoot(pathname: string): string {
  const all = NAV_ITEMS.map((i) => i.href);
  return all.find((href) => isActive(pathname, href)) ?? "/dashboard";
}

export { isTyping, showPreferences };

export const SETTINGS_COMMAND = "nav.settings";


/**
 * Pins or unpins the sidebar (`sidebarPinned` in the project's ui-state). Pinned, it is the full
 * labelled sidebar in the layout; unpinned, an icon rail that expands as an overlay on hover.
 * Optimistic: the layout changes at once; on failure SyncedStore rolls back and we toast.
 */
export async function toggleSidebarPin(): Promise<void> {
  const v = uiState.value;
  if (!v) return;
  const sidebarPinned = !v.sidebarPinned;
  try {
    await uiState.update({ ...v, sidebarPinned });
  } catch (e) {
    toasts.push({
      kind: "error",
      message: t(sidebarPinned ? "Couldn't pin the sidebar: {reason}" : "Couldn't unpin the sidebar: {reason}", {
        reason: toAppError(e).message,
      }),
    });
  }
}

/**
 * The shell's navigation as commands: "Go to Templates" (Ctrl+2, not while typing),
 * "Preferences…" (Ctrl+, works while typing; opens the Preferences window), plus "New flow".
 */
export function navCommands(go: (href: string) => void): Command[] {
  return [
    ...NAV_ITEMS.map(
      (i): Command => ({
        id: i.id,
        title: "Go to {page}",
        params: () => ({ page: t(i.label) }),
        section: "goto",
        icon: i.icon,
        defaultKeys: [i.shortcut],
        notWhileTyping: true,
        when: i.advanced ? isAdvanced : undefined,
        run: () => go(i.href),
      }),
    ),
    {
      id: SETTINGS_COMMAND,
      title: "Preferences…",
      section: "goto",
      keywords: ["Settings", "Options"],
      defaultKeys: ["Ctrl+,"],
      whileTyping: true,
      run: () => showPreferences(),
    },
    // The Flows page opens its New flow dialog for ?new.
    { id: "flows.new", title: "New flow", section: "create", when: isAdvanced, run: () => go("/flows?new") },
  ];
}
