// Starts the real output server (src-tauri/examples/fixture_server.rs) and drives its live state
// over stdin (full list in that file's header): `show`, `hide`, `fields <json>`, `clear`,
// `restart`, `template <presetId>`, `vars <json>`, `autoout <ms|off>`, project screens
// (`addtemplate`, `savescreen`, `layershow`, `rect`, `liveedit`, `dir`, ...), flows (`saveflow`,
// `runflow`, `flowstate`, `openproject`, ...), the flow editor (`saveflowdraft`, `deployflow`,
// `checkflow`, `subscribedebug`, `debugevents`, `files`, ...) and `quit`. It prints `PORT=<n>` once
// listening and `OK <cmd>` (or `OK <value>`) / `ERR <msg>` after each command.

import { test as base } from "@playwright/test";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createInterface } from "node:readline";

export interface Fixture {
  base: string;
  /** Sends a command; resolves with the text after `OK ` (an id, a path, or the command name). */
  cmd(line: string): Promise<string>;
}

async function startFixture(): Promise<{ fixture: Fixture; stop: () => Promise<void> }> {
  const child: ChildProcessWithoutNullStreams = spawn(
    "cargo",
    ["run", "--quiet", "--example", "fixture_server", "--manifest-path", "src-tauri/Cargo.toml"],
    { stdio: ["pipe", "pipe", "pipe"], shell: process.platform === "win32" },
  );
  const lines = createInterface({ input: child.stdout });
  const waiting: Array<(line: string) => void> = [];
  let stderr = "";
  child.stderr.on("data", (d) => (stderr += d));
  lines.on("line", (line) => waiting.shift()?.(line.trim()));
  const nextLine = () => new Promise<string>((resolve) => waiting.push(resolve));

  const first = await Promise.race([
    nextLine(),
    new Promise<string>((_, reject) =>
      child.once("exit", (code) => reject(new Error(`fixture exited (${code}): ${stderr}`))),
    ),
  ]);
  const port = /^PORT=(\d+)$/.exec(first)?.[1];
  if (!port) throw new Error(`unexpected fixture output: ${first}\n${stderr}`);

  let queue: Promise<unknown> = Promise.resolve();
  const cmd = (line: string) => {
    const run = queue.then(async () => {
      const reply = nextLine();
      child.stdin.write(line + "\n");
      const r = await reply;
      if (!r.startsWith("OK")) throw new Error(`fixture: ${line} -> ${r}`);
      return r.slice(3);
    });
    queue = run.catch(() => {});
    return run;
  };

  return {
    fixture: { base: `http://127.0.0.1:${port}`, cmd },
    stop: async () => {
      child.stdin.end("quit\n");
      await new Promise((r) => {
        const t = setTimeout(() => {
          child.kill();
          r(null);
        }, 3000);
        child.once("exit", () => {
          clearTimeout(t);
          r(null);
        });
      });
    },
  };
}

export const test = base.extend<{ live: Fixture }, { server: Fixture }>({
  server: [
    async ({}, use) => {
      const { fixture, stop } = await startFixture();
      await use(fixture);
      await stop();
    },
    // First run compiles the example.
    { scope: "worker", timeout: 300_000 },
  ],
  // Every test starts from a clean screen with default fields.
  live: async ({ server }, use) => {
    await server.cmd("clear");
    await server.cmd(`fields {"name":"Ada Lovelace","title":"Analyst & Mathematician"}`);
    await use(server);
    await server.cmd("clear");
  },
});

/**
 * For tests that load built-in presets (`template <id>`): their own fixture server, so the shared
 * `server`'s test lower third (which `template` would replace for good) stays untouched.
 * `presetLive` starts every test off air with auto-out off.
 */
export const presetTest = base.extend<{ presetLive: Fixture }, { presetServer: Fixture }>({
  presetServer: [
    async ({}, use) => {
      const { fixture, stop } = await startFixture();
      await use(fixture);
      await stop();
    },
    { scope: "worker", timeout: 300_000 },
  ],
  presetLive: async ({ presetServer }, use) => {
    await presetServer.cmd("clear");
    await presetServer.cmd("autoout off");
    await use(presetServer);
    await presetServer.cmd("clear");
  },
});

/**
 * For the style matrix (e2e/styles.spec.ts): a fixture server of its own per worker, so the
 * project templates and screens it creates never show up in the shared `server`'s project.
 */
export const styleTest = base.extend<{}, { styleServer: Fixture }>({
  styleServer: [
    async ({}, use) => {
      const { fixture, stop } = await startFixture();
      await use(fixture);
      await stop();
    },
    { scope: "worker", timeout: 300_000 },
  ],
});

/**
 * For tests that open another project (`openproject`): a fixture server of their own per test,
 * so the shared `server`'s project stays the one every other test uses.
 */
export const isolatedTest = base.extend<{ own: Fixture }>({
  own: async ({}, use) => {
    const { fixture, stop } = await startFixture();
    await use(fixture);
    await stop();
  },
});

export { expect } from "@playwright/test";
