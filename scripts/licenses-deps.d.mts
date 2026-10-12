/** Dev dependencies whose runtime code is compiled into the app bundle. */
export const BUNDLED_DEV: string[];
/** Normal (not dev/build) dependency lines of a Cargo.toml, trimmed and sorted. */
export function cargoDependencyLines(toml: string): string[];
/** sha256 (hex) over the declared shipped dependencies of the repo at `root`. */
export function depsFingerprint(root: string): string;
