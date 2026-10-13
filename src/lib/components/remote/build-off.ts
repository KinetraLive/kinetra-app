// `$remote` in builds without remote control (release builds until kinetra-cloud's relay is
// live; see svelte.config.js and ./build-on.ts): nothing to show, no switch. The core refuses to
// turn it on in such builds too (core::remote::REMOTE_BUILD).

import type { Component } from "svelte";

export const REMOTE_BUILD: boolean = false;
export const loadChip: (() => Promise<Component>) | null = null;
export const loadSection: (() => Promise<Component>) | null = null;
export const setRemote: ((on: boolean) => Promise<unknown>) | null = null;
