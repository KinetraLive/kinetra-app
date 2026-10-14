# Kinetra Control API (protocol 1)

Kinetra's local Control API lets other programs on the show computer or the local network
control Kinetra: Bitfocus Companion (the `kinetra-graphics` module), Stream Deck setups and
scripts. It has two parts:

- a **WebSocket** (`/api/control`) that pushes Kinetra's state as it changes and takes commands:
  use it for anything with buttons, tally or feedback;
- **REST** routes for one-off commands from scripts (`curl`, a cron job).

`docs/control-api.fixture.json` is generated from the app's code: the protocol's constants and
a real example of every message and command. Test clients against it. Where it and this page
disagree, the fixture wins (and please report it).

## Switching it on

Preferences › Network & API › **Control API**: the switch (off by default, per computer), the
address to use, **Copy token** and the connected programs. The API is free (no Pro needed).

- **Address.** The Control API runs on Kinetra's output server: `127.0.0.1:7770` by default
  (canary builds `7771`; the port is a project setting). The server exists only while a
  project is open, so with no project open, connections are refused. To reach it from another
  computer, turn on "Allow access from other devices on the network" (Preferences › Server);
  Windows asks once to allow it through the firewall. There is no extra allowlist: the token
  protects it.
- **Token.** One per computer, for every project, kept in the OS keychain. It looks like
  `kt_` + 43 characters. **New token…** replaces it and disconnects every client.

## Every request

Requests go through these checks, in this order:

| Check | Answer |
|---|---|
| The request has an `Origin` header (every browser request does) | `403 {"code":"origin"}` |
| The Control API is off | `503 {"code":"disabled"}` |
| 10 wrong tokens from this IP in the last minute | `429 {"code":"rate_limited"}` until the minute is over |
| `Authorization: Bearer <token>` missing or wrong | `401 {"code":"token"}` |

Web pages can't use the API: browsers send `Origin` on every WebSocket and on cross-site
requests, and can't add the `Authorization` header to a WebSocket. Tokens in the query string
are never accepted.

Errors are `{"code": "…", "message": "…"}`. Match on `code`; `message` is a sentence for
operators and may change.

## WebSocket: `GET /api/control`

Send the token in `Authorization: Bearer <token>` and ask for the subprotocol
`kinetra-control-local.v1` (optional, but the server confirms it). All messages are JSON text
frames; a client message is at most 128 KB.

```
→ hello     {"t":"hello","protocol":1,"client":"companion-module-kinetra-graphics/1.0.0"}
← welcome   {"t":"welcome","protocol":1,"app":"Kinetra","version":"1.0.0","channel":"stable",
             "now":1792000000000,"project":{"name":"Nordlys Live"},
             "features":["studio_take","automation","timer","run_flow"]}
← snapshot  {"t":"snapshot","seq":0,"state":{…}}
← diff      {"t":"diff","seq":1,"cause":"c1","ops":[{"op":"put","path":"/layers/s1main01ab.l1/on_air","value":true}]}
→ command   {"t":"command","corr":"c1","action":"take","args":{"layer":"s1main01ab.l1"}}
← result    {"t":"result","corr":"c1","ok":true}
← result    {"t":"result","corr":"c2","ok":false,"code":"not_found","message":"That graphic doesn't exist."}
→ resync    {"t":"resync"}
```

- **`hello`** must be the first message, within 5 s. `client` (≤ 64 characters) is how
  Preferences lists you; include your name and version.
- **`welcome`**: `protocol` (refuse to work with a higher one than you know), the app's
  version and channel, `now` (the server's clock, epoch ms: use it to correct for clock skew
  when you show media remaining) and `features` (optional commands this version has).
- **`snapshot`**: the whole state (below), on connect and whenever a diff would be too big
  (more than 256 ops), or after you fell behind.
- **`diff`**: what changed since the previous `seq`, as JSON Pointer ops: `put` (set or
  replace the value at `path`) and `del` (remove it). Objects are diffed key by key; arrays and
  plain values are replaced whole. Keys are escaped per RFC 6901 (`~0` for `~`, `~1` for `/`).
  Apply them in order; `seq` goes up by one each time. The first change after a quiet moment is
  sent at once, then at most one diff per 50 ms (several changes become one diff). `cause` is
  the `corr` of the command whose change this diff carries, when there is one.
- **`command`** / **`result`**: `corr` (1–64 characters, yours) comes back in the result.
  Commands run one at a time in the order sent. Up to 16 may wait (`busy` beyond), at most 50
  per second (`rate_limited` beyond). Nothing is queued across reconnects or replayed.
- **`resync`**: asks for a snapshot (at most one per second; a second request waits for its
  slot).
