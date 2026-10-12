// Opens the "New template" gallery from anywhere. The gallery is its own lazy chunk: nothing loads
// until the first `show()`.
//
//   const gallery = new GalleryLauncher();
//   <Button onclick={gallery.show}>New template</Button>
//   {#if gallery.component}
//     <gallery.component open={gallery.open} onclose={gallery.close} />
//   {/if}
//
// Creating a template closes the gallery and navigates to its editor (`/templates/<id>`).
// Opened from a layer ("Browse styles…"): the parent passes `layer` (a GalleryLayer), and null for
// "New template".

import type { Component } from "svelte";
import { toAppError } from "$lib/api";
import { toasts } from "$lib/stores/toasts.svelte";

/** The layer a gallery opened from "Browse styles…" is about. */
export interface GalleryLayer {
  screenId: string;
  layerId: string;
  name: string;
  templateId: string;
}

export type GalleryProps = {
  open: boolean;
  layer?: GalleryLayer | null;
  onclose: () => void;
};
export type GalleryComponent = Component<GalleryProps>;

let mod: Promise<GalleryComponent> | undefined;

/** Loads the Gallery component (cached; a failed load is retried next time). */
export function loadGallery(): Promise<GalleryComponent> {
  mod ??= import("./Gallery.svelte").then((m) => m.default as GalleryComponent);
  mod.catch(() => (mod = undefined));
  return mod;
}

export class GalleryLauncher {
  /** The loaded component; null until the first `show()`. */
  component = $state.raw<GalleryComponent | null>(null);
  open = $state(false);

  show = async (): Promise<void> => {
    try {
      this.component ??= await loadGallery();
      this.open = true;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't open the gallery: ${toAppError(e).message}` });
    }
  };

  close = (): void => {
    this.open = false;
  };
}
