# Testing

## The suites

| Suite | Command | What it covers |
|---|---|---|
| Rust | `cargo test --manifest-path src-tauri/Cargo.toml` | The core (`src-tauri/src/core/`), against in-memory stores and local mock servers (axum); never the real network or keychain |
| Lint | `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | Every target, examples included |
| Types | `pnpm check` | svelte-check and tsc, the renderer too |
| Frontend | `pnpm test` | vitest in jsdom: components with mocked `$lib/api` modules, pure logic, scripts |
| End to end | `pnpm test:e2e` | Playwright on the installed Microsoft Edge against the real Rust server (`examples/fixture_server.rs`) and the real renderer |

Run the tests that cover a change while working, and the full suites once before a change is
done.

## Lazily loaded components in vitest tests

**Preload every lazily loaded component a test opens, or mock it.**

Much of the UI loads with a dynamic `import()` the first time it is needed: dialogs, panes, strips
and editor chunks. A test that opens one (a click that shows a dialog, a page that mounts a lazy
section) waits for Vite to compile that chunk on first use. That compile is usually fast, but on
slow CI runners and under the full suite's parallel load it can take longer than the 1 s that
`waitFor` and `findBy` wait. The test then fails now and then, never when run alone. Three flaky
tests had this cause (2026-10-28/29).

The fix is to load the chunk once before the tests, in the test file:

```ts
// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (docs/TESTING.md).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/ConfirmDialog.svelte"),
    import("$lib/components/sources/new/NewSourceDialog.svelte"),
  ]);
});
```

The component's own `import()` then resolves from the module cache at once. A `vi.mock` of the
module works too, when the test doesn't need the real component.

A preloaded component mounts during the test (before, it often arrived after the test ended), so
the API modules it calls must be mocked as well. A missing one shows as a vitest "Unhandled
Errors" block (`No "…" export is defined on the "$lib/api" mock`) while every test still passes,
and the run fails: check the `Errors` line of the summary, not only `Tests`.

**Never fix such a flake by raising a timeout.** A longer wait hides the cause and slows every run.

### Finding what a test opens

A static look at the imports overstates it (most lazy components behind a rendered page are never
opened by its tests). To see what a test file really opens, run it alone with a Vite plugin that
logs every module it compiles, then keep the `import()` targets of what the file renders:

```js
// vitest.lazylog.config.mjs (temporary; don't commit)
import { appendFileSync } from "node:fs";
import { mergeConfig } from "vite";
import base from "./vite.config.js";
export default async (env) =>
  mergeConfig(await (typeof base === "function" ? base(env) : base), {
    plugins: [{ name: "lazylog", enforce: "pre", transform(_code, id) { appendFileSync(process.env.LAZYLOG, id.split("?")[0] + "\n"); } }],
  });
```

```sh
LAZYLOG=opened.txt pnpm exec vitest run --config vitest.lazylog.config.mjs src/lib/components/x/x.test.ts
```

Every `import("….svelte")` target of the rendered components that appears in `opened.txt` is a
component the test opens: preload it.
