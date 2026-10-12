//! Every license state through `evaluate_token`, the on-air
//! rule, and the crate's committed test vectors run through our wrapper.

use ed25519_dalek::SigningKey;
use kinetra_license::entitlements::{CORE_APP, PRO_AI, PRO_POLLS};
use kinetra_license::evaluate::{Access, Status};
use kinetra_license::keys::{key_info, KeyStatus};
use kinetra_license::revocation::RevocationList;
use kinetra_license::token::sign_license;
use kinetra_license::{KeySet, LicenseClaims, Slot};
use serde_json::Value;

use super::*;

const MACHINE: &str = "857a78b7a23b14cd718a9cbf9b90b0577225543ddcce5ed21854a4985f25ee27";
const T0: i64 = 1_791_590_400;
const DAY: i64 = 86_400;

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}

fn keys() -> KeySet {
    KeySet::from_infos(&[key_info("test-k1", &key(1), KeyStatus::Active)], 0).unwrap()
}

fn claims() -> LicenseClaims {
    LicenseClaims {
        typ: "license".into(),
        v: 1,
        lid: "lic-1".into(),
        aid: "act-1".into(),
        sub: "acc-1".into(),
        plan: "pro_yearly".into(),
        ent: vec![CORE_APP.into(), PRO_AI.into(), PRO_POLLS.into()],
        machine: MACHINE.into(),
        slot: Slot::Main,
        iat: T0,
        grace: T0 + 30 * DAY,
        reduced: T0 + 44 * DAY,
        ends_at: Some(T0 + 365 * DAY),
        updates_until: None,
        nonce: None,
    }
}

fn free_claims() -> LicenseClaims {
    LicenseClaims {
        plan: "free".into(),
        ent: vec![CORE_APP.into()],
        ends_at: None,
        ..claims()
    }
}

fn token(c: &LicenseClaims) -> String {
    sign_license(c, "test-k1", &key(1))
}

fn eval_at(c: &LicenseClaims, now: i64) -> Eval {
    eval_with(Some(&token(c)), now, T0, None)
}

fn eval_with(
    token: Option<&str>,
    now: i64,
    last_server_time: i64,
    rev: Option<&RevocationList>,
) -> Eval {
    let k = keys();
    evaluate_token(
        token,
        &Context {
            keys: &k,
            machine: MACHINE,
            now,
            last_server_time,
            release_date: T0,
            revocations: rev,
        },
    )
}

fn has(e: &Eval, ent: &str) -> bool {
    e.entitlements.iter().any(|x| x == ent)
}

#[test]
fn no_license_is_core_app_only() {
    let e = eval_with(None, T0, 0, None);
    assert_eq!((e.status, e.access), (None, Access::Free));
    assert_eq!(e.entitlements, vec![CORE_APP.to_string()]);
}

#[test]
fn free_license_is_core_app() {
    let e = eval_at(&free_claims(), T0 + DAY);
    assert_eq!(e.status, Some(Status::Active));
    assert!(has(&e, CORE_APP) && !has(&e, PRO_AI));
}

#[test]
fn pro_online_has_pro_entitlements() {
    let e = eval_at(&claims(), T0 + DAY);
    assert_eq!((e.status, e.access), (Some(Status::Active), Access::Full));
    assert!(has(&e, PRO_AI) && has(&e, CORE_APP));
    assert_eq!(e.offline_days_left, 29);
}

#[test]
fn offline_grace_reduced_then_free() {
    // Within the 30-day grace: full.
    assert_eq!(eval_at(&claims(), T0 + 29 * DAY).access, Access::Full);
    // Reduced mode: entitlements still listed (existing Pro work keeps running).
    let r = eval_at(&claims(), T0 + 31 * DAY);
    assert_eq!(
        (r.status, r.access),
        (Some(Status::Reduced), Access::Reduced)
    );
    assert!(has(&r, PRO_AI));
    assert_eq!(r.offline_days_left, 0);
    // Past reduced: Free.
    let f = eval_at(&claims(), T0 + 45 * DAY);
    assert_eq!(
        (f.status, f.access),
        (Some(Status::OfflineExpired), Access::Free)
    );
    assert!(!has(&f, PRO_AI) && has(&f, CORE_APP));
}

#[test]
fn expired_subscription_is_free() {
    let mut c = claims();
    c.ends_at = Some(T0 + 2 * DAY);
    let e = eval_at(&c, T0 + 3 * DAY);
    assert_eq!((e.status, e.access), (Some(Status::Expired), Access::Free));
    assert!(!has(&e, PRO_AI));
}

#[test]
fn revoked_on_the_list_is_free() {
    let list = RevocationList {
        typ: "revocations".into(),
        v: 1,
        iat: T0,
        licenses: vec![],
        activations: vec!["act-1".into()],
    };
    let e = eval_with(Some(&token(&claims())), T0 + DAY, T0, Some(&list));
    assert_eq!((e.status, e.access), (Some(Status::Revoked), Access::Free));
    // Another activation on the list doesn't touch ours.
    let other = RevocationList {
        activations: vec!["act-9".into()],
        ..list
    };
    assert_eq!(
        eval_with(Some(&token(&claims())), T0 + DAY, T0, Some(&other)).access,
        Access::Full
    );
}

#[test]
fn clock_turned_back_is_free_until_a_refresh() {
    // The local clock is a day behind a server time already seen.
    let e = eval_with(Some(&token(&claims())), T0 + 5 * DAY, T0 + 6 * DAY, None);
    assert_eq!(
        (e.status, e.access),
        (Some(Status::ClockTampered), Access::Free)
    );
    // Within the 5-minute tolerance it isn't tampering.
    let ok = eval_with(
        Some(&token(&claims())),
        T0 + 6 * DAY - 60,
        T0 + 6 * DAY,
        None,
    );
    assert_eq!(ok.access, Access::Full);
    // Behind the token's own iat counts too (no stored server time needed).
    assert_eq!(
        eval_with(Some(&token(&claims())), T0 - DAY, 0, None).status,
        Some(Status::ClockTampered)
    );
}

