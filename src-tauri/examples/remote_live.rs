//! Live remote control v1.1 check against the dev API (run by hand, only with the owner's OK).
//!
//! ```text
//! KINETRA_TRUSTED_KEYS=src-tauri/license/dev-trusted-keys.json KINETRA_DEV_PROFILE=dev \
//!   KINETRA_API_BASE=https://vps-b17d613f.tail33aa62.ts.net:8445 cargo run --example remote_live
//! ```
//! Signs in as the dev profile (`cloud_live.rs` explains how). A throwaway project "Claude remote
//! live test" switches remote control on: the protocol the server picked, its features, the link.
//! Then the collaborators API: the list and the server's answers to an invite of your own
//! address, a change and a removal of a collaborator that doesn't exist (no email is sent).
//!
//! With `REMOTE_LIVE_INVITE=<address>` (the owner names it) it also invites that address as
//! Watch only, finds it pending, changes it to Data editor, invites it again (to see the server's
//! answer), and cancels the invite. `REMOTE_LIVE_HOLD=<seconds>` keeps remote control on that
//! long before ending (the invite then stays open, back at Watch only, so the invitee can join),
//! printing who connects, what they may do, what they did and what was refused. While holding,
//! lines appended to the file `REMOTE_LIVE_CMDS` act on the invitee: `perms watch|data|full|
//! <code,code>`, `invite` (again), `list`, `end`. The project has a lower third on "Main".
//! Prints codes, counts and states; addresses only as given; never the session or license.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kinetra_lib::core::cloud::config;
use kinetra_lib::core::events::NullSink;
use kinetra_lib::core::model::{NewScreen, Rect, ScreenLayer, ScreenLayerKind, TemplateSource};
use kinetra_lib::core::protocol::Fields;
use kinetra_lib::core::state::AppState;
use kinetra_lib::error::AppError;

fn check(ok: bool, what: &str) {
    println!("{} {what}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        std::process::exit(1);
    }
}

/// The API's answer to a failed call, as the UI gets it.
fn answer(r: Result<impl std::fmt::Debug, AppError>) -> String {
    match r {
        Ok(_) => "ok".into(),
        Err(AppError::Cloud {
            api_code,
            message,
            details,
        }) => format!("{api_code} \"{message}\" details={details:?}"),
        Err(e) => format!("app error: {e}"),
    }
}

