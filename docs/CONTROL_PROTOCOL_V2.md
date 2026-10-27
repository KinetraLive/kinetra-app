<!-- A copy of kinetra-cloud `docs/CONTROL_PROTOCOL_V2.md` (main d646984, 2026-10-27; a DRAFT for review). kinetra-cloud's version wins; refresh this copy when it changes. -->

# Kinetra cloud control protocol 2 (draft)

**Status: draft for review, 2026-10-26; updated 2026-10-27 with the app team's answers**
(kinetra-private `docs/HANDOFF_TO_CLOUD_CONTROL_V2.md`, f174944). Remote control v1.1. Plan
and decisions: `docs/PLAN.md` → Remote control v1.1. What the app builds, in order:
`docs/APP_HANDOFF_CONTROL_V2.md`.

This document lists **only what changes** from protocol 1 (`docs/CONTROL_PROTOCOL.md`).
Everything not mentioned here is exactly as in protocol 1, including §0 (never interrupt
graphics on air, the app is the authority, commands are never queued, off by default per
project). Section numbers in brackets point into protocol 1. When protocol 2 is approved and
built, it is folded into `CONTROL_PROTOCOL.md` and protocol 1 becomes an appendix there until
it's removed.

The app team has answered the handoff's questions; nothing waits on them any more. Marked
**(new)**: what was added from their answers and requests, for review.

## 0. What changes, in short

1. **No anonymous viewers, no guest link.** Only invited collaborators (Kinetra accounts)
   connect. `crew` never contains `anonymous` entries.
2. **Permissions instead of roles.** Every collaborator has a set of permissions chosen by the
   owner (§2). `actor.role` becomes `actor.permissions`; the app checks the permission for each
   action.
3. **New commands:** timers, variants, data source rows, Studio cue/take/cut and the button
   panel (§5).
4. **New state:** variants, data sources, the button panel (§4).
5. **Live preview:** downscaled JPEG pictures of Program (and Preview in Studio mode), on a
   **second WebSocket** that the app opens only while someone watches (§6). The app says how
   many streams it can capture (§6.1).
6. **Collaborators API for the app** (new): the owner invites and manages collaborators from
   the app's remote control panel, not only on the control site (§9).
7. Protocol 1 apps keep working for protocol 1 features for 6 months after the first app
   release with protocol 2 (§8).

## 1. Versions and negotiation [§1, §4, §14]

- The app offers `Sec-WebSocket-Protocol: kinetra-control.v2, kinetra-control.v1` (a protocol 2
  app may list both; the server picks the highest it supports and echoes it). `hello.protocol`
  must match the chosen subprotocol, else fatal `bad_message`.
- `welcome` gets two fields:

  ```json
  {
    "t": "welcome", "protocol": 2, "mode": "run", "…": "as in protocol 1",
    "features": ["permissions", "invites", "preview", "timers", "variants", "rows", "studio",
                 "panel"],
    "limits": { "…": "protocol 1 limits", "preview_frame_bytes": 262144, "preview_fps": 5,
                "preview_streams": 4 }
  }
  ```

  `features` lists what the server has switched on. Features ship on the server in the order
  permissions → preview → the rest, so an app built for all of protocol 2 may meet a server
  that announces only some. The app **only** sends state parts and answers demands for
  announced features, and the server never sends a command or a `preview_demand` for a
  feature it didn't announce. `permissions` is always present in protocol 2. `invites`
  means the collaborators API (§9) is available; until it is, the app's **Invite
  collaborator** button opens the control site's Collaborators page (`crew_url` from
  `registered`).
- `hello` gets one optional field (new), only from apps that can capture the live picture:

  ```json
  { "t": "hello", "protocol": 2, "…": "as in protocol 1", "preview": { "max_streams": 2 } }
  ```

  `max_streams` (1–4) is how many preview streams the app captures at once (each captured
  screen costs the app ≈ 75–100 MB). It can change while connected (`preview_caps`, §6.1). No
  `preview` = the app can't capture (for example Kinetra on macOS today): the server never
  sends `preview_demand` and browsers see "The live picture isn't available on this Kinetra
  computer".
- Compatible additions stay within protocol 2, as in protocol 1 §14.

### IDs (new)

Screen, layer, template, source, cue (flow) and button ids, the cue list id `cues` and variant
ids are stored in the project, so they're the same every time it's opened: the audit and the
control site (for example which screen someone watches) can rely on them across runs.

