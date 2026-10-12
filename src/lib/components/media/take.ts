// Take on a media layer (or a template layer with video / image fields) that isn't loaded on every output asks once: "Not loaded on 1 output.
// Take anyway?" (a ConfirmDialog mounted on <body>). The core never blocks a take; this only asks.
// A lazy chunk: Control imports it when a media layer is taken.

import { mount, unmount } from "svelte";
import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
import type { LiveLayerView } from "$lib/types";
import { notReadyQuestion } from "./media";

let open = false;

/** Resolves true to take: at once when the layer is loaded everywhere (or nothing is connected),
 * else after "Take anyway". A second question while one is open resolves false. */
export function confirmTake(layer: Pick<LiveLayerView, "name" | "media" | "readiness">): Promise<boolean> {
  const question = notReadyQuestion(layer.media?.readiness ?? layer.readiness);
  if (question === null) return Promise.resolve(true);
  if (open) return Promise.resolve(false);
  open = true;
  return new Promise((resolve) => {
    const done = (ok: boolean) => {
      open = false;
      resolve(ok);
      queueMicrotask(() => void unmount(instance));
    };
    const instance = mount(ConfirmDialog, {
      target: document.body,
      props: {
        title: `Take ${layer.name}?`,
        message: question,
        confirmLabel: "Take anyway",
        onconfirm: () => done(true),
        oncancel: () => done(false),
      },
    });
  });
}
