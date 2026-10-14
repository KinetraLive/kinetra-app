//! The real output server for Playwright e2e (`e2e/fixture.ts`), without Tauri.
//!
//! `cargo run --example fixture_server -- [--port N]` creates a throwaway project in a temp folder
//! and opens it like the app does (real `AppState`: screens, live state and the project folder
//! watcher; backups off), binds `127.0.0.1:N` (default: any free port) and prints `PORT=<n>`.
//! Then it reads commands from stdin, one per line, and answers each with exactly one line,
//! `OK <cmd>` (or `OK <value>` where noted) or `ERR <message>`.
//!
//! The test screen (`test`, layer `lower-third`):
//! - `show` / `hide`: the test lower third on / off air
//! - `fields <json object>`: merge field values, e.g. `fields {"name":"Grace"}`
//! - `template <presetId> [variantId]`: load a built-in preset (e.g. `template classic-bar`, or
//!   `template lower-third frame`) into the test screen's layer: off air, new `templateRev`,
//!   field values = the field labels, variables at the variant's defaults, the preset's auto-out
//!   (variant: the preset's default unless given)
//! - `vars <json object>`: set variable values as the user sees them, validated like the editor's
//!   (e.g. `vars {"accent":"#ff0000","radius":"8"}`); sends `vars` with the changed keys only
//! - `autoout <ms|off>`: take the layer off air this long after each `show` (any 1..=600000 ms,
//!   so tests can use short ones); `off` disables it. A change re-sends the layer (same rev) so
//!   the renderer's `--auto-out-ms` follows
//! - `clear`: take every layer of the test screen off air
//! - `fadetracking on|off`: remote control's exact visibility (`LiveHub::set_fade_tracking`)
//! - `visible <screenId> <layerId>` → `OK true|false`: whether a project screen's layer can be
//!   seen on an output (on air, or its out-animation not reported finished by every counted
//!   renderer: `LiveHub::visible`; reserved screens like `test` are never tracked)
//!
//! Project screens (the same code paths as the app's commands):
//! - `addtemplate <presetId>`: creates a project template from a preset → `OK <templateId>`
//! - `newscreen <NewScreen json>`: e.g.
//!   `newscreen {"name":"Main","width":1920,"height":1080,"background":"transparent"}`
//!   → `OK <screenId>`
//! - `savescreen <ScreenDoc json>`: validates, writes and applies by the live rules (creates the
//!   screen if the id is new) → `OK <screenId>`. A layer's `sizing` (`fit` | `reflow`, absent =
//!   reflow) and `opacity` (0–100, absent = 100) are content like `rect` (on air they follow
//!   `liveedit` / Studio mode)
//! - `layershow <screenId> <layerId>` / `layerhide <screenId> <layerId>`: put a screen layer on /
//!   off air (`layershow` applies staged changes first)
//! - `rect <screenId> <layerId> <Rect json>`: the drag path (`set_layer_rect`, memory only), e.g.
//!   `rect abc l1 {"x":10,"y":20,"w":640,"h":360}`
//! - `liveedit onTake|immediate`: the project's live-edit mode (default `onTake`)
//! - `dir` → `OK <absolute project dir>` (tests may edit files there; the watcher picks them up)
//!
//! Control and sources (the same code paths as the app's commands):
//! - `addsource <json>`: a text source, e.g.
//!   `addsource {"name":"Guests","columns":[{"key":"name","label":"Name"}],"rows":[{"name":"Ada"}]}`
//!   (`rows` optional; index 0) → `OK <sourceId>`. Bind it with `savescreen` (a layer's
//!   `bindings`, e.g. `"bindings":{"name":{"sourceId":"<id>","column":"name"}}`)
//! - `stepsource <sourceId> <delta>`: Next (`1`) / Prev (`-1`), clamped; bound screens follow the
//!   live rules → `OK <new index>`
//! - `setfields <screenId> <layerId> <json object>`: Control's persisted field edit
//!   (`set_layer_fields`: live rules, coalesced write; bound or unknown keys → `ERR`)
//! - `getsource <sourceId>` → `OK <SourceDoc json>` (`get_source`)
//! - `savesource <SourceDoc json>`: the Sources page's save (`save_source`: validated, written,
//!   bound screens re-applied) → `OK <the saved SourceDoc json>`
//! - `clearscreen <screenId>`: every on-air layer of the screen out (animated)
//! - `clearall`: every on-air layer of every user screen out (animated)
//! - `panic`: every screen (and `test`) off air at once: renderers get `cut`, auto-outs cancelled
//!
//! Templates with list / clock / timer fields, timers and auto-advance (the same code paths
//! as the app's commands):
//! - `addtemplatedoc <json>`: a project template from `{name, html, css, fields, variables?,
//!   autoOutMs?, design?}` (`design` as in `template.json`: `{w, h, x?, y?, sizing}`) (fields as in `template.json`, e.g. a `list` field with `columns`, a `clock` with
//!   `format`, a `timer`), created blank and then saved (validated like the editor's save)
//!   → `OK <templateId>`
//! - `settimer <screenId> <layerId> <key> <TimerAction json>`: e.g.
//!   `settimer abc l1 clock {"action":"set","ms":5000}`, `... {"action":"start"}`,
//!   `... {"action":"configure","dir":"down","end":"hide"}`; saved, on air at once even under
//!   `onTake` → `OK <the timer's TimerState json>`
//! - `autoadvance <sourceId> <AutoAdvance json|null>`: e.g.
//!   `autoadvance src {"everyMs":1000,"loop":true,"transition":"update"}`; armed while a bound
//!   layer is on air → `OK <nextAt epoch ms, or "off" when not armed>`
//!
//! Schedules, recipes and images (the same code paths as the app's commands):
//! - `addschedule <json>`: a schedule source from `{name, items, ripple?, labels?}` (items as in
//!   the source file: `{id, title, start, end, …}` with local date-times `YYYY-MM-DDTHH:MM`)
//!   → `OK <sourceId>`
//! - `scheduleaction <sourceId> <ScheduleAction json>`: e.g.
//!   `scheduleaction src {"do":"delay","itemId":"a","minutes":5}` → `OK <nowId|-> <nextId|->`
//! - `scheduleview <sourceId>` → `OK <ScheduleView json>`
//! - `clock <ms>`: schedules and time-of-day triggers run this many ms ahead of real time (`0` =
//!   real time), so tests can put a boundary seconds away → `OK <schedule now, epoch ms>`
//! - `installrecipe <InstallRecipe json>`: e.g.
//!   `installrecipe {"recipeId":"event-schedule","name":"Day 1","screen":{"kind":"new","name":"Event","width":1920,"height":1080}}`
//!   → `OK <instanceId>`
//! - `recipeinstance <id>` → `OK <RecipeInstance json>`
//! - `relinkrecipe <instanceId> <sourceId>`: points the recipe at another source of its kind
//!   (flows, its layers' bindings, the instance) → `OK <RecipeInstance json>`
//! - `importasset <absolute path>`: copies an image into `assets/` → `OK </assets/<name>>`
//! - `autoadvance` (above) takes the slideshow settings too: `shuffle`, `paused`,
//!   `itemDurations`.
//!
//! Flows (the same code paths as the app's commands):
//! - `saveflow <NewFlow json>`: validates, writes and deploys a new flow → `OK <flowId>`, e.g.
//!   `saveflow {"name":"Go","armed":true,"nodes":[{"id":"c","type":"trigger.cue","config":{}}],"edges":[]}`
//! - `updateflow <SaveFlow json>`: replaces a flow's deployed graph (`save_flow`) → `OK`
//! - `armflow <id> <true|false>`: arm / disarm a flow
//! - `runflow <id> [value]`: run a flow from its Cue trigger (`value` passed as text)
//! - `stopflow <id>`: stop its running instance
//! - `flowstate <id>` → `OK running` or `OK idle`
//! - `flows` → `OK <json array of FlowInfo>`
//! - `flowtemplates` → `OK <json array of FlowTemplateInfo>` (the built-in gallery, in order)
//! - `addflowtemplate <AddFlowTemplate json>`: `add_flow_template` (all flows or none)
//!   → `OK <json array of the created flow ids>`, e.g.
//!   `addflowtemplate {"templateId":"stinger-once","slots":{"stinger":"main/lay1"}}`
//! - `automationoff <true|false>`: `UiState.control.automationOff`, saved like the UI saves it
//! - `uistate` → `OK <UiState json>`
//! - `setuistate <UiState json>`: `set_ui_state` (clamped and saved like the UI's)
//!   → `OK <the stored UiState json>`
//! - `openproject <absolute dir>`: closes the current project and opens that folder (it must hold
//!   a `project.json`), migrating rules and macros to flows like the app does on open → `OK <dir>`
//!
//! Flow editor (the same code paths as the app's commands):
//! - `checkflow <json {nodes, edges, id?}>`: validation without saving (`check_flow`)
//!   → `OK <FlowCheck json>` (`{errors, warnings}`, each `{node, message}`)
//! - `subscribedebug <id>` / `unsubscribedebug <id>`: counted live debug subscriptions
//!   (`subscribe_flow_debug` / `unsubscribe_flow_debug`)
//! - `debugevents` → `OK <json array of the FlowDebug payloads emitted since the last call>`
//!   (`flow-debug` events, ≤ 10 per second per subscribed flow)
//! - `saveflowdraft <id> <json {nodes, edges}>`: the editor's autosave (`save_flow_draft`: limits
//!   and ids only; a draft equal to the deployed graph is dropped) → `OK`
//! - `deployflow <id>` / `discardflowdraft <id>`: `deploy_flow` (validates the draft, `ERR` with
//!   the first error) / `discard_flow_draft`
//! - `deleteflow <id>`: `delete_flow`
//! - `files` → `OK <json array of the project-files-changed payloads since the last call>`
//!   (`{kind, id, change, message}`; edit files under `dir` to trigger the watcher)
//!
//! Template variants (the same code paths as the app's commands):
//! - `addvariant <templateId> <fromVariant> <name…>`: `create_template_variant` (duplicates
//!   `fromVariant`; the name may contain spaces) → `OK <variantId>`
//! - `savevariant <templateId> <variantId> <json {html, css, vars?}>`: `save_template_variant`
//!   (screen layers using it follow by the live rules)
//! - `setvariant <screenId> <layerId> <variant> [crossfade|outIn]`: `set_layer_variant` (saved;
//!   on air it follows `liveedit`: `onTake` stages it, `immediate` swaps with the transition,
//!   default `crossfade`)
//! - `layerview <screenId> <layerId>` → `OK <LiveLayerView json>` (`onAir`, `pending`, `variant`:
//!   with `pending`, the staged one), plus the applied (Program) `sizing` (`fit` | `reflow`),
//!   `opacity` (0–100) and the template's `design` size (`{w, h}` or null)
//! - `writev1template <id> <json {name, html, css, fields?, variables?}>`: writes a schema-1
//!   template folder (`templates/<id>/template.json` v1 + `template.html` + `template.css`) into
//!   the open project, as an older Kinetra did; `openproject <dir>` (the `dir` answer) then
//!   migrates it to variants (`variants/default/`) after a `pre-migrate` backup → `OK <id>`
//! - `gettemplate <templateId>` → `OK <TemplateDoc json>` (every variant)
//! - `listtemplates` → `OK <json array of TemplateSummary>` (`presetUpdate` set when the preset
//!   it was made from ships a newer version)
//! - `updatetemplate <templateId>`: `update_template_from_preset` (a `pre-update` backup, then
//!   the preset's latest looks; screens using it follow the live rules)
//!   → `OK <TemplateUpdated json>` (`{doc, backup}`)
//! - `undotemplateupdate <templateId> <backupId>`: `undo_template_update` (the template as the
//!   backup holds it) → `OK <TemplateDoc json>`
//!
//! Style library (the same code paths as the app's commands):
//! - `presets` → `OK <json array of PresetInfo>` (gallery order, with kind, legacy, variants)
//! - `liststyles` → `OK <json array of StyleInfo>`
//! - `applystyle <style> <project|screenId/layerId> [crossfade|outIn]`: `apply_style` (copies
//!   the variant from library presets, switches layers by the live rules; default `crossfade`)
//!   → `OK <ApplyStyleResult json>`
//! - `undostyle <token>`: `undo_apply_style` (`ERR Nothing to undo` for a stale token)
//! - `brand <BrandKit json>`: `save_brand_kit` (validated; every screen re-applies)
//!   → `OK <the stored BrandKit json>`
//!
//! Studio mode (Preview / Program; the same code paths as the app's commands):
//! - `studio <StudioSettings json> [take|discard|keepStaged]`: `set_studio`, e.g.
//!   `studio {"enabled":true,"previewAfterTake":"keep","transition":{"kind":"crossfade","ms":500}}`;
//!   turning it off while a Preview has changes needs the exit (`ERR Preview has changes`
//!   otherwise)
//! - `studioop <screenId> <StudioOp json>`: `studio_op` (the single studio entry point), e.g.
//!   `studioop abc {"op":"cue","layerId":"l1","visible":true}`, `studioop abc {"op":"take"}`,
//!   `... {"op":"take","transition":{"kind":"cut","ms":100}}`, `... {"op":"tbarBegin"}`,
//!   `... {"op":"tbarSet","p":0.5}`, `... {"op":"tbarEnd","commit":true}`, `... {"op":"clear"}`
//!   → `OK <PreviewView json>`
//! - `preview <screenId>` → `OK <PreviewView json>`
//!
//! MIDI / OSC control (the same code paths as the app's commands and the MIDI / OSC inputs):
//! - `savecontrols <ControlsDoc json>`: `save_controls` (validated, written coalesced)
//!   → `OK <the stored ControlsDoc json>`
//! - `getcontrols` → `OK <ControlsDoc json>`
//! - `runcontrol <ControlTarget json> <press|release|level:<0..1>>`: `run_control`, the one entry
//!   point MIDI and OSC use, e.g. `runcontrol {"kind":"command","commandId":"live.toggle:abc/l1"} press`,
//!   `runcontrol {"kind":"value","screenId":"abc","layerId":"l1","target":{"kind":"opacity"}} level:0.5`
//!   (values are paced: ≤ 30 applies per second per target, the latest wins)
//! - `setosc <OscSettings json>`: the project's OSC settings (`set_project_settings`), e.g.
//!   `setosc {"enabled":true,"port":9123,"allow":[],"feedback":{"host":"127.0.0.1","port":9124}}`;
//!   the real listener (`server::osc`) follows on 127.0.0.1 (0.0.0.0 with LAN access on)
//! - `lanaccess <true|false>`: the project's "allow access from other devices"
//! - `oscstatus` → `OK <OscStatus json>` (`{"state":"listening","port":9123,"lan":false}`, …)
//!
//! MIDI (the same code paths as a real MIDI input; Windows has no virtual MIDI ports, so the
//! fixture uses an in-memory backend with one device, `TestPad` (input and output), that records
//! what is sent to it). Device names in these commands have no spaces:
//! - `midiin <device> <hex bytes>`: handles a message as if `device` sent it (mappings,
//!   `trigger.midi` flows, learn), e.g. `midiin TestPad 90 24 7f` (note-on, note 36, velocity
//!   127) or `midiin TestPad b0077f` (CC 7 = 127). The device need not be open
//! - `midiout` → `OK <json array of {output, bytes}>`: the LED feedback sent since the last call
//! - `midiprefs <MidiPrefs json>`: `prefs.json` `midi` (inputs on / off, LED feedback per
//!   device), applied at once, e.g.
//!   `midiprefs {"inputs":{},"feedback":{"TestPad":{"output":null,"profile":"generic"}}}`
//! - `mididevices <json {inputs, outputs}>`: replaces the test backend's ports and reports a
//!   hot-plug → `OK <MidiDevice[] json>`
//! - `midilearn <true|false>`: open / close a learn session (counted)
//! - `midievents` → `OK <json array of {event, payload}>`: `midi-learn`, `control-conflict` and
//!   `midi-devices` events since the last call
//!
//! Media (part 3; the same code paths as the app's commands):
//! - `importmedia <absolute path>`: an import job's copy (`import_media`: checks, chunked copy,
//!   atomic place, probe, screens using the file re-applied), run to the end before answering
//!   → `OK </assets/<name>>` or `ERR <the file's error>`
//! - `assets` → `OK <json array of AssetInfo>` (`list_assets`, with `media`: the probe)
//! - `mediactl <screenId> <layerId> <json {action: play|pause|stop|restart|seek, ms?}>`:
//!   `media_control` (Program, at once) → `OK <MediaPlayback json>`
//! - media layers in `savescreen`: `{id, name, kind: "media", rect, media: {asset, fit, inMs,
//!   outMs, volume, muted, endAction, autoplay}, opacity?, anchor?}`; `layerview` has `media:
//!   {playback, readiness: {ready, total, error}, durationMs}` (readiness counts Program
//!   renderers: not `bus=preview`, not `mute=1`)
//!
//! The Preview output of a screen is `/screen/<id>/preview` (WebSocket `/ws?screen=<id>&bus=preview`).
//!
//! API sources (the same code paths as the app's commands). The fixture keeps secrets in an
//! in-memory store, never the OS keychain:
//! - `apisource <SourceDoc json>`: a new API source (`create_api_source`) saved with the given
//!   settings (`save_source`; `id` optional, `name` required), e.g.
//!   `apisource {"name":"Scores","kind":"api","request":{"url":"http://127.0.0.1:<port>/__mock/api/scores","method":"GET","headers":[]},"format":"json","mapping":{"rows":"/items","columns":[{"key":"name","label":"Name","kind":"text","path":"/name"}],"csv":{"header":true,"delimiter":"auto"}},"poll":{"intervalMs":1000,"onlyWhenUsed":false,"paused":false}}`
//!   → `OK <sourceId>`
//! - `apistatus <sourceId>` → `OK <ApiStatus json>` (as in `SourceSummary.api`)
//! - `setsecret <sourceId> <header> <value>`: `set_source_secret` (value = the rest of the line)
//!   → `OK <SourceDoc json>` (the header's `secret` id set, `stored: true`)
//! - `retryapi <sourceId>`: `retry_api_source` (fetch now; clears a 401/403 hold and backoff)
//! - `pauseapi <sourceId> <true|false>`: `set_api_paused`
//! - `watchapi <sourceId> <true|false>`: `watch_api_source` (counted)
//! - `testapi <SourceDoc json>`: `test_api_source` → `OK <ApiTestResult json>`
//! - `apitiming <intervalMs|off> [capMs]`: test timing: every poll interval (below the 1 s
//!   minimum allowed) and the backoff cap → `OK`
//! - `apitasks` → `OK <number of poll tasks>`
//!
//! The scriptable mock API on the same server, `/__mock/api/<name>` (GET and POST):
//! - `mockapi <name> <json {status?, contentType?, body?, headers?, etag?}>`: what it answers
//!   (default 200, `application/json`, empty body); with `etag`, a request whose
//!   `If-None-Match` equals it gets 304. `headers` is an object (e.g. `{"Retry-After":"2"}`).
//!   Replaces the previous script; the hit count and the recorded request stay.
//! - `mockhits <name>` → `OK <requests served so far>` (304s included)
//! - `mockreq <name>` → `OK <json object of the last request's headers>` (lower-case names;
//!   `{}` before the first request)
//! - An unknown name answers 404.
//!
//! A test that opens another project should run on its own fixture server.
//!
//! Remote control against a mock of kinetra-cloud's relay (the real `core::remote` connection):
//! - `remotemock` → `OK <port>`: starts the relay mock (once) and signs in with `pro.control`
//!   (the debug-only entitlement override); it answers `hello`, `register`, pings and `end`
//! - `remoteon` / `remoteoff` → `OK <RemoteView json>`; `remotelock on|off`; `remoteview`
//! - `remotesend <json>`: a server message to the app (e.g. a `command`)
//! - `remotewait <t> <n> [key=value]` → `OK <the last json>`: waits ≤ 5 s for n messages of
//!   type t from the app (e.g. `remotewait result 1 corr=c1`)
//! - `remotestate` → `OK {"seq", "state"}`: the relay's copy (last snapshot + diffs)
//!
//! Server:
//! - `restart`: stop the server and bind the same port again, keeping live state
//! - `quit`: stop and exit (also on stdin EOF); the temp project is deleted

