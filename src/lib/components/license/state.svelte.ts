// The UI's copy of the core's license view (display only: the core gates every Pro feature on
// its own state). Loaded with the license host (main window, after first paint).

import { on } from "$lib/api";
import * as api from "$lib/api/license";
import type { LicenseView } from "$lib/types";
import { toasts } from "$lib/stores/toasts.svelte";
import { cloudMessage, redeemText } from "./messages";
import { openInBrowser } from "./open";

class LicenseState {
  view = $state.raw<LicenseView | null>(null);
  /** "I have a license key": the key to redeem once signed in. */
  pendingKey = $state<string | null>(null);
  /** Dialogs the operator closed this session (shown again on the next change). */
  dismissed = $state({ movePro: "", seats: "", beta: false });
  #off: (() => void) | null = null;

  start() {
    if (this.#off) return;
    this.#off = on("license-changed", (v) => this.#apply(v));
    api.licenseView().then(
      (v) => this.#apply(v),
      () => {},
    );
  }

  #apply(v: LicenseView) {
    this.view = v;
    const key = this.pendingKey;
    if (key && v.signedIn) {
      this.pendingKey = null;
      void this.redeem(key);
    }
  }

  async signIn() {
    try {
      const v = await api.signInStart();
      if (v.state === "waiting") void openInBrowser(v.url);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't start sign-in: ${cloudMessage(e)}` });
    }
  }

  /** Redeems now when signed in; otherwise signs in first and redeems after. */
  async redeem(key: string): Promise<boolean> {
    if (!this.view?.signedIn) {
      this.pendingKey = key;
      await this.signIn();
      return false;
    }
    try {
      toasts.push(redeemText(await api.licenseRedeem(key)));
      return true;
    } catch (e) {
      toasts.push({ kind: "error", message: cloudMessage(e) });
      return false;
    }
  }

  async run(label: string, f: () => Promise<void>) {
    try {
      await f();
    } catch (e) {
      toasts.push({ kind: "error", message: `${label}: ${cloudMessage(e)}` });
    }
  }
}

export const license = new LicenseState();
