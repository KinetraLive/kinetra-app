// Output server status, mirrored from the backend's `server-status` event.
//
//   $effect(() => server.watch());   // in any component that shows the status
//
// Ref-counted: the first watcher loads `serverStatus()` and subscribes to `server-status`; the
// last unwatch releases the listener. Nothing listens while nothing watches. `status` keeps its
// last value between watchers so a remount paints immediately (then refreshes).

import { on } from "$lib/api";
import { serverStatus } from "$lib/api/live";
import type { ServerStatus } from "$lib/types";
import { toasts } from "./toasts.svelte";

class ServerStore {
  /** Replaced wholesale on every change. `null` until first loaded. */
  status = $state.raw<ServerStatus | null>(null);

  #watchers = 0;
  #off: (() => void) | null = null;
  /** Bumped per subscription so a stale `serverStatus()` reply never overwrites an event. */
  #gen = 0;

  watch(): () => void {
    if (this.#watchers++ === 0) this.#start();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      if (--this.#watchers === 0) this.#stop();
    };
  }

  #start() {
    const gen = ++this.#gen;
    let gotEvent = false;
    this.#off = on("server-status", (s) => {
      gotEvent = true;
      this.status = s;
    });
    serverStatus().then(
      (s) => {
        if (gen === this.#gen && !gotEvent) this.status = s;
      },
      () => {
        // No backend (plain browser) or a transient failure: keep the last known status.
      },
    );
  }

  #stop() {
    this.#gen++;
    this.#off?.();
    this.#off = null;
  }
}

export const server = new ServerStore();

/** `http://127.0.0.1:7770` → `127.0.0.1:7770`. */
export function hostPort(url: string): string {
  return url.replace(/^https?:\/\//, "").replace(/\/+$/, "");
}

/** Local URL of a screen, or null while the server isn't running. */
export function screenUrl(status: ServerStatus | null, screenId: string): string | null {
  return status?.state === "running" ? `${status.url}/screen/${screenId}` : null;
}

/** LAN URL of a screen, or null when not running or LAN access is off / has no address. */
export function lanScreenUrl(status: ServerStatus | null, screenId: string): string | null {
  return status?.state === "running" && status.lanUrl ? `${status.lanUrl}/screen/${screenId}` : null;
}

/** One-line human status for the server, or null when stopped / unknown. */
export function statusText(status: ServerStatus | null): string | null {
  switch (status?.state) {
    case "running":
      return `Running at ${status.url}`;
    case "starting":
      return "Starting…";
    case "error":
      return status.code === "portInUse"
        ? `Port ${status.port} is in use — choose another port.`
        : status.message;
    default:
      return null;
  }
}

/** Copies a URL to the clipboard and confirms (or reports the failure) with a toast. */
export async function copyUrl(url: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(url);
    toasts.push({ kind: "success", message: `Copied ${url}` });
  } catch {
    toasts.push({ kind: "error", message: "Couldn't copy the URL to the clipboard." });
  }
}
