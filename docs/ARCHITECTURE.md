# Kinetra architecture

A map of the code for contributors. For the template file format see
[TEMPLATE_FORMAT.md](TEMPLATE_FORMAT.md); for the cloud API the app talks to see [API.md](API.md).

## Overview

```
┌──────────── Tauri app ────────────┐
│  SvelteKit UI (dashboard/control) │──invoke──▶ Tauri commands ─┐
└───────────────────────────────────┘                            ▼
                                                    ┌── Rust core (AppState) ──┐
OBS / vMix / browser ──HTTP/WS──▶ axum server ─────▶│ project files, live state│
Companion / scripts ──/api/*──▶ axum server ─────▶│                          │
MIDI / OSC ─────────────────────▶ control actions ─▶│ sources, flows, outputs  │
                                                    └──────────────────────────┘
```

- **Shell:** Tauri 2 (Rust). The app UI is SvelteKit (SPA, Svelte 5 runes, TypeScript strict).
- **Renderer:** a tiny vanilla TypeScript bundle in `renderer/`. It is what OBS, vMix, display
  outputs and the app's own previews load, at `/screen/<id>`. There is one rendering path.
- **Single source of truth:** project state lives in the Rust core (`AppState`); what is on air
  per screen lives in `LiveHub`. The UI and renderers only reflect it.
- **Changes flow** UI → Tauri command → `LiveHub` → a `tokio::sync::broadcast` channel → `/ws` →
  renderers. Each message is serialized once for all renderers; a renderer that falls behind gets
  a fresh snapshot instead of a growing queue. The UI mirrors live state from `live-changed`
  events and never polls.
- **One async runtime:** the axum server, pollers and other network work run as tokio tasks on
  Tauri's runtime. Long-running work stops through a `oneshot` or `watch` channel.
- **The output server follows the open project:** no project, no listening socket. It binds to
  `127.0.0.1` (default port 7770) unless LAN access is turned on.
- **Network calls happen in Rust.** API sources and polls are fetched by the core and pushed to
  renderers; renderers never fetch external URLs, and secrets never reach the browser.
- **Outside clients are read-only** (the WebSocket only pushes), except OSC, which is accepted
  from this computer and, with LAN access on, from allowlisted addresses.

## Repository layout

```
src/                 SvelteKit app (routes, components, stores, commands/palette, api wrappers)
src/lib/api/         typed wrappers around Tauri invoke: the only place commands are called
src/lib/types.ts     mirrors src-tauri/src/core/model.rs
renderer/            browser-source renderer (pnpm workspace package)
  src/protocol.ts      mirrors src-tauri/src/core/protocol.rs
src-tauri/src/
  commands/          #[tauri::command] handlers, thin, call into core
  core/              project, templates, screens, sources, live state, flows, outputs (no Tauri
                     or axum types, so it is unit-testable; events go through core/events.rs)
  server/            axum routes, the WebSocket hub, OSC
presets/             built-in templates, recipes and flow templates (embedded in the app)
e2e/                 Playwright specs against the real Rust server (fixture_server example)
archive/             shelved features, not built (see archive/README.md)
```

Mirrored pairs must change together: `core/model.rs` ⇄ `src/lib/types.ts` and
`core/protocol.rs` ⇄ `renderer/src/protocol.ts`.

## Project folder (user data)

A project is a plain folder of JSON, HTML and CSS, portable and git-friendly:

```
<project>/
  project.json            name, settings (port, LAN access, default resolution)
  ui-state.json           per-project UI state (control layout, hotkeys, editor view)
  controls.json           MIDI mappings
  screens/<id>.json       a screen: size, background, layers
  templates/<id>/         template.json + variants/<variantId>/{template.html, template.css}
  sources/<id>.json       data tables, schedules, API sources, polls
  flows/<id>.json         automation graphs
  recipes/<id>.json       installed recipes
  assets/                 images, video and audio, served at /assets/<name>
  .backups/               automatic config snapshots (never assets/)
```

- Every JSON file has a `schemaVersion`. Old files are migrated on open; a file from a newer
  version is never overwritten.
- Files are written atomically (temp file, then rename). A corrupt file is renamed to
  `<file>.corrupt-<timestamp>` and defaults load; it never stops the app.
- The folder is watched (`core/watch.rs`): an edit made outside Kinetra reloads into the output;
  Kinetra's own writes are ignored.
- Secrets (API keys, auth headers) live only in the OS keychain (`core/secrets.rs`), never in the
  project folder, exports, backups, logs or errors.

## Live state

- `core/live.rs` holds the live state per screen as pure operations that return the messages to
  broadcast. `core/screen_live.rs` applies screen changes through one path (`apply_doc`).
- **Live rules:** adding, removing and reordering layers apply at once. Content changes apply at
  once off air; on air they are staged until the next take ("Changes on next take") or applied
  immediately, depending on the Live edit setting.
- **Studio mode** (Preview / Program): edits go to a per-screen Preview diff; Take moves it to
  Program with a cut, crossfade or out-and-in transition, or by hand with the T-bar.
- **Clear and PANIC:** clear animates out; PANIC cuts every screen at once with no animation.
- `/ws` speaks protocol v1 (`core/protocol.rs`): the server sends snapshots and diffs (`layer`,
  `state`, `fields`, `vars`, `rect`, `take`, …). The only message a renderer sends is media
  readiness.

## Renderer

- Each layer renders in its own open Shadow DOM; the template's CSS goes into the shadow root.
- Templates compile once per revision; field updates patch bound nodes in place, so animations
  don't restart.
