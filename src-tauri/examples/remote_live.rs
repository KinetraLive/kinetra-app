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
//! long before ending, printing who connects and what they may do (open the control link).
//! Prints codes, counts and states; addresses only as given; never the session or license.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kinetra_lib::core::cloud::config;
use kinetra_lib::core::events::NullSink;
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
    st.create_project(&parent, "Claude remote live test")
        .expect("project");

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

    if let Ok(addr) = std::env::var("REMOTE_LIVE_INVITE") {
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
        for p in pending {
            rt.block_on(st.remote_collaborator_remove(&p.id))
                .expect("cancel");
        }
        let list = rt.block_on(st.remote_collaborators()).expect("list");
        check(
            !list
                .collaborators
                .iter()
                .any(|x| x.email.eq_ignore_ascii_case(addr.trim())),
            "the invite was cancelled",
        );
    }

    if let Some(secs) = std::env::var("REMOTE_LIVE_HOLD")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        println!(
            "holding remote control on for {secs} s: {}",
            v.controller_url.clone().unwrap_or_default()
        );
        let until = Instant::now() + Duration::from_secs(secs);
        let mut seen = String::new();
        while Instant::now() < until {
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
        for a in st.remote_activity().iter().rev() {
            println!("activity: {}", a.text);
        }
    }

    st.remote_end().expect("end");
    st.remote_set(false).expect("off");
    println!("done (the test show stays on the account; delete it on the control site)");
}