#[test]
fn perpetual_not_covered_for_newer_builds() {
    let mut c = claims();
    c.ends_at = None;
    c.updates_until = Some(T0 - DAY); // release_date T0 is after it
    let e = eval_at(&c, T0 + DAY);
    assert_eq!(
        (e.status, e.access),
        (Some(Status::NotCovered), Access::Free)
    );
}

#[test]
fn beta_license_ends_hard_and_has_a_short_grace() {
    // A beta: subscription to the program end, 3-day grace, no reduced period.
    let mut c = claims();
    c.plan = "beta".into();
    c.grace = T0 + 3 * DAY;
    c.reduced = T0 + 3 * DAY;
    c.ends_at = Some(T0 + 20 * DAY);
    assert_eq!(eval_at(&c, T0 + 2 * DAY).access, Access::Full);
    assert_eq!(
        eval_at(&c, T0 + 4 * DAY).status,
        Some(Status::OfflineExpired)
    );
    assert_eq!(eval_at(&c, T0 + 21 * DAY).status, Some(Status::Expired));
}

#[test]
fn forged_other_machine_or_unknown_key_is_no_license() {
    // Signed by a key we don't trust.
    let forged = sign_license(&claims(), "test-k1", &key(9));
    assert_eq!(eval_with(Some(&forged), T0 + DAY, T0, None), Eval::none());
    // A token for another machine.
    let mut other = claims();
    other.machine = "0".repeat(64);
    assert_eq!(eval_at(&other, T0 + DAY), Eval::none());
    // An edited payload.
    let t = token(&claims());
    let mut parts: Vec<String> = t.split('.').map(String::from).collect();
    parts[2] = parts[2].replacen('e', "f", 1);
    assert_eq!(
        eval_with(Some(&parts.join(".")), T0 + DAY, T0, None),
        Eval::none()
    );
}

#[test]
fn seat_moved_to_free_after_downgrade() {
    // A refresh that moved this computer from Pro to Free (seat taken elsewhere / Pro ended)
    // gives a Free token: Pro is gone, core.app stays.
    let e = eval_at(&free_claims(), T0 + DAY);
    assert!(!has(&e, PRO_AI) && has(&e, CORE_APP));
}

#[test]
fn losing_access_waits_while_on_air() {
    let pro: Vec<String> = vec![CORE_APP.into(), PRO_AI.into()];
    let free: Vec<String> = vec![CORE_APP.into()];
    // Downgrade on air: nothing changes yet, and it's pending.
    let e = settle(&pro, &free, true);
    assert_eq!(e, pro);
    assert!(pending(&e, &free));
    // Off air: applies.
    let e = settle(&e, &free, false);
    assert_eq!(e, free);
    assert!(!pending(&e, &free));
    // Upgrade on air applies at once.
    assert_eq!(settle(&free, &pro, true), pro);
    // Mixed: gains now, losses wait.
    let a: Vec<String> = vec![CORE_APP.into(), PRO_POLLS.into()];
    let b: Vec<String> = vec![CORE_APP.into(), PRO_AI.into()];
    let e = settle(&a, &b, true);
    assert!(e.contains(&PRO_AI.to_string()) && e.contains(&PRO_POLLS.to_string()));
}

#[test]
fn nonces_are_url_safe_and_fresh() {
    let a = nonce().unwrap();
    let b = nonce().unwrap();
    assert_eq!(a.len(), 32);
    assert_ne!(a, b);
    assert!(a
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
}

#[test]
fn clock_skew_details() {
    let d = serde_json::json!({"server_time": 1_791_590_400});
    assert_eq!(skew_server_time(Some(&d)), Some(1_791_590_400));
    assert_eq!(skew_server_time(None), None);
}

/// The crate's committed vectors (`license/test-vectors.json`, copied from
/// kinetra-license `tests/vectors/tokens.json` at the pinned tag) through our wrapper.
#[test]
fn crate_test_vectors_through_evaluate_token() {
    let v: Value =
        serde_json::from_str(include_str!("../../../../license/test-vectors.json")).unwrap();
    let keys = KeySet::from_trusted_json(&v["keyset"].to_string()).unwrap();
    let machine = v["machine"].as_str().unwrap();
    let base_now = v["now"].as_i64().unwrap();
    for t in v["vectors"].as_array().unwrap() {
        let name = t["name"].as_str().unwrap();
        let token = t["token"].as_str().unwrap();
        let now = t["now"].as_i64().unwrap_or(base_now);
        let release = t["app_release_date"].as_i64().unwrap_or(0);
        // A vector may verify against another machine (wrong_machine).
        let machine = t["machine"].as_str().unwrap_or(machine);
        let e = evaluate_token(
            Some(token),
            &Context {
                keys: &keys,
                machine,
                now,
                last_server_time: 0,
                release_date: release,
                revocations: None,
            },
        );
        let ok = t["verify"].as_str() == Some("ok");
        if !ok {
            assert_eq!(e, Eval::none(), "{name}: must not verify");
            continue;
        }
        let want = match t["status"].as_str().unwrap() {
            "active" => Status::Active,
            "expired" => Status::Expired,
            "not_covered" => Status::NotCovered,
            "reduced" => Status::Reduced,
            "offline_expired" => Status::OfflineExpired,
            "revoked" => Status::Revoked,
            "clock_tampered" => Status::ClockTampered,
            other => panic!("{name}: unknown status {other}"),
        };
        assert_eq!(e.status, Some(want), "{name}");
    }
}
