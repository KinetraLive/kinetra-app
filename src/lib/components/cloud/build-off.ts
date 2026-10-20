// `$cloud` in builds without cloud backup (release builds until the production API has cloud
// projects switched on; see svelte.config.js and ./build-on.ts): nothing to show, nothing to ask.
// The core refuses and runs nothing in such builds too (core::cloud::projects::CLOUD_BUILD).

import type { Component } from "svelte";

export const CLOUD_BUILD: boolean = false;
export const loadTopBar: (() => Promise<Component>) | null = null;
export const history: { open: () => void; ready: () => boolean } | null = null;
export const loadQuitCheck: (() => Promise<(exitApp: () => Promise<void>) => Promise<boolean>>) | null = null;
export const loadPane: (() => Promise<{ default: Component }>) | null = null;
