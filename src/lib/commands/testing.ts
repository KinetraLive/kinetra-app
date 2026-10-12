// Test helper: what the shell (AppShell) does for keys, for tests that render a page without it.
// Installs the capture-phase keydown that runs bound commands and sets the context's route.
// Returns the cleanup.

import { provideContext, runShortcut, type CommandContext } from "./registry.svelte";

export function installShortcuts(context: Partial<CommandContext> = {}): () => void {
  const onkeydown = (e: KeyboardEvent) => {
    if (e.defaultPrevented) return;
    if (runShortcut(e)) {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  window.addEventListener("keydown", onkeydown, true);
  const drop = provideContext({ project: true, ...context });
  return () => {
    window.removeEventListener("keydown", onkeydown, true);
    drop();
  };
}
