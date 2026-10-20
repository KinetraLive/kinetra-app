# Kinetra API

Base URL `https://api.kinetra.live`. Every endpoint is under `/v1`.
This is the part of the Kinetra cloud API the app uses: sign-in, licensing, AI templates and
live polls. The app's client is `src-tauri/src/core/cloud/`.

## Conventions

- JSON in and out (`Content-Type: application/json`). Timestamps are RFC 3339 UTC.
  IDs are UUID v7. Money is integer minor units plus an ISO 4217 currency code.
- Every response carries `x-request-id` (UUID v7). Quote it when reporting a problem.
- Responses are `Cache-Control: no-store` unless an endpoint says otherwise.
- Authenticated endpoints take `Authorization: Bearer <session_token>`, an app session
  from the device-code flow. Tokens are opaque; their prefix (`kntr_s_`) is only for
  recognising leaked tokens.
- Rate limits: every endpoint per client IP (IPv6 per /64); login endpoints also per IP
  and per email address; authenticated endpoints also per account. Over a limit you get
  `429 rate_limited` with `Retry-After` in seconds.

## Errors

Every error has this shape. Match on `code`; `message` is for humans (safe to show) and
may change. Some errors add `details` (listed below).

```json
{"error": {"code": "rate_limited", "message": "Too many requests. Try again shortly."}}
{"error": {"code": "seats_full", "message": "…", "details": {"activations": [ … ]}}}
```

| Status | `code` | Meaning | What the app should do |
|---|---|---|---|
| 400 | `bad_request` | Malformed or invalid input. `message` says what's wrong. | Show `message`. |
| 400 | `authorization_pending` | Device flow: user hasn't approved yet. | Keep polling at `interval`. |
| 400 | `slow_down` | Device flow: polled faster than `interval`. | Wait longer between polls. |
| 400 | `expired_token` | Device flow: the code expired. | Start again. |
| 400 | `access_denied` | Device flow: the user pressed Deny. | Stop; show "sign-in was denied". |
| 401 | `unauthorized` | No credentials sent. | Sign in. |
| 401 | `invalid_token` | Session or code unknown, expired, revoked, or account deleted. | Drop the token; sign in again. |
| 401 | `invalid_credentials` | Admin login failed (link, TOTP or reused code). | Request a new link. |
| 403 | `forbidden` | Authenticated but not allowed. | |
| 404 | `not_found` | No such endpoint or resource. | |
| 405 | `method_not_allowed` | Wrong HTTP method; see `Allow`. | |
| 408 | `timeout` | The request took longer than the server's limit. | Retry later. |
| 409 | `conflict` | State conflict (e.g. restoring an account whose email is in use). | Show `message`. |
| 413 | `payload_too_large` | Request body over the limit. | |
| 415 | `unsupported_media_type` | Body isn't `application/json`. | |
| 429 | `rate_limited` | Too many requests. | Wait `Retry-After` seconds. |
| 500 | `internal_error` | Bug on our side. Details are only in server logs. | Retry later; report `x-request-id`. |
| 503 | `service_unavailable` | Temporarily unavailable (database down, deploy). | Retry later. Never block the user's work on it. |

Licensing codes (`/v1/license/*`, `DELETE /v1/me`):

