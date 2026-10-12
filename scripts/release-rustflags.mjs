// Prints CARGO_ENCODED_RUSTFLAGS for release builds: the Cargo home (registry and git
// dependencies), the toolchain's sysroot (rustc maps std's paths back to an installed rust-src)
// and the repository are remapped, so panic locations in the binary carry no local build paths.
// Usage: CARGO_ENCODED_RUSTFLAGS="$(node scripts/release-rustflags.mjs)".
import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const cargoHome = path.resolve(process.env.CARGO_HOME || path.join(os.homedir(), ".cargo"));
const sysroot = execFileSync("rustc", ["--print", "sysroot"], { cwd: path.join(repo, "src-tauri"), encoding: "utf8" }).trim();

const flags = [
  `--remap-path-prefix=${cargoHome}=/cargo`,
  `--remap-path-prefix=${sysroot}=/rustc`,
  `--remap-path-prefix=${repo}=/kinetra`,
];
// The 0x1f separator keeps paths with spaces intact.
process.stdout.write(flags.join("\x1f"));
