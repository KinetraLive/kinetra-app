// Commands that pages register only while they are open (the layout editor, the flow editor,
// Control's PANIC and Clear all), and the shell's own (navigation, palette, theme, …), which the
// Preferences window (no shell) doesn't register. Keyboard shortcuts registers the ones not registered right now
// as inert copies (never listed, never run: `when` is false) while it is open, so the binding table
// knows their default keys and `conflicts()` sees them. Built from the pages' own command
// factories with no-op actions: titles, ids and defaults can't drift.

import { appCommands } from "$lib/commands/menu/commands";
import type { Command } from "$lib/commands/registry.svelte";
import { controlCommands } from "$lib/components/control/commands.svelte";
import type { ControlCtl } from "$lib/components/control/control.svelte";
import { flowEditorCommands } from "$lib/components/flows/graph/commands";
import { editorCommands } from "$lib/components/screens/editor/commands";
import { DEFAULT_SCREEN_EDITOR } from "$lib/components/screens/editor/limits";
import { shellCommands } from "$lib/components/shell/commands";

const noop = () => {};
const never = () => false;

/** The pages' commands, with actions that do nothing. */
export function pageCommands(): Command[] {
  return [
    ...shellCommands({ openPalette: noop, go: noop, toggleAutomation: noop, automationOff: never, toggleRemote: noop, remoteOn: never }),
    // The native menu's app commands (registered by the main window's menu sync).
    ...appCommands(),
    ...controlCommands({ capturing: null } as unknown as ControlCtl, [], []),
    ...editorCommands({
      view: () => DEFAULT_SCREEN_EDITOR,
      setView: noop,
      selectionSize: () => 0,
      layerCount: () => 0,
      hasGuides: never,
      selectAll: noop,
      align: noop,
      distribute: noop,
      clearGuides: noop,
      toggleLock: noop,
      editSafeAreas: noop,
    }),
    ...flowEditorCommands({
      deploy: noop,
      deployBlocked: () => null,
      undo: noop,
      redo: noop,
      canUndo: never,
      canRedo: never,
      selectAll: noop,
      fit: noop,
    }),
  ];
}

/** A copy that is never listed or run (its keys and defaults still count for bindings). */
export const inert = (c: Command): Command => ({ ...c, when: never });