use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use kinetra_lib::core::control_actions::ControlInput;
use kinetra_lib::core::events::{names, EventSink};
use kinetra_lib::core::live::{FieldDef, LiveError, TEST_LAYER_ID, TEST_SCREEN_ID};
use kinetra_lib::core::live_hub::LiveHub;
use kinetra_lib::core::midi::backend::TestBackend;
use kinetra_lib::core::model::{
    AddFlowTemplate, ApplyStyleScope, AutoAdvance, BrandKit, ControlTarget, ControlsDoc, FlowEdge,
    FlowNode, InstallRecipe, LiveEdit, MidiPrefs, NewFlow, NewScreen, OscSettings, Prefs, Rect,
    SaveFlow, ScheduleAction, ScheduleItem, ScheduleLabels, ScreenDoc, SourceColumn, SourceDoc,
    SourceKind, SourceStep, StudioExit, StudioOp, StudioSettings, TemplateDesign, TemplateDoc,
    TemplateSource, TimerAction, UiState, VariableDef, VariantTransition,
};
use kinetra_lib::core::protocol::{Fields, ServerMsg};
use kinetra_lib::core::secrets::{MemorySecrets, SecretStore};
use kinetra_lib::core::state::AppState;
use kinetra_lib::core::{flow_templates, presets, preview};
use kinetra_lib::server::{serve_with, AssetDir, OscManager};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

