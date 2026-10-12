// "Apply style…": one command with a pick step (the styles some preset ships), registered by the
// shell. Tiny: the style list (./commands.ts) and the apply (./apply.ts) are lazy chunks, loaded
// when the step opens and when a style is picked. Running applies the style to the whole project.
// Not in the template editor: its open document would be stale after a style adds variants.

import type { Command } from "$lib/commands/registry.svelte";
import type { StyleInfo } from "$lib/types";

class Offered {
  /** null until loaded (the step shows no options meanwhile, then updates). */
  list = $state.raw<readonly StyleInfo[] | null>(null);
  #loading = false;

  /** The offered styles; starts loading them on first use (again after a failure). */
  get(): readonly StyleInfo[] {
    if (this.list === null && !this.#loading) {
      this.#loading = true;
      import("./commands")
        .then((m) => m.offeredStyles())
        .then(
          (l) => (this.list = l),
          () => {},
        )
        .finally(() => (this.#loading = false));
    }
    return this.list ?? [];
  }
}

export const offeredStyles = new Offered();

export const applyStyleCommand: Command = {
  id: "style.apply",
  title: "Apply style…",
  section: "settings",
  when: (ctx) => ctx.project && ctx.route !== "templateEditor",
  next: {
    kind: "pick",
    placeholder: "Choose a style",
    options: () => offeredStyles.get().map((s) => ({ id: s.id, title: s.name, keywords: [s.description] })),
    run: async (id) => {
      const style = offeredStyles.list?.find((s) => s.id === id);
      if (!style) return;
      const { applyToProject } = await import("./apply");
      await applyToProject(style);
    },
  },
};