| Status | `code` | Meaning | What the app should do |
|---|---|---|---|
| 400 | `clock_skew` | Request timestamp more than 5 min off. `details.server_time`. | "Your computer's date or time is wrong", show the difference; keep the current license state. |
| 400 | `replayed_request` | Nonce already used. | Retry with a new nonce (it's a bug if it repeats). |
| 400 | `invalid_license_key` | Key format, typo, unknown, or already used by someone else. | Show `message` as-is (it says which). |
| 403 | `no_license` | No usable license (rare: every account gets Free). | Keep running with `core.app`; sign in again. |
| 403 | `license_expired` | Subscription ended. | "Your Pro subscription ended on …", link to renew. Apply after the show. |
| 403 | `license_revoked` | License revoked (refund, fraud, account deleted). | Free, after the show; "Contact hello@kinetra.live". |
| 409 | `seats_full` | All seats in use, on the license you asked for (`license_id`), or on Free when nothing else fits. `details.activations` (id, machine_name, slot, last_refreshed_at). | Offer "Deactivate *name* and use this computer", or "Use as backup". |
| 409 | `backup_in_use` | The license already has a backup. `details.activations`. | Offer deactivating the existing backup. |
| 409 | `backup_not_included` | The license's plan has no backup computer (Pro, Team). | "This plan doesn't include a backup computer"; offer using a seat (move the plan here). |
| 409 | `activation_revoked` | This computer was deactivated (elsewhere) or the activation is unknown. | Activate again (it's free and unlimited); if that fails, follow its error. |
| 409 | `active_license` | Account deletion needs `confirm_license_revocation`. `details.active_licenses`. | Show the warning, ask again. |
| 503 | `license_service_unavailable` | No usable signing key on the server. | Treat as offline; keep the current token. |

Poll codes (`/v1/polls/*`): `entitlement_required`, `poll_open`, `poll_has_votes`,
`poll_not_open`, `poll_limit_reached`, `poll_monthly_limit_reached`, `poll_expired`; see
**Polls → Errors** for meanings and what the app should do.

Codes are stable: they are never renamed or reused, only added.

## Health

### `GET /v1/health`

**200** `{"status": "ok", "version": "0.1.0"}`
**Errors:** `503 service_unavailable` (the database didn't answer in
time), `429`.

## Sign-in

People create an account and sign in with email + password on the account site
(`https://account.kinetra.live`, see "Account site" below). The app never handles
passwords: it uses the device flow, and the user signs in and approves in the browser.

### `POST /v1/auth/device`

Starts the device-code flow for the desktop app.

Request `{"device_name": "STUDIO-PC"}`. Shown to the user when they approve and in their
device list; control characters removed, max 100 chars. `null` is allowed.

**200**
```json
{
  "device_code": "kntr_dc_…",
  "user_code": "RKPH-XHVM",
  "verification_uri": "https://account.kinetra.live/device",
  "verification_uri_complete": "https://account.kinetra.live/device?code=RKPH-XHVM",
  "expires_in": 600,
  "interval": 5
}
```
Show `user_code` and open `verification_uri_complete` in the browser. Keep `device_code`
secret. **Errors:** `429` (per IP).

### `POST /v1/auth/device/token`

Poll every `interval` seconds until approved, denied or expired.

Request `{"device_code": "kntr_dc_…"}`

**200**
```json
{
  "session_token": "kntr_s_…",
  "token_type": "Bearer",
  "expires_at": "2027-10-09T20:27:47Z",
  "user": {"id": "…", "email": "ola@example.com", "first_name": "Ola", "last_name": "Nordmann",
           "organization": null, "created_at": "…", "last_login_at": "…",
           "email_verified_at": "…", "password_changed_at": "…"}
}
```
The device code works once. Store the session token in the OS keychain.
**Errors:** `400 authorization_pending`, `400 slow_down`, `400 expired_token`,
`400 access_denied`, `401 invalid_token` (unknown or already used), `429`.

App sessions last 365 d and expire after
90 d without use.

### `POST /v1/auth/logout` (auth)

Ends the current session. **204**. **Errors:** `401`.

## Account

### `GET /v1/me` (auth)

**200** `{"id", "email", "first_name", "last_name", "organization", "created_at",
"last_login_at", "email_verified_at", "password_changed_at"}` (`organization` may be
`null`; accounts from before registration existed may have no names). **Errors:** `401`.

### `GET /v1/me/sessions` (auth)

**200**
```json
{"sessions": [{"id": "…", "kind": "app", "device_name": "STUDIO-PC", "created_at": "…",
               "last_seen_at": "…", "expires_at": "…", "current": true}]}
```
`kind` is `app` (desktop app) or `web` (account site). Newest activity first.

### `DELETE /v1/me/sessions/{id}` (auth)

Signs that device out. **204**. **Errors:** `401`, `404` (not yours or already ended).

### `GET /v1/me/export` (auth)

Everything stored about the account (GDPR), as a JSON download
(`Content-Disposition: attachment`): account, sessions, device authorizations, account
events, licenses, AI generations, and the account's polls (`polls`: question, options
with totals, status, dates) with `poll_usage` (polls created per month). Never contains
token values or hashes, or individual votes.

### `DELETE /v1/me` (auth)

Deletes the account. All sessions end immediately, and the account's polls (with all
their votes) are deleted at once; other data is erased permanently after
30 d. Signing in again with the same address creates a
new, empty account.