- At most 8 clients connect at once (`503 {"code":"busy"}` before the upgrade beyond that).
- The server pings every 20 s. Answer pings (every WebSocket library does).

### Close codes

| Code | Why | What to do |
|---|---|---|
| 1001 | The server is stopping, or another project opened | Reconnect with backoff |
| 4400 | Not JSON, an unknown message type, a binary frame, or no `hello` in time | Fix the client |
| 4401 | The token was regenerated | Ask the user for the new token |
| 4403 | Disconnected from Preferences | Don't reconnect at once |
| 4408 | 8 clients are connected | Retry later |
| 4409 | The Control API was turned off | Show "Turn on Control API in Kinetra" |

A refused connection (no server, wrong port) means Kinetra isn't running or has no project
open. Reconnect with backoff, e.g. 0.5, 1, 2, 4, 8 s with jitter, and reset after a minute
connected.

## State

The state is one JSON tree. Layer keys are `<screen id>.<layer id>`: they stay the same when
things are renamed (ids are `[A-Za-z0-9_-]`, so the `.` is unambiguous). Store these ids in
buttons, not names.

```jsonc
{
  "screens": { "<screen>": { "name": "Main", "layers": ["<screen>.<layer>", …] } },  // layers top first
  "screen_order": ["<screen>", …],
  "templates": {
    "<template>": {
      "name": "Lower third",
      "fields": [ { "key": "name", "label": "Name", "type": "text" }, … ],  // + columns (list), format (clock)
      "variants": [ { "id": "default", "name": "Default" }, … ],
      "ranges": [ { "key": "radius", "label": "Corner radius", "min": 0, "max": 40, "step": 1 }, … ]
    }
  },
  "layers": {
    "<screen>.<layer>": {
      "kind": "template",
      "screen": "<screen>",
      "name": "Guest lower third",
      "template": "<template>",
      "on_air": false,                  // from take until clear (not until the animation ends)
      "values": { "name": "Ingrid Vale", … },   // every field but clocks, as text
      "bound": ["title"],               // fields fed by a data source: set_fields refuses them
      "pending": false,                 // changes staged for the next take ("On take" live edit)
      "variant": "default",
      "opacity": 100,                   // 0–100
      "ready": { "loaded": 2, "total": 3 } | null   // media in the layer preloaded on the outputs
    },
    "<screen>.<media layer>": {
      "kind": "media",
      "screen": "<screen>", "name": "Intro", "on_air": false, "opacity": 100,
      "media": {
        "type": "video",                // image | video | audio
        "name": "intro.mp4",
        "playback": { "state": "playing", "pos_ms": 0, "at": 1792000000000,
                      "in_ms": 0, "out_ms": null, "duration_ms": 31000 }
      },
      "ready": { "loaded": 1, "total": 1 } | null
    }
  },
  "cue_lists": {
    "cues": {
      "name": "Cues",
      "cues": { "<flow>": { "name": "Open", "note": "", "running": false } },
      "order": ["<flow>", …],
      "current": "<flow>" | null,        // the last cue run (from anywhere)
      "next": "<flow>" | null            // what go_next runs
    }
  },
  "cue_list_order": ["cues"],
  "api_flows": { "<flow>": { "name": "Goal", "armed": true } },   // flows with an API call trigger
  "sources": { "<source>": { "name": "Guests", "kind": "text", "index": 0, "count": 12, "label": "Ingrid Vale" } },
  "source_order": ["<source>", …],      // by name
  "automation_off": false,
  "studio_mode": false,
  "preview": { "<screen>": ["<screen>.<layer>", …] },   // Studio mode: what's cued in Preview
  "assets": [ { "path": "/assets/ingrid.png", "name": "ingrid.png", "kind": "image" } ]
}
```

- **`values`**: the value of every field the template has (its default when the layer has
  none), as text: `list` and `timer` values are compact JSON strings, `clock` fields are left
  out. In Studio mode they are Preview's values (what the next take shows), as in Kinetra's
  Control page.
- **Media position**: while `playback.state` is `playing`, the position at server time `t` is
  `pos_ms + (t - at)`, clamped to the out point (or the duration). Use `welcome.now` to correct
  for your clock.
- **Timers** don't tick in the state: a timer value is `{"run", "at", "base", "dir", "end"}`
  (running since server time `at` from `base` ms, counting `up` or `down`). Compute the display
  yourself.
- **`label`** of a source: the first column of its current row (schedules: the NOW item).
- **`assets`**: what an image or video field may be set to.

## Commands

`layer` is a layer key (`<screen>.<layer>`). Commands go through the same paths as Kinetra's
own buttons, MIDI and OSC: in Studio mode `take`, `clear` and `toggle` cue into Preview, and
"On take" live edit stages edits of on-air layers.

