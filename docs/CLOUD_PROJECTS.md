# Kinetra cloud projects (v1)

**Status: approved 2026-10-14 (phase 0); built in phases 1–3 (2026-10-14/15). Changes
approved 2026-10-15: multipart parts carry an MD5 instead of a SHA-256 (§6); the app
verifies the SHA-256 of everything it downloads (§0 rule 5, §10).** Written in
kinetra-cloud first; the app repo copies it. Plan and product decisions: `docs/PLAN.md` →
Cloud projects v1. Privacy text: `docs/PRIVACY_CLOUD_PROJECTS.md`.

Cloud backup and version history for app projects. The app uploads a project folder's
files straight to storage (Cloudflare R2, EU) with short-lived URLs from the server, then
commits a **version**: a list of paths and file hashes. Any computer signed in to the same
account can list versions and download one. It is **backup and history, not sync**:
nothing is merged and the server never pushes anything to a running app.

## 0. Rules that override everything else

1. **Never interrupt graphics on air.** Cloud backup runs in the background at low
   priority. While anything is on air the app pauses or limits cloud transfers (§2.2).
   No cloud error, full quota, lapsed plan or outage ever blocks saving, playout, rendering
   or opening a local project, and never shows a modal dialog during a show.
2. **The local project is the primary copy.** The cloud copy is a backup. The app never
   deletes or overwrites local files because of anything the server says; restoring is
   always an explicit user action (§10).
3. **Never merged.** If two computers back up the same project from different starting
   points, both versions are kept and marked as a conflict (§9). The user chooses.
4. **Only file bytes go to R2; only metadata goes to the API.** The app never sends file
   contents to `api.kinetra.live`, and the server never reads them.
5. **Every downloaded file is checked before use.** The app computes the SHA-256 of every
   file it downloads or restores and compares it with the manifest. A mismatch is refused:
   the file isn't used or written into the project, the download is retried once, and if it
   still doesn't match the app reports the file as damaged. Storage only verifies whole
   files for single uploads; for multipart uploads (§6) this check is what catches a
   damaged large file.

## 1. Words

| Word | Meaning |
|---|---|
| **Project** | One app project, identified by the app's project UUID (`app_project_id`) and by the server's `id`. Belongs to one account. |
| **Blob** | The bytes of one file, named by its SHA-256 (lowercase hex). Stored once per account, shared by every version and project that contains the same bytes. |
| **Manifest** | The list of files in a version: path, SHA-256, size, modified time (§8). |
| **Version** | A committed manifest plus metadata (time, computer, app version, label). Immutable. |
| **Head** | A project's newest version. Never expires. |
| **Base** | The version the app last knew as the head when it built a new version. |
| **Kept** | A version the user marked to keep; doesn't expire with retention. |

## 2. App behaviour

### 2.1 When to back up

- Backup is per project and only for accounts whose `GET /v1/cloud/quota` says it's
  available (§4). Off by default per project or on by default is the app team's choice;
  the user can turn it off per project.
- Automatically **`client.backup_interval_s` (default 300 s) after the last change** while
  there are changes not yet backed up, on project close, and on **Back up now**.
- At most one backup per project at a time. A backup that fails is retried later
  (§3.4); it never blocks the next local save.
- Send `reason`: `auto`, `close` or `manual` (§7).

### 2.2 Priority and on-air policy

- One transfer at a time per app, low thread and IO priority; hashing at low priority,
  cached by (path, size, mtime) so unchanged files aren't re-read.
- **While anything is on air** the app applies `client.on_air` from the quota response,
  unless the user changed it in the app's settings:
  - `pause` (default): abort in-flight transfers at once and start no new ones until
    nothing is on air. An aborted part (≤ 16 MiB) is uploaded again later.
  - `limit`: cap cloud transfers to `client.on_air_limit_kbps` (default 2 000 kbit/s).
- Hashing large media also pauses while on air under `pause`.
- Downloads (restore) are user-started; the same policy applies to them.

### 2.3 What is in a backup

- Every regular file under the project folder, except files the app knows are
  regenerable or machine-local: caches, thumbnails, render output, lock and temp files,
  per-computer settings, `.DS_Store`, `Thumbs.db`, `desktop.ini`. The app's exclude list
  is part of the app, not this contract.