- The template root gets `data-state="in" | "out"`; out-animations finish before removal.
- Text values are always set as text; only `html` fields are inserted raw. Templates are
  sandboxed twice: `core/template_safety.rs` rejects scripts, external URLs and similar on every
  save, and renderer pages are served with a strict Content Security Policy.
- No timers or `requestAnimationFrame` at rest. Clock and timer fields share one scheduler that
  runs only while such a field is on air.
- Size budget: 9 KB gzipped for the main bundle (raised from 8 KB on 2026-10-13 for remote
  control's `gone` report; browser sources still load instantly); media and output support load
  as lazy chunks.

## Sources and automation

- **Sources** are tables a layer's fields bind to: data tables, schedules (`core/schedule.rs`),
  API sources (`core/api_poll.rs`, `core/api_map.rs`) and live polls (`core/polls.rs`). A change
  re-applies every bound screen through the live rules.
- **Flows** (`core/flows.rs`, `core/flow_engine.rs`) are the one automation model: a graph of
  trigger, data, logic and action nodes joined by event wires (control flow) and data wires
  (values pulled when an event passes). `core/flow_nodes.rs` is the port table of every node
  type; `src/lib/components/flows/graph/ports.ts` must match it (checked against
  `ports.fixture.json`; update it with `UPDATE_PORTS_FIXTURE=1 cargo test ports_fixture`).
  Deployed flows compile into a trigger index that the existing event hooks look up: nothing
  polls, and no task runs at rest. Actions use the same core paths as Control.
- **Recipes** (`core/recipes.rs`) install a source, templates, layers and flows in one step.
- **Automation off** stops every automatic trigger; cues and operator actions still run.
- **MIDI and OSC** go through one entry point, `AppState::run_control`, the same commands the
  command palette uses.
- **The Control API** (`core/control_api/`, `server/control.rs`; reference [CONTROL_API.md](CONTROL_API.md))
  is a token-protected WebSocket and REST interface on the output server for Companion and
  scripts. Its state is remote control's tree in a local flavor; one feed task builds it while a
  client is connected and shares each diff with every client; commands use the same paths as
  MIDI and OSC. `docs/control-api.fixture.json` is generated from the code for clients to test
  against.

## Remote control

People control a running show from a browser at `control.kinetra.live`. The contract is
[CONTROL_PROTOCOL.md](CONTROL_PROTOCOL.md) (a copy of kinetra-cloud's; the protocol wins).
It is in a build only with `KINETRA_REMOTE=1` (debug builds and `pnpm dev` have it; release
builds get it from a CI variable once the relay is live; see [RELEASING.md](RELEASING.md)).

- **One outgoing WebSocket** to `api.kinetra.live` while the switch is on for the open project
  (`core/remote/`); nothing connects in to the show machine. The switch lasts for the app
  session only: every start begins off. Off costs nothing (no task, no socket).
- **The app is the authority.** A remote take, clear or GO runs through `run_control`, a field
  edit through `set_layer_fields`, exactly like the local buttons (Studio mode, On take staging,
  coalesced saves). Every command is checked again in the protocol's order (role, Lock remote
  take, target, bound fields, stale against what the sender saw, values). A data editor's edit
  is refused under the live state's lock if the layer is shown, so a take in between can't slip
  past the check.
- **Never interrupts graphics:** the render path only wakes the connection; it builds the
  remote state (screens, layers, values, cues, read-only Preview and media layers) on the
  blocking pool and sends JSON Pointer diffs. A stalled or dropped link only delays remote
  control; it reconnects with backoff and resumes the run, replaying nothing.
- **Exact on air:** outputs report when a layer's out-animation has finished (`{t:"gone"}`);
  remote users see a layer on air until then (at most 3.5 s; `core/live_hub/fades.rs`).
- **The project's ID** (`project.json` `id`, written the first time remote control is turned on)
  names the show; a copied project shares it ("Move it here?" moves the run), and "Make this a
  separate show" gives it a new one.
- **Test connection** uses the protocol's probe mode; diagnostics hold IDs, sizes, timings and
  codes only, never values, names, tokens or account IDs.
- Tests: `core/remote/tests.rs` (a mock relay in the test), `e2e/remote-control.spec.ts` (the
  fixture server's mock relay and the real renderer), `e2e/remote-visibility.spec.ts`.

## App UI

- Shared state lives in `src/lib/stores/*.svelte.ts`; `SyncedStore` gives optimistic writes with
  rollback.
- One command registry (`src/lib/commands/`) drives the palette (Ctrl+K), keyboard shortcuts and
  the native menus.
- Heavy UI (CodeMirror, the flow canvas, dialogs) loads with dynamic `import()`. Live preview
  iframes are expensive (a WebView2 process each): they exist only while visible.
- Styling is plain CSS with design tokens in `src/app.css`; `src/lib/styles/tokens.test.ts`
  checks contrast and token use. Icons are imported per icon from `lucide-svelte/icons/<name>`.
- Every user-facing string goes through `t()` (`src/lib/i18n.ts`).

## Conventions

- Rust: no `unwrap()`/`expect()` outside tests and startup; errors convert into `AppError`
  (`error.rs`). Commands that touch disk are `async` and use `spawn_blocking`. Never hold a lock
  across file I/O.
- IDs are short `nanoid` slugs (10 characters).
- Idle should cost nothing: no polling, no timers at rest, background threads block.
- UI copy is written for operators, in US English.

## Tests

```
cargo test   --manifest-path src-tauri/Cargo.toml
pnpm check                     # svelte-check + tsc
pnpm test                      # vitest
pnpm test:e2e                  # Playwright on installed Microsoft Edge against the real server
```

Renderer and server changes need `pnpm test:e2e`; screenshot baselines are in
`e2e/__screenshots__/`.
