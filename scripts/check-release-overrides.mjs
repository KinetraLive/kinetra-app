// Proves the debug-only test overrides are not in release builds:
//
//   node scripts/check-release-overrides.mjs
//
// Runs `cargo check --release --example release_guard` (src-tauri/examples/release_guard.rs calls
// every override) and passes only if it fails with exactly the expected "doesn't exist" errors,
// one per override. If the example compiles (or fails for another reason), an override may have
// reached release builds and this exits 1. CI runs it in the release jobs.

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

/** The overrides release_guard.rs calls; each must be missing in a release build. */
export const OVERRIDES = ["set_test_entitlements", "load_unchecked_for_tests"];

/**
 * Problems (empty = fine) from cargo's JSON messages: every error must be E0599 (no method) or
 * E0425 (no function) about one of OVERRIDES, and each override must have one.
 */
export function verdict(messages) {
  const errors = messages
    .filter((m) => m.reason === "compiler-message" && m.message?.level === "error")
    .map((m) => m.message)
    .filter((m) => !/^aborting due to|^could not compile/.test(m.message ?? ""));
  if (errors.length === 0) return ["release_guard compiled in release: a test override is in release builds"];
  const problems = [];
  const seen = new Set();
  for (const e of errors) {
    const name = OVERRIDES.find((o) => (e.message ?? "").includes(o));
    const code = e.code?.code ?? "";
    if (!name || !["E0599", "E0425"].includes(code)) {
      problems.push(`unexpected error (not a missing override): ${code} ${e.message}`);
      continue;
    }
    seen.add(name);
  }
  for (const o of OVERRIDES) if (!seen.has(o)) problems.push(`${o} exists in release builds`);
  return problems;
}

function main() {
  const root = resolve(fileURLToPath(import.meta.url), "../..");
  const r = spawnSync(
    "cargo",
    ["check", "--release", "--example", "release_guard", "--manifest-path", "src-tauri/Cargo.toml", "--message-format=json", "--quiet"],
    { cwd: root, encoding: "utf8", maxBuffer: 64 * 1024 * 1024, shell: process.platform === "win32" },
  );
  const messages = (r.stdout ?? "")
    .split("\n")
    .filter((l) => l.startsWith("{"))
    .map((l) => JSON.parse(l));
  const problems = verdict(messages);
  if (problems.length) {
    for (const p of problems) console.error(`release overrides: ${p}`);
    if (r.stderr) console.error(r.stderr.slice(-2000));
    process.exit(1);
  }
  console.log(`release overrides: none in release builds (${OVERRIDES.join(", ")})`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
