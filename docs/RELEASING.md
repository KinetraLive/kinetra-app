# Releasing Kinetra

Two channels, built by GitHub Actions in `KinetraLive/kinetra-app`, uploaded to Cloudflare R2
(`kinetra-downloads`, served at `https://download.kinetra.live`):

| | Stable | Canary |
|---|---|---|
| Trigger | a tag `vX.Y.Z` on `main` | nightly at 03:00 UTC when `main` changed since the published canary, or by hand (Actions › Canary release › Run workflow, `main` only) |
| Version | `X.Y.Z` (must equal `tauri.conf.json` `version`) | `X.Y.Z-canary.YYYYMMDD.N` (`X.Y.Z` from `tauri.conf.json`, `N` = the workflow run number) |
| App | Kinetra · `no.rorvikmedieteknikk.kinetra` | Kinetra Canary · `no.rorvikmedieteknikk.kinetra.canary` (own config folder, keychain service `Kinetra Canary`, default output port 7771, canary icon): installs next to stable and never shares sign-in, license or settings |
| `KINETRA_CHANNEL` | `stable` | `canary` (update feed `latest-canary.json`; a beta that ends blocks canary builds) |
| Tests before upload | full suite (fmt, cargo audit, clippy, cargo test, check, vitest, e2e) | fast suite (no e2e; cargo audit too) |
| Update manifest | `updates/latest.json` | `updates/latest-canary.json` |

Platforms: **Windows x64** (NSIS `-setup.exe`; with Tauri v2 the updater installs that same
file, signed by its `.sig`) and **macOS universal** (`.dmg` + updater `.app.tar.gz` + `.sig`). macOS is **optional**: a release goes
out with Windows alone when the macOS job fails, and the manifest then lists Windows only.

## Workflows (`.github/workflows/`)

- `canary.yml`: nightly (03:00 UTC; skipped when `main` is still at the commit in
  `releases/canary/latest.json`) or manual (always builds; refuses any branch but `main`) →
  `build.yml` (channel canary) → `publish.yml`. One publish at a time per channel.
- `stable.yml`: tag `v*` → checks the tag is on `main` and equals the config version → build →
  publish.
- `dry-run.yml`: manual (choose channel) → everything a release does (version, tests, builds,
  signing when secrets exist, checksums, manifests), **uploads nothing**; the files are kept as
  workflow artifacts for 7 days.
- `rollback.yml`: manual (channel, version) → points that channel's manifest and `latest.json`
  back to an existing release (its files are checked first). Never deletes anything.
- `build.yml` (reusable): a test job (Windows) and one build job per platform, in parallel:
  checkout, Node + pnpm (cached), Rust (cached with `Swatinem/rust-cache`),
  `tauri build` with the channel config, updater signing, optional code signing, upload
  as workflow artifacts. Publishing waits for all of them (macOS may fail without stopping it).
- `publish.yml` (reusable): downloads the artifacts, writes `SHA256SUMS`, uploads the release
  files (immutable), then `releases/<channel>/latest.json`, then the manifest **last**, then
  verifies from `https://download.kinetra.live`.

Logic lives in small Node scripts without dependencies, unit-tested by `pnpm test`:
- `.github/scripts/release-lib.mjs`: the pure parts (versions, file names, notes, the config
  overlay, minisign key ids, `SHA256SUMS`, manifest, `latest.json`, the upload plan, the rollback
  record check); `release.mjs` runs them (`version`, `validate`, `config`, `collect`, `notes`,
  `assemble`, `verify`, `rollback`); `sign-windows.mjs` is Tauri's Windows sign hook (Azure
  Trusted Signing through `trusted-signing-cli`, its settings read from the environment).
- `scripts/version.mjs`: `pnpm run version:check` / `pnpm run version:set X.Y.Z` (tauri.conf.json,
  Cargo.toml + Cargo.lock, package.json).
- `scripts/canary-icon.mjs` (`pnpm run icon:canary [source.png]`): draws the CANARY band on the
  stable icon in Edge and writes `src-tauri/icons/canary/`. Run it again when the stable icon
  changes.