| `action` | `args` | Does |
|---|---|---|
| `take` | `layer` | On air (Studio mode: cue into Preview). |
| `clear` | `layer` | Off air, animated (Studio mode: cue out). |
| `toggle` | `layer` | Take when off air, clear when on. |
| `update` | `layer` | Apply staged changes to an on-air layer (Studio mode: take the screen's Preview). |
| `set_fields` | `layer`, `values` (1–64 keys → text; numbers and booleans are written as text), `take?` | Sets the fields, all or nothing, then takes the layer if `take`. Refused: unknown keys, bound fields (`bound`), clocks and timers (`read_only`, use `timer`), image / video values that aren't `""` or a path from `assets`, `html` with markup Kinetra doesn't allow. |
| `run_cue` | `cue` (a flow id from `cue_lists`) | Runs the cue, as its GO button does (also while Automation is off). |
| `go_next` | – | Runs `cue_lists.cues.next`. `not_found` after the last cue. |
| `clear_screen` | `screen` | Every on-air layer of the screen off air. |
| `clear_all` | – | Every layer of every screen off air. |
| `panic` | – | PANIC: everything off air at once, no animation. |
| `source_step` | `source`, `dir`: `next` \| `prev` | Steps the source's current row. |
| `media` | `layer` (media), `op`: `play` \| `pause` \| `stop` \| `restart` \| `seek`, `sec` (seek) | Transport, on Program at once (also in Studio mode). |
| `set_level` | `layer`, `target`: `opacity` \| `var:<key>` (a range variable), `value` 0–1 | A fader: opacity 0–100 %, or the variable from min to max (snapped to its step). Paced to 30 per second per target, the latest value wins. |
| `studio_take` | `screen`, `transition?`: `cut` \| `auto` (the project's) | Takes the screen's Preview to Program. `not_studio` outside Studio mode. |
| `timer` | `layer`, `field` (a timer field's key), `op`: `start` \| `pause` \| `reset` | Controls a timer. |
| `automation` | `off`: `true` \| `false` \| `"toggle"` | Automation off (the switch next to PANIC): nothing goes on air by itself. |
| `run_flow` | `flow` (an id from `api_flows`), `value?` (text, ≤ 16 KB) | Fires the flow's **API call** trigger with `value`. Runs while Automation is off (like a cue). `disarmed` when the flow is disarmed. |

Refusal codes: `invalid` (bad arguments), `not_found`, `bound`, `read_only`, `not_studio`,
`disarmed`, `rate_limited`, `busy`, `disabled`, `failed` (anything else, with a sentence).

## REST

Same token, same checks. A command answers `200 {"ok": true}` or its refusal as
`{"code", "message"}` with this status: `invalid` 400, `not_found` 404, `bound` /
`read_only` / `not_studio` / `disarmed` 409, `rate_limited` 429, `busy` / `disabled` 503,
`failed` 500. At most 50 commands per second per IP. Bodies are JSON objects (an empty body
counts as `{}`).

```
GET    /api/state                                   → {"state": {…}}  (the tree above)
POST   /api/command                                 {"action": "…", "args": {…}}  (any command)
POST   /api/layers/{screen}/{layer}/take            (also clear, toggle, update)
PATCH  /api/layers/{screen}/{layer}/fields          {"values": {"name": "Ingrid Vale"}, "take": true}
POST   /api/cues/{flow}/run
POST   /api/cues/next
POST   /api/screens/{screen}/clear
POST   /api/clear-all
POST   /api/panic
POST   /api/sources/{source}/next                   (or prev)
POST   /api/flows/{flow}/run                        {"value": "Home 2–1"}  (optional)
```

```sh
TOKEN=kt_…   # Preferences › Network & API › Control API › Copy token
curl -X POST -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7770/api/layers/s1main01ab/l1/take
curl -X PATCH -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"values":{"name":"Ingrid Vale","title":"Host"},"take":true}' \
  http://127.0.0.1:7770/api/layers/s1main01ab/l1/fields
```

## Flows: the API call trigger

A flow whose trigger is **API call** (Flows › add a trigger › API call) runs when a client sends
`run_flow` or `POST /api/flows/{id}/run`. The request's `value` comes out of the trigger's
● value, so one button can, for example, put a score into a graphic and take it. The flow
editor shows the exact request to send, with a Copy button. A flow has at most one API call
trigger; it fires at most 10 times per second.

## Versioning

`protocol` is 1. Additions that old clients can ignore (new state keys, new optional
arguments, new commands listed in `features`) don't change it; anything else does. Clients
should ignore state keys they don't know and refuse a higher protocol.
