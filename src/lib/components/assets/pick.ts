// Opens the asset picker from anywhere and resolves with the choice:
//
//   const { pickAsset } = await import("$lib/components/assets/pick");
//   const v = await pickAsset({ current: value, label: "Logo" });
//   if (v !== null) onchange(v);   // "/assets/<name>", or "" for None; null = cancelled
//
// The picker (a native modal <dialog>) is its own lazy chunk, mounted on <body> while open and
// unmounted when it closes. Only one is open at a time: a second call while one is open resolves
// null.

import { mount, unmount } from "svelte";
import type { AssetPickerProps } from "./AssetPicker.svelte";

export interface PickOptions {
  /** The current value (its image is selected when it exists). */
  current?: string;
  /** What is being picked for (the dialog title says "Choose image for <label>"). */
  label?: string;
}

let chunk: Promise<typeof import("./AssetPicker.svelte")> | null = null;
let busy = false;

/** Loads the picker chunk (cached; a failed load is retried next time). */
export function loadAssetPicker() {
  chunk ??= import("./AssetPicker.svelte");
  chunk.catch(() => (chunk = null));
  return chunk;
}

/** Resolves with the chosen `/assets/<name>` value, "" for None, or null when cancelled. */
export async function pickAsset(options: PickOptions = {}): Promise<string | null> {
  if (busy) return null;
  busy = true;
  try {
    const { default: Picker } = await loadAssetPicker();
    return await new Promise<string | null>((resolve) => {
      const target = document.body;
      const props: AssetPickerProps = {
        current: options.current ?? "",
        label: options.label,
        onclose: (value) => {
          resolve(value);
          // After the close handler returns (the dialog restores focus first).
          queueMicrotask(() => void unmount(instance));
        },
      };
      const instance = mount(Picker, { target, props });
    });
  } finally {
    busy = false;
  }
}