struct Server {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

// ---- The scriptable mock API (`/__mock/api/<name>`) ----------------------------------------

#[derive(serde::Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct MockScript {
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    etag: Option<String>,
}

#[derive(Default)]
struct MockApi {
    script: MockScript,
    hits: u64,
    last: std::collections::BTreeMap<String, String>,
}

type Mocks = Arc<Mutex<std::collections::HashMap<String, MockApi>>>;

fn mocks_lock(m: &Mocks) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, MockApi>> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

async fn mock_api(
    axum::extract::State(mocks): axum::extract::State<Mocks>,
    axum::extract::Path(name): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::http::{header, HeaderName, HeaderValue, StatusCode};
    use axum::response::IntoResponse;
    let mut all = mocks_lock(&mocks);
    let Some(api) = all.get_mut(&name) else {
        return (StatusCode::NOT_FOUND, "no such mock").into_response();
    };
    api.hits += 1;
    api.last = headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let script = api.script.clone();
    drop(all);
    let mut extra = axum::http::HeaderMap::new();
    for (k, v) in &script.headers {
        if let (Ok(k), Ok(v)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            extra.insert(k, v);
        }
    }
    if let Some(etag) = &script.etag {
        if let Ok(v) = HeaderValue::from_str(etag) {
            extra.insert(header::ETAG, v);
        }
        let matches = headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == etag);
        if matches {
            return (StatusCode::NOT_MODIFIED, extra).into_response();
        }
    }
    let status = StatusCode::from_u16(script.status.unwrap_or(200)).unwrap_or(StatusCode::OK);
    let ctype = script
        .content_type
        .unwrap_or_else(|| "application/json".into());
    if let Ok(v) = HeaderValue::from_str(&ctype) {
        extra.insert(header::CONTENT_TYPE, v);
    }
    (status, extra, script.body).into_response()
}

fn mock_router(mocks: &Mocks) -> axum::Router {
    axum::Router::new()
        .route(
            "/__mock/api/{name}",
            axum::routing::get(mock_api).post(mock_api),
        )
        .with_state(mocks.clone())
}

// ---- The poll API mock (`pollmock`): JSON answers and a scriptable live stream ------------------
// Raw TCP on its own port (the app's poll client streams SSE; axum here has no streaming body
// without extra crates). The poll object is set with `pollmock`, events are pushed with
// `pollpush <event> <json>` (which also becomes the next snapshot) and the stream dropped with
// `polldrop`.

#[derive(Clone)]
enum PollPush {
    Event(String, String),
    Drop,
}

struct PollMock {
    port: u16,
    poll: Arc<Mutex<String>>,
    push: tokio::sync::broadcast::Sender<PollPush>,
    connects: Arc<std::sync::atomic::AtomicU64>,
}

static POLL_MOCK: std::sync::OnceLock<PollMock> = std::sync::OnceLock::new();

/// A mock of kinetra-cloud's remote control relay (`docs/CONTROL_PROTOCOL.md`): answers
/// `hello` with `welcome`, `register` with `registered`, pings with pongs and `end` with close
/// 1000; records every message the app sends; `remotesend` pushes server messages.
struct RelayMock {
    port: u16,
    to_app: Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>,
    received: Mutex<Vec<serde_json::Value>>,
}

static RELAY_MOCK: std::sync::OnceLock<Arc<RelayMock>> = std::sync::OnceLock::new();

async fn relay_conn(socket: axum::extract::ws::WebSocket, mock: Arc<RelayMock>) {
    use axum::extract::ws::Message;
    use futures_util::{SinkExt, StreamExt};
    let (mut tx, mut rx) = socket.split();
    let (out, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    *mock.to_app.lock().unwrap_or_else(PoisonError::into_inner) = Some(out.clone());
    let writer = tokio::spawn(async move {
        while let Some(t) = out_rx.recv().await {
            let msg = if t == "__close" {
                Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1000,
                    reason: "ended".into(),
                }))
            } else {
                Message::Text(t.into())
            };
            if tx.send(msg).await.is_err() {
                break;
            }
        }
    });
    let now = kinetra_lib::core::fsx::now_epoch_ms;
    while let Some(Ok(Message::Text(t))) = rx.next().await {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(t.as_str()) else {
            continue;
        };
        mock.received
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(v.clone());
        let reply = match v["t"].as_str().unwrap_or_default() {
            "hello" => serde_json::json!({"t": "welcome", "protocol": 1, "mode": "run",
                "server_version": "fixture relay", "server_time_ms": now(), "heartbeat_s": 20,
                "limits": {}}),
            "register" => serde_json::json!({"t": "registered", "corr": v["corr"],
                "show_id": "fixture-show", "run_id": "fixture-run", "resumed": false,
                "crew_url": "https://control.kinetra.live/s/x/crew",
                "controller_url": "https://control.kinetra.live/s/x"}),
            "ping" => serde_json::json!({"t": "pong", "corr": v["corr"],
                "ping_sent_ms": v["sent_ms"], "received_ms": now(), "sent_ms": now()}),
            "end" => {
                let _ = out.send("__close".into());
                continue;
            }
            _ => continue,
        };
        let _ = out.send(reply.to_string());
    }
    writer.abort();
}

/// Starts the relay mock (once) and points the cloud service at it, signed in with
/// `pro.control` (the debug-only entitlement override).
fn relay_mock_start(rt: &Runtime, state: &AppState) -> Result<u16, String> {
    let mock = RELAY_MOCK
        .get_or_init(|| {
            let listener = rt
                .block_on(TcpListener::bind(("127.0.0.1", 0)))
                .expect("relay mock bind");
            let port = listener.local_addr().expect("relay mock port").port();
            let mock = Arc::new(RelayMock {
                port,
                to_app: Mutex::new(None),
                received: Mutex::new(Vec::new()),
            });
            let m = mock.clone();
            let app = axum::Router::new().route(
                "/v1/control/app",
                axum::routing::get(move |ws: axum::extract::WebSocketUpgrade| {
                    let m = m.clone();
                    async move {
                        ws.protocols(["kinetra-control.v1"])
                            .on_upgrade(move |s| relay_conn(s, m))
                    }
                }),
            );
            rt.spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            mock
        })
        .clone();
    let secrets = Arc::new(MemorySecrets::default());
    secrets
        .set("kinetra-session", "kntr_s_fixture")
        .map_err(text)?;
    secrets
        .set("kinetra-license-token", "1:fixture-license-token")
        .map_err(text)?;
    state.cloud_service().set_test_env(
        &format!("http://127.0.0.1:{}", mock.port),
        secrets,
        "857a78b7a23b14cd718a9cbf9b90b0577225543ddcce5ed21854a4985f25ee27",
    );
    state.cloud_init();
    state
        .cloud_service()
        .set_test_entitlements(&["core.app", "pro.control"], true);
    Ok(mock.port)
}

