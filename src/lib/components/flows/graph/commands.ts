// The flow editor's commands (palette section "Edit", key scope "flowEditor"), registered by
// FlowEditor while it is mounted. Ctrl+Enter deploys even while typing; undo and redo leave a text
// field's own undo alone. Preferences > Keyboard shortcuts lists them from here too.

import type { Command } from "$lib/commands/registry.svelte";
import { t } from "$lib/i18n";

export interface FlowEditorActions {
  deploy: () => unknown;
  /** Why Deploy is disabled, or null. */
  deployBlocked: () => string | null;
  undo: () => void;
  redo: () => void;
  canUndo: () => boolean;
  canRedo: () => boolean;
  selectAll: () => void;
  fit: () => void;
}

const BASE = { section: "edit", keyScope: "flowEditor" } as const;

export function flowEditorCommands(a: FlowEditorActions): Command[] {
  return [
    {
      ...BASE,
      id: "flow.deploy",
      title: "Deploy flow",
      defaultKeys: ["Ctrl+Enter"],
      whileTyping: true,
      run: () => void a.deploy(),
      disabled: a.deployBlocked,
    },
    {
      ...BASE,
      id: "flow.undo",
      title: "Undo",
      defaultKeys: ["Ctrl+Z"],
      notWhileTyping: true,
      run: a.undo,
      disabled: () => (a.canUndo() ? null : t("Nothing to undo")),
    },
    {
      ...BASE,
      id: "flow.redo",
      title: "Redo",
      defaultKeys: ["Ctrl+Shift+Z"],
      notWhileTyping: true,
      run: a.redo,
      disabled: () => (a.canRedo() ? null : t("Nothing to redo")),
    },
    { ...BASE, id: "flow.selectAll", title: "Select all nodes", run: a.selectAll },
    { ...BASE, id: "flow.fit", title: "Fit the flow in view", run: a.fit },
  ];
}
