// The listening half of `on()` (index.ts), loaded on first use with `@tauri-apps/api/event`: kept
// out of the core module, which is in the picker's initial bundle.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Listens to a backend event as this window, not "any": events sent to other windows only
 * (`menu`, `prefs-section`) and cross-window events this window caused (`prefs-changed`, …) skip
 * it. The handler gets the payload.
 */
export function listenHere<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  const label = (
    window as { __TAURI_INTERNALS__?: { metadata?: { currentWebview?: { label?: string } } } }
  ).__TAURI_INTERNALS__?.metadata?.currentWebview?.label;
  const options = label ? { target: { kind: "WebviewWindow" as const, label } } : undefined;
  return listen<T>(event, (e) => handler(e.payload), options);
}