/// The relay mock's copy of the remote state: the last snapshot plus every diff after it.
fn relay_state() -> Result<serde_json::Value, String> {
    let mock = RELAY_MOCK.get().ok_or("no relay mock (remotemock first)")?;
    let msgs = mock
        .received
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let mut tree = serde_json::Value::Null;
    let mut seq = 0u64;
    for m in msgs {
        match m["t"].as_str() {
            Some("snapshot") => {
                tree = m["state"].clone();
                seq = m["seq"].as_u64().unwrap_or(0);
            }
            Some("diff") => {
                seq = m["seq"].as_u64().unwrap_or(seq);
                for op in m["ops"].as_array().into_iter().flatten() {
                    let path = op["path"].as_str().unwrap_or_default();
                    let Some((parent, key)) = path.rsplit_once('/') else {
                        continue;
                    };
                    let key = key.replace("~1", "/").replace("~0", "~");
                    let target = if parent.is_empty() {
                        Some(&mut tree)
                    } else {
                        tree.pointer_mut(parent)
                    };
                    if let Some(serde_json::Value::Object(o)) = target {
                        if op["op"] == "put" {
                            o.insert(key, op["value"].clone());
                        } else {
                            o.remove(&key);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(serde_json::json!({"seq": seq, "state": tree}))
}

async fn poll_mock_conn(
    mut sock: tokio::net::TcpStream,
    poll: Arc<Mutex<String>>,
    push: tokio::sync::broadcast::Sender<PollPush>,
    connects: Arc<std::sync::atomic::AtomicU64>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let Ok(n) = sock.read(&mut tmp).await else {
            return;
        };
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let path = head.split(' ').nth(1).unwrap_or("").to_string();
    let current = poll.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let answer = |status: &str, body: String| {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    if path.ends_with("/events") {
        connects.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut rx = push.subscribe();
        let start = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\nevent: snapshot\ndata: {current}\n\n"
        );
        if sock.write_all(start.as_bytes()).await.is_err() {
            return;
        }
        while let Ok(p) = rx.recv().await {
            let text = match p {
                PollPush::Event(e, d) => format!("event: {e}\ndata: {d}\n\n"),
                PollPush::Drop => return,
            };
            if sock.write_all(text.as_bytes()).await.is_err() {
                return;
            }
        }
        return;
    }
    let reply = if path == "/v1/polls" {
        answer("200 OK", format!("{{\"polls\":[{current}]}}"))
    } else if path.starts_with("/v1/polls/quota") {
        answer("200 OK", r#"{"plan":null,"limits":null,"usage":{"open_polls":0,"polls_this_month":0,"polls":1},"month_resets_at":null}"#.to_string())
    } else if path.starts_with("/v1/polls/") {
        answer("200 OK", current)
    } else {
        answer(
            "404 Not Found",
            r#"{"error":{"code":"not_found","message":"-"}}"#.to_string(),
        )
    };
    let _ = sock.write_all(reply.as_bytes()).await;
}

/// Starts the mock (once) and points the app's account at it: signed in (a session in memory) and
/// this poll known on this machine ("Show my existing polls"), so it streams without Pro.
fn poll_mock_start(rt: &Runtime, state: &AppState, poll: &str) -> Result<String, String> {
    let mock = POLL_MOCK.get_or_init(|| {
        let listener = rt
            .block_on(TcpListener::bind(("127.0.0.1", 0)))
            .expect("poll mock bind");
        let port = listener.local_addr().expect("poll mock port").port();
        let (push, _) = tokio::sync::broadcast::channel(64);
        let poll: Arc<Mutex<String>> = Arc::default();
        let connects: Arc<std::sync::atomic::AtomicU64> = Arc::default();
        let (p, c, tx) = (poll.clone(), connects.clone(), push.clone());
        rt.spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                tokio::spawn(poll_mock_conn(sock, p.clone(), tx.clone(), c.clone()));
            }
        });
        PollMock {
            port,
            poll,
            push,
            connects,
        }
    });
    *mock.poll.lock().unwrap_or_else(PoisonError::into_inner) = poll.to_string();
    let secrets = Arc::new(MemorySecrets::default());
    // The keychain account the cloud service reads the session from.
    secrets
        .set("kinetra-session", "kntr_s_fixture")
        .map_err(text)?;
    state.cloud_service().set_test_env(
        &format!("http://127.0.0.1:{}", mock.port),
        secrets,
        "857a78b7a23b14cd718a9cbf9b90b0577225543ddcce5ed21854a4985f25ee27",
    );
    state.cloud_init();
    let polls = rt.block_on(state.list_polls(true)).map_err(text)?;
    let first = polls.into_iter().next().ok_or("the mock lists no poll")?;
    let doc = state.create_poll_source("Poll", first).map_err(text)?;
    Ok(doc.id)
}

fn start(
    rt: &Runtime,
    addr: SocketAddr,
    hub: &Arc<LiveHub>,
    assets: &AssetDir,
    mocks: &Mocks,
) -> std::io::Result<(u16, Server)> {
    let listener = rt.block_on(TcpListener::bind(addr))?;
    let port = listener.local_addr()?.port();
    let (stop, rx) = oneshot::channel::<()>();
    let hub = hub.clone();
    let assets = assets.clone();
    let mock = mock_router(mocks);
    let extra: kinetra_lib::server::ExtraRoutes = Arc::new(move |_| mock.clone());
    let task = rt.spawn(async move {
        let shutdown = async move {
            let _ = rx.await;
        };
        if let Err(e) = serve_with(listener, hub, assets, Some(extra), shutdown).await {
            eprintln!("server error: {e}");
        }
    });
    Ok((port, Server { stop, task }))
}

fn stop(rt: &Runtime, server: Server) {
    let _ = server.stop.send(());
    let mut task = server.task;
    let stopped = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), &mut task)
            .await
            .is_ok()
    });
    if !stopped {
        task.abort();
    }
}

fn parse_port() -> Result<u16, String> {
    let mut args = std::env::args().skip(1);
    let mut port = 0;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                let v = args.next().ok_or("--port needs a value")?;
                port = v.parse().map_err(|_| format!("bad port: {v}"))?;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(port)
}

fn json<T: serde::de::DeserializeOwned>(what: &str, text: &str) -> Result<T, String> {
    serde_json::from_str(text.trim()).map_err(|e| format!("bad {what} json: {e}"))
}

/// `<screenId> <layerId>[ <rest>]`.
fn ids(arg: &str) -> Result<(&str, &str, &str), String> {
    let mut parts = arg.trim().splitn(3, ' ');
    let screen = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or("missing screen id")?;
    let layer = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or("missing layer id")?;
    Ok((screen, layer, parts.next().unwrap_or("")))
}

/// `addsource` argument.
#[derive(serde::Deserialize)]
struct NewSource {
    name: String,
    columns: Vec<SourceColumn>,
    #[serde(default)]
    rows: Vec<Fields>,
}

/// `addschedule` argument.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewSchedule {
    name: String,
    #[serde(default)]
    items: Vec<ScheduleItem>,
    #[serde(default)]
    ripple: bool,
    #[serde(default)]
    labels: Option<ScheduleLabels>,
}

/// `addtemplatedoc` argument.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewTemplateDoc {
    name: String,
    html: String,
    css: String,
    fields: Vec<FieldDef>,
    #[serde(default)]
    variables: Vec<VariableDef>,
    #[serde(default)]
    auto_out_ms: Option<u32>,
    #[serde(default)]
    design: Option<TemplateDesign>,
}

/// `savevariant` argument.
#[derive(serde::Deserialize)]
struct VariantLook {
    html: String,
    css: String,
    #[serde(default)]
    vars: Fields,
}

/// `writev1template` argument.
#[derive(serde::Deserialize)]
struct V1Template {
    name: String,
    html: String,
    css: String,
    #[serde(default)]
    fields: Vec<FieldDef>,
    #[serde(default)]
    variables: Vec<VariableDef>,
}

/// `checkflow` argument.
#[derive(serde::Deserialize)]
struct CheckFlow {
    nodes: Vec<FlowNode>,
    edges: Vec<FlowEdge>,
    /// The flow being edited (also reports a Run flow of itself).
    #[serde(default)]
    id: Option<String>,
}

/// Keeps the `flow-debug` payloads for `debugevents` and the `project-files-changed` ones for
/// `files`; every other event is discarded.
#[derive(Default)]
struct DebugCapture {
    debug: Mutex<Vec<serde_json::Value>>,
    files: Mutex<Vec<serde_json::Value>>,
    midi: Mutex<Vec<serde_json::Value>>,
}

impl EventSink for DebugCapture {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let list = match event {
            names::MIDI_LEARN | names::CONTROL_CONFLICT | names::MIDI_DEVICES => {
                self.midi
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(serde_json::json!({ "event": event, "payload": payload }));
                return;
            }
            names::FLOW_DEBUG => &self.debug,
            names::PROJECT_FILES_CHANGED => &self.files,
            _ => return,
        };
        list.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(payload);
    }
}