- Symlinks are not followed or stored. Files the project uses from **outside** its folder
  aren't backed up; the app shows "N files used by this project are outside the project
  folder and aren't backed up".
- A file that changes while being backed up: before committing, the app checks size and
  mtime again and re-hashes and re-uploads any changed file.
- Paths must follow §8.2. A file whose path can't be stored (e.g. a name Windows can't
  hold) is skipped and listed in the app's backup report, never silently dropped.

### 2.4 Project identity

`app_project_id` must be unique per project. When a project is duplicated ("Save as",
copying the folder, importing a copy) the app **must** give the copy a new UUID, or the
two projects will show up as conflicting versions of one cloud project. The app stores the
server's project `id` and the last known head `version_id` locally with the project, per
account.

### 2.5 States the app shows

Quietly (status area, project menu; never a modal while on air):

| `state` (§4) | App shows |
|---|---|
| `ok` | Last backup time; nothing else |
| `warning` | "Cloud storage is 80 % full." |
| `full` | "Cloud storage is full. Your project is saved on this computer; cloud backup is paused. Free space by deleting old versions or projects, or upgrade." |
| `lapsed` | "Your plan no longer includes cloud backup. Your backups can be downloaded until <purge_at>." |
| `not_in_plan` | Cloud backup is offered as an upgrade, no error |
| server unreachable | "Cloud backup: offline, will retry." |

## 3. Conventions

### 3.1 Transport and auth

- `https://api.kinetra.live/v1/cloud/…`, JSON, conventions as in `docs/API.md` (UUID v7,
  RFC 3339 UTC, `x-request-id`).
- `Authorization: Bearer <app session token>` (`kntr_s_…`), the same session as the rest
  of the API. `User-Agent: Kinetra/<app version> (<os>)`.