Request `{"confirm_email": "ola@example.com"}` (must match, case-insensitive)
**200** `{"status": "deleted", "purge_after": "…"}`
**Errors:** `400 bad_request` (confirmation doesn't match), `401`.

### Account deletion with licenses

`DELETE /v1/me` additionally needs `"confirm_license_revocation": true` while the account
has an active license (else `409 active_license`). Deletion revokes every license and
deactivates every computer immediately (audited).

### New sign-in emails

When the desktop app signs in (device flow), the account owner gets an email naming the
device, with a link to sign it out. Password
changes and resets always email a notice.

## Licensing

All `/v1/license/*` calls except `keys`
and `revocations` need the app session. Activate and refresh need a fresh `nonce` (16–128
URL-safe chars) and `timestamp` (unix seconds, within 5 min of the server).
Rate limits: 120/h per account (anti-hammering, not a transfer cap); redeem 5/h per
account and 10/h per IP.

**Plan names.** `plan` is a stable code the app may branch on; `plan_name` is the name to
show (e.g. `business_monthly` → "Kinetra Team"). Show `plan_name`, never the code.

### Free and the best license

Every account has a Free license (created on sign-in). Activate and refresh always put
the computer on the account's best usable license; refresh moves it up (Free → Pro) or
down (Pro ended → Free), in which case the response has a new `activation_id`. When a
better license is full, the response includes:
```json
"upgrade_blocked": {"license_id": "…", "plan": "pro_yearly", "reason": "seats_full",
                    "activations": [{"id", "machine_name", "slot", "last_refreshed_at", …}]}
```
`reason` is `seats_full`, `backup_in_use`, or `backup_not_included` (a backup was asked
for and the better license's plan has no backup computer; `activations` is empty).

### `GET /v1/license/keys` (public)

Signed key set, `Cache-Control: public, max-age=300`. Feed it to
`KeySet::apply_update`.
```json
{"keyset": "<base64url JSON {typ:keyset, v, iat, keys:[{kid, public_key, status}]}>",
 "signatures": [{"kid": "k20261010-3fa1", "sig": "…"}]}
```

### `GET /v1/license/revocations` (public)

`{"revocations": "kntr1.<kid>.<payload>.<sig>"}`, `Cache-Control: public, max-age=300`.
Verify with `revocation::verify_revocations`.

### `GET /v1/license?machine=<fingerprint>`

**200** `{"licenses": [License], "server_time": 1791590400}`; activations on the given
machine have `this_machine: true`. `License`:
```json
{"id", "plan": "pro_yearly", "plan_name": "Kinetra Pro (yearly)", "kind": "subscription", "status": "active",
 "entitlements": ["core.app", "pro.ai", "pro.cloud_sync", "pro.polls", "pro.team"], "seats": 1, "seats_used": 1,
 "backup_included": false, "backup_used": false,
 "starts_at", "ends_at": "2027-10-10T…Z", "updates_until": null, "revoked_at": null,
 "activations": [{"id", "machine_name", "slot": "main", "created_at",
                  "last_refreshed_at", "this_machine": true}]}
```
`kind` is `free` (no dates), `subscription` (`ends_at`) or `perpetual` (`updates_until`).
`backup_included` is `true` only when the plan includes a backup computer (one activation
with `slot: "backup"` that doesn't use a seat): by default Free, Beta, Day Pass, Enterprise,
NFR and Nonprofit, not Pro or Team. Show
"Use as backup" only then; otherwise a backup activation fails with
`409 backup_not_included`. `backup_used` is `true` while a backup is active; it can be
`true` with `backup_included: false` for a backup made before the plan lost it (it keeps
working until deactivated).
Beta licenses (`plan: "beta"`) carry `beta_program_id`. Licenses redeemed from a key carry
`key_hint`: the key masked to its first group, `"KNTR-4B7Q-••••-••••"` (`"KNTR-••••-••••-••••"`
for keys redeemed before 2026-10-10). Only that prefix is stored; the full key is never
shown again. Show it with the `id` in Preferences › License as "License key (redeemed)". `status` is `active`, `expired` or
`revoked`.

### `POST /v1/license/activate`

Request
```json
{"machine": "<64 hex>", "machine_name": "STUDIO-PC", "slot": "main",
 "license_id": null, "nonce": "<random>", "timestamp": 1791590400}
```
`slot` defaults to `main`; `license_id` defaults to the best usable license (and
then a full better license shows up as `upgrade_blocked` instead of an error).

**200**
```json
{"token": "kntr1.…", "server_time": 1791590400, "activation_id": "…", "slot": "main",
 "license": License}
```
Same machine again → same `activation_id`, fresh token.
**Errors:** `400 bad_request` (fingerprint/nonce format), `400 clock_skew`,
`400 replayed_request`, `403 license_expired` / `403 license_revoked` (only for an
explicit `license_id`; otherwise it falls back), `404 not_found` (`license_id` not yours),
`409 seats_full`, `409 backup_in_use`, `409 backup_not_included`, `401`, `429`, `503 license_service_unavailable`.

### `POST /v1/license/refresh`

Request `{"activation_id", "machine", "nonce", "timestamp"}`. **200** same shape as
activate; `activation_id` changes if the computer moved to a better or remaining license.
**Errors:** `409 activation_revoked` (deactivated by the user or an admin, replaced,
unknown, other machine or other account: activate again), `400 clock_skew`,
`400 replayed_request`, `409 seats_full` (nothing left that fits), `401`, `429`, `503`.

### `GET /v1/beta/status?program=<slug>`

Public by slug; with an app session it also includes every beta the account is in.
`{"status": "kntr1.<kid>.<payload>.<sig>"}` (`typ: "beta_status"`), verify with
`beta::verify_beta_status`. Payload `programs`: `[{"id", "slug", "name", "state":
"upcoming"|"active"|"ended", "starts_at", "ends_at", "end_message"}]` (unix times;
`end_message` lines separated by `\n`). Unknown slug → empty list.
`Cache-Control: public, max-age=300` without a session, `no-store` with one.

### `POST /v1/license/deactivate`

Request `{"activation_id"}` (any of the account's computers). **204**. Unlimited.
**Errors:** `404` (not yours or already deactivated), `401`, `429`.

### `POST /v1/license/redeem`

Request `{"key": "kntr 4b7x 9q2z hhhh"}` (any formatting). **200** `License`. Redeeming
your own key again returns the same license.
**Errors:** `400 invalid_license_key` (message says: format, typo, unknown, or already
used), `401`, `429` (hard limits).

## AI

Template generation through the server. All calls need the app
session and the machine fingerprint (Free quotas are counted per machine too).

### `GET /v1/ai/quota?machine=<fingerprint>`

**200**
```json
{"enabled": true, "disabled_for_account": false, "plan": "pro", "plan_name": "Kinetra Pro",
 "generate_template": {"used": 3, "limit": 50, "period": "month", "resets_at": 1793491200,
                       "day_used": 1, "day_limit": 10, "day_resets_at": 1791676800},
 "tweak_template": {"used": 12, "limit": 300, "period": "month", "resets_at": 1793491200}}
```
`plan` is `pro`, `business`, `enterprise`, `beta` or `free` (a stable code); `plan_name` is what to show (`business` is shown as "Kinetra Team").
Enterprise quotas are never below Team's. Free has `"period": "total"` and no reset. `enabled` is
false when AI is switched off globally or for this account (`disabled_for_account: true`).

### Jobs

Generations take from seconds to a few minutes, longer than Cloudflare keeps a request
open, so they run as **jobs**: start one, then poll it.

1. `POST /v1/ai/templates/generate` or `/tweak` validates the input and reserves quota
   right away. Input, quota, budget and busy errors come back immediately (table below).
   Otherwise: **202**
   ```json
   {"job_id": "…", "status": "pending", "poll_after_ms": 2000, "expires_at": "…"}
   ```
2. `GET /v1/ai/jobs/{job_id}?machine=<fingerprint>` every `poll_after_ms`:
   - `{"job_id", "task", "status": "pending", "poll_after_ms"}`, keep polling;
   - `{"job_id", "task", "status": "succeeded", "template": {…}, "model": "…", "quota": {…}}`.
     **The result is returned once and then deleted**; a later poll gets `410 ai_job_gone`.
   - `{"job_id", "task", "status": "declined" | "invalid" | "incomplete" | "failed",
     "error": {"code", "message"}}`, with codes `ai_declined`, `ai_output_invalid`,
     `ai_incomplete`, `ai_unavailable`. None of these count against the quota.
   - `404` for a job that isn't this account's (or doesn't exist); `410 ai_job_gone` once
     fetched or after 1 h.

### `POST /v1/ai/templates/generate`

Request
```json
{"machine": "<64 hex>", "prompt": "Lower third for a football match",
 "template_type": "lower_third", "brand_colors": ["#c8102e", "#ffffff"],
 "fonts": ["Barlow Condensed", "Barlow"], "size": {"width": 960, "height": 200}}
```
Only `machine` and `prompt` are required. Prompt ≤ 2000
characters; up to 4 hex colors
(used as `accent`, `secondary`, … defaults); up to 2 fonts from the curated list
(heading, body; "Inter" is read as IBM Plex Sans, as in the app); `size` is the design box
(16–8192). `template_type` is a hint: lower_third, full_screen, title, bug, ticker,
scoreboard, countdown, schedule, list, other. Unknown fields are rejected. **202** job.

The result's `template` is a package per `docs/TEMPLATE_FORMAT.md`:
`{"manifest": <template.json>, "files": {"<variantId>": {"html", "css"}}}`.

### `POST /v1/ai/templates/tweak`

Request `{"machine", "template": {"manifest": …, "files": …}, "instruction": "Make it blue"}`
(a package valid per the spec; instruction ≤ 2000
characters). **202** job.

**Errors when starting a job:**

| Status | `code` | Meaning | App |
|---|---|---|---|
| 400 | `bad_request` | Input invalid (empty prompt or instruction, bad colors, invalid template to tweak, bad fingerprint) | Show `message` |
| 400 | `ai_input_too_long` | Prompt or instruction over the limit: "The description is too long: at most 2000 characters." `details`: `{"field": "description" \| "instruction", "max_chars"}` | Show `message`; keep the text so the user can shorten it |
| 403 | `ai_disabled_for_account` | An admin switched AI off for this account | Show `message` (AI is disabled for this account; contact hello@kinetra.live); hide AI features |
| 429 | `ai_quota_exhausted` | `details.scope`: `day`, `month` or `total`; `details.resets_at` (unix) for day/month | Show `message`; Free: "Upgrade to Pro" |
| 429 | `ai_busy` | This account already has the maximum jobs running | Wait for the running one |
| 503 | `ai_paused` | The monthly AI budget is used up | "AI is paused for the rest of the month" |
| 503 | `ai_disabled` | AI switched off | Hide or disable AI features |
| 401, 429 `rate_limited` | | as elsewhere | |

Errors reported by a finished job (in its poll response): `ai_declined` ("Couldn't
generate this template, try rephrasing"), `ai_output_invalid`, `ai_incomplete`,
`ai_unavailable`, as listed above.

## Polls

Live polls: the owner creates and runs polls in the app; the audience votes on
`https://poll.kinetra.live/<code>` (see **Voting site** below). All `/v1/polls` calls need
the app session. **Creating, editing and opening need a license with `pro.polls`**
(Pro, Team, Enterprise, betas that include it); listing, results, the stream, close, reset and
delete work without it, so a poll is never stranded when a plan ends. A poll that isn't
the caller's answers **404**, exactly like one that doesn't exist. Owner requests are also
limited per account (120 per minute).

**Limits per plan**:

| Plan | Open at once | Created per calendar month (UTC) | Votes per poll | Options |
|---|---|---|---|---|
| Pro (`pro_*`, `nfr`, `nonprofit`, `day_pass`: any non-beta license with `pro.polls`) | 3 | 30 | 500 | 2–8 |
| Team (`business_monthly`) | 15 | 50 | 2,000 | 2–8 |
| Enterprise | 25 | unlimited | 20,000 | 2–8 |
| Beta | the program's own limits; each unset one is Pro's | | | |

The best plan of the account wins: Enterprise, Team, Pro, then beta. For everyone: question 1–200 characters, options
1–80 characters each (trimmed, no line breaks or control characters, no duplicates
ignoring case), optional duration 30 s – 24 h, lifetime
7 days from creation, then deleted even if open, deletion
12 h after closing. A poll that reaches its vote
limit **stays open** and refuses further votes ("This poll has reached its vote limit");
the owner sees `vote_limit_reached: true`.

**Poll object** (every response and stream event below):
```json
{"id": "0199…", "code": "K7P2QX", "url": "https://poll.kinetra.live/K7P2QX",
 "question": "Who wins tonight?",
 "options": [{"position": 0, "label": "Home", "votes": 12},
             {"position": 1, "label": "Away", "votes": 7}],
 "status": "open", "show_results": true, "duration_seconds": 120,
 "total_votes": 19, "max_votes": 500, "vote_limit_reached": false, "round": 1,
 "created_at": "…", "updated_at": "…", "opened_at": "…", "closes_at": "…",
 "closed_at": null, "delete_after": "…", "expires_at": "…"}
```
- `status`: `draft` (never opened), `open`, `closed`. A timed poll reports `closed` from
  `closes_at` on (voting stops exactly then); `closed_at` is the real end.
- `code`: 6 characters from `23456789ABCDEFGHJKMNPQRSTVWXYZ` (no 0/O, 1/I/L, U), unique.
  Case-insensitive on the voting site.
- `show_results`: voters see the results after voting (and when closed). Default true.
- `max_votes`: per round, from the plan when the poll was last opened.
- `round`: starts at 1; `reset` starts a new round (the same phones may vote again).
- `delete_after`: when the poll and its votes are hard-deleted: `closed_at` + 12 h, or
  `expires_at` while not closed (whichever comes first).

### `GET /v1/polls/quota`

**200**
```json
{"plan": "pro", "plan_name": "Kinetra Pro",
 "limits": {"open_polls": 3, "polls_per_month": 30, "votes_per_poll": 500,
            "options_per_poll": 8, "max_duration_seconds": 86400,
            "lifetime_seconds": 604800, "delete_after_close_seconds": 43200},
 "usage": {"open_polls": 1, "polls_this_month": 4, "polls": 6},
 "month_resets_at": "2026-11-01T00:00:00Z"}
```
`plan` is `pro`, `business`, `enterprise`, `beta`, or `null` without `pro.polls` (then `limits` and
`plan_name` are `null`; `plan_name` is what to show, `business` → "Kinetra Team"; show "Polls are part of Pro"). `polls_per_month: null` = unlimited. `usage.polls`
counts polls that exist now (drafts, open, closed awaiting deletion).

### `GET /v1/polls`

**200** `{"polls": [<poll>, …]}`, newest first.

### `POST /v1/polls`

Request
```json
{"question": "Who wins tonight?", "options": ["Home", "Away", "Draw"],
 "duration_seconds": 120, "show_results": true}
```
`duration_seconds` and `show_results` are optional. Unknown fields are rejected.
**201** `<poll>` with `status: "draft"`. Counts toward this month's polls (deleting it
doesn't give one back).

### `GET /v1/polls/{id}`

**200** `<poll>` with current totals. The plain fallback when the stream isn't available.

### `PATCH /v1/polls/{id}`

Any of `{"question", "options", "duration_seconds", "show_results"}`.
`duration_seconds: null` removes the time limit; a missing field is left as it is.
- `question`, `options`: only while **not open and without votes** (reset first).
- `duration_seconds`: only while not open.
- `show_results`: any time (also while open).

**200** `<poll>`.

### `POST /v1/polls/{id}/open`

Draft or closed → open. Starts the timer when the poll has a duration (`closes_at`, never
after `expires_at`), clears a pending deletion, and takes `max_votes` from the current
plan. Opening an open poll changes nothing. **200** `<poll>`.

### `POST /v1/polls/{id}/close`

Open → closed; `delete_after` = now + 12 h. Closing a closed poll changes nothing.
**200** `<poll>`.

### `POST /v1/polls/{id}/reset`

Deletes this round's votes, zeroes the totals, `round` + 1. Any status. **200** `<poll>`.

### `DELETE /v1/polls/{id}`

Hard delete now, with every vote. **204**.

### `GET /v1/polls/{id}/events` (live results)

Server-Sent Events (`text/event-stream`). Every event's `data` is the whole poll object,
so a client that reconnects or misses an event needs no replay.

| Event | When |
|---|---|
| `snapshot` | First event after connecting. |
| `totals` | Votes changed. Several votes within 250 ms arrive as one event (at most 4 per second). |
| `status` | Opened, closed (by the owner, the timer or the lifetime), reset, edited, or `show_results` changed. |
| `deleted` | The poll was deleted (owner, admin, or the 12-hour purge). The stream then ends. |

```
event: totals
data: {"id":"0199…","status":"open","total_votes":20,…}

:ping
```
- A comment line `:ping` every 30 s keeps Cloudflare and
  nginx from closing an idle connection. Ignore it.
- The stream ends after 12 h, on server restart, or within
  5 min of the session being revoked. **Reconnect** with a short backoff; the first
  event is a fresh `snapshot`. If connecting fails, poll `GET /v1/polls/{id}` every few
  seconds instead.
- At most 5 streams per account at once;
  more → `429 rate_limited`. A closed connection frees its slot.
- Errors before the stream starts are the usual JSON errors (401, 404, 429).

**Errors** (`/v1/polls/*`):

| Status | `code` | Meaning | App |
|---|---|---|---|
| 400 | `bad_request` | Input invalid; `message` says what (e.g. "A poll needs 2 to 8 options on your plan.", "The question is too long: at most 200 characters.") | Show `message` |
| 403 | `entitlement_required` | Creating, editing or opening needs `pro.polls` | "Polls are part of Kinetra Pro"; existing polls stay usable |
| 404 | `not_found` | No such poll, or not yours | Drop it from the list |
| 409 | `poll_open` | Question, options or duration changed while open | Close first |
| 409 | `poll_has_votes` | Question or options changed while the round has votes | Offer reset |
| 409 | `poll_not_open` | Closing a draft | |
| 409 | `poll_limit_reached` | Already `details.max` polls open | "Close a poll to open this one" |
| 409 | `poll_monthly_limit_reached` | `details.max` polls created this month; `details.resets_at` (unix) | Show `message` and the reset date |
| 409 | `poll_expired` | Opening a poll past its lifetime (it's about to be deleted) | Create a new one |
| 401, 429 `rate_limited` | | as elsewhere | |

## Cloud projects

Backup and version history of projects. The full contract (bodies, app behaviour, manifest
and path rules, conflicts) is `docs/CLOUD_PROJECTS.md`, a copy of kinetra-cloud's; this is
the summary. Needs a usable license with `pro.cloud_sync` (not a day pass); without one but
with data stored, the account is read-only (`state: lapsed`). File bytes never come here:
the app uploads to and downloads from storage with presigned URLs (valid 10 minutes). All
bearer auth, rate-limited per account (commits also per project). Projects and versions
that aren't yours answer `404`. The app's client is `src-tauri/src/core/cloud/projects/`.

| Endpoint | Answer |
|---|---|
| `GET /v1/cloud/quota` | **200** state (`ok`, `warning`, `full`, `lapsed`, `not_in_plan`), usage, limits, today's upload, lapse and retention-change dates, client defaults |
| `GET /v1/cloud/projects` (`?trashed=include`) | **200** `{"projects": [...]}` |
| `POST /v1/cloud/projects` `{"app_project_id", "name", "if_trashed"?}` | **201** new / **200** existing; `409 project_in_trash`, `409 too_many_projects` |
| `GET` / `PATCH {"name"}` / `DELETE /v1/cloud/projects/{id}` | **200** project (`DELETE` = to the trash for 7 days) |
| `POST /v1/cloud/projects/{id}/restore` | **200** project; `409 conflict` if another live project has its app ID |
| `POST /v1/cloud/blobs/check` `{"blobs": [{"sha256", "size"}]}` (≤ 1 000) | **200** `{"missing": [{sha256, size, upload}], "reserved_until"}`; `upload` is `{kind: "put", url, headers, expires_at}` or, over 16 MiB, `{kind: "multipart", upload_id, part_size, part_count}`; `403 storage_full` / `daily_upload_limit` (`details.quota`), `400 file_too_large` |
| `POST /v1/cloud/uploads/{id}/parts` `{"parts": [{"number", "md5"}]}` (≤ 100) | **200** `{"parts": [{number, url, headers, expires_at}]}` (each part bound to its length and MD5) |
| `GET /v1/cloud/uploads/{id}` | **200** `{sha256, size, part_size, part_count, parts_done: [{number, etag}], expires_at}` (resume) |
| `POST /v1/cloud/uploads/{id}/complete` `{"parts": [{"number", "etag"}]}` | **200** `{sha256, state: "stored"}`; `409 upload_mismatch` |
| `DELETE /v1/cloud/uploads/{id}` | **204** |
| `POST /v1/cloud/projects/{id}/versions` (`Idempotency-Key`) | **201** `{"version", "conflict"}` (**200** for a retry); `409 missing_blobs` (`details.missing`), `400 invalid_path` / `too_many_files` / `manifest_too_large`, `409 project_in_trash` |
| `GET /v1/cloud/projects/{id}/versions?before=&limit=` | **200** `{"versions", "next"}`, newest first |
| `GET /v1/cloud/projects/{id}/versions/{vid}/manifest` | **200** `{"version", "manifest"}` |
| `PATCH /v1/cloud/projects/{id}/versions/{vid}` `{"kept"?, "label"?, "conflict_resolved"?}` | **200** version; `409 too_many_kept` |
| `DELETE /v1/cloud/projects/{id}/versions/{vid}` | **204**; `409 cannot_delete_head` |
| `POST /v1/cloud/blobs/download` `{"sha256s": [...]}` (≤ 1 000) | **200** `{"urls": [{sha256, url, expires_at}], "missing": [...]}` |

### Errors

| Status | `code` | Meaning | What the app should do |
|---|---|---|---|
| 400 | `invalid_path` | A manifest path breaks the path rules; `details: {index, reason}`. | Skip that file, list it in the backup report. |
| 400 | `too_many_files` | Over 50 000 files in a version. | Tell the user. |
| 400 | `manifest_too_large` | Commit body over 8 MB. | Tell the user. |
| 400 | `file_too_large` | A file over `max_file_bytes`; `details: {sha256, max_file_bytes}`. | Skip that file, list it in the backup report. |
| 403 | `cloud_not_in_plan` | No cloud backup in the plan. | Offer the upgrade; no error styling. |
| 403 | `cloud_read_only` | Lapsed: writes refused; `details.purge_at`. | Show the deletion date; downloads still work. |
| 403 | `storage_full` | Would exceed `limit_bytes`; `details.quota`. | Pause uploads; commit what's already stored. |
| 403 | `daily_upload_limit` | Today's upload allowance used; `details.quota`. | Retry after `upload_today.resets_at`. |
| 409 | `project_in_trash` | The project is in the trash; `details: {project_id, purge_at}`. | Offer Restore / Start a new cloud backup / Turn off backup. |
| 409 | `too_many_projects` | The plan's project limit. | Tell the user. |
| 409 | `too_many_kept` | The plan's kept-versions limit; `details.limit`. | Tell the user. |
| 409 | `missing_blobs` | Committed before every upload finished; `details.missing`. | Upload those, commit again with the same key. |
| 409 | `upload_mismatch` | Multipart parts missing, out of order, or the size is wrong. | Start that file again. |
| 409 | `cannot_delete_head` | The newest version can't be deleted. | Delete the project instead. |
| 503 | `cloud_unavailable` | Cloud backup is switched off or storage didn't answer. | "Cloud backup: offline, will retry"; back off. |
