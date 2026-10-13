// Control's Studio mode (Preview / Program): settings and the on/off switch with its exit
// question. Part of the Control route's chunk, so it stays small; the rest are lazy chunks: `ui`
// (loaded once Studio mode is on: takes and cues (./ops.ts), the studio stage, the layer rows'
// studio parts, the palette commands, the toolbar's settings menu and the exit question).
// `take` / `cue` wait for `ui` when it isn't there yet.
//
// Settings go through `setStudio` (the server saves them in
// ui-state and may refuse turning off while Preview differs from Program); the local ui-state copy
// follows optimistically so other Control writes never send stale studio settings.
//
// One transition, as in OBS: the project default (`settings.transition`), shown and edited in the
// studio column, the settings popover and the palette, all saved through `setStudio`. TAKE (button,
// Space/Enter, palette, flows) and the T-bar send no transition, so the server uses it.

import { toAppError } from "$lib/api";
import { setStudio } from "$lib/api/live";
import { toasts } from "$lib/stores/toasts.svelte";
import { uiState } from "$lib/stores/uiState.svelte";
import type { StudioSettings, TakeTransition } from "$lib/types";

export type StudioExit = "take" | "discard" | "keepStaged";

/** Settings of a project that never had Studio mode on (ui-state without `control.studio`). */
export const DEFAULT_STUDIO: StudioSettings = {
  enabled: false,
  previewAfterTake: "keep",
  transition: { kind: "crossfade", ms: 500 },
};

/** The lazy studio chunk (./ui.svelte.ts). */
export type StudioUi = typeof import("./ui.svelte");

export class StudioCtl {
  readonly #settings: () => StudioSettings | undefined;
  settings = $derived.by(() => this.#settings() ?? DEFAULT_STUDIO);
  on = $derived(this.settings.enabled);

  /** The transition every take uses: the saved project default. */
  transition = $derived(this.settings.transition);

  /** The lazy studio chunk, loaded once Studio mode is on. */
  ui = $state.raw<StudioUi | null>(null);
  /** Screens with a pending Preview while the exit question is open (null = closed). */
  asking = $state.raw<string[] | null>(null);
  #uiLoad: Promise<void> | null = null;

  constructor(settings: () => StudioSettings | undefined) {
    this.#settings = settings;
  }

  loadUi(): Promise<void> {
    this.#uiLoad ??= import("./ui.svelte").then(
      (m) => void (this.ui = m),
      () => void (this.#uiLoad = null),
    );
    return this.#uiLoad;
  }

  /** Saves a new default transition (the studio column, the palette). */
  setTransition = (transition: TakeTransition): Promise<void> => this.set({ transition });

  /** Saves settings through the server; the local ui-state copy follows (rolled back on failure).
   *  Resolves with the error, or null. */
  async save(next: StudioSettings, exit: StudioExit | null): Promise<unknown> {
    const before = uiState.value;
    if (before) uiState.set({ ...before, control: { ...before.control, studio: next } });
    const mine = uiState.value;
    try {
      await setStudio(next, exit);
      return null;
    } catch (e) {
      if (before && uiState.value === mine) uiState.set(before);
      return e;
    }
  }

  /** New settings (the switch keeps its state); a failure is a toast. */
  set = async (patch: Partial<StudioSettings>): Promise<void> => {
    const e = await this.save({ ...this.settings, ...patch }, null);
    if (e) toasts.push({ kind: "error", message: `Couldn't change Studio mode: ${toAppError(e).message}` });
  };

  /** The switch: on at once; off asks Take / Keep / Discard when a Preview differs from Program. */
  toggle = async (): Promise<void> => {
    if (this.on) {
      await (await this.#chunk())?.turnOff(this);
      return;
    }
    this.ui?.previews.clear();
    void this.loadUi();
    const e = await this.save({ ...this.settings, enabled: true }, null);
    if (e) toasts.push({ kind: "error", message: `Couldn't turn Studio mode on: ${toAppError(e).message}` });
  };

  /** The exit question's answer (null = Cancel: Studio mode stays on). */
  exit = async (choice: StudioExit | null): Promise<void> => {
    this.asking = null;
    if (choice) await this.ui?.turnOff(this, choice);
  };

  /** TAKE: the screen's Preview to Program (null = the project default). */
  take = async (screenId: string, transition: TakeTransition | null = null): Promise<boolean> => {
    return (await this.#chunk())?.take(screenId, transition) ?? false;
  };

  /** Cues a layer in or out of Preview. */
  cue = async (screenId: string, layerId: string): Promise<void> => {
    await (await this.#chunk())?.cue(screenId, layerId);
  };

  async #chunk(): Promise<StudioUi | null> {
    if (!this.ui) await this.loadUi();
    return this.ui;
  }
}
