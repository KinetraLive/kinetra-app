// Opening the Preferences window from the main window (links, toasts, commands). Tiny: Control,
// the dashboard and the shell import it.

import { openPreferences, toAppError } from "$lib/api";
import { t } from "$lib/i18n";
import { toasts } from "$lib/stores/toasts.svelte";

/** Opens (or focuses) the Preferences window at `section` (a section id or an old anchor such as
 * "backups"); a failure is a toast. */
export function showPreferences(section?: string): void {
  openPreferences(section).catch((e: unknown) => {
    toasts.push({ kind: "error", message: t("Couldn't open Preferences: {reason}", { reason: toAppError(e).message }) });
  });
}

/**
 * A click handler for a link to Preferences (`<a href="/settings#server" onclick={…}>`): opens
 * the window instead of navigating. The href stays for "copy link" and as a fallback (the
 * /settings route opens the window too).
 */
export function preferencesLink(section?: string, after?: () => void) {
  return (e: MouseEvent) => {
    e.preventDefault();
    after?.();
    showPreferences(section);
  };
}

/**
 * The old /settings route: opens the window at the section the hash names ("#backups" →
 * Advanced › Backups), then leaves (back, or the dashboard when there is no history).
 */
export function redirectSettings(hash: string, nav: { canGoBack: boolean; back: () => void; replace: (href: string) => void }): void {
  showPreferences(hash.replace(/^#/, "") || undefined);
  if (nav.canGoBack) nav.back();
  else nav.replace("/dashboard");
}
