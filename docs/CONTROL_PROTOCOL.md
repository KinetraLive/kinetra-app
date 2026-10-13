# Kinetra cloud control protocol (v1)

**Status: approved 2026-10-13 (protocol 1).** Written in kinetra-cloud first; the app repo copies
it. Plan and product decisions: `docs/PLAN.md` → M6a.

Cloud control lets people control a running Kinetra app from a browser at
`control.kinetra.live`. The app keeps **one outgoing WebSocket** to the server; browsers connect
to the server too, and the server relays between them. Nothing ever connects in to the show
machine.

This document is the contract for the **app ↔ server** connection (§1–§12a). The browser
leg (§13) is internal to kinetra-cloud and only summarised.

## 0. Rules that override everything else

1. **Never interrupt graphics on air.** Losing the connection, a server restart, a lapsed
   license or any error in this protocol only stops *remote control*. The app never takes
   anything off air, pauses playout, or blocks local operators because of it.
2. **The app is the authority.** The server checks role, lock, size and rate before it
   forwards a command, but the app checks everything again against its real state and decides.
   The server never edits state; it only mirrors what the app sends.
3. **Commands are never queued.** The server forwards a command only while the app is
   connected; otherwise the browser gets `show_offline` at once. The app executes a command
   only when it arrives, never later.
4. **Remote control is off by default per project.** The app connects only for a project
   where the user has turned it on, and disconnects when it's turned off or the project closes.
   The switch stays on for the rest of the app session (also after a run ends), but **every
   app start begins with it off**: the app never connects by itself after a restart.
5. The app shows a **"remote users connected"** indicator (who, which role) whenever anyone is
   connected, and offers **Lock remote take** (§9.3).

## 1. Transport

| | |
|---|---|
| URL | `wss://api.kinetra.live/v1/control/app` (TLS only; `ws://` is refused) |
| Auth | `Authorization: Bearer <app session token>` on the upgrade request (the same `kntr_s_…` session the app uses for the API) |
| Subprotocol | `Sec-WebSocket-Protocol: kinetra-control.v1` (required; the server echoes it) |
| `User-Agent` | `Kinetra/<app version> (<os>)`, as for other API calls |
| Frames | Text frames only, UTF-8 JSON, **one message per frame**. Binary frames → close `4000`. No permessage-deflate. |
| Max frame | 2 MB (a snapshot); see §11 for every limit. Larger → close `1009`. |

Failures before the upgrade are ordinary HTTP responses with the API's JSON error body
(`{"error":{"code","message"}}`): `401 unauthorized` (missing, invalid or revoked session:
sign in again), `400 bad_request` (missing or wrong subprotocol), `429 rate_limited`
(`Retry-After`), `503 server_busy` (`Retry-After`).

## 2. Messages

Every message is a JSON object with a type field `t`. Fields not listed here must be ignored
by the receiver (forward compatibility). IDs:

- Server IDs (`show_id`, `run_id`, user IDs): UUID strings.
- App IDs (screens, layers, templates, cue lists, cues): the app's own stable IDs,
  `[A-Za-z0-9_-]{1,64}`. They must stay the same for the life of the run.
- Times: `*_ms` fields are Unix epoch milliseconds (UTC); durations say their unit.
- **Correlation IDs (`corr`):** every message that asks for an answer (`register`, `command`,
  `ping`, `resync`) carries `corr`, a UUID (v7 preferred) made by whoever started the
  exchange, and every answer to it (`registered`, `result`, `pong`, the `snapshot` after a
  `resync`, an `error`) echoes the same `corr`. A command's `corr` is made by the browser and
  stays the same from the button press to the result, through every log on the way (§12a).

A message the server can't accept (malformed JSON, unknown `t` from the app, a missing or
wrong field, a message out of order) gets an `error` (§10) with the message's `corr` if it had
one and, if `fatal`, the connection closes. Unknown `t` **from the server** must be ignored by the app.

## 3. Connection lifecycle