**Row ids** are the exception: data source rows have no stored id, so the app derives one from
the row's content (a short hash of its cells plus an occurrence number for identical rows). It
is stable across runs while the row is unchanged; editing a row's cells gives it a new id. For
`select_row` that means a row edited since the browser saw it is `not_found`, and the
source's `current` changing id makes `step_row`/`select_row` stale (§5).

## 2. Collaborators and permissions [§8, §9.2]

A **collaborator** is a Kinetra account the owner invited to a show. The owner picks
permissions per collaborator when inviting and can change or revoke them at any time; changes
apply at once. A collaborator without any permission can still follow the show (its state,
but not the live picture, which needs `watch_preview`).

| Permission | Allows |
|---|---|
| `take_clear` | `take`, `clear` |
| `edit_fields` | `update_fields` on a layer that is **not** on air |
| `edit_live_fields` | `update_fields` on a layer that **is** on air (always given together with `edit_fields`) |
| `run_cues` | `run_cue` |
| `studio` | `studio_cue`, `studio_uncue`, `studio_take` |
| `timers` | `timer` |
| `variants` | `set_variant` |
| `rows` | `select_row`, `step_row` (data source rows) |
| `button_panel` | `press_button` |
| `watch_preview` | Watching the live picture (§6). Server-side only; never in a command. |

