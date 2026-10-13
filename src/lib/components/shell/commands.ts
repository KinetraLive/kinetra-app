// The shell's commands (palette, navigation, theme, sidebar pin, Automation off / on, Remote
// control on / off, Apply
// style…) as a factory: AppShell registers them with real actions, and Preferences > Keyboard
// shortcuts (another window, without the shell) lists inert copies from the same factory, so
// titles, ids and default keys can't drift.

import type { Command } from "$lib/commands/registry.svelte";
import { applyStyleCommand } from "$lib/components/styles/command.svelte";
import { theme } from "$lib/stores/theme.svelte";
import { navCommands, toggleSidebarPin } from "./nav";

export interface ShellActions {
  openPalette: () => void;
  go: (href: string) => void;
  toggleAutomation: () => void;
  automationOff: () => boolean;
  /** Absent in builds without remote control: no command then. */
  toggleRemote?: () => void;
  remoteOn: () => boolean;
}

export function shellCommands(a: ShellActions): Command[] {
  return [
    {
      id: "palette.open",
      title: "Show all commands",
      section: "settings",
      defaultKeys: ["Ctrl+K", "Ctrl+P", "Ctrl+Shift+P"],
      whileTyping: true,
      noRecent: true,
      run: a.openPalette,
    },
    ...navCommands(a.go),
    { id: "shell.theme", title: "Toggle light / dark theme", section: "settings", scope: "setting", run: () => theme.toggle() },
    {
      id: "shell.pin",
      title: "Pin or unpin the sidebar",
      section: "settings",
      scope: "setting",
      defaultKeys: ["Ctrl+B"],
      notWhileTyping: true,
      run: toggleSidebarPin,
    },
    {
      id: "shell.automation",
      get title() {
        return a.automationOff() ? "Automation on" : "Automation off";
      },
      section: "settings",
      run: a.toggleAutomation,
    },
    ...(a.toggleRemote
      ? [
          {
            id: "shell.remote",
            get title() {
              return a.remoteOn() ? "Remote control off" : "Remote control on";
            },
            keywords: ["remote", "crew", "browser"],
            section: "settings" as const,
            run: a.toggleRemote,
          },
        ]
      : []),
    applyStyleCommand,
  ];
}
