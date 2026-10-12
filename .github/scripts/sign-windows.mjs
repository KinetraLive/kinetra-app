// Tauri's Windows sign hook in CI (`bundle.windows.signCommand`, set by `release.mjs config` only
// when the Azure Trusted Signing secrets exist): signs one file with trusted-signing-cli. The
// endpoint, account and profile come from the environment, so they are never written into the
// build config; the service principal (AZURE_TENANT_ID / _CLIENT_ID / _CLIENT_SECRET) is read
// by trusted-signing-cli itself. Prints nothing of either.

import { spawnSync } from "node:child_process";

const file = process.argv[2];
const env = (k) => {
  const v = process.env[k];
  if (!v) {
    console.log(`::error::${k} is not set`);
    process.exit(1);
  }
  return v;
};
if (!file) {
  console.log("::error::sign-windows.mjs: no file to sign");
  process.exit(1);
}

const r = spawnSync(
  "trusted-signing-cli",
  [
    "-e", env("AZURE_SIGNING_ENDPOINT"),
    "-a", env("AZURE_SIGNING_ACCOUNT"),
    "-c", env("AZURE_SIGNING_PROFILE"),
    "-d", "Kinetra",
    file,
  ],
  { stdio: ["ignore", "inherit", "inherit"], shell: false },
);
if (r.error) console.log(`::error::trusted-signing-cli could not start: ${r.error.code ?? "error"}`);
process.exit(r.status ?? 1);