fn main() {
    let profile = std::env::var("KINETRA_DEV_PROFILE").unwrap_or_default();
    let base = config::api_base();
    if profile.is_empty() || base == config::API_BASE {
        eprintln!("Set KINETRA_DEV_PROFILE and KINETRA_API_BASE (the dev API).");
        std::process::exit(2);
    }
    let tmp = std::env::temp_dir().join(format!("kinetra-remote-live-{}", std::process::id()));
    let config_dir = tmp.join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let appdata = std::env::var("APPDATA").unwrap();
    let theirs = Path::new(&appdata)
        .join("no.rorvikmedieteknikk.kinetra")
        .join(format!("dev-{profile}"))
        .join("license.json");
    std::fs::copy(&theirs, config_dir.join("license.json"))
        .expect("the dev profile's license.json");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let st = AppState::load(config_dir.clone(), Arc::new(NullSink)).expect("state");
    st.live().set_runtime(rt.handle().clone());
    st.cloud_init();
    let reached = rt.block_on(st.cloud_sync());
    let lv = st.license_view();
    println!(
        "license sync reached the server: {reached}; signed in: {}; status={} control={}",
        lv.signed_in, lv.status, lv.features.control
    );
    check(lv.signed_in, "the dev profile is signed in");
    check(lv.features.control, "the dev license has remote control");
    let own = lv
        .account
        .as_ref()
        .map(|a| a.email.clone())
        .unwrap_or_default();

    let parent = tmp.join("shows");
    std::fs::create_dir_all(&parent).unwrap();
    // `REMOTE_LIVE_PROJECT=<folder>` reopens an earlier run's project (the same show and its
    // collaborators).
    if let Ok(p) = std::env::var("REMOTE_LIVE_PROJECT") {
        st.open_project(Path::new(&p)).expect("open project");
    } else {
        st.create_project(&parent, "Claude remote live test")
            .expect("project");
        let screen = add_lower_third(&st);
        add_v2_show(&st, &screen);
    }
    println!(
        "project: {}",
        st.project_dir().map(|d| d.display().to_string()).unwrap_or_default()
    );

    // Remote control on: protocol 2 and its features.
    st.remote_set(true).expect("remote control on");
    let t = Instant::now();
    loop {
        let v = st.remote_view();
        let state = serde_json::to_value(&v.phase).unwrap()["state"].clone();
        if state == "live" {
            break;
        }
        if state == "stopped" || t.elapsed() > Duration::from_secs(30) {
            println!("phase: {:?}", v.phase);
            check(false, "remote control went live");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let v = st.remote_view();
    println!(
        "live: protocol={:?} features={:?} invites={} server={:?}",
        v.protocol, v.features, v.invites, v.server_version
    );
    let host = |u: &Option<String>| {
        u.as_deref()
            .and_then(|u| u.split('/').nth(2))
            .unwrap_or("-")
            .to_string()
    };
    println!(
        "control link on {} (crew page on {})",
        host(&v.controller_url),
        host(&v.crew_url)
    );
    check(v.protocol == Some(2), "the server picked protocol 2");
    check(
        v.features.iter().any(|f| f == "permissions"),
        "permissions are announced",
    );
    check(v.invites, "invites are announced");
    check(v.controller_url.is_some(), "the control link arrived");

    // The collaborators API.
    let list = rt.block_on(st.remote_collaborators()).expect("list");
    println!(
        "collaborators: {} (limit {} of {}); statuses {:?}",
        list.collaborators.len(),
        list.limit.used,
        list.limit.max,
        list.collaborators
            .iter()
            .map(|c| c.status.as_str())
            .collect::<Vec<_>>()
    );
    if !own.is_empty() {
        println!(
            "invite own address → {}",
            answer(rt.block_on(st.remote_invite(&own, &["watch_preview".into()])))
        );
    }
    let nobody = "0193aaaa-0000-7000-8000-000000000001";
    println!(
        "change a collaborator that doesn't exist → {}",
        answer(rt.block_on(st.remote_collaborator_set(nobody, &["watch_preview".into()])))
    );
    println!(
        "remove a collaborator that doesn't exist → {}",
        answer(rt.block_on(st.remote_collaborator_remove(nobody)))
    );

    let hold = std::env::var("REMOTE_LIVE_HOLD")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());
    let invitee = std::env::var("REMOTE_LIVE_INVITE").ok();
    // Holding with an invite already open (an earlier run): reuse it, no second email.
    let open = invitee.as_deref().and_then(|addr| {
        let list = rt.block_on(st.remote_collaborators()).ok()?;
        list.collaborators
            .into_iter()
            .find(|x| x.email.eq_ignore_ascii_case(addr.trim()))
    });
    if let (Some(c), Some(_)) = (&open, hold) {
        println!(
            "reusing the open invite: status={} permissions={:?}",
            c.status, c.permissions
        );
        let watch = ["watch_preview".to_string()];
        rt.block_on(st.remote_collaborator_set(&c.id, &watch))
            .expect("back to Watch only");
    } else if let Some(addr) = invitee.clone() {
        let c = rt
            .block_on(st.remote_invite(&addr, &["watch_preview".into()]))
            .expect("invite");
        println!(
            "invited: status={} permissions={:?} expires={:?}",
            c.status, c.permissions, c.expires_at
        );
        check(c.status == "pending", "the invite is pending");
        let list = rt.block_on(st.remote_collaborators()).expect("list");
        check(
            list.collaborators.iter().any(|x| x.id == c.id),
            "the invite is listed",
        );
        let data = ["edit_fields", "rows", "watch_preview"].map(String::from);
        let changed = rt
            .block_on(st.remote_collaborator_set(&c.id, &data))
            .expect("change");
        check(
            changed.permissions.len() == 3,
            "the permissions changed to Data editor",
        );
        println!(
            "invite the same address again → {}",
            answer(rt.block_on(st.remote_invite(&addr, &["watch_preview".into()])))
        );
        let list = rt.block_on(st.remote_collaborators()).expect("list");
        let pending: Vec<_> = list
            .collaborators
            .iter()
            .filter(|x| x.email.eq_ignore_ascii_case(addr.trim()))
            .collect();
        println!(
            "rows for that address now: {} ({:?})",
            pending.len(),
            pending.iter().map(|x| &x.status).collect::<Vec<_>>()
        );
        if hold.is_none() {
            cancel(&rt, &st, &addr);
        } else {
            // Inviting a pending address again replaces the invite (a new id), so find it again.
            let list = rt.block_on(st.remote_collaborators()).expect("list");
            let row = list
                .collaborators
                .iter()
                .find(|x| x.email.eq_ignore_ascii_case(addr.trim()))
                .expect("the invite");
            println!("the invite now: id changed={}", row.id != c.id);
            let watch = ["watch_preview".to_string()];
            rt.block_on(st.remote_collaborator_set(&row.id, &watch))
                .expect("back to Watch only");
            println!("the invite stays open (Watch only) while holding");
        }
    }

    if let Some(secs) = hold {
        println!(
            "holding remote control on for {secs} s: {}",
            v.controller_url.clone().unwrap_or_default()
        );
        let until = Instant::now() + Duration::from_secs(secs);
        let mut seen = String::new();
        let cmds = std::env::var("REMOTE_LIVE_CMDS").ok();
        let read_cmds = || -> Vec<String> {
            cmds.as_deref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|t| t.lines().map(str::to_string).collect())
                .unwrap_or_default()
        };
        let mut done_lines = read_cmds().len();
        let (mut acts, mut refused) = (0usize, 0usize);
        'hold: while Instant::now() < until {
            // Newest first: print what came since the last look, oldest of those first.
            let mut activity = st.remote_activity();
            activity.reverse();
            for a in activity.iter().skip(acts) {
                println!("activity: {} ({})", a.text, a.role);
            }
            acts = activity.len();
            let r = st.remote_diagnostics()["refused"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for x in r.iter().skip(refused) {
                println!(
                    "refused: {} {} → {}",
                    x["action"].as_str().unwrap_or("?"),
                    x["layer"].as_str().or(x["cue"].as_str()).unwrap_or(""),
                    x["code"].as_str().unwrap_or("?")
                );
            }
            refused = r.len();
            let lines = read_cmds();
            for line in lines.iter().skip(done_lines) {
                let line = line.trim();
                println!("> {line}");
                if line == "end" {
                    break 'hold;
                }
                if let Some(addr) = invitee.as_deref() {
                    run_command(&rt, &st, addr, line);
                }
            }
            done_lines = done_lines.max(lines.len());
            let crew: Vec<String> = st
                .remote_view()
                .crew
                .iter()
                .map(|m| {
                    format!(
                        "{} {:?} watching={}",
                        m.name,
                        m.permissions,
                        m.watching.len()
                    )
                })
                .collect();
            let now = crew.join("; ");
            if now != seen {
                println!("crew: [{now}]");
                seen = now;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        if let Some(addr) = invitee.as_deref() {
            cancel(&rt, &st, addr);
        }
    }

    st.remote_end().expect("end");
    st.remote_set(false).expect("off");
    println!("done (the test show stays on the account; delete it on the control site)");
}

/// Protocol 2's parts: a guest list bound to the lower third (rows), a countdown (timer), a cue
/// that takes the countdown, and a button panel using every kind of step.
fn add_v2_show(st: &AppState, screen: &str) {
    let dir = st.project_dir().expect("project dir");
    let cd = st
        .templates()
        .create(
            &dir,
            "Countdown",
            &TemplateSource::Preset {
                preset_id: "countdown".into(),
            },
        )
        .expect("countdown template")
        .id;
    // The guest list.
    let mut src = st.create_source("Guests").expect("source");
    let row = |name: &str, title: &str| {
        Fields::from([
            ("name".to_string(), name.to_string()),
            ("title".to_string(), title.to_string()),
        ])
    };
    src.meta.kind = serde_json::from_value(serde_json::json!({
        "kind": "text",
        "columns": [{"key": "name", "label": "Name"}, {"key": "title", "label": "Title"}],
        "rows": [row("Kari Nordmann", "Producer"), row("Ola Hansen", "Director"),
                 row("Ingrid Vale", "Host"), row("Per Berg", "Guest")],
        "index": 0,
    }))
    .expect("source kind");
    let sid = st.save_source(src).expect("save source").id;
    // Bind the lower third to it; add the countdown.
    let mut doc = st.screens().doc(screen).expect("screen");
    for l in &mut doc.meta.layers {
        if let ScreenLayerKind::Template { bindings, .. } = &mut l.kind {
            for col in ["name", "title"] {
                bindings.insert(
                    col.into(),
                    serde_json::from_value(serde_json::json!({"sourceId": sid, "column": col}))
                        .expect("binding"),
                );
            }
        }
    }
    doc.meta.layers.push(ScreenLayer {
        id: "cd1".into(),
        name: "Countdown".into(),
        rect: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        kind: ScreenLayerKind::Template {
            template_id: cd,
            fields: [("title", "Back in")]
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .into(),
            vars: Fields::new(),
            bindings: Default::default(),
            variant: None,
            sizing: None,
            opacity: None,
        },
        anchor: None,
    });
    st.save_screen(doc).expect("save screen");
    // A cue that takes the countdown.
    let cue: kinetra_lib::core::model::NewFlow = serde_json::from_value(serde_json::json!({
        "name": "Countdown on", "armed": true, "retrigger": "restart", "origin": null,
        "nodes": [
            {"id": "c", "position": {"x": 0, "y": 0}, "type": "trigger.cue", "config": {}},
            {"id": "t", "position": {"x": 200, "y": 0}, "type": "action.take",
             "config": {"screenId": screen, "layerId": "cd1"}},
        ],
        "edges": [{"id": "e1", "from": {"node": "c", "port": "fired"},
                   "to": {"node": "t", "port": "in"}, "kind": "event"}],
    }))
    .expect("cue flow");
    let flow = st.create_flow(cue).expect("create cue").flow.id;
    // The button panel.
    let panel: kinetra_lib::core::model::PanelDoc = serde_json::from_value(serde_json::json!({
        "buttons": [
            {"id": "b1guest0on", "label": "Guest on", "color": "green",
             "actions": [{"kind": "take", "screen": screen, "layer": "lt1"}]},
            {"id": "b2guest0of", "label": "Guest off", "color": "grey",
             "actions": [{"kind": "clear", "screen": screen, "layer": "lt1"}]},
            {"id": "b3breaking", "label": "Breaking news", "color": "red", "confirm": true,
             "actions": [{"kind": "setFields", "screen": screen, "layer": "lt1",
                          "values": {"name": "Breaking news", "title": "Live from Oslo"}},
                         {"kind": "take", "screen": screen, "layer": "lt1"}]},
            {"id": "b4clock0go", "label": "Start clock", "color": "blue",
             "actions": [{"kind": "timer", "screen": screen, "layer": "cd1",
                          "field": "countdown", "op": "start"},
                         {"kind": "runCue", "flow": flow}]},
            {"id": "b5studio00", "label": "Take Preview", "color": "orange",
             "actions": [{"kind": "studioTake", "screen": screen, "mode": "take"}]},
        ]
    }))
    .expect("panel");
    let v = st.save_panel(panel).expect("save panel");
    println!(
        "v2 show: guest list {sid}, countdown cd1, cue {flow}, {} buttons, {} panel issues",
        v.doc.buttons.len(),
        v.issues.len()
    );
}

/// Something to take and edit: a lower third on "Main". Returns the screen id.
fn add_lower_third(st: &AppState) -> String {
    let dir = st.project_dir().expect("project dir");
    let lt = st
        .templates()
        .create(
            &dir,
            "Lower third",
            &TemplateSource::Preset {
                preset_id: "lower-third".into(),
            },
        )
        .expect("template")
        .id;
    let mut doc = st
        .create_screen(&NewScreen {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .expect("screen")
        .doc;
    let fields: Fields = [("name", "Kari Nordmann"), ("title", "Producer")]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .into();
    doc.meta.layers.push(ScreenLayer {
        id: "lt1".into(),
        name: "Lower third".into(),
        rect: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        kind: ScreenLayerKind::Template {
            template_id: lt,
            fields,
            vars: Fields::new(),
            bindings: Default::default(),
            variant: None,
            sizing: None,
            opacity: None,
        },
        anchor: None,
    });
    let id = doc.id.clone();
    st.save_screen(doc).expect("save screen");
    id
}

/// Removes every row for `addr` (pending or joined) and checks none is left.
fn cancel(rt: &tokio::runtime::Runtime, st: &AppState, addr: &str) {
    let list = rt.block_on(st.remote_collaborators()).expect("list");
    for p in list
        .collaborators
        .iter()
        .filter(|x| x.email.eq_ignore_ascii_case(addr.trim()))
    {
        rt.block_on(st.remote_collaborator_remove(&p.id))
            .expect("cancel");
    }
    let list = rt.block_on(st.remote_collaborators()).expect("list");
    check(
        !list
            .collaborators
            .iter()
            .any(|x| x.email.eq_ignore_ascii_case(addr.trim())),
        "the invitee was removed",
    );
}

/// One line of the command file, acting on the invitee's row.
fn run_command(rt: &tokio::runtime::Runtime, st: &AppState, addr: &str, line: &str) {
    let list = match rt.block_on(st.remote_collaborators()) {
        Ok(l) => l,
        Err(e) => return println!("list → {}", answer(Err::<(), _>(e))),
    };
    let row = list
        .collaborators
        .iter()
        .find(|x| x.email.eq_ignore_ascii_case(addr.trim()));
    let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
    let screen = st
        .project_dir()
        .ok()
        .and_then(|d| st.screens().list(&d).ok())
        .and_then(|l| l.into_iter().find(|s| s.name == "Main"))
        .map(|s| s.id)
        .unwrap_or_default();
    match cmd {
        // This computer's own controls, to watch the control site follow.
        "studio" => {
            use kinetra_lib::core::model::{
                PreviewAfterTake, StudioExit, StudioSettings, TakeTransition, TakeTransitionKind,
            };
            let r = st.set_studio(
                StudioSettings {
                    enabled: arg.trim() == "on",
                    preview_after_take: PreviewAfterTake::Keep,
                    transition: TakeTransition {
                        kind: TakeTransitionKind::Crossfade,
                        ms: 300,
                    },
                },
                Some(StudioExit::Discard),
            );
            println!("studio {} → {}", arg.trim(), answer(r));
        }
        "take" | "clear" => {
            use kinetra_lib::core::control_actions::ControlInput;
            use kinetra_lib::core::model::ControlTarget;
            let r = st.run_control(
                &ControlTarget::Command {
                    command_id: format!("live.{cmd}:{screen}/{}", arg.trim()),
                },
                ControlInput::Press,
            );
            println!("{cmd} {} → {}", arg.trim(), answer(r));
        }
        "list" => {
            for c in &list.collaborators {
                println!(
                    "  {} {} {:?} named={}",
                    c.email,
                    c.status,
                    c.permissions,
                    c.name.is_some()
                );
            }
        }
        "invite" => println!(
            "invite again → {}",
            answer(rt.block_on(st.remote_invite(addr, &["watch_preview".into()])))
        ),
        "perms" => {
            let codes: Vec<String> = match arg.trim() {
                "watch" => vec!["watch_preview".into()],
                "data" => ["edit_fields", "rows", "watch_preview"]
                    .map(String::from)
                    .to_vec(),
                "full" => kinetra_lib::core::remote::perms::ALL
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                other => other.split(',').map(|s| s.trim().to_string()).collect(),
            };
            match row {
                Some(r) => println!(
                    "perms {codes:?} → {}",
                    answer(rt.block_on(st.remote_collaborator_set(&r.id, &codes)))
                ),
                None => println!("no row for the invitee"),
            }
        }
        _ => println!("unknown command"),
    }
}