- The **owner** always has every permission and is the only one who manages collaborators.
- Unknown permission codes must be ignored by the app (they belong to later additions).
- The server checks the permission (with its copy of the state for "on air") before
  forwarding; the app checks again against its real state. A missing permission is `permission`
  (protocol 2's name for `forbidden`); `update_fields` on an on-air layer without
  `edit_live_fields` stays `on_air`.

### `actor` in `command` [§8]

```json
"actor": { "id": "<user uuid>", "name": "Kari N.",
           "permissions": ["edit_fields", "rows", "watch_preview"] }
```

The permissions at the moment the server forwarded the command. `role` is gone in protocol 2.

### `crew` [§9.1]

```json
{
  "t": "crew",
  "connected": [
    { "id": "<user uuid>", "name": "Kari N.", "since_ms": 1791590400000,
      "permissions": ["take_clear", "edit_fields", "edit_live_fields", "run_cues"],
      "watching": [{ "screen": "s1", "kind": "program" }] }
  ]
}
```

Also sent when someone's permissions change or they start or stop watching (debounced 1 s).
`watching` is empty when they don't watch the picture. The app's "remote users connected"
list shows name, a short summary of the permissions ("Full operator", "Data editor", "Watch
only", or "Custom") and whether they're watching.

### Lock remote take [§9.3]

While `remote.take_locked` is on, the app (and the server, before forwarding) rejects with
`locked` everything that changes what's on air:

- `take`, `clear`, `run_cue`, `studio_take`;
- `update_fields`, `timer`, `set_variant` on a layer that is on air;
- `select_row`, `step_row` on a source used by a layer that is on air (§4.2);
- `press_button` on a button with `on_air: true` (§4.3).

`studio_cue` and `studio_uncue` change only what's cued, so they're allowed while locked.

## 3. Messages added or changed (overview)

| Direction | `t` | Section |
|---|---|---|
| server → app | `command` (new actions, `actor.permissions`) | §2, §5 |
| server → app | `crew` (`permissions`, `watching`) | §2 |
| server → app | `preview_demand` | §6.1 |
| app → server | `hello` (`preview.max_streams`) | §1 |
| app → server | `preview_caps` | §6.1 |
| app → server | `preview_state` (per stream) | §6.1 |
| preview socket | `preview_hello`, `preview_welcome`, `credit`, binary frames | §6.2 |

## 4. State additions [§6.1]

### 4.1 Variants

```jsonc
"templates": {
  "<template id>": {
    "name": "Lower third", "fields": [ … ],
    "variants": [ { "id": "default", "name": "Default" }, { "id": "compact", "name": "Compact" } ]
  }
},
"layers": {
  "<layer id>": { "…": "as in protocol 1", "variant": "compact" }
}
```

- `variants`: the template's variants in display order (1–12, ids and names as in
  `TEMPLATE_FORMAT.md` §5). Names only: never HTML, CSS or variable values.
- `variant`: the variant the layer shows now (the template's default when the layer has none).
  Template layers only.
- `variant_next` (new): only on an on-air layer whose variant change waits for the next take
  (the project's **Live edit** setting is "On take", §5); absent otherwise. Browsers show
  "Compact on next take".

### 4.2 Data sources and rows

```jsonc
"sources": {
  "<source id>": {
    "name": "Speakers",                        // 1–100
    "rows": [ { "id": "r1", "label": "Ingrid Vale" }, { "id": "r2", "label": "Kari Nordmann" } ],
    "truncated": false,                        // true: more rows exist than are listed
    "current": "r1"                            // the source's current row, or null
  }
},
"source_order": ["<source id>", "…"],
"layers": {
  "<layer id>": { "…": "as in protocol 1", "sources": ["<source id>"] }
}
```

- Only sources that feed a field from their **current row** (`TEMPLATE_FORMAT.md` §3, bound
  fields); a source bound whole to a `list` field has no current row and isn't listed.
- `rows`: up to 1 000, in the app's order. `label` (1–100): the row's first two text columns
  joined, as Control's sources strip shows it. Row ids are content-derived (§1, IDs). More
  than 1 000 rows → the first 1 000 and `truncated: true`; `step_row` still moves through
  every row.
- `layers.*.sources`: the sources whose current row feeds this layer's bound fields. The
  server uses it for the lock and to show which layers a row change affects.
- The current row belongs to the **source**: one per source, shared by every layer bound to
  it (confirmed by the app team).
- Row labels are show content: like field values they're never stored or logged.

### 4.3 Button panel

```jsonc
"panel": {
  "buttons": {
    "<button id>": {
      "label": "Sponsor in",                   // 1–40
      "color": "blue",                         // grey | red | orange | yellow | green | blue | purple
      "confirm": false,                        // true: the browser asks for a second tap
      "on_air": true                           // its actions can change what's on air
    }
  },
  "order": ["<button id>", "…"]                // ≤ 48 buttons
}
```

- The panel is configured **in the app** and saved with the project; the state only says what
  to show. The actions behind a button are never sent: the browser sends `press_button` with
  the button's id and the app runs its own definition (§5).
- `on_air` is computed by the app from the button's actions (true if any action takes, clears,
  runs a cue, takes in Studio, or changes a layer that is on air when pressed; when unsure,
  true). The lock and the browser's warning colour use it.
- Absent or empty `buttons` = no panel (the control site hides it).

### 4.4 Studio [§6.1 `preview`]

`studio_mode` and `preview` stay as in protocol 1. The app's Studio model (confirmed by the
app team):

- Take is **per screen**. The state carries `"studio": { "take_scope": "screen" }`; `"all"` is
  reserved and Kinetra doesn't send it.
- A screen's Preview is its Program plus what's cued. Cueing changes **one layer's**
  visibility in that screen's Preview and adds to what's cued, never replaces it. A layer can
  be on Program and Preview at once (on air and not cued out).
- A take uses the project's transition: cut, crossfade (with its duration) or out/in. There
  is no separate Cut button locally; a remote `cut` is a take with the cut transition.

### 4.5 Timers

No new state: timer fields already carry their `TimerState` in `values` (protocol 1 §6.1).
Browsers show the running time from it, using the clock offset from ping/pong (§10.1).

### 4.6 Limits [§11]

The snapshot limit stays 2 MB. New limits: sources 32, rows 1 000 per source (labels ≤ 100),
buttons 48 (labels ≤ 40), variants per template 12, preview streams 4 (§6.1).

## 5. Commands [§8]

| `action` | `args` | Permission | Changes what's on air when | Stale if (after `seen_seq`) |
|---|---|---|---|---|
| `take` | `layer` | `take_clear` | always | as protocol 1 |
| `clear` | `layer` | `take_clear` | always | as protocol 1 |
| `update_fields` | `layer`, `values` | `edit_fields` (+ `edit_live_fields` on air) | the layer is on air | as protocol 1 |
| `run_cue` | `cue_list`, `cue` | `run_cues` | always | as protocol 1 |
| `timer` | `layer`, `field`, `op`: `start \| pause \| reset` | `timers` | the layer is on air | the field's value changed |
| `set_variant` | `layer`, `variant` | `variants` | the layer is on air | the layer's `variant` or `variant_next` changed |
| `select_row` | `source`, `row` | `rows` | a layer using the source (`layers.*.sources`) is on air | the source's `current` changed |
| `step_row` | `source`, `delta`: `1 \| -1` | `rows` | as `select_row` | the source's `current` changed |
| `studio_cue` | `screen`, `layer` | `studio` | never | the screen's `preview` changed |
| `studio_uncue` | `screen`, `layer` | `studio` | never | the screen's `preview` changed |
| `studio_take` | `screen`, `mode`: `take \| cut` | `studio` | always | the screen's `preview` or any of its layers' `on_air` changed |
| `press_button` | `button` | `button_panel` | the button's `on_air` | never (see below) |

- **`timer`:** `start` and `pause` as the local buttons; `reset` sets the field back to its
  default `TimerState`, paused, as the local reset. `field` must be a `timer` field of the
  layer's template (`timer` fields stay read-only for `update_fields`). Local "set to a time"
  and changing direction or end action aren't remote in v1.1 (candidates for later).
- **`set_variant`:** exactly as choosing the variant locally. Off air it changes at once. On
  air it follows the project's **Live edit** setting: "On take" stages the variant for the
  next take (`variant_next`), "Immediately" swaps now with the transition Control uses for
  variants (crossfade by default, or out/in). The app never refuses a variant change because
  of an animation: a new change ends a running swap at once. Unknown variant → `not_found`.
  Staged or not, it counts as changing what's on air (lock, §2).
- **`select_row` / `step_row`:** as picking or stepping the row locally; on air the bound
  fields update live. Stepping **stops** at the first and last row (no wrap): a step past the
  end is `ok` and changes nothing, as the local ◀ ▶. `select_row` with a row that doesn't
  exist (or was edited, §1) → `not_found`.
- **`studio_cue` / `studio_uncue` / `studio_take`:** as the local Studio controls (§4.4).
  `mode: take` uses the project's transition, `cut` the cut transition. `studio_take` with
  nothing cued does what the local take does. Outside Studio mode → `busy` with the app's
  message ("Studio mode is off on the show machine").
- **`press_button`:** the app looks up **its own** definition of the button and runs its
  actions in order. The button permission authorises the whole button: the actions are not
  checked against the actor's other permissions (the owner pre-approved them by building the
  button). Every other check applies to each action (lock, target exists, `read_only`,
  `bound`, value validation; no stale check). The first failing action stops the button;
  actions before it stay done (no rollback). The result says which:

  ```json
  { "t": "result", "corr": "…", "ok": false, "code": "not_found",
    "message": "Layer \"Sponsor\" no longer exists.", "details": { "step": 2, "steps": 3 } }
  ```

  Button actions (the app's panel editor offers exactly these): take layer, clear layer, run
  cue, set fixed field values on a layer, set variant, timer start/pause/reset, select row,
  Studio take/cut. 1–8 actions per button. A button whose definition is gone → `not_found`.
  Every action goes through the same paths as the local controls, shows in Control, and the
  app's remote panel lists the press. PANIC and Clear all are never button actions or
  commands: they stay local-only.

### What the app checks [§8, "What the app checks"]

Unchanged order; step 4 becomes **Permission** (per the table above; `on_air` for
`update_fields` on air without `edit_live_fields`), step 5 **Lock** uses the list in §2, and
step 7 **Stale** uses the table above. `press_button` skips step 4 for its actions (only
`button_panel` is checked) and step 7.

### Result codes [§8 `result`]

`permission` replaces `forbidden` in protocol 2 (an app that sends `forbidden` is treated the
same). Browsers can also get `app_too_old` from the server: the show machine runs protocol 1
and the action needs protocol 2 (§8); it never reaches the app.

### Audit [§8 Audit]

As protocol 1, plus the permission used. For `press_button`: the button id, not its actions.
For rows: the source and row ids, never labels. For timers and variants: the field key or
variant id.

## 6. Live preview (new)

Downscaled pictures of what's on Program, and on Preview in Studio mode, so collaborators see
the result of what they do. Only sent **while someone with `watch_preview` watches**, only for
the screens they look at, and never at the cost of the output.

### 6.1 Demand (control socket)

#### `preview_demand` (server → app)

```json
{
  "t": "preview_demand",
  "streams": [
    { "stream": 1, "screen": "s1", "kind": "program" },
    { "stream": 2, "screen": "s1", "kind": "preview" }
  ],
  "fps": 5, "max_width": 640, "quality": 70,
  "url": "wss://api.kinetra.live/v1/control/app/preview",
  "ticket": "<single-use, 60 s; present only when streams isn't empty>"
}
```

- Replaces the previous demand completely. `streams: []` = nobody watches: stop capturing and
  close the preview socket (the server waits 10 s after the last watcher leaves before sending
  it).
- `stream`: a small number the server assigns, used in frame headers; the same (screen, kind)
  keeps its number for the run. `kind: "preview"` only in Studio mode.
- **At most `min(preview.max_streams, limits.preview_streams)` streams** (new). The server
  keeps the streams watchers asked for first; browsers that want the others see "Preview
  limit reached on the Kinetra computer". If a demand still has more streams than the app's
  cap (it lowered the cap just before), the app serves the first ones in the demand's order
  and reports the rest as `limit` in `preview_state`.
- `fps`, `max_width`, `quality` (JPEG 1–100): upper limits; the app may send less.
- If the preview socket is already open, the app ignores `ticket` and just switches streams.

#### `preview_caps` (app → server, new)

```json
{ "t": "preview_caps", "max_streams": 3 }
```

The app's capture cap changed (1–4; same meaning as `hello.preview.max_streams`). The server
sends a new `preview_demand` within the cap. Coalesced like other app messages: at most 1 per
second.

#### `preview_state` (app → server, per stream)

```json
{
  "t": "preview_state",
  "streams": [
    { "stream": 1, "active": true },
    { "stream": 2, "active": false, "reason": "unsupported",
      "message": "This output can't be captured." }
  ]
}
```

The app sends it when what it can deliver changes for any stream of the current demand. It
lists **every** stream of that demand and replaces the previous `preview_state`; a new
demand's streams count as active until the app says otherwise. `reason` for an inactive
stream:

| `reason` | When |
|---|---|
| `busy` | The machine needs its resources for playout (an NDI output dropping frames, late frames, CPU). |
| `disabled` | **Allow remote preview** is off for this project (a per-project switch in the app, on by default). |
| `unsupported` | This output can't be captured. |
| `limit` | Beyond the app's `max_streams` (see above). |

Browsers show the stream's `message` instead of its picture. A stream id not in the current
demand → ignored.

### 6.2 Preview socket

| | |
|---|---|
| URL | `url` from `preview_demand` (`wss://api.kinetra.live/v1/control/app/preview`) |
| Auth | `Authorization: Bearer <app session token>` (as the control socket) **and** the ticket |
| Subprotocol | `kinetra-control-preview.v1` |
| Frames | First text, then **binary** frames app → server; text frames server → app |

```
app                                                      server
 |-- upgrade (Bearer, kinetra-control-preview.v1) ---------->|
 |-- {"t":"preview_hello","run_id":"…","ticket":"…"} ------->|   within 10 s
 |<------------------- {"t":"preview_welcome","credit":2} ---|   (or close 4401)
 |== binary frame ==========================================>|   one per credit
 |<-------------------------------- {"t":"credit","n":1} ----|   when the server is ready for more
 |== binary frame ==========================================>|
 |                    …                                      |
 |<------------------------------------------ close 1000 ----|   demand empty, or run ended
```

- The ticket is bound to the run and the session's account, single use, 60 s.
- **Credit:** the app may send only as many frames as it has credit. The server grants new
  credit as it forwards frames, so a slow connection simply lowers the frame rate instead of
  filling buffers. With no credit, drop frames; never queue them. A frame sent without credit
  → close `4000`.
- Ping/pong as protocol 1 §10.1 (text). No message for 60 s → dead; reopen only if the demand
  still asks for streams, with the backoff of §10.2.
- The server closes the preview socket when the control socket ends, the run ends, or the
  demand becomes empty (the app may also close it then). Close codes as protocol 1 §10
  (`4401` for a bad or used ticket).
- Losing the preview socket never affects the control socket, and the other way round only
  ends the preview.

#### Frame format

A binary WebSocket message: a 20-byte header (big-endian), then the image.

| Bytes | Field |
|---|---|
| 0 | Format version, `1` |
| 1 | Codec: `1` = JPEG |
| 2–3 | `stream` (from `preview_demand`) |
| 4–7 | Frame number per stream, +1 per frame sent |
| 8–15 | Capture time, Unix ms (app clock) |
| 16–17 | Width in pixels |
| 18–19 | Height in pixels |
| 20… | The image (baseline JPEG, sRGB) |

At most `limits.preview_frame_bytes` (256 KB) in total; larger → close `1009`. Unknown
version or codec → close `4000`.

#### What the app sends

- **The output as seen,** scaled to at most `max_width` (keeping the aspect ratio), composited
  over a **checkerboard** where the output is transparent: 16 px squares at the encoded size,
  `#2b2b2b` and `#3a3a3a` (dark, so white text stays visible). A screen with a background
  colour shows it (as its display output does); without one the renderer is transparent and
  gets the checkerboard (as NDI's alpha would).
- **Only changes:** if the picture hasn't changed since the last frame sent on that stream,
  send nothing. The server keeps the latest frame per stream for new watchers and shows
  "unchanged for N s", so a still picture costs no bandwidth at all.
- **At most `fps` frames per second per stream,** fewer when there's no credit. **While
  graphics are on air the app sends at most 1 picture per second per screen** (owner
  decision): a lower rate, not a pause, so `preview_state` stays active.
- **Never at the cost of playout:** capture and encode off the render path, at low priority.
  If capturing would delay a rendered frame or the machine is busy, skip frames or send
  `preview_state` `busy`. Preview is the first thing to go.
- Nothing from the preview is logged or saved by the app.

### 6.3 Server side (summary)

Relays frames to watching browsers; keeps only the latest frame per stream in memory; never
stores, logs or transcodes them. Each browser has one frame slot: a slow browser skips frames,
it never builds a queue. Watchers per run are limited by plan (Pro 3, Team 10, Enterprise 25),
streams per run by the app's `max_streams` and `limits.preview_streams` (4), and total preview
traffic by a server-wide cap (frame rates drop first, then new watchers get "Preview busy").

## 7. Browser leg [§13] (internal, summary)

- `subscribe` takes a ticket only (no viewer link secret). The server sends `permissions`
  (replaces `role`) on subscribe and on every change.
- New messages for the new commands and the panel; `activity` lines for them ("Kari stepped
  Speakers to Ingrid Vale" uses the row label from the state, in memory only).
- Preview: `watch {screen, kinds}` / `unwatch` on the control socket; pictures arrive on a
  separate browser socket `wss://control.kinetra.live/v1/ws/preview` (ticket in the first
  message, binary frames with the same header, an ack per frame). Per-stream state (the app's
  `preview_state` and the server's "Preview limit reached on the Kinetra computer") is
  forwarded per stream.

## 8. Protocol 1 apps [§14]

Protocol 1 apps keep working, for protocol 1 features only, until **6 months after the first
stable app release with protocol 2** (no date yet; betas don't start the clock; the server's `KINETRA_CONTROL_V1_UNTIL`; then a v1 `hello` gets
`4001 unsupported_protocol`, "Update Kinetra to use remote control").

Toward a protocol 1 app the server:

- checks permissions itself, then puts the protocol 1 role that allows exactly that command in
  `actor.role`: `operator` for `take`, `clear`, `run_cue` and `update_fields` on an on-air
  layer; `data_editor` for `update_fields` on a layer that isn't on air. The app's role check
  then passes exactly when the server allowed the command, so **for protocol 1 apps only the
  server checks permissions** (documented in `SECURITY.md`);
- sends `crew` with a display role: `operator` if the collaborator has `take_clear`,
  `edit_live_fields` and `run_cues`, `data_editor` if they have `edit_fields`, else `viewer`;
- refuses every protocol 2 action with `app_too_old` without forwarding it, and never sends
  `preview_demand`. The control site shows those controls disabled with "Update Kinetra on the
  show machine to use this", and the owner sees a banner with the end date while their show
  machine uses protocol 1.

## 9. Collaborators API for the app (new)

So the owner can invite and manage collaborators from the app's remote control panel. The
same records as the control site's Collaborators page: an invite made in the app shows there
and the other way round, and the same rules apply (the invite email, the plan's collaborator
limit, a pending invite to the same address is replaced, changes act at once on open
connections). Announced by `"invites"` in `welcome.features` (§1).

- On `api.kinetra.live`, with the app's session (`Authorization: Bearer kntr_s_…`), like
  `/v1/cloud/*`.
- The show is found by the **project's id** (`project.json` `id`, the `project_id` the app
  sends in `register`) among the signed-in account's own shows. Only the owner's sessions
  reach a show: for anyone else, and for a project that has never been registered for remote
  control, the answer is `404 not_found` (the control site's rule: not yours is 404, never
  403).
- Documented in `docs/API.md` with the error codes when built (phase 1).

| Request | Body | Answer |
|---|---|---|
| `GET /v1/control/projects/{project_id}/collaborators` | | `200` `{"show_id", "limit": {"max", "used"}, "collaborators": [Collaborator]}`, joined first, then pending, each by name/email |
| `POST /v1/control/projects/{project_id}/collaborators` | `{"email", "permissions": [code]}` | `201` Collaborator (`status: "pending"`); the server sends the invite email |
| `PATCH /v1/control/projects/{project_id}/collaborators/{id}` | `{"permissions": [code]}` | `200` Collaborator; a joined collaborator's open connections get the new permissions at once |
| `DELETE /v1/control/projects/{project_id}/collaborators/{id}` | | `204`; a pending invite stops working, a joined collaborator is disconnected at once |

```jsonc
// Collaborator
{
  "id": "0193…",                      // the user id once joined, the invite id while pending
  "status": "joined",                 // joined | pending
  "email": "kari@example.com",
  "name": "Kari Nordmann",            // null while pending
  "permissions": ["edit_fields", "rows", "watch_preview"],
  "invited_at": "2026-10-27T10:00:00Z",   // null for collaborators from before permissions
  "joined_at": "2026-10-27T10:05:00Z",    // null while pending
  "expires_at": null                      // a pending invite's expiry
}
```

- `permissions`: any of the 10 codes (§2), in any order, duplicates ignored; empty is allowed
  (follows the show, no picture). Presets (Full operator, Data editor, Watch only) are only a
  client shortcut; only codes are stored. `edit_live_fields` without `edit_fields` →
  `edit_fields` is added (§2).
- Creating an invite answers the same whether or not the address has a Kinetra account
  (no account enumeration).
- `limit`: the plan's collaborator limit for this show; `used` counts joined collaborators and
  open invites, as the control site does.

| Status | `code` | When | What the app should do |
|---|---|---|---|
| 400 | `bad_request` | Invalid email, your own address, `permissions` missing | Show `message` |
| 400 | `unknown_permission` | A code the server doesn't know; `details.codes` | Bug in the app; drop the code |
| 404 | `not_found` | No show of yours for this project, or no such collaborator | Hide the list; offer the control site |
| 409 | `already_collaborator` | The address has already joined | "Already a collaborator: change their permissions instead" |
| 409 | `collaborator_limit` | Joined + open invites at the plan's limit; `details.max` | "Your plan allows *max* collaborators per show" |
| 429 | `rate_limited` | Invites per owner (the control site's limit, shared), other calls per account | Wait `Retry-After` |

Audited as on the control site (`control_audit`, actor = the owner, the permissions, never
the email in logs).

## 10. Decisions (owner, 2026-10-26)

1. No viewer role and no guest link; only invited collaborators (Kinetra accounts).
2. Per-collaborator permissions, changeable and revocable at any time; only the owner invites
   and manages collaborators.
3. `watch_preview` is its own permission. Preview watchers per run: Pro 3, Team 10,
   Enterprise 25.
4. Preview over transparent areas uses a checkerboard.
5. The button panel is configured in the app; actions: take, clear, run cue, set field
   values, set variant, timer start/pause/reset, select row, Studio take/cut.
6. "List picking" = data source rows only.
7. Studio follows the app's model; the `studio` permission also allows cueing to preview.
8. Protocol 1 apps keep working via the role mapping for 6 months after the first stable app
   release with protocol 2 (no date yet).

Added 2026-10-27 (owner, via the app team):

9. **Invite collaborator** in the app, for the owner only (§9).
10. The app captures at most 2 preview streams by default, configurable up to 4
    (`hello.preview.max_streams`, `preview_caps`); per-stream `preview_state`.
11. While graphics are on air: 1 picture per second per screen, not a pause; the busy rules
    still pause with `busy`.
12. **Allow remote preview** is a per-project switch in the app, on by default (off →
    `disabled`).
13. The "no dropped frames at 5 fps" check runs on a mid-range laptop from around 2020 (4–6
    cores, 16 GB RAM, integrated or entry-level graphics).
14. PANIC and Clear all stay local-only; `button_panel` alone is enough to press a button.