```
app                                             server
 |-- upgrade (Bearer, kinetra-control.v1) ------->|
 |-- hello --------------------------------------->|   within 10 s of the upgrade
 |<------------------------------------- welcome --|   (or error + close)
 |-- register ------------------------------------>|
 |<---------------------------------- registered --|   (or error, not fatal: §5)
 |-- snapshot ------------------------------------>|   within 10 s of registered
 |-- diff, diff, … ------------------------------->|
 |<--------------------------------- command ------|
 |-- result -------------------------------------->|
 |<-------------------------------------- crew -----|  whenever who's connected changes
 |<-- ping / pong --> both ways every 20 s ------->|
 |-- end ----------------------------------------->|   remote control turned off / project closed
 |<------------------------------- close 1000 -----|
```

A connection carries **one run of one show** (§5). To control another project, end the run
and register again on the same connection, or reconnect.

## 4. Handshake

### `hello` (app → server)

```json
{
  "t": "hello",
  "protocol": 1,
  "app_version": "1.4.0",
  "machine": "<64 hex: the license machine fingerprint>",
  "license_token": "<the current signed license token for this machine>",
  "nonce": "<16–128 random URL-safe characters, new for every hello>",
  "timestamp": 1791590400
}
```

The server checks, in order: protocol supported; the session; the token (signature, `kid`,
bound to `machine`, not revoked, issued to this session's account); the activation it names is
live; the license has `pro.control`; `nonce` unused and `timestamp` within 5 minutes
(as in license activation, `LICENSING.md` §5). Use the token from the latest refresh; if the
license changed, refresh first.

### `welcome` (server → app)

```json
{
  "t": "welcome",
  "protocol": 1,
  "server_version": "kinetra-cloud 0.9.0 (fff6168)",
  "server_time_ms": 1791590400123,
  "heartbeat_s": 20,
  "limits": {
    "snapshot_bytes": 2097152, "diff_bytes": 262144, "diffs_per_s": 60,
    "snapshots_per_s": 1, "result_timeout_ms": 5000
  }
}
```

The app must respect the `limits` it receives (they come from server config and may differ
from the defaults in §11).

Fatal handshake errors: `unsupported_protocol`, `unauthorized`, `entitlement_required`,
`clock_skew` (`details.server_time_ms`), `replayed_request`, `bad_message`, `timeout`.

## 5. Shows and runs

A **show** is saved on the server per (owner account, project): its name, its crew and their
roles, the viewer join link. A **run** is one live session of a show and lives only while the
app is connected (plus the offline window). No screens, layers, values or cues are ever stored.

### `register` (app → server)

```json
{
  "t": "register",
  "corr": "<uuid: echoed in registered or error>",
  "project_id": "<uuid: the project's stable ID>",
  "name": "Nordlys Live 2026",
  "resume_run": "<run_id from the previous connection, or null>",
  "takeover": false
}
```

- `name`: 1–100 characters; updates the show's name.
- `resume_run`: after a reconnect, the run ID from `registered`. If that run is still in its
  offline window (30 min by default) it continues: browsers keep their place and see
  "Reconnected". Otherwise a new run starts (`resumed: false`). Always send a fresh snapshot
  afterwards either way.
- `takeover`: another machine has a live run of this show (e.g. moving to the backup
  computer). With `true`, that machine's connection gets `replaced` (§10) and this one takes
  the run over. Only the same account can do this.

### `registered` (server → app)

```json
{
  "t": "registered",
  "corr": "<corr of the register>",
  "show_id": "0193…",
  "run_id": "0193…",
  "resumed": false,
  "crew_url": "https://control.kinetra.live/s/0193…/crew",
  "controller_url": "https://control.kinetra.live/s/0193…"
}
```

Crew (invites, roles, the viewer link) is managed on the control site; the app links to
`crew_url` ("Manage remote users…").

Non-fatal errors (the connection stays open; the app may retry or show the message):
`show_in_use` (`details.machine_name` of the machine holding the run; retry with `takeover`),
`run_limit_reached` (`details.max`: the plan's live runs are in use), `show_limit_reached`
(`details.max`), `bad_request` (field problems).

### `rename` (app → server)

`{"t":"rename","name":"…"}`: the project was renamed during a run.

### `end` (app → server)

`{"t":"end","reason":"disabled"}`: remote control turned off, project closed or app quitting
(`reason`: `disabled | project_closed | quit | other`). The switch stays as the user left it
(§0.4): after the server ends a run the app shows "Remote control ended" with **Start again**,
and doesn't reconnect by itself. The server ends the run, tells the
browsers "The show machine ended remote control", and closes with `1000`. A connection that
just drops instead leaves the run **offline** until it resumes or the offline window passes.

## 6. State

The app describes what a browser needs to see and control as one JSON tree: the **state**. It
is a view for remote control, not the project file; the app decides what goes in, within the
schema below.

### 6.1 Schema

```jsonc
{
  "remote": { "take_locked": false },          // §9.3
  "screens": {
    "<screen id>": {
      "name": "Main",                          // 1–100
      "layers": ["<layer id>", "…"]            // display order, top first
    }
  },
  "screen_order": ["<screen id>", "…"],
  "templates": {
    "<template id>": {
      "name": "Lower third",                   // 1–100
      "fields": [                              // TEMPLATE_FORMAT.md §3, in display order
        { "key": "name", "label": "Name", "type": "text" },
        { "key": "tags", "label": "Tags", "type": "list",
          "columns": [{ "key": "tag", "label": "Tag" }] }
      ]
    }
  },
  "layers": {
    "<layer id>": {
      "screen": "<screen id>",
      "name": "Lower third 2",                 // 1–100, what operators call it
      "template": "<template id>",
      "on_air": false,                         // see below
      "values": { "name": "Ingrid Vale", "tags": "[{\"tag\":\"Live\"}]" },
      "bound": ["role"],                       // keys fed by a data source: read-only remotely
      "remote_editable": true                  // false: the app doesn't allow remote edits here
    }
  },
  "cue_lists": {
    "<cue list id>": {
      "name": "Show",
      "cues": { "<cue id>": { "name": "Open", "note": "" } },   // name 1–100, note ≤ 500
      "order": ["<cue id>", "…"],
      "current": "<cue id or null>",           // last cue run
      "next": "<cue id or null>"               // what GO runs
    }
  },
  "cue_list_order": ["<cue list id>", "…"],
  "studio_mode": false,                        // the app is in Studio mode
  "preview": {                                 // read-only: what's cued on preview, per screen
    "<screen id>": ["<layer id>", "…"]         // top first; [] or absent = nothing cued
  },
  "assets": [ { "path": "/assets/ingrid.jpg", "name": "ingrid.jpg", "kind": "image" } ]
}
```

- **`on_air`** is `true` from the moment a layer is taken until its out-animation has
  finished: whenever any of it can be seen on the output. Roles and the lock use this value.
- **`values`**: the **effective** value of every field the template declares (the layer's
  value, or the template default when it has none), as strings in the template format's
  encoding (`list` and `timer` as compact JSON strings). `clock` fields have no value (omit).
  Lists ≤ 64 KB, other values ≤ 16 KB.
- **Field definitions** carry `key`, `label`, `type`, and `columns` for lists, `format` for
  clocks. Template HTML, CSS, variables and variants are never sent.
- **`preview`** (read-only in v1): in Studio mode, the layers cued on each screen's preview,
  i.e. what the next take would put on air. Those layers are normal entries in `layers` (with
  `on_air: false` until taken). Outside Studio mode `studio_mode` is `false` and `preview` is
  `{}`. No command reads or changes preview; browsers show it next to program.
- **`assets`**: what an `image` / `video` field may be set to remotely (the browser shows a
  picker of names; no files are sent). `kind` is `image | video`. ≤ 1 000 entries.
- Only objects keyed by ID and leaf arrays (ID lists, `fields`, `bound`, `assets`) are used,
  so every change can be addressed by a path (§7).
- Every ID referenced (a screen's layers, a layer's screen and template, orders) must exist in
  the same state. The server rejects a snapshot or diff that breaks this.

### 6.2 What may be edited remotely (v1)

| Field type | Remote edit |
|---|---|
| `text`, `multiline`, `number`, `color`, `html`, `list` | Yes (the app validates and sanitises exactly as for a local edit; `html` is sanitised by the app's rules) |
| `image`, `video` | Yes, but only to a `path` from `assets` (or `""`) |
| `clock` | No value to edit |
| `timer` | Read-only in v1 (shown, not started/paused remotely); start/pause/reset come in v1.1 |
| Any key in the layer's `bound` | Read-only |
| Any field of a layer with `remote_editable: false` | Read-only |

## 7. Snapshots and diffs

### `snapshot` (app → server)

```json
{ "t": "snapshot", "seq": 0, "state": { … } }
```

A snapshot sent because of a `resync` carries that resync's `corr`.

The whole state. `seq` is a non-negative integer the app chooses (start anywhere; it only
has to increase by 1 per diff after this). Send one after every `registered`, after every
`resync`, and whenever the state is easier to resend than to diff (at most 1 per second).
Browsers that join later get the server's copy; they never cost the app anything.

### `diff` (app → server)

```json
{
  "t": "diff",
  "seq": 43,
  "cause": "<corr of the command that caused it, if any>",
  "ops": [
    { "op": "put", "path": "/layers/l2/on_air", "value": true },
    { "op": "put", "path": "/layers/l7/values/name", "value": "Kari Nordmann" },
    { "op": "del", "path": "/layers/l9" },
    { "op": "put", "path": "/screens/s1/layers", "value": ["l2", "l7"] }
  ]
}
```

- `seq` = previous `seq` + 1, exactly. A gap, a repeat, or a diff before the first snapshot
  → the server sends `resync` and ignores diffs until the next snapshot.
- `path`: a JSON Pointer (RFC 6901: `~1` for `/`, `~0` for `~`) into the state.
  `put` = add or replace (the parent must exist); `del` = remove (the target must exist).
  1–256 ops per diff, applied **in order and atomically**: if one op is invalid, or the
  result breaks §6.1, the whole diff is rejected with `resync`.
- Send a diff **after** the change has happened in the app (e.g. `on_air: true` once the take
  has started). On-air changes should go out within 100 ms; other changes may be batched (at
  most 20 diffs per second recommended, 60 hard limit).
- `cause` (optional, recommended): when a diff is the effect of a remote command, the
  command's `corr`. Lets logs and the browser measure press → result → visible change.
- The server forwards diffs to browsers (coalesced under load) and keeps its copy current.

### `resync` (server → app)

`{"t":"resync","corr":"…","reason":"seq_gap"}`, reasons `seq_gap | invalid_op | invalid_state |
too_large | before_snapshot`. Reply with a `snapshot` (echoing `corr`) within 10 s, or the connection is closed
with `timeout`. Meanwhile browsers keep the last good state with a "Syncing…" note, and
commands still work.

## 8. Commands

### `command` (server → app)

```json
{
  "t": "command",
  "corr": "0193…",
  "action": "update_fields",
  "args": { "layer": "l7", "values": { "name": "Kari Nordmann" } },
  "actor": { "id": "<user uuid>", "name": "Kari N.", "role": "data_editor" },
  "seen_seq": 42,
  "sent_ms": 1791590400456
}
```

| `action` | `args` | Roles | Does |
|---|---|---|---|
| `take` | `layer` | operator | Takes the layer on air, as the local Take button. |
| `clear` | `layer` | operator | Takes the layer off air, as the local Clear button. |
| `update_fields` | `layer`, `values` (1–64 keys → string) | data editor (layer **not on air**), operator (any layer) | Sets the fields, as typing them locally and pressing Update. All or nothing. |
| `run_cue` | `cue_list`, `cue` | operator | Runs that cue, as GO on it locally. The browser's GO sends the `next` cue it sees. |

Clear all is not in v1 (operator only when added). Timer start/pause/reset come in v1.1.

- `actor.role` is the role at the time the server forwarded it. The `actor` is shown in the
  app's remote users panel / activity log if the app has one; `name` is ≤ 40 characters and
  only for display.
- `seen_seq`: the last `seq` the sender's browser had applied when the person pressed the
  button. Used for the stale check below.

### What the app checks (in this order, first failure wins)

1. **Duplicate** `corr` seen in the last 60 s → send the same result again, do nothing.
   (`corr` is also the idempotency key: a browser keeps it when it retries.)
2. Remote control still enabled for this project → else `disabled`.
3. `action` known → else `unsupported`. `args` well-formed → else `invalid`.
4. **Role** per the table → else `forbidden`. For `update_fields` by a data editor on a layer
   that is on air → `on_air`.
5. **Lock**: `take_locked` and the command changes what's on air (`take`, `clear`, `run_cue`,
   `update_fields` on an on-air layer) → `locked`.
6. **Target** exists → else `not_found`. `layer.remote_editable` → else `read_only`. Field
   keys exist on the template → `invalid` (`details.field`); key in `bound` → `bound`;
   `clock`/`timer` or a key the app doesn't allow remotely → `read_only`.
7. **Stale** (the target changed after `seen_seq`):
   - `take`: the layer's `on_air` changed after `seen_seq` → `stale`.
   - `clear`: the layer's `on_air` changed after `seen_seq` → `stale`.
   - `update_fields`: any of the given keys changed after `seen_seq` → `stale`
     (`details.fields`). Other keys changing doesn't matter.
   - `run_cue`: the list's `current` or `next` changed after `seen_seq` → `stale`
     (stops two people pressing GO from running two cues).
   The app tracks, per layer field, per layer's `on_air` and per cue list, the `seq` of the
   last diff that changed it. If `seen_seq` is older than the app's history (e.g. before the
   last snapshot), treat as stale.
8. **Value validation** exactly as local edits (`invalid`, `details.field`, a short
   `message` the browser shows as is).
9. **Same as local:** `take` on a layer that is already on air, and `clear` on one that is
   off air, do exactly what the local Take / Clear buttons do in that situation, and the
   result says what the local action would (`ok`, or the same refusal with its message).

Then execute it. Commands arrive in the order the server received them; process them one at a
time in that order.

### `result` (app → server)

```json
{ "t": "result", "corr": "0193…", "ok": true, "received_ms": 1791590400470, "done_ms": 1791590400492 }
{ "t": "result", "corr": "0193…", "ok": false, "code": "stale",
  "message": "Changed on the show machine.", "details": { "fields": ["name"] } }
```

- `received_ms` / `done_ms` (optional, app clock): when the command arrived and when the app
  answered, for the timing in §12a.
- Reply as soon as the command is accepted and started (a take: when it starts animating),
  within `result_timeout_ms` (5 s). After that the server tells the sender `timeout` ("No
  answer from the show machine; it may still happen. Check the state.") and drops a late
  result.
- The effect itself reaches browsers through `diff`s, not through the result.
- `code` for `ok: false`: `disabled`, `unsupported`, `invalid`, `forbidden`, `on_air`,
  `locked`, `not_found`, `read_only`, `bound`, `stale`, `busy` (the app
  can't do it right now, e.g. a modal dialog is open), `internal`. `message` ≤ 200 characters,
  shown to the sender; never include stack traces or file paths.

### Checked by the server before forwarding (the app never sees these)

Role (with the server's copy of `on_air`), lock, size and rate limits, the app being
connected. Browsers can get: `forbidden`, `on_air`, `locked`, `show_offline`, `rate_limited`,
`too_large`, `timeout`, `busy` (the app's queue is full). The server's checks are a filter,
not a replacement for §8's.

### Audit

The server writes one audit row per command, including rejected ones: show, run, `corr`,
actor, role, action, layer or cue list + cue, **field keys (never values)**, result code, time. Kept 30
days. The app doesn't need to send anything for this.

## 9. Crew, presence and the lock

### 9.1 `crew` (server → app)

Sent after `registered` and whenever someone connects, disconnects or changes role
(debounced 1 s):

```json
{
  "t": "crew",
  "connected": [
    { "id": "<user uuid>", "name": "Kari N.", "role": "operator", "since_ms": 1791590400000 },
    { "id": null, "name": "Guest 3", "role": "viewer", "anonymous": true, "since_ms": 1791590410000 }
  ]
}
```

The app shows **"N remote users connected"** while `connected` isn't empty, and the list on
click (name, role, since). Anonymous viewers have `id: null` and the display name they
entered when joining (optional, ≤ 40 characters), or **"Guest N"** if they left it empty. The
names are shown to the owner (here and on the control site), never to other viewers.

### 9.2 Roles

`viewer` (sees everything, sends nothing; also anonymous viewers via the show's join link),
`data_editor` (`update_fields` on layers not on air), `operator` (everything in §8). The
owner is always an operator. Managed on the control site (`crew_url`).

### 9.3 Lock remote take

A switch in the app. While on, `remote.take_locked` is `true` in the state (send the change
as a diff at once) and the app rejects with `locked` every remote command that changes what's
on air: `take`, `clear`, `run_cue`, and `update_fields` on an on-air layer. Data editors can
still prepare layers that aren't on air. Browsers show LOCKED and disable those controls. The
server also rejects them without forwarding, but the app's check is the one that counts.

## 10. Errors, closing and reconnecting

### `error` (server → app)

```json
{ "t": "error", "code": "show_in_use", "message": "…", "fatal": false,
  "corr": "<corr of the message it answers, if any>", "details": { … } }
```

If `fatal`, the server closes right after with the close code below; the close reason is the
same `code`. Show `message` to the user only where §10's table says so.

| Close | `code` | Meaning | App does |
|---|---|---|---|
| 1000 | `ended` | Run ended (`end`, or ended by the owner on the web) | Stop; show "Remote control ended" with Start again (§5) |
| 1001 | `server_restart` | Server restarting (deploy) | Reconnect (§10.2) |
| 1009 | `too_large` | A frame over the limit | Bug: log, reconnect, send a smaller snapshot |
| 4000 | `bad_message` | Protocol violation | Bug: log, reconnect with backoff |
| 4001 | `unsupported_protocol` | This app is too old | Stop; "Update Kinetra to use remote control" |
| 4401 | `unauthorized` | Session revoked or expired, account deleted | Stop; ask the user to sign in again |
| 4403 | `entitlement_required` | No `pro.control` (or license revoked, or the lapse grace ended) | Stop; show `message` (it explains the plan) |
| 4408 | `timeout` | No hello/snapshot in time, or no pong for 60 s | Reconnect |
| 4409 | `replaced` | Another machine took the run over | Stop; "Remote control moved to <machine>" (`message`) |
| 4410 | `show_deleted` | The owner deleted the show on the web | Stop; next register creates a new show (empty crew) |
| 4412 | `clock_skew` | `timestamp` more than 5 min off | Stop; "Your computer's date or time is wrong" |
| 4413 | `replayed_request` | Nonce reused | Bug: reconnect with a new nonce |
| 4429 | `rate_limited` | Sustained limit abuse | Reconnect after `details.retry_after_s` |
| 4503 | `server_busy` | Server at capacity | Reconnect after `details.retry_after_s` |
| 1006 | (none) | Network drop | Reconnect |

### 10.1 Heartbeat

Either side may ping; the server pings every `heartbeat_s` (20 s) and the app should too.

```json
{ "t": "ping", "corr": "…", "sent_ms": 1791590400000 }
{ "t": "pong", "corr": "…", "ping_sent_ms": 1791590400000,
  "received_ms": 1791590400021, "sent_ms": 1791590400022 }
```

The pinger notes its own clock when the pong arrives (`now_ms`), then:

- **round trip** = (`now_ms` − `ping_sent_ms`) − (`sent_ms` − `received_ms`)
- **clock offset** of the other side ≈ ((`received_ms` − `ping_sent_ms`) + (`sent_ms` − `now_ms`)) / 2

Answer a ping at once (not queued behind diffs). The app shows the round trip in its remote
users panel; the server keeps both for diagnostics (§12a). No message at all for 60 s → the
connection is dead; reconnect. Answer WebSocket-level ping frames as well (libraries do).

### 10.2 Reconnecting

- Backoff 0.5 s, 1, 2, 4, 8, then every 15 s, each ±30 % random jitter; reset after 60 s
  connected. Never give up while remote control is on; show "Remote control reconnecting…"
  quietly (it's not an on-air problem).
- Before a deploy the server sends `{"t":"server_restart","retry_after_ms":3000}` then closes
  with 1001: wait a random 0–`retry_after_ms`, then reconnect.
- On reconnect: `hello` (new nonce) → `register` with `resume_run` → `snapshot`.
- Results for commands from before the drop are lost; the browser already got `timeout` or
  `show_offline`. Don't replay anything.

### 10.3 License changes during a run

- `{"t":"license_warning","ends_ms":…,"message":"…"}`: the license lapsed or was revoked;
  remote control continues until `ends_ms` (12 h grace by default). Show it unobtrusively.
- Then `entitlement_required` (fatal). Local playout is unaffected.

## 11. Limits (defaults; the server's actual values arrive in `welcome.limits`)

| Limit | Default |
|---|---|
| Snapshot | 2 MB serialised |
| Diff | 256 KB, 1–256 ops |
| Snapshots | 1 per second |
| Diffs | 60 per second (20 recommended) |
| Screens / layers / templates | 32 / 1 000 / 200 |
| Fields per template | 64 (as `TEMPLATE_FORMAT.md`) |
| Cue lists / cues per list | 32 / 500 |
| Assets | 1 000 |
| Names | 1–100 characters; cue notes ≤ 500 |
| Field values | lists ≤ 64 KB, others ≤ 16 KB |
| JSON depth | 16 |
| Result timeout | 5 s |
| Handshake / resync reply | 10 s each |
| Offline window (run resumable) | 30 min |
| Max run length | 24 h, then ended with `ended` (register again to continue) |

Per-plan limits (live runs, crew, browsers, commands per second) are checked by the server;
the app sees them only as `register` errors.

## 12. Security notes for the app

- The socket uses the app's normal session token over TLS; never log it, the license token
  or any message content beyond types and counts.
- Treat every command as untrusted input from the internet: validate it exactly like a local
  edit, never evaluate or execute anything from it, never open URLs or files from it
  (`image`/`video` values must be one of the `assets` paths you sent).
- Send only what remote users need. Don't put file system paths, secrets, or data from
  projects other than the open one in the state.
- Remote control must never be able to change project settings, templates, outputs,
  licensing or the remote control switch itself.

## 12a. Diagnostics

What each side records so a problem in a show can be traced from a button press on a phone to
the show machine and back. **Never** field values, state content, tokens, license tokens or
IP addresses, on either side.

- **Server logs (JSON, per run):** `run_id`, `show_id`, connection (`app` / browser number),
  `corr`, message type, size in bytes, timing (queue time in the hub, time to the app, time
  to the result), result and error codes, close codes and reasons, resyncs with their reason,
  round trip and clock offset from pings (sampled once a minute per connection).
- **Admin page per live run** (admin site, Tailscale only): the connected app (app version,
  protocol version, machine name, connected since, round trip, clock offset), connected
  browsers (role, protocol version, since, round trip; names only for members), message
  rates per type and direction (last minute / since start), `seq` and the last snapshot's
  size and age, resync count, and the last 20 errors and rejected commands (code, `corr`,
  time; no values).
- **Control site, "Connection details" panel:** round trip to the server and to the show
  machine, protocol version, server version, show machine connected or not (and since
  when), time since the last update received, last `seq`, and the `corr` of the last
  command, so a crew member can read it to support.
- **The app (recommended):** log message type, `corr`, size and timing per message, close
  codes and reconnects, and show the round trip and the last error in the remote users
  panel. Same rule: no values.

## 13. Browser leg (internal, summary)

Browsers use `wss://control.kinetra.live/v1/ws?show=<show_id>` on the control site's own
origin. The first message is `subscribe` with a single-use ticket (members) or the viewer
link secret (anonymous viewers). The server sends `snapshot` (from its copy), `diff`s
(coalesced), `status` (`live | offline | ended`, app round trip, `syncing`), `role`,
`result` for the sender's commands, and `activity` lines ("Kari took Lower third 2") to
everyone. Commands from browsers carry their own `corr` (UUID, kept on retry) and
`seen_seq`; browsers ping like the app (§10.1) for the Connection details panel. Details live in the code and may change without notice; the app never sees this
leg.

## 14. Versioning

- This is protocol **1** (`kinetra-control.v1`, `protocol: 1`).
- Compatible changes stay in v1: new optional fields, new message types the app may ignore,
  new `action`s (an older app answers `unsupported`), new `code`s (treat unknown failure codes
  as a generic failure with the `message`).
- Anything else is v2 under a new subprotocol; the server supports the previous version for
  at least 6 months after the app release that adopts the new one.

## 15. Example

```text
→ hello      {"t":"hello","protocol":1,"app_version":"1.4.0","machine":"9f…","license_token":"…","nonce":"q8…","timestamp":1791590400}
← welcome    {"t":"welcome","protocol":1,"server_version":"…","server_time_ms":1791590400123,"heartbeat_s":20,"limits":{…}}
→ register   {"t":"register","corr":"a1…","project_id":"77…","name":"Nordlys Live","resume_run":null,"takeover":false}
← registered {"t":"registered","corr":"a1…","show_id":"01…","run_id":"02…","resumed":false,"crew_url":"…","controller_url":"…"}
→ snapshot   {"t":"snapshot","seq":0,"state":{…, "layers":{"l7":{"screen":"s1","name":"Lower third 2","template":"lt","on_air":false,"values":{"name":"Ingrid Vale"},"bound":[],"remote_editable":true}}, …}}
← crew       {"t":"crew","connected":[{"id":"03…","name":"Kari N.","role":"data_editor","since_ms":…}]}
← command    {"t":"command","corr":"04…","action":"update_fields","args":{"layer":"l7","values":{"name":"Kari Nordmann"}},"actor":{"id":"03…","name":"Kari N.","role":"data_editor"},"seen_seq":0,"sent_ms":…}
→ result     {"t":"result","corr":"04…","ok":true}
→ diff       {"t":"diff","seq":1,"cause":"04…","ops":[{"op":"put","path":"/layers/l7/values/name","value":"Kari Nordmann"}]}
← command    {"t":"command","corr":"05…","action":"take","args":{"layer":"l7"},"actor":{"id":"06…","name":"Ola","role":"operator"},"seen_seq":1,"sent_ms":…}
→ result     {"t":"result","corr":"05…","ok":true}
→ diff       {"t":"diff","seq":2,"cause":"05…","ops":[{"op":"put","path":"/layers/l7/on_air","value":true}]}
← command    {"t":"command","corr":"07…","action":"update_fields","args":{"layer":"l7","values":{"name":"Typo"}},"actor":{"id":"03…","name":"Kari N.","role":"data_editor"},"seen_seq":2,"sent_ms":…}
             (never arrives: the server rejects it with on_air before forwarding)
← ping       {"t":"ping","corr":"08…","sent_ms":1791590420000}
→ pong       {"t":"pong","corr":"08…","ping_sent_ms":1791590420000,"received_ms":1791590420019,"sent_ms":1791590420020}
→ end        {"t":"end","reason":"disabled"}
← close 1000 ended
```

## 16. Decisions (owner, 2026-10-13)

1. The remote control switch stays on for the app session, including after a run ends; it is
   off again after every app restart (§0.4, §5).
2. A remote take on a layer already on air (and a clear of one off air) behaves exactly like
   the local action (§8).
3. Timer start/pause/reset come in v1.1.
4. Anonymous viewers enter an optional display name (default "Guest N"), shown to the owner
   (§9.1).
5. Read-only preview state (what's cued in Studio mode) is in v1 (§6.1); nothing else is
   added for now.
6. Diagnostics: timestamped ping/pong, correlation IDs on every request and its answer,
   per-run logs, an admin page per live run, and a Connection details panel (§10.1, §12a).
