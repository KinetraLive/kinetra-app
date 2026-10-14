// Tauri's Windows sign hook in CI (`bundle.windows.signCommand`, set by `release.mjs config` only
// when Artifact Signing is set up in the "release" environment): signs one file with signtool and
// Microsoft's Artifact Signing dlib. The workflow prepares both (ARTIFACT_SIGNING_SIGNTOOL,
// _DLIB, _METADATA: endpoint, account and profile, Azure CLI credential only) and signs in with
// `azure/login` (OIDC, no client secret) before the build, so this script holds no credentials.

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
  env("ARTIFACT_SIGNING_SIGNTOOL"),
  [
    "sign", "/v",
    "/fd", "SHA256",
    "/tr", "http://timestamp.acs.microsoft.com", "/td", "SHA256",
    "/dlib", env("ARTIFACT_SIGNING_DLIB"),
    "/dmdf", env("ARTIFACT_SIGNING_METADATA"),
    "/d", "Kinetra",
    file,
  ],
  { stdio: ["ignore", "inherit", "inherit"], shell: false },
);
if (r.error) console.log(`::error::signtool could not start: ${r.error.code ?? "error"}`);
process.exit(r.status ?? 1);
