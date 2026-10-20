// `$cloud` in builds with cloud backup (KINETRA_CLOUD=1, `pnpm dev` and tests; see
// svelte.config.js): the top bar's part, the palette's history, the quit check and Preferences ›
// Cloud backup, all loaded lazily (the picker's "In the cloud" is `$cloud-picker`). Builds without it resolve
// `$cloud` to ./build-off.ts, so none of this (or its text) is in them; CI checks that
// (scripts/check-cloud-build.mjs). Both modules export the same names and types. The core
// decides the same way at its build (core::cloud::projects::CLOUD_BUILD).

import type { Component } from "svelte";
import { historyReady, openHistory } from "./history/host";

/** This build has cloud backup. */
export const CLOUD_BUILD: boolean = true;

/** The top bar's part: the chip and the version history it opens. */
export const loadTopBar: (() => Promise<Component>) | null = () =>
  import("./TopBarCloud.svelte").then((m) => m.default as Component);

/** The palette's "Version history" (open it; whether the open project has a backup). Plain
 * functions: this module is in every page (the picker too), so it stays tiny. */
export const history: { open: () => void; ready: () => boolean } | null = {
  open: () => openHistory(),
  ready: historyReady,
};

/** "Back up before quitting?" (true = it took over quitting). */
export const loadQuitCheck: (() => Promise<(exitApp: () => Promise<void>) => Promise<boolean>>) | null = () =>
  import("./exit").then((m) => m.backUpBeforeQuit);

/** Preferences › Cloud backup. */
export const loadPane: (() => Promise<{ default: Component }>) | null = () =>
  import("$lib/components/settings/panes/CloudPane.svelte");
