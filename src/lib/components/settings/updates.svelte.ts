// Preferences > About: Check for updates and Install and restart (D2, D3). One instance per window,
// so a result survives switching sections. Checks are manual only (never at startup).
//
//   idle → checking → current | available | failed
//   available → installing (update-progress) → the app restarts | available + installError
//
// A failed check always says "Couldn't check for updates. Try again later." (the real error is in
// the log). Install is disabled with the reason while anything is on air (`UpdateCheck.blocked`,
// rechecked by the backend: an `onAir` error shows its message).

import { on, toAppError } from "$lib/api";
import { checkForUpdate, installUpdate } from "$lib/api/app";
import type { UpdateCheck, UpdateProgress } from "$lib/types";
import { updateRequest } from "./state.svelte";

export const CHECK_FAILED = "Couldn't check for updates. Try again later.";
export const UP_TO_DATE = "Kinetra is up to date.";

export type UpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "current" }
  | { kind: "available"; check: UpdateCheck; installError: string | null }
  | { kind: "installing"; check: UpdateCheck; progress: UpdateProgress | null }
  | { kind: "failed" };

export class Updater {
  state = $state.raw<UpdateState>({ kind: "idle" });
  /** The last `updateRequest.seq` answered (a request made before About mounted still runs). */
  #handled = 0;

  get busy(): boolean {
    return this.state.kind === "checking" || this.state.kind === "installing";
  }

  /** Runs a check when one was requested since the last (Help › Check for updates). */
  answerRequest(seq: number) {
    if (seq <= this.#handled) return;
    this.#handled = seq;
    void this.check();
  }

  async check(): Promise<void> {
    if (this.busy) return;
    this.state = { kind: "checking" };
    try {
      const c = await checkForUpdate();
      this.state = c.available && c.version ? { kind: "available", check: c, installError: null } : { kind: "current" };
    } catch {
      this.state = { kind: "failed" };
    }
  }

  async install(): Promise<void> {
    const s = this.state;
    if (s.kind !== "available" || s.check.blocked) return;
    this.state = { kind: "installing", check: s.check, progress: null };
    const off = on("update-progress", (p) => {
      if (this.state.kind === "installing") this.state = { ...this.state, progress: p };
    });
    try {
      // Resolves only if the app didn't restart.
      await installUpdate();
    } catch (e) {
      const err = toAppError(e);
      const blocked = err.code === "onAir" ? err.message : null;
      this.state = {
        kind: "available",
        check: { ...s.check, blocked },
        installError: blocked ? null : `Couldn't install the update: ${err.message}`,
      };
    } finally {
      off();
    }
  }
}

export const updater = new Updater();

/** Asks About (now or when it next mounts) to check for updates. */
export function requestUpdateCheck() {
  updateRequest.seq++;
}
