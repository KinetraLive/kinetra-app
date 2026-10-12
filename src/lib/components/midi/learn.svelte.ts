// One MIDI Learn session: opens the core's learn mode (`midiLearn(true)`; counted there), takes the
// first `midi-learn` message, then closes it. Esc cancels. Used by Preferences > MIDI & OSC, the
// Assign MIDI popover and the flow editor's MIDI in trigger.
//
//   const learn = new Learn();
//   learn.start((l) => …);   // "Press a key, pad or fader…" while learn.active
//   learn.stop();            // cancel (also on teardown)

import { toAppError } from "$lib/api";
import { midiLearn, onMidiLearn } from "$lib/api/midi";
import { toasts } from "$lib/stores/toasts.svelte";
import type { MidiLearned } from "$lib/types";

export const LEARN_HINT = "Press a key, pad or fader…";

export class Learn {
  /** Waiting for a message. */
  active = $state(false);
  #close: (() => void) | null = null;

  start(onlearned: (l: MidiLearned) => void, oncancel?: () => void): void {
    this.stop();
    this.active = true;
    const offEvent = onMidiLearn((l) => {
      this.stop();
      onlearned(l);
    });
    const onkey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      this.stop();
      oncancel?.();
    };
    window.addEventListener("keydown", onkey, true);
    let opened = false;
    const close = () => {
      offEvent();
      window.removeEventListener("keydown", onkey, true);
      if (opened) void midiLearn(false).catch(() => {});
    };
    this.#close = close;
    midiLearn(true).then(
      () => {
        opened = true;
        // Stopped before the core answered: close the session it just opened.
        if (this.#close !== close) void midiLearn(false).catch(() => {});
      },
      (e) => {
        if (this.#close !== close) return;
        this.stop();
        oncancel?.();
        toasts.push({ kind: "error", message: `Couldn't start MIDI Learn: ${toAppError(e).message}` });
      },
    );
  }

  /** Cancels a waiting session (no-op otherwise). */
  stop(): void {
    const close = this.#close;
    if (!close) return;
    this.#close = null;
    this.active = false;
    close();
  }
}