The canary identity is `src-tauri/tauri.canary.conf.json` (name, identifier, window title,
icons; merged by `tauri build --config`), plus two compile-time defaults from
`KINETRA_CHANNEL=canary` (`core::model::CANARY_BUILD`): output port 7771 for new projects and the
keychain service `Kinetra Canary`. The config folder follows the identifier. The update feed
already follows the channel (`updates.rs`).

## Layout in R2

```
releases/<channel>/<version>/Kinetra_<version>_x64-setup.exe          (canary: Kinetra-Canary_<version>_…)
releases/<channel>/<version>/Kinetra_<version>_x64-setup.exe.sig      (updater signature)
releases/<channel>/<version>/Kinetra_<version>_universal.dmg
releases/<channel>/<version>/Kinetra_<version>_universal.app.tar.gz (+ .sig)
releases/<channel>/<version>/SHA256SUMS
releases/<channel>/<version>/release.json                           (the manifest of this release, for rollback)
releases/<channel>/latest.json                                      (website: direct installer links)
updates/latest.json            (stable updater manifest)
updates/latest-canary.json     (canary updater manifest)
```

- File names never contain spaces (`collect` renames tauri's `Kinetra Canary_…` output), so
  URLs need no escaping.
- Release files: `Cache-Control: public, max-age=31536000, immutable` (a version is never
  overwritten: the upload refuses a version that already exists).
- Manifests and `latest.json`: `Cache-Control: public, max-age=60`.
- Order: files → `SHA256SUMS` → `release.json` → `releases/<channel>/latest.json` →
  `updates/latest*.json`. A client never sees a manifest pointing at missing files.

**Updater manifest** (Tauri v2 static JSON):
```json
{ "version": "0.2.0", "notes": "…", "pub_date": "2026-10-11T12:00:00Z",
  "platforms": {
    "windows-x86_64": { "signature": "<.sig contents>", "url": "https://download.kinetra.live/releases/stable/0.2.0/Kinetra_0.2.0_x64-setup.exe" },
    "darwin-aarch64": { "signature": "…", "url": ".../Kinetra_0.2.0_universal.app.tar.gz" },
    "darwin-x86_64":  { "signature": "…", "url": ".../Kinetra_0.2.0_universal.app.tar.gz" } } }
```
Notes: the version's section in `CHANGELOG.md` (stable), else the tag message; canary: the
commit subjects since the previous canary (the `CHANGELOG.md` "Unreleased" section when present).

**`releases/<channel>/latest.json`** (website download page):
`{ "channel", "version", "commit", "pub_date", "notes", "files": { "windows": {name, url, sha256, size}, "macos": {…} } }`
(`commit` lets the next canary list the commits since this one).

## Checks

Before uploading:
- the version: a stable tag `vX.Y.Z` equals `tauri.conf.json`, `Cargo.toml` and `package.json`
  (`pnpm run version:check`); the tag is on `main`;
- the tests pass (full suite once for stable; fast suite for canary);
- every expected artifact exists and has a `.sig` (updater); the `.sig` is a minisign signature
  whose key id matches the public key in `tauri.conf.json` (the Tauri v2 CLI has no verify
  command; a wrong key would otherwise only show when a client refuses the update);
- the version doesn't exist in R2 yet.

Release builds carry no source maps, and `scripts/release-rustflags.mjs` remaps the Cargo home,
the Rust sysroot and the checkout in panic locations, so no build machine paths end up in the app.

After uploading: fetch `updates/latest*.json` and the Windows installer from
`https://download.kinetra.live` and compare against `SHA256SUMS` (retries for up to 2 min).

## Secrets (repo › Settings › Secrets and variables › Actions)

| Name | Required | What |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | yes | contents of the updater private key file (`pnpm tauri signer generate`), kept offline and never in the repo |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | yes | contents of `kinetra-updater.key.password` |
| `R2_ACCESS_KEY_ID` | yes (publish) | R2 API token, Object Read & Write on `kinetra-downloads` only |
| `R2_SECRET_ACCESS_KEY` | yes (publish) | its secret |
| `R2_ENDPOINT` | yes (publish) | `https://<account-id>.r2.cloudflarestorage.com` (R2 › Overview › S3 API; not the public bucket URL). `.github/scripts/r2-env.sh` trims stray whitespace or a line break from the three R2 secrets and drops a bucket path, and says what's wrong with the endpoint without showing it |
| `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` | optional | Windows code signing (Azure Trusted Signing service principal) |
| `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_SIGNING_PROFILE` | optional | the Trusted Signing endpoint (e.g. `https://weu.codesigning.azure.net`), account and certificate profile |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD` | optional | Developer ID Application certificate (.p12, base64) and its password |
| `APPLE_SIGNING_IDENTITY` | optional | e.g. `Developer ID Application: Rørvik Medieteknikk (TEAMID)` |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | optional | notarization (app-specific password) |

Without the optional ones the job logs `::warning::` "Windows code signing skipped: no Azure
Trusted Signing secrets" (or the macOS equivalent) and builds unsigned installers. Secrets are
only ever passed as environment variables to the steps that need them, never echoed; scripts
never print environment variables.

## How to release

**Canary:** merge to `main`; the next nightly run (03:00 UTC) publishes it. For one now: Actions ›
"Canary release" › Run workflow (branch `main`). It builds, tests, uploads and points
`latest-canary.json` at it (about 15–25 min).

**Stable:**
1. Bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`
   (`pnpm run version:set 0.2.0` does all three) and add a `## 0.2.0` section to `CHANGELOG.md`.
2. Merge to `main`.
3. Tag: `git tag -a v0.2.0 -m "Kinetra 0.2.0" && git push origin v0.2.0`.
4. Watch the "Stable release" workflow; it verifies the download at the end.

**Dry run:** Actions › "Dry run" › Run workflow › channel. Download the artifacts from the run to
try the installers.

## How to roll back

Actions › "Rollback" › channel + version (e.g. `0.1.4`). It checks that
`releases/<channel>/<version>/release.json` and the files it lists exist, then writes that
release's manifest to `updates/latest*.json` and its `latest.json` to
`releases/<channel>/latest.json`. Clients on a newer version don't downgrade (the updater only
installs newer versions); new installs and clients on older versions get the rolled-back one. To
undo a rollback, run it again with the newer version. Nothing is ever deleted.