- Access is decided by the server from the account's best usable license
  (`pro.cloud_sync` plus the plan's limits), not from the token on the computer.
- Requests to R2 go to the URL the server returned, with exactly the headers it returned,
  and no `Authorization` header.

### 3.2 Sizes

All sizes are integers in bytes. Shown to users as GB = 10⁹ bytes.

### 3.3 Errors

`{"error": {"code", "message", "details"?}}` as in `docs/API.md`. Codes in §11.

### 3.4 Retries

- `429 rate_limited`: wait `Retry-After`.
- `503 commit_incomplete` (commit only): the server confirmed part of the uploads and saved
  that; commit again right away with the same `Idempotency-Key` until it answers
  otherwise. Not a failure, and not counted against the commit rate limit.
- `5xx`, `503 cloud_unavailable`, network errors: exponential backoff from 5 s to 15 min
  with jitter. A backup that can't finish is resumed later from where it stopped (blobs
  already uploaded aren't uploaded again).
- R2 responses: `200` = stored. `403` = URL expired or signature mismatch → ask the server
  for a new URL. `400 BadDigest` = the bytes didn't match the hash → the file changed:
  re-hash and start that file again.

### 3.5 Presigned URLs

Valid for **10 minutes** (`expires_at` in each response), checked by R2 when the request
starts, so a slow upload that started in time finishes. Ask for URLs just before using
them, not for a whole project up front. URLs are secrets until they expire: never log
them (log the path without the query string).

## 4. Quota

### `GET /v1/cloud/quota`

**200**

```json
{
  "plan": "pro_monthly",
  "state": "ok",
  "used_bytes": 7340032000,
  "pending_bytes": 0,
  "limit_bytes": 20000000000,
  "percent": 36,
  "warn_percent": 80,
  "projects": {"used": 12, "limit": 100},
  "max_file_bytes": 5000000000,
  "retention_days": 30,
  "kept_per_project": 50,
  "upload_today": {"used_bytes": 1200000000, "limit_bytes": 40000000000,
                   "resets_at": "2026-10-15T00:00:00Z"},
  "lapse": null,
  "retention_change": null,
  "client": {"backup_interval_s": 300, "on_air": "pause", "on_air_limit_kbps": 2000,
             "multipart_threshold_bytes": 16777216, "part_size_bytes": 16777216}
}
```

- `state`: `ok`, `warning` (≥ `warn_percent`), `full` (≥ 100 %: new file bytes are
  refused), `lapsed` (read-only, §10.2), `not_in_plan` (no cloud backup and nothing
  stored).
- `used_bytes` counts each stored blob once per account, across all projects, versions,
  kept versions and trash, plus manifests. `pending_bytes` = reserved for uploads in
  progress; both count against `limit_bytes`.
- `lapse`: `{"since", "purge_at"}` when lapsed.
- `retention_change`: `{"retention_days", "from"}` after a downgrade (30-day grace).
- `client`: defaults for §2; the user's own settings in the app take precedence.

## 5. Projects

Project object:

```json
{"id": "…", "app_project_id": "…", "name": "Election night",
 "created_at": "…", "updated_at": "…",
 "head": <version summary> | null,
 "versions": 41, "kept": 3, "conflict": false,
 "trashed_at": null, "purge_at": null}
```

`conflict` is true while any version of the project has an unresolved conflict (§9).

### `GET /v1/cloud/projects` (`?trashed=include`)

**200** `{"projects": [<project>, …]}`, most recently updated first. Trashed projects only
with `trashed=include`.

### `POST /v1/cloud/projects`

Register a project, or get it if it's already registered.
`{"app_project_id", "name", "if_trashed": "error" | "restore" | "new"}`
(`if_trashed` default `error`).

**201** new project, **200** existing (name unchanged; rename with `PATCH`).
`409 project_in_trash` (`details: {project_id, purge_at}`) when the only match is in the
trash and `if_trashed` is `error`: the app offers *Restore*, *Start a new cloud backup*
(`new`) or *Turn off backup for this project*. `409 too_many_projects`,
`403 cloud_not_in_plan`, `403 cloud_read_only`.

`name`: 1–200 characters, no control characters. Stored as metadata (see the privacy
text); don't put secrets in it.

### `GET /v1/cloud/projects/{id}` → **200** project

### `PATCH /v1/cloud/projects/{id}` `{"name"}` → **200** project

### `DELETE /v1/cloud/projects/{id}`

Moves the project to the trash. **200** project with `trashed_at` and `purge_at` (7 days).
Backups to it then get `409 project_in_trash`. The trash still counts toward storage.

### `POST /v1/cloud/projects/{id}/restore` → **200** project

From the trash, before `purge_at`. `409 conflict` if another non-trashed project now has
the same `app_project_id`.

## 6. Uploading blobs

### `POST /v1/cloud/blobs/check`

Which of these files does the server still need? `{"blobs": [{"sha256", "size"}, …]}`,
at most 1 000 per call.

**200**

```json
{
  "missing": [
    {"sha256": "…", "size": 482113,
     "upload": {"kind": "put", "url": "https://….eu.r2.cloudflarestorage.com/…",
                "headers": {"content-length": "482113",
                            "x-amz-checksum-sha256": "<base64 of the digest>"},
                "expires_at": "…"}},
    {"sha256": "…", "size": 2147483648,
     "upload": {"kind": "multipart", "upload_id": "…", "part_size": 16777216,
                "part_count": 128}}
  ],
  "reserved_until": "…"
}
```

- Blobs already stored for the account aren't listed: nothing to do.
- The bytes of the missing blobs are **reserved** against storage and today's upload
  allowance until `reserved_until` (24 h). Calling check again for the same blobs extends
  the reservation and returns fresh URLs; for a multipart upload already in progress it
  returns the same `upload_id`.
- Refused as a whole, with nothing reserved: `403 storage_full`, `403 daily_upload_limit`
  (both with `details.quota`), `400 file_too_large` (`details: {sha256, max_file_bytes}`),
  `403 cloud_read_only`, `403 cloud_not_in_plan`. The app then commits what it can
  (§7: a version adding no new bytes is still accepted) or shows the state (§2.5).
- `size` ≤ `multipart_threshold_bytes` → `put`, otherwise `multipart`.

### Single upload

`PUT <url>` with the file's bytes and exactly `upload.headers`. R2 checks length and
SHA-256. Nothing to report back: the commit confirms it (§7).

### Multipart upload (larger files, resumable)

Every part is `part_size` bytes except the last (`part_size` can be larger than
`part_size_bytes` from the quota for very large files: at most 10 000 parts). The app keeps
`upload_id` with the project so it can resume after a restart.

Parts are checked with **MD5**, not SHA-256: R2 doesn't implement SHA-256 checksums on
parts (`501 NotImplemented`, checked 2026-10-15). Storage refuses a part whose bytes don't
match its MD5 (`400 BadDigest`). The whole file's SHA-256 isn't verified by storage for
multipart uploads; a wrong hash can only damage the uploading account's own data.

#### `POST /v1/cloud/uploads/{upload_id}/parts`

`{"parts": [{"number": 1, "md5": "<32 lowercase hex: MD5 of this part>"}, …]}` (1-based, at
most 100 per call). **200** `{"parts": [{"number", "url", "headers", "expires_at"}, …]}`;
`headers` are `content-length` and `content-md5`. `PUT <url>` with that part's bytes and
`headers`; keep the `ETag` response header. `400` for a number outside 1…`part_count` or a
malformed `md5`; `404` if the upload expired or isn't yours.

#### `GET /v1/cloud/uploads/{upload_id}`

**200** `{"sha256", "size", "part_size", "part_count", "parts_done": [{"number", "etag"}],
"expires_at"}`: what R2 already has, to resume.

#### `POST /v1/cloud/uploads/{upload_id}/complete`

`{"parts": [{"number", "etag"}, …]}` (all of them, in order). **200**
`{"sha256", "state": "stored"}` (also for a retry after a completion that went through).
`409 upload_mismatch` if parts are missing, out of order, or the total size is wrong: start
that file again (check, new upload). The blob is confirmed by the next commit, as for a
single upload.

#### `DELETE /v1/cloud/uploads/{upload_id}` → **204**

Abort (e.g. the file changed). Unfinished uploads are aborted by the server after
`expires_at` anyway.

## 7. Committing a version

### `POST /v1/cloud/projects/{id}/versions`

Header `Idempotency-Key: <uuid>` (a retry with the same key returns the first result).

```json
{
  "base_version_id": "…" ,
  "restored_from": null,
  "reason": "auto",
  "label": null,
  "activation_id": "…",
  "app_version": "1.4.0",
  "manifest": {"format": 1, "files": [ … §8 … ]}
}
```

- `base_version_id`: the head the app last knew; `null` for the first version.
- `restored_from`: set when the user restored an older version locally and backs up from
  it (§10); the base is still the head the app knew, so this isn't a conflict.
- `activation_id`: this computer's license activation; the version shows its machine name.
- `label`: optional, ≤ 100 characters.
- Body at most 8 MB (≈ 50 000 files). Bodies over 16 MB get `413 payload_too_large`.
- A project keeps at most 2 000 versions (`max_versions`); beyond that the oldest ones that
  aren't kept, the head or in an open conflict are removed after the commit.

**201** `{"version": <version>, "conflict": null | {"with_version": <version summary>}}`.
The new version is the head. `conflict` is set when `base_version_id` wasn't the head
(§9).

The server confirms uploads it hasn't seen yet in storage during the commit, at most
`commit_check_max` (2 000) per request and for at most a few seconds; each confirmed upload
is saved at once. With more left: `503 commit_incomplete` (`details.remaining`), commit
again with the same key and it carries on (§3.4). Missing blobs are reported before that.

Errors: `409 missing_blobs` (`details.missing`: up to 1 000 SHA-256s that aren't stored
yet: upload them and commit again), `503 commit_incomplete` (`details.remaining`), `400 invalid_path` (`details: {index, reason}`),
`400 too_many_files`, `400 manifest_too_large`, `409 project_in_trash`,
`403 cloud_read_only`, `429 rate_limited` (20 commits per hour per project; idempotent retries, `missing_blobs`
and `commit_incomplete` don't count).

A commit that adds **no new bytes** (only removals, renames or files already stored) is
accepted even when `state` is `full`.

Version object:

```json
{"id": "…", "project_id": "…", "created_at": "…",
 "base_version_id": "…", "restored_from": null, "reason": "auto",
 "machine_name": "STUDIO-PC", "app_version": "1.4.0", "label": null,
 "kept": false, "is_head": true, "damaged": false,
 "file_count": 312, "logical_bytes": 1830000000, "new_bytes": 4200000,
 "conflict": null | {"with_version_id": "…", "resolved_at": null},
 "expires_at": null}
```

`logical_bytes` = sum of the files in this version; `new_bytes` = bytes this version
stored for the first time. `expires_at` = when retention will delete it (null for the head,
kept versions and unresolved conflicts). `damaged` = the server found missing blobs (rare;
admin alerted): it can't be fully restored.

## 8. Manifest

### 8.1 Format

```json
{"format": 1,
 "files": [
   {"path": "project.json", "sha256": "…", "size": 5321,
    "modified_at": "2026-10-14T09:12:44Z"},
   {"path": "assets/intro.mov", "sha256": "…", "size": 2147483648,
    "modified_at": "2026-10-01T18:00:00Z"}
 ]}
```

- `sha256`: of the whole file, 64 lowercase hex. `size` in bytes; empty files are fine.
- `modified_at`: restored as the file's mtime.
- Order doesn't matter; the server stores it sorted by path. Folders are implied by paths;
  empty folders aren't stored.
- Unknown fields are refused in `format: 1`.

### 8.2 Paths

So every version can be restored on Windows and macOS:

- Relative, `/` as separator, UTF-8 in **NFC**, at most 1 024 bytes, each segment at most
  255 bytes.
- No empty segments, `.` or `..`, leading `/`, `\`, NUL or control characters, or any of
  `< > : " | ? *`.
- No segment that ends in a space or a dot, and no Windows reserved names (`CON`, `PRN`,
  `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, with or without an extension, any case).
- Unique ignoring case (`Logo.png` and `logo.png` in one version is refused).
- At most 50 000 files.

## 9. Conflicts

- If `base_version_id` isn't the current head when a commit arrives (another computer
  backed up in between), the commit still succeeds. The new version becomes the head and
  has `conflict.with_version_id` = the previous head. Both are complete versions; nothing
  is merged or dropped.
- The app shows, without interrupting anything: "This project was also backed up from
  *OTHER-PC* at 14:02. Both versions are kept." with *Open the other version* (download
  into a new folder, §10) and *Dismiss*.
- Resolved when the user dismisses it (`PATCH … {"conflict_resolved": true}`) or when a
  later commit has the conflicting head as its base (the user carried on from it). Until
  then both versions are protected from retention and count toward storage.
- After a conflict commit the app's new base is the version it just committed.

## 10. Versions and restore

### `GET /v1/cloud/projects/{id}/versions?before=<version_id>&limit=50`

**200** `{"versions": [<version>, …], "next": "<version_id>" | null}`, newest first,
`limit` ≤ 200.

### `GET /v1/cloud/projects/{id}/versions/{vid}/manifest`

**200** `{"version": <version>, "manifest": {"format": 1, "files": [ … ]}}`.

### `POST /v1/cloud/blobs/download`

`{"sha256s": [ … ]}` (at most 1 000; only those the computer doesn't already have).
**200** `{"urls": [{"sha256", "url", "expires_at"}], "missing": [ … ]}`. `GET <url>`;
verify the SHA-256 of what arrives before using it (§0 rule 5): write to a temporary file,
hash it, and only move it into place when it matches.

### `PATCH /v1/cloud/projects/{id}/versions/{vid}`

`{"kept": true | false, "label": "…" | null, "conflict_resolved": true}` (any of them).
**200** version. `409 too_many_kept` (`details.limit`).

### `DELETE /v1/cloud/projects/{id}/versions/{vid}` → **204**

Any version except the head (`409 cannot_delete_head`; delete the project instead).

### Restoring (app)

- Always an explicit user action, never while the target project is on air.
- Default: download into a **new folder** next to the project (the app gives it a new
  `app_project_id` if the user keeps it as a separate project, §2.4).
- *Replace this project with this version*: only after the app has backed up the current
  local state (or made a local copy), then writes the files and sets `restored_from` on
  the next commit.
- The app reuses local files with the same hash instead of downloading them.
- Every downloaded file is checked against the manifest's SHA-256 before it's used (§0
  rule 5). On a mismatch after one retry the restore stops before touching the project
  and names the damaged files; a restore into a new folder may keep the other files and
  mark the damaged ones missing. A version marked `damaged` by the server is restored the
  same way (its missing files fail the check).

### 10.1 Retention

A version is deleted by the server when it's older than `retention_days` **and** it isn't
the head, kept, or in an unresolved conflict. The head of a project is never deleted by
retention.

### 10.2 Lapsed plan

When the account no longer has cloud backup in its plan: `state: lapsed`, read-only.
List, manifest, download and delete still work; check, upload, commit, register and
`PATCH` return `403 cloud_read_only` (`details.purge_at`). Retention pauses. Everything is
deleted at `purge_at` (90 days after the lapse; emails 30, 7 and 1 day before). A plan
with cloud backup before then restores normal service.

### 10.3 Downgrade

Over the new storage limit → `state: full` until under it; nothing is deleted because of
the limit. The new plan's shorter retention applies from `retention_change.from` (30 days
after the change).

## 11. Error codes

| Status | `code` | When | App |
|---|---|---|---|
| 400 | `bad_request` | Malformed input | Bug; log `x-request-id` |
| 400 | `invalid_path` | §8.2; `details: {index, reason}` | Skip that file, report it |
| 400 | `too_many_files` | > 50 000 files | Tell the user |
| 400 | `manifest_too_large` | Body > 8 MB | Tell the user |
| 400 | `file_too_large` | Over `max_file_bytes`; `details` | Skip that file, report it |
| 401 | `invalid_token` | Session gone | Sign in again |
| 403 | `cloud_not_in_plan` | No cloud backup in the plan | Offer the upgrade |
| 403 | `cloud_read_only` | Lapsed; `details.purge_at` | §2.5 |
| 403 | `storage_full` | Would exceed `limit_bytes`; `details.quota` | §2.5; commit without new bytes |
| 403 | `daily_upload_limit` | Today's upload allowance used; `details.quota` | Retry after `resets_at` |
| 404 | `not_found` | Unknown, purged or another account's | Re-register the project |
| 409 | `project_in_trash` | `details: {project_id, purge_at}` | §5 |
| 409 | `too_many_projects` | Plan limit | Tell the user |
| 409 | `too_many_kept` | Plan limit; `details.limit` | Tell the user |
| 409 | `missing_blobs` | Commit before uploads finished; `details.missing` | Upload, commit again |
| 409 | `upload_mismatch` | Multipart parts or size wrong | Restart that file |
| 409 | `cannot_delete_head` | | Delete the project instead |
| 409 | `conflict` | Restore clashes with another project | Tell the user |
| 429 | `rate_limited` | `Retry-After` | Wait |
| 503 | `cloud_unavailable` | Storage or cloud backup temporarily off | Back off, retry |
| 503 | `commit_incomplete` | Commit confirmed part of the uploads; `details.remaining` | Commit again now, same key; not a failure |

## 12. Limits (defaults, config on the server)

| Plan | Storage | Projects | Max file | Retention | Kept per project | Upload per day |
|---|---|---|---|---|---|---|
| Pro, NFR, nonprofit, beta | 20 GB | 100 | 5 GB | 30 days | 50 | 40 GB |
| Team | 100 GB | 500 | 5 GB | 90 days | 100 | 200 GB |
| Enterprise | 100 GB (custom per account) | 1 000 | 5 GB | 90 days | 100 | 400 GB |
| Free, day pass | none | | | | | |

The app reads the real values from `GET /v1/cloud/quota`; never hardcode them.

## 13. Example: a backup

1. `POST /v1/cloud/projects` → `id` (first time only).
2. Hash changed files (cache by path + size + mtime).
3. `POST /v1/cloud/blobs/check` with the manifest's blobs in batches of 1 000 →
   upload what's missing (`PUT`, or multipart for big files), pausing while on air.
4. `POST /v1/cloud/projects/{id}/versions` with `base_version_id` = last known head →
   store the new `version.id` as the head. If `conflict` is set, show §9.
5. On `409 missing_blobs`, upload those and commit again with the same
   `Idempotency-Key`.

## 14. Changes to this contract

Additive changes (new fields, new error codes, new endpoints) don't change `/v1` or the
manifest `format`; the app ignores unknown response fields and treats unknown error
codes by status. Anything else gets a new manifest `format` or `/v2`, agreed with the app
team first.
