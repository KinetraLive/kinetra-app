// Opens the media picker from anywhere and resolves with the choice:
//
//   const { pickMedia } = await import("$lib/components/media/pick");
//   const a = await pickMedia({ accept: ["video"], current: value, label: "Clip" });
//   if (a !== null) onchange(a === "" ? "" : a.url);   // null = cancelled, "" = None
//
// The picker is its own lazy chunk, mounted on <body> while open. Only one is open at a time: a
// second call while one is open resolves null.

import { mount, unmount } from "svelte";
import type { AssetInfo, MediaKind } from "$lib/types";
import type { MediaPickerProps } from "./MediaPicker.svelte";

export interface PickMediaOptions {
  accept: readonly MediaKind[];
  /** The current `/assets/<name>` value (selected first). */
  current?: string;
  /** What is being picked for ("Choose video for Clip"). */
  label?: string;
  /** Offer None (an empty value). */
  allowNone?: boolean;
}

let busy = false;

const NOUN: Record<string, string> = { video: "video", audio: "audio", image: "image" };

/** The dialog title: "Choose video for Clip", "Choose media". */
export function pickTitle(accept: readonly MediaKind[], label?: string): string {
  const what = accept.length === 1 ? NOUN[accept[0]] : "media";
  return label ? `Choose ${what} for ${label}` : `Choose ${what}`;
}

export async function pickMedia(options: PickMediaOptions): Promise<AssetInfo | "" | null> {
  if (busy) return null;
  busy = true;
  try {
    const { default: Picker } = await import("./MediaPicker.svelte");
    return await new Promise<AssetInfo | "" | null>((resolve) => {
      const props: MediaPickerProps = {
        accept: options.accept,
        current: options.current ?? "",
        title: pickTitle(options.accept, options.label),
        allowNone: options.allowNone,
        onclose: (value) => {
          resolve(value);
          queueMicrotask(() => void unmount(instance));
        },
      };
      const instance = mount(Picker, { target: document.body, props });
    });
  } finally {
    busy = false;
  }
}

/** "Add layer › Media…": picks a file and returns a new media layer for it (null = cancelled). */
export async function pickMediaLayer(
  screen: { width: number; height: number },
  newId: () => string,
): Promise<import("$lib/types").MediaScreenLayer | null> {
  const a = await pickMedia({ accept: ["video", "audio", "image"] });
  if (!a) return null;
  const { newMediaLayer } = await import("./media");
  return newMediaLayer(a, screen, newId());
}
