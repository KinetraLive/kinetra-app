// `$remote` in builds with remote control (KINETRA_REMOTE=1, or `pnpm dev`; see svelte.config.js):
// the chip, the Preferences block and the switch, all loaded lazily. Builds without it resolve
// `$remote` to ./build-off.ts, so none of this (or its text) is in them; CI checks that
// (scripts/check-remote-build.mjs). Both modules export the same names and types.

import type { Component } from "svelte";

/** This build offers remote control. */
export const REMOTE_BUILD: boolean = true;

/** The top bar's chip and its panel. */
export const loadChip: (() => Promise<Component>) | null = () =>
  import("./RemoteChip.svelte").then((m) => m.default as Component);

/** Preferences › Network & API › Remote control. */
export const loadSection: (() => Promise<Component>) | null = () =>
  import("$lib/components/settings/RemoteSection.svelte").then((m) => m.default as Component);

/** The palette's "Remote control on / off". */
export const setRemote: ((on: boolean) => Promise<unknown>) | null = (on) =>
  import("$lib/api/remote").then((m) => m.remoteSet(on));