## Rotating secrets

- **R2 keys:** Cloudflare › R2 › Manage API tokens › create a new token (Object Read & Write,
  `kinetra-downloads` only), replace `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY`, run a dry run
  and a canary, then delete the old token.
- **Updater signing key:** rotating it is a release of its own. Generate a new pair
  (`pnpm tauri signer generate -w <path>`), ship a release that still verifies with the OLD key
  but has the NEW public key in `tauri.conf.json` (sign that release with the old key), then
  switch `TAURI_SIGNING_PRIVATE_KEY` / `_PASSWORD` to the new key for every later release. Clients
  that skip that bridge release must reinstall. Keep the old private key offline until no
  supported client depends on it.
- **Azure Trusted Signing:** rotate the service principal's client secret in Entra ID (App
  registrations › Certificates & secrets), replace `AZURE_CLIENT_SECRET`, delete the old secret.
- **Apple:** export the renewed Developer ID certificate as .p12 (base64), replace
  `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD`; app-specific passwords are revoked and
  replaced at appleid.apple.com (`APPLE_PASSWORD`).

## Supply chain

- Every action is pinned to a full commit SHA with its version in a comment
  (`dtolnay/rust-toolchain` has no releases: a `master` commit, `toolchain: stable` passed in).
  Dependabot (`.github/dependabot.yml`) opens one grouped pull request a week that bumps them;
  review the changelogs it links like any dependency change, and run a dry run before merging a
  major bump.
- R2 keys only reach the publish and rollback jobs; the dry run gets no secrets for assembling.
- macOS uses keyring's `apple-native` feature (the Keychain).

## Icons

The app icon's master is `src-tauri/icons/kinetra-app-icon-1024.png`. After changing it:
`pnpm tauri icon src-tauri/icons/kinetra-app-icon-1024.png` (then delete the `android/` and `ios/`
folders it adds), then `pnpm run icon:canary`.