fn take_all(list: &Mutex<Vec<serde_json::Value>>) -> Vec<serde_json::Value> {
    std::mem::take(&mut *list.lock().unwrap_or_else(PoisonError::into_inner))
}

/// The fixture's in-memory MIDI ports (one device, `TestPad`).
fn midi_backend() -> &'static Arc<TestBackend> {
    static B: std::sync::OnceLock<Arc<TestBackend>> = std::sync::OnceLock::new();
    B.get_or_init(|| TestBackend::new(&["TestPad"], &["TestPad"]))
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// What a command answers: quit or not, and the value after `OK` (default: the command name).
type Outcome = (bool, Option<String>);

/// Runs one command.
#[allow(clippy::too_many_arguments)]
fn run(
    line: &str,
    rt: &Runtime,
    state: &AppState,
    server: &mut Option<Server>,
    addr: SocketAddr,
    assets: &AssetDir,
    debug: &DebugCapture,
    mocks: &Mocks,
    osc: &OscManager,
) -> Result<Outcome, String> {
    let hub = state.live();
    let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
    let s = TEST_SCREEN_ID;
    let l = TEST_LAYER_ID;
    let mut value = None;
    match cmd {
        "show" => hub.show(s, l).map(drop).map_err(text)?,
        "hide" => hub.hide(s, l).map(drop).map_err(text)?,
        "clear" => hub.clear(s).map(drop).map_err(text)?,
        "fadetracking" => hub.set_fade_tracking(arg.trim() == "on"),
        "visible" => {
            let (screen, layer) = arg
                .trim()
                .split_once(' ')
                .ok_or("visible <screen> <layer>")?;
            value = Some(hub.visible(screen, layer).to_string());
        }
        "fields" => {
            let fields: Fields = json("fields", arg)?;
            hub.set_fields(s, l, fields).map(drop).map_err(text)?
        }
        "template" => {
            let mut it = arg.split_whitespace();
            let p = presets::load(it.next().unwrap_or("")).map_err(text)?;
            let look = match it.next() {
                Some(v) => p
                    .variant(v)
                    .ok_or_else(|| format!("preset {} has no variant {v}", p.info.id))?
                    .clone(),
                None => p.default_look().clone(),
            };
            hub.update_layer(s, l, |layer| preview::load_template(layer, &p.meta, &look))
                .map(drop)
                .map_err(text)?
        }
        // The e2e sandbox test: raw markup into the test layer, bypassing validation (debug only).
        "rawtemplate" => {
            #[derive(serde::Deserialize)]
            struct Raw {
                html: String,
                #[serde(default)]
                css: String,
            }
            let r: Raw = json("rawtemplate", arg)?;
            hub.update_layer(s, l, |layer| {
                Ok::<_, kinetra_lib::core::live::LiveError>(
                    kinetra_lib::core::preview::load_unchecked_for_tests(layer, &r.html, &r.css),
                )
            })
            .map(drop)
            .map_err(text)?
        }
        // The hardening: what a save / import says about markup (empty = allowed).
        "checkmarkup" => {
            #[derive(serde::Deserialize)]
            struct Raw {
                html: String,
                #[serde(default)]
                css: String,
            }
            let r: Raw = json("checkmarkup", arg)?;
            let mut p = kinetra_lib::core::template_safety::check_html(&r.html);
            p.extend(kinetra_lib::core::template_safety::check_css(&r.css));
            value = Some(serde_json::to_string(&p).map_err(text)?);
        }
        "vars" => {
            let values: Fields = json("vars", arg)?;
            hub.update_layer(s, l, |layer| preview::set_variable_values(layer, values))
                .map(drop)
                .map_err(text)?
        }
        "autoout" => {
            let ms = match arg.trim() {
                "off" => None,
                v => Some(
                    v.parse::<u32>()
                        .ok()
                        .filter(|ms| (1..=600_000).contains(ms))
                        .ok_or_else(|| format!("bad autoout: {v} (ms 1..=600000 or off)"))?,
                ),
            };
            hub.update_layer(s, l, |layer| {
                // Changed: a same-rev `layer` message (the renderer's --auto-out-ms), as the app.
                let changed = layer.auto_out_ms != ms;
                layer.auto_out_ms = ms;
                Ok::<_, LiveError>(if changed {
                    vec![ServerMsg::Layer {
                        layer: layer.to_wire(),
                    }]
                } else {
                    Vec::new()
                })
            })
            .map(drop)
            .map_err(text)?
        }
        "addtemplate" => {
            let preset_id = arg.trim();
            let p = presets::load(preset_id).map_err(text)?;
            let dir = state.project_dir().map_err(text)?;
            let source = TemplateSource::Preset {
                preset_id: preset_id.to_string(),
            };
            let doc = state
                .templates()
                .create(&dir, &p.info.name, &source)
                .map_err(text)?;
            value = Some(doc.id);
        }
        "newscreen" => {
            let new: NewScreen = json("NewScreen", arg)?;
            value = Some(state.create_screen(&new).map_err(text)?.doc.id);
        }
        "savescreen" => {
            let doc: ScreenDoc = json("ScreenDoc", arg)?;
            value = Some(state.put_screen(doc).map_err(text)?.doc.id);
        }
        "layershow" => {
            let (screen, layer, _) = ids(arg)?;
            hub.show(screen, layer).map(drop).map_err(text)?
        }
        "layerhide" => {
            let (screen, layer, _) = ids(arg)?;
            hub.hide(screen, layer).map(drop).map_err(text)?
        }
        "rect" => {
            let (screen, layer, rest) = ids(arg)?;
            let rect: Rect = json("Rect", rest)?;
            state
                .set_layer_rect(screen, layer, rect)
                .map(drop)
                .map_err(text)?
        }
        "liveedit" => {
            let mode = match arg.trim() {
                "onTake" => LiveEdit::OnTake,
                "immediate" => LiveEdit::Immediate,
                v => return Err(format!("bad liveedit: {v} (onTake or immediate)")),
            };
            let mut ui = state.ui_state().map_err(text)?;
            ui.control.live_edit = mode;
            state.set_ui_state(ui).map(drop).map_err(text)?
        }
        // ---- Live polls ----
        "pollsource" => {
            let wire: kinetra_lib::core::cloud::api::PollWire = json("poll", arg)?;
            let doc = state
                .create_poll_source("Poll", wire.into())
                .map_err(text)?;
            value = Some(doc.id);
        }
        "polldata" => {
            let wire: kinetra_lib::core::cloud::api::PollWire = json("poll", arg)?;
            state.poll_apply(wire.into());
        }
        "polltoscreen" => {
            let (source, screen) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: polltoscreen <sourceId> <screenId>")?;
            let st = state
                .add_poll_to_screen(source, screen.trim())
                .map_err(text)?;
            value = st.doc.meta.layers.last().map(|l| l.id.clone());
        }
        "pollmock" => value = Some(poll_mock_start(rt, state, arg.trim())?),
        "pollpush" => {
            let (event, data) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: pollpush <event> <json>")?;
            let mock = POLL_MOCK.get().ok_or("no poll mock (pollmock first)")?;
            *mock.poll.lock().unwrap_or_else(PoisonError::into_inner) = data.to_string();
            let _ = mock
                .push
                .send(PollPush::Event(event.to_string(), data.to_string()));
        }
        "polldrop" => {
            let mock = POLL_MOCK.get().ok_or("no poll mock (pollmock first)")?;
            let _ = mock.push.send(PollPush::Drop);
        }
        "pollconnects" => {
            let mock = POLL_MOCK.get().ok_or("no poll mock (pollmock first)")?;
            value = Some(
                mock.connects
                    .load(std::sync::atomic::Ordering::SeqCst)
                    .to_string(),
            );
        }
        "remotemock" => value = Some(relay_mock_start(rt, state)?.to_string()),
        "remoteon" => {
            let v = state.remote_set(true).map_err(text)?;
            value = Some(serde_json::to_string(&v).map_err(text)?);
        }
        "remoteoff" => {
            let v = state.remote_set(false).map_err(text)?;
            value = Some(serde_json::to_string(&v).map_err(text)?);
        }
        "remotelock" => {
            state.remote_lock(arg.trim() == "on").map_err(text)?;
        }
        "remoteview" => value = Some(serde_json::to_string(&state.remote_view()).map_err(text)?),
        "remotesend" => {
            let mock = RELAY_MOCK.get().ok_or("no relay mock (remotemock first)")?;
            let tx = mock
                .to_app
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .ok_or("the app isn't connected to the relay mock")?;
            let v: serde_json::Value = json("message", arg)?;
            tx.send(v.to_string()).map_err(text)?;
        }
        "remotewait" => {
            // `remotewait <t> <n> [key=value]`: waits (≤ 5 s) until the app sent ≥ n messages
            // of type t (with that top-level string field); answers the last one.
            let mut it = arg.split_whitespace();
            let t = it.next().ok_or("remotewait <t> <n>")?.to_string();
            let n: usize = it.next().unwrap_or("1").parse().map_err(text)?;
            let filter = it
                .next()
                .and_then(|f| f.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()));
            let mock = RELAY_MOCK.get().ok_or("no relay mock (remotemock first)")?;
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let hits: Vec<serde_json::Value> = mock
                    .received
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .iter()
                    .filter(|m| m["t"] == t.as_str())
                    .filter(|m| {
                        filter
                            .as_ref()
                            .is_none_or(|(k, v)| m[k.as_str()].as_str() == Some(v.as_str()))
                    })
                    .cloned()
                    .collect();
                if hits.len() >= n {
                    value = Some(hits.last().map(|m| m.to_string()).unwrap_or_default());
                    break;
                }
                if std::time::Instant::now() > deadline {
                    return Err(format!("only {} {t} message(s) from the app", hits.len()));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        "remotestate" => value = Some(relay_state()?.to_string()),
        "pollconn" => {
            let conn = state.poll_service().conn(arg.trim());
            value = Some(serde_json::to_string(&conn).map_err(text)?);
        }
        "dir" => {
            let dir = state.project_dir().map_err(text)?;
            value = Some(dir.display().to_string());
        }
        "addsource" => {
            let new: NewSource = json("source", arg)?;
            let mut doc = state.create_source(&new.name).map_err(text)?;
            doc.meta.kind = SourceKind::Text {
                columns: new.columns,
                rows: new.rows,
                index: 0,
                auto: None,
            };
            value = Some(state.save_source(doc).map_err(text)?.id);
        }
        "stepsource" => {
            let (id, delta) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: stepsource <id> <delta>")?;
            let delta: i32 = delta
                .trim()
                .parse()
                .map_err(|_| format!("bad delta: {delta}"))?;
            let summary = state
                .step_source(id, SourceStep::By { delta })
                .map_err(text)?;
            value = Some(summary.index.to_string());
        }
        "setfields" => {
            let (screen, layer, rest) = ids(arg)?;
            let fields: Fields = json("fields", rest)?;
            state
                .set_layer_fields(screen, layer, fields)
                .map(drop)
                .map_err(text)?
        }
        "getsource" => {
            let doc = state.get_source(arg.trim()).map_err(text)?;
            value = Some(serde_json::to_string(&doc).map_err(text)?);
        }
        "savesource" => {
            let doc: SourceDoc = json("SourceDoc", arg)?;
            let saved = state.save_source(doc).map_err(text)?;
            value = Some(serde_json::to_string(&saved).map_err(text)?);
        }
        "clearscreen" => state.live_clear(arg.trim()).map(drop).map_err(text)?,
        "clearall" => state.live_clear_all().map(drop).map_err(text)?,
        "panic" => state.live_panic().map(drop).map_err(text)?,
        "restart" => {
            if let Some(old) = server.take() {
                stop(rt, old);
            }
            let (_, new) =
                start(rt, addr, hub, assets, mocks).map_err(|e| format!("rebind failed: {e}"))?;
            *server = Some(new);
        }
        "addtemplatedoc" => {
            let new: NewTemplateDoc = json("template", arg)?;
            let dir = state.project_dir().map_err(text)?;
            let created = state
                .templates()
                .create(&dir, &new.name, &TemplateSource::Blank)
                .map_err(text)?;
            let mut doc: TemplateDoc = created;
            doc.variants[0].html = new.html;
            doc.variants[0].css = new.css;
            doc.meta.fields = new.fields;
            doc.meta.variables = new.variables;
            doc.meta.auto_out_ms = new.auto_out_ms;
            doc.meta.design = new.design;
            let saved = state.templates().save(&dir, doc).map_err(text)?;
            state.template_saved(&saved);
            value = Some(saved.id);
        }
        "settimer" => {
            let (screen, layer, rest) = ids(arg)?;
            let (key, action) = rest
                .trim()
                .split_once(' ')
                .ok_or("usage: settimer <screenId> <layerId> <key> <TimerAction json>")?;
            let action: TimerAction = json("TimerAction", action)?;
            let view = state.set_timer(screen, layer, key, action).map_err(text)?;
            let current = state
                .live()
                .with_layer(screen, layer, |l| l.fields.get(key).cloned())
                .map_err(text)?;
            value = Some(
                current
                    .or_else(|| view.fields.get(key).cloned())
                    .unwrap_or_default(),
            );
        }
        "autoadvance" => {
            let (id, rest) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: autoadvance <sourceId> <json|null>")?;
            let auto: Option<AutoAdvance> = json("AutoAdvance", rest)?;
            let summary = state.set_auto_advance(id, auto).map_err(text)?;
            value = Some(summary.next_at.map_or("off".into(), |n| n.to_string()));
        }
        "addschedule" => {
            let new: NewSchedule = json("schedule", arg)?;
            let mut doc = state.create_schedule(&new.name).map_err(text)?;
            doc.meta.kind = SourceKind::Schedule {
                items: new.items,
                ripple: new.ripple,
                next_override: None,
                labels: new.labels.unwrap_or_default(),
            };
            value = Some(state.save_source(doc).map_err(text)?.id);
        }
        "scheduleaction" => {
            let (id, rest) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: scheduleaction <sourceId> <ScheduleAction json>")?;
            let action: ScheduleAction = json("ScheduleAction", rest)?;
            let view = state.schedule_action(id, action).map_err(text)?;
            value = Some(format!(
                "{} {}",
                view.now_id.as_deref().unwrap_or("-"),
                view.next_id.as_deref().unwrap_or("-")
            ));
        }
        "scheduleview" => {
            let view = state.schedule_view(arg.trim()).map_err(text)?;
            value = Some(serde_json::to_string(&view).map_err(text)?);
        }
        "clock" => {
            let skew: i64 = arg
                .trim()
                .parse()
                .map_err(|_| format!("bad clock skew: {arg}"))?;
            let clock = state.sources().clock();
            let zone = clock.now().zone;
            clock.set_test(skew, zone);
            state.replan();
            value = Some(clock.now().ms.to_string());
        }
        "installrecipe" => {
            let req: InstallRecipe = json("InstallRecipe", arg)?;
            value = Some(state.install_recipe(&req).map_err(text)?.id);
        }
        "recipeinstance" => {
            let inst = state
                .rules()
                .instances()
                .into_iter()
                .find(|i| i.id == arg.trim())
                .ok_or_else(|| format!("no recipe instance {}", arg.trim()))?;
            value = Some(serde_json::to_string(&inst).map_err(text)?);
        }
        "relinkrecipe" => {
            let mut it = arg.split_whitespace();
            let (Some(id), Some(src)) = (it.next(), it.next()) else {
                return Err("usage: relinkrecipe <instanceId> <sourceId>".into());
            };
            let inst = state.relink_recipe_source(id, src).map_err(text)?;
            value = Some(serde_json::to_string(&inst).map_err(text)?);
        }
        "flowtemplates" => {
            value = Some(serde_json::to_string(&flow_templates::list()).map_err(text)?);
        }
        "addflowtemplate" => {
            let req: AddFlowTemplate = json("AddFlowTemplate", arg)?;
            let ids: Vec<String> = state
                .add_flow_template(req)
                .map_err(text)?
                .into_iter()
                .map(|i| i.flow.id)
                .collect();
            value = Some(serde_json::to_string(&ids).map_err(text)?);
        }
        "saveflow" => {
            let new: NewFlow = json("NewFlow", arg)?;
            value = Some(state.create_flow(new).map_err(text)?.flow.id);
        }
        "updateflow" => {
            let req: SaveFlow = json("SaveFlow", arg)?;
            state.save_flow(req).map(drop).map_err(text)?
        }
        "armflow" => {
            let (id, armed) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: armflow <id> <true|false>")?;
            let armed = match armed.trim() {
                "true" => true,
                "false" => false,
                v => return Err(format!("bad armed: {v} (true or false)")),
            };
            state.set_flow_armed(id, armed).map(drop).map_err(text)?
        }
        "runflow" => {
            let (id, v) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
            let v = (!v.is_empty()).then(|| v.to_string());
            state.run_flow(id, v).map_err(text)?
        }
        "stopflow" => state.stop_flow(arg.trim()).map_err(text)?,
        "flowstate" => {
            let running = state.flow_engine().is_running(arg.trim());
            value = Some(if running { "running" } else { "idle" }.into());
        }
        "flows" => {
            let list = state.list_flows().map_err(text)?;
            value = Some(serde_json::to_string(&list).map_err(text)?);
        }
        "openproject" => {
            // Like the app: the current project is flushed and closed, the next one opened (and
            // migrated). The renderer server keeps running; `/assets/*` follows.
            let project = state.open_project(Path::new(arg.trim())).map_err(text)?;
            assets.set_project(state.project_dir().ok());
            value = Some(project.path);
        }
        "uistate" => {
            let ui = state.ui_state().map_err(text)?;
            value = Some(serde_json::to_string(&ui).map_err(text)?);
        }
        "setuistate" => {
            let ui: UiState = json("UiState", arg)?;
            let saved = state.set_ui_state(ui).map_err(text)?;
            value = Some(serde_json::to_string(&saved).map_err(text)?);
        }
        "automationoff" => {
            let off = match arg.trim() {
                "true" => true,
                "false" => false,
                v => return Err(format!("bad automationoff: {v} (true or false)")),
            };
            let mut ui = state.ui_state().map_err(text)?;
            ui.control.automation_off = off;
            state.set_ui_state(ui).map(drop).map_err(text)?
        }
        "importmedia" => {
            let dir = state.project_dir().map_err(text)?;
            let errors = std::sync::Mutex::new(Vec::new());
            let names = kinetra_lib::core::assets::run_import(
                &dir,
                "fixture",
                &[arg.trim().to_string()],
                &std::sync::atomic::AtomicBool::new(false),
                &|p| {
                    if let Some(e) = p.error {
                        errors.lock().unwrap().push(e);
                    }
                },
            );
            let Some(name) = names.first() else {
                return Err(errors
                    .into_inner()
                    .unwrap()
                    .pop()
                    .unwrap_or_else(|| "import failed".into()));
            };
            state.media_file_changed(&dir, name);
            value = Some(kinetra_lib::core::assets::url_of(name));
        }
        "assets" => {
            let list = state.list_assets().map_err(text)?;
            value = Some(serde_json::to_string(&list).map_err(text)?);
        }
        "mediactl" => {
            #[derive(serde::Deserialize)]
            struct Ctl {
                action: kinetra_lib::core::model::MediaAction,
                #[serde(default)]
                ms: Option<u64>,
            }
            let (screen, layer, rest) = ids(arg)?;
            let c: Ctl = json("{action, ms?}", rest)?;
            let view = hub
                .media_control(screen, layer, c.action, c.ms)
                .map_err(text)?;
            let pb = view.media.map(|m| m.playback);
            value = Some(serde_json::to_string(&pb).map_err(text)?);
        }
        "importasset" => {
            let list = state
                .import_assets(&[arg.trim().to_string()])
                .map_err(text)?;
            value = list.first().map(|a| a.url.clone());
        }
        "checkflow" => {
            let req: CheckFlow = json("{nodes, edges}", arg)?;
            let check = state
                .check_flow_of(req.id.as_deref(), req.nodes, req.edges)
                .map_err(text)?;
            value = Some(serde_json::to_string(&check).map_err(text)?);
        }
        "subscribedebug" => state.subscribe_flow_debug(arg.trim()).map_err(text)?,
        "unsubscribedebug" => state.unsubscribe_flow_debug(arg.trim()).map_err(text)?,
        "debugevents" => {
            value = Some(serde_json::to_string(&take_all(&debug.debug)).map_err(text)?);
        }
        "files" => {
            value = Some(serde_json::to_string(&take_all(&debug.files)).map_err(text)?);
        }
        "saveflowdraft" => {
            let (id, rest) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: saveflowdraft <id> <json {nodes, edges}>")?;
            let req: CheckFlow = json("{nodes, edges}", rest)?;
            state
                .save_flow_draft(id, req.nodes, req.edges)
                .map(drop)
                .map_err(text)?
        }
        "deployflow" => state.deploy_flow(arg.trim()).map(drop).map_err(text)?,
        "discardflowdraft" => state
            .discard_flow_draft(arg.trim())
            .map(drop)
            .map_err(text)?,
        "deleteflow" => state.delete_flow(arg.trim()).map_err(text)?,
        "addvariant" => {
            let mut it = arg.trim().splitn(3, ' ');
            let (Some(t), Some(from), Some(name)) = (it.next(), it.next(), it.next()) else {
                return Err("usage: addvariant <templateId> <fromVariant> <name>".into());
            };
            let dir = state.project_dir().map_err(text)?;
            let before: Vec<String> = state
                .templates()
                .get(&dir, t)
                .map_err(text)?
                .variants
                .into_iter()
                .map(|v| v.id)
                .collect();
            let doc = state
                .templates()
                .create_variant(&dir, t, from, name)
                .map_err(text)?;
            state.template_saved(&doc);
            value = doc
                .variants
                .into_iter()
                .map(|v| v.id)
                .find(|id| !before.contains(id));
        }
        "savevariant" => {
            let mut it = arg.trim().splitn(3, ' ');
            let (Some(t), Some(v), Some(rest)) = (it.next(), it.next(), it.next()) else {
                return Err("usage: savevariant <templateId> <variantId> <json>".into());
            };
            let look: VariantLook = json("{html, css, vars?}", rest)?;
            let dir = state.project_dir().map_err(text)?;
            let doc = state
                .templates()
                .save_variant(&dir, t, v, &look.html, &look.css, look.vars)
                .map_err(text)?;
            state.template_saved(&doc);
        }
        "setvariant" => {
            let mut it = arg.split_whitespace();
            let (Some(screen), Some(layer), Some(variant)) = (it.next(), it.next(), it.next())
            else {
                return Err(
                    "usage: setvariant <screenId> <layerId> <variant> [crossfade|outIn]".into(),
                );
            };
            let transition = match it.next().unwrap_or("crossfade") {
                "crossfade" => VariantTransition::Crossfade,
                "outIn" => VariantTransition::OutIn,
                v => return Err(format!("bad transition: {v} (crossfade or outIn)")),
            };
            state
                .set_layer_variant(screen, layer, variant, transition)
                .map(drop)
                .map_err(text)?
        }
        "layerview" => {
            let (screen, layer, _) = ids(arg)?;
            let view = hub.layer_view(screen, layer).map_err(text)?;
            let mut v = serde_json::to_value(&view).map_err(text)?;
            let (sizing, design) = state
                .live()
                .with_layer(screen, layer, |l| (l.sizing, l.template.design))
                .map_err(text)?;
            v["sizing"] = serde_json::to_value(sizing).map_err(text)?;
            v["opacity"] = hub
                .with_layer(screen, layer, |l| l.opacity)
                .map_err(text)?
                .into();
            v["design"] = serde_json::to_value(design).map_err(text)?;
            value = Some(v.to_string());
        }
        "writev1template" => {
            let (id, rest) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: writev1template <id> <json>")?;
            if !kinetra_lib::core::templates::is_id(id) {
                return Err(format!("bad template id: {id}"));
            }
            let v1: V1Template = json("{name, html, css, fields?, variables?}", rest)?;
            let dir = state
                .project_dir()
                .map_err(text)?
                .join("templates")
                .join(id);
            std::fs::create_dir_all(&dir).map_err(text)?;
            let meta = serde_json::json!({
                "schemaVersion": 1,
                "name": v1.name,
                "description": "",
                "preset": null,
                "fields": v1.fields,
                "variables": v1.variables,
                "autoOutMs": null,
                "updatedAt": "2026-01-01T00:00:00Z",
            });
            std::fs::write(dir.join("template.html"), v1.html).map_err(text)?;
            std::fs::write(dir.join("template.css"), v1.css).map_err(text)?;
            std::fs::write(
                dir.join("template.json"),
                serde_json::to_string_pretty(&meta).map_err(text)?,
            )
            .map_err(text)?;
            value = Some(id.to_string());
        }
        "gettemplate" => {
            let dir = state.project_dir().map_err(text)?;
            let doc = state.templates().get(&dir, arg.trim()).map_err(text)?;
            value = Some(serde_json::to_string(&doc).map_err(text)?);
        }
        "listtemplates" => {
            let dir = state.project_dir().map_err(text)?;
            let list = state.templates().list(&dir).map_err(text)?;
            value = Some(serde_json::to_string(&list).map_err(text)?);
        }
        "updatetemplate" => {
            let r = state
                .update_template_from_preset(arg.trim())
                .map_err(text)?;
            value = Some(serde_json::to_string(&r).map_err(text)?);
        }
        "undotemplateupdate" => {
            let mut it = arg.split_whitespace();
            let (Some(id), Some(backup)) = (it.next(), it.next()) else {
                return Err("usage: undotemplateupdate <templateId> <backupId>".into());
            };
            let doc = state.undo_template_update(id, backup).map_err(text)?;
            value = Some(serde_json::to_string(&doc).map_err(text)?);
        }
        "presets" => {
            value = Some(serde_json::to_string(&presets::list()).map_err(text)?);
        }
        "liststyles" => {
            value = Some(serde_json::to_string(&state.list_styles()).map_err(text)?);
        }
        "applystyle" => {
            let mut it = arg.split_whitespace();
            let style = it
                .next()
                .ok_or("usage: applystyle <style> <project|screenId/layerId> [crossfade|outIn]")?;
            let scope = match it
                .next()
                .ok_or("missing scope (project or screenId/layerId)")?
            {
                "project" => ApplyStyleScope::Project,
                other => {
                    let (screen_id, layer_id) = other.split_once('/').ok_or_else(|| {
                        format!("bad scope: {other} (project or screenId/layerId)")
                    })?;
                    ApplyStyleScope::Layer {
                        screen_id: screen_id.to_string(),
                        layer_id: layer_id.to_string(),
                    }
                }
            };
            let transition = match it.next().unwrap_or("crossfade") {
                "crossfade" => VariantTransition::Crossfade,
                "outIn" => VariantTransition::OutIn,
                v => return Err(format!("bad transition: {v} (crossfade or outIn)")),
            };
            let result = state.apply_style(style, &scope, transition).map_err(text)?;
            value = Some(serde_json::to_string(&result).map_err(text)?);
        }
        "undostyle" => state.undo_apply_style(arg.trim()).map_err(text)?,
        "brand" => {
            let brand: BrandKit = json("BrandKit", arg)?;
            let saved = state.save_brand_kit(brand).map_err(text)?;
            value = Some(serde_json::to_string(&saved).map_err(text)?);
        }
        "studio" => {
            let arg = arg.trim();
            let (settings, exit) = match arg.rsplit_once(' ') {
                Some((s, e @ ("take" | "discard" | "keepStaged"))) => {
                    (s, Some(json::<StudioExit>("exit", &format!("\"{e}\""))?))
                }
                _ => (arg, None),
            };
            let settings: StudioSettings = json("StudioSettings", settings)?;
            state.set_studio(settings, exit).map_err(text)?
        }
        "studioop" => {
            let (screen, rest) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: studioop <screenId> <StudioOp json>")?;
            let op: StudioOp = json("StudioOp", rest)?;
            let view = state.apply_studio_op(screen, op).map_err(text)?;
            value = Some(serde_json::to_string(&view).map_err(text)?);
        }
        "savecontrols" => {
            let doc: ControlsDoc = json("ControlsDoc", arg)?;
            let saved = state.save_controls(doc).map_err(text)?;
            value = Some(serde_json::to_string(&saved).map_err(text)?);
        }
        "midiin" => {
            let (device, hex) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: midiin <device> <hex bytes>")?;
            let bytes = kinetra_lib::core::midi::parse::parse_hex(hex)
                .ok_or_else(|| format!("bad MIDI bytes: {hex}"))?;
            state.inject_midi(device, &bytes);
        }
        "midiout" => {
            let sent = midi_backend().take_sent();
            value = Some(serde_json::to_string(&sent).map_err(text)?);
        }
        "midiprefs" => {
            let midi: MidiPrefs = json("MidiPrefs", arg)?;
            state.set_prefs(Prefs {
                midi,
                ..state.prefs()
            });
        }
        "mididevices" => {
            #[derive(serde::Deserialize)]
            struct Ports {
                inputs: Vec<String>,
                outputs: Vec<String>,
            }
            let p: Ports = json("{inputs, outputs}", arg)?;
            midi_backend().set_ports(p.inputs, p.outputs);
            state.midi_devices_changed();
            value = Some(serde_json::to_string(&state.midi_devices()).map_err(text)?);
        }
        "midilearn" => state.midi_learn(match arg.trim() {
            "true" => true,
            "false" => false,
            v => return Err(format!("bad flag: {v} (true or false)")),
        }),
        "midievents" => {
            value = Some(serde_json::to_string(&take_all(&debug.midi)).map_err(text)?);
        }
        "getcontrols" => {
            let doc = state.get_controls().map_err(text)?;
            value = Some(serde_json::to_string(&doc).map_err(text)?);
        }
        "setosc" => {
            let osc: OscSettings = json("OscSettings", arg)?;
            let mut settings = state.current_project().ok_or("no project")?.settings;
            settings.osc = Some(osc);
            state.set_project_settings(settings).map_err(text)?;
        }
        "lanaccess" => {
            let mut settings = state.current_project().ok_or("no project")?.settings;
            settings.lan_access = match arg.trim() {
                "true" => true,
                "false" => false,
                v => return Err(format!("bad flag: {v} (true or false)")),
            };
            state.set_project_settings(settings).map_err(text)?;
        }
        "oscstatus" => value = Some(serde_json::to_string(&osc.status()).map_err(text)?),
        "runcontrol" => {
            let (target, input) = arg
                .trim()
                .rsplit_once(' ')
                .ok_or("usage: runcontrol <ControlTarget json> <press|release|level:0.5>")?;
            let target: ControlTarget = json("ControlTarget", target)?;
            let input = match input {
                "press" => ControlInput::Press,
                "release" => ControlInput::Release,
                v => ControlInput::Level(
                    v.strip_prefix("level:")
                        .and_then(|n| n.parse::<f32>().ok())
                        .ok_or_else(|| {
                            format!("bad input: {v} (press, release or level:<0..1>)")
                        })?,
                ),
            };
            state.run_control(&target, input).map_err(text)?
        }
        "preview" => {
            let view = state.preview_view(arg.trim()).map_err(text)?;
            value = Some(serde_json::to_string(&view).map_err(text)?);
        }
        "mockapi" => {
            let (name, rest) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: mockapi <name> <json>")?;
            let script: MockScript = json("mock script", rest)?;
            mocks_lock(mocks)
                .entry(name.to_string())
                .or_default()
                .script = script;
        }
        "mockhits" => {
            let hits = mocks_lock(mocks).get(arg.trim()).map_or(0, |m| m.hits);
            value = Some(hits.to_string());
        }
        "mockreq" => {
            let last = mocks_lock(mocks)
                .get(arg.trim())
                .map(|m| m.last.clone())
                .unwrap_or_default();
            value = Some(serde_json::to_string(&last).map_err(text)?);
        }
        "apisource" => {
            let mut v: serde_json::Value = json("SourceDoc", arg)?;
            let name = v
                .get("name")
                .and_then(|n| n.as_str())
                .ok_or("apisource needs a name")?
                .to_string();
            let created = state.create_api_source(&name).map_err(text)?;
            v["id"] = serde_json::Value::String(created.id.clone());
            v["kind"] = serde_json::Value::String("api".into());
            let doc: SourceDoc = serde_json::from_value(v).map_err(text)?;
            value = Some(state.save_source(doc).map_err(text)?.id);
        }
        "apistatus" => {
            let id = arg.trim();
            let status = state
                .list_sources()
                .map_err(text)?
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(|| format!("no source {id}"))?
                .api;
            value = Some(serde_json::to_string(&status).map_err(text)?);
        }
        "setsecret" => {
            let mut it = arg.trim().splitn(3, ' ');
            let (Some(id), Some(header), Some(v)) = (it.next(), it.next(), it.next()) else {
                return Err("usage: setsecret <sourceId> <header> <value>".into());
            };
            let doc = state.set_source_secret(id, header, v).map_err(text)?;
            value = Some(serde_json::to_string(&doc).map_err(text)?);
        }
        "retryapi" => state.retry_api_source(arg.trim()).map(drop).map_err(text)?,
        "pauseapi" | "watchapi" => {
            let (id, on) = arg
                .trim()
                .split_once(' ')
                .ok_or("usage: pauseapi|watchapi <sourceId> <true|false>")?;
            let on = match on.trim() {
                "true" => true,
                "false" => false,
                v => return Err(format!("bad flag: {v} (true or false)")),
            };
            if cmd == "pauseapi" {
                state.set_api_paused(id, on).map(drop).map_err(text)?
            } else {
                state.watch_api_source(id, on).map_err(text)?
            }
        }
        "testapi" => {
            let doc: SourceDoc = json("SourceDoc", arg)?;
            let result = rt.block_on(state.test_api_source(doc)).map_err(text)?;
            value = Some(serde_json::to_string(&result).map_err(text)?);
        }
        "apitiming" => {
            let mut it = arg.split_whitespace();
            let interval = match it.next() {
                Some("off") | None => None,
                Some(v) => Some(Duration::from_millis(
                    v.parse().map_err(|_| format!("bad interval: {v}"))?,
                )),
            };
            let cap = match it.next() {
                Some(v) => Some(Duration::from_millis(
                    v.parse().map_err(|_| format!("bad cap: {v}"))?,
                )),
                None => None,
            };
            state.api().set_test_timing(interval, cap);
        }
        "apitasks" => value = Some(state.api().task_count().to_string()),
        "quit" => return Ok((true, None)),
        other => return Err(format!("unknown command: {other}")),
    }
    Ok((false, value))
}

fn main() {
    let port = match parse_port() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");

    // A throwaway project, opened the way the app opens one (screens, live state, watcher).
    let tmp = tempfile::tempdir().expect("temp dir");
    let debug = Arc::new(DebugCapture::default());
    let state = AppState::load(tmp.path().join("config"), debug.clone()).expect("app state");
    // API source secrets stay in memory: the fixture never touches the OS keychain.
    state
        .api()
        .set_secret_store(Arc::new(MemorySecrets::default()));
    let mut prefs = state.prefs();
    prefs.backup.enabled = false;
    state.set_prefs(prefs);
    // MIDI from the in-memory test device (`midiin`, `midiout`).
    state.set_midi_backend(Some(midi_backend().clone()));
    state
        .create_project(tmp.path(), "Fixture")
        .expect("fixture project");
    let hub = state.live().clone();
    // `/assets/*` from the fixture project, like the app's server manager does.
    let assets = AssetDir::default();
    assets.set_project(state.project_dir().ok());
    // Commands run on this (non-runtime) thread; auto-out and remove timers run on the fixture's
    // runtime.
    hub.set_runtime(rt.handle().clone());
    // OSC control follows the project's settings (off until `setosc`), like the app's.
    let osc = OscManager::new(&state, rt.handle().clone());
    state.add_project_observer(Arc::new(osc.clone()));
    let mocks: Mocks = Arc::default();
    let (port, first) = start(
        &rt,
        SocketAddr::from(([127, 0, 0, 1], port)),
        &hub,
        &assets,
        &mocks,
    )
    .unwrap_or_else(|e| {
        eprintln!("bind failed: {e}");
        std::process::exit(1);
    });
    // `restart` rebinds this exact port.
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut server = Some(first);

    let stdout = std::io::stdout();
    let reply = |text: String| {
        let mut out = stdout.lock();
        let _ = writeln!(out, "{text}");
        let _ = out.flush();
    };
    reply(format!("PORT={port}"));

    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        // Some writers (e.g. .NET) start the stream with a UTF-8 BOM.
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        let cmd = line.split_once(' ').map_or(line, |(c, _)| c);
        match run(
            line,
            &rt,
            &state,
            &mut server,
            addr,
            &assets,
            &debug,
            &mocks,
            &osc,
        ) {
            Ok((quit, value)) => {
                reply(format!("OK {}", value.as_deref().unwrap_or(cmd)));
                if quit {
                    break;
                }
            }
            Err(e) => reply(format!("ERR {}", e.replace('\n', " "))),
        }
    }

    if let Some(s) = server.take() {
        stop(&rt, s);
    }
    osc.shutdown();
    // Stops the watcher (joined) before the folder goes away.
    state.close_project();
    drop(state);
    rt.shutdown_timeout(Duration::from_millis(500));
    drop(tmp);
}
