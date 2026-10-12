//! Live licensing check against the real Kinetra API (run by hand by a maintainer):
//! device-flow sign-in → activate → refresh → deactivate → reactivate → sign out.
//!
//! `cargo run --example license_live`
//!
//! Uses this computer's real machine fingerprint and the production trusted keys, but keeps
//! every secret in memory (never the keychain the app uses) and a temporary config folder. No
//! AI calls. Prints the code to approve in the browser and the license view after each step.

use std::sync::Arc;
use std::time::{Duration, Instant};

use kinetra_lib::core::cloud::api::SlotReq;
use kinetra_lib::core::cloud::service::SigninView;
use kinetra_lib::core::cloud::{config, machine};
use kinetra_lib::core::events::NullSink;
use kinetra_lib::core::secrets::MemorySecrets;
use kinetra_lib::core::state::AppState;

fn show(step: &str, st: &AppState) {
    let v = st.license_view();
    println!(
        "[{step}] signedIn={} status={} access={} plan={:?} entitlements={:?} slot={:?} online={:?} lastCheckAt={:?} offlineDaysLeft={} notice={:?} upgradeBlocked={}",
        v.signed_in,
        v.status,
        v.access,
        v.plan,
        v.entitlements,
        v.slot,
        v.online,
        v.last_check_at,
        v.offline_days_left,
        v.notice,
        v.upgrade_blocked.is_some()
    );
}

fn main() {
    let tmp = std::env::temp_dir().join(format!("kinetra-license-live-{}", std::process::id()));
    let st = AppState::load(tmp.join("config"), Arc::new(NullSink)).expect("state");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    st.live().set_runtime(rt.handle().clone());
    let machine = machine::fingerprint().expect("MachineGuid");
    println!(
        "machine fingerprint {machine}, name {}",
        machine::machine_name()
    );
    st.cloud_service().set_test_env(
        config::API_BASE,
        Arc::new(MemorySecrets::default()),
        &machine,
    );
    st.cloud_init();
    show("start", &st);

    // 1. Sign in (device flow): the owner approves in the browser.
    let v = rt.block_on(st.sign_in_start()).expect("device flow start");
    if let SigninView::Waiting { user_code, url, .. } = &v {
        println!("APPROVE: open {url} and check the code {user_code}");
    }
    let t = Instant::now();
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let v = st.license_view();
        if v.signed_in && v.status != "none" {
            break;
        }
        match v.signin {
            SigninView::Waiting { .. } | SigninView::Idle => {}
            other => {
                println!("sign-in ended: {other:?}");
                return;
            }
        }
        if t.elapsed() > Duration::from_secs(660) {
            println!("timed out waiting for approval");
            return;
        }
    }
    show("signed in + activated", &st);

    // 2. Refresh.
    let ok = rt.block_on(st.cloud_sync());
    show(&format!("refresh (reached={ok})"), &st);

    // 3. Deactivate this computer (its activation from GET /v1/license).
    let licenses = rt.block_on(st.license_list()).expect("license list");
    let own = licenses
        .iter()
        .flat_map(|l| l.activations.iter())
        .find(|a| a.this_machine == Some(true))
        .map(|a| a.id.clone());
    println!(
        "licenses: {:?}",
        licenses
            .iter()
            .map(|l| format!(
                "{} {} {} seats {}/{}",
                l.plan, l.kind, l.status, l.seats_used, l.seats
            ))
            .collect::<Vec<_>>()
    );
    match own {
        Some(aid) => {
            rt.block_on(st.license_deactivate(&aid))
                .expect("deactivate");
            show("deactivated", &st);
        }
        None => println!("no activation marked this_machine"),
    }

    // 4. Reactivate.
    rt.block_on(st.license_activate(SlotReq::Main, None))
        .expect("reactivate");
    show("reactivated", &st);

    // 5. Sign out (ends this test's app session on the server).
    rt.block_on(st.sign_out()).expect("sign out");
    show("signed out", &st);
    let _ = std::fs::remove_dir_all(&tmp);
}
