//! Protocol 2 against the mock relay: negotiation, `welcome.features`, permissions on every
//! command, the v2 `crew`, and the collaborators API (protocol 2 §9).

use super::*;

/// The mock speaks protocol 2 (and 1) and announces `features`.
fn v2(e: &Env, features: &'static [&'static str]) {
    e.cfg(|c| {
        c.protocols = &["kinetra-control.v2", "kinetra-control.v1"];
        c.features = features;
    });
}

fn code(r: &Value) -> String {
    r["code"].as_str().unwrap_or("ok").to_string()
}

#[test]
fn protocol_2_is_negotiated_and_its_features_reach_the_view() {
    let e = env();
    v2(&e, &["permissions", "invites"]);
    e.live();
    let hello = &e.of("hello")[0];
    assert_eq!(hello["protocol"], 2);
    // No live picture yet: the app doesn't say it can capture.
    assert!(hello.get("preview").is_none(), "{hello}");
    let v = e.st.remote_view();
    assert_eq!(v.protocol, Some(2));
    assert!(v.invites);
    assert_eq!(v.features, vec!["permissions", "invites"]);
}

#[test]
fn a_protocol_1_server_announces_nothing() {
    let e = env();
    // Features in a protocol 1 welcome aren't protocol 1: ignored.
    e.cfg(|c| c.features = &["invites"]);
    e.live();
    assert_eq!(e.of("hello")[0]["protocol"], 1);
    let v = e.st.remote_view();
    assert_eq!(v.protocol, Some(1));
    assert!(!v.invites && v.features.is_empty());
}

#[test]
fn every_command_checks_the_actors_permissions() {
    let e = env();
    v2(&e, &["permissions"]);
    e.live();
    let s = || e.seq();
    let take = json!({"layer": "l1"});
    let edit = |v: &str| json!({"layer": "l1", "values": {"name": v}});
    let cue = json!({"cue_list": "cues", "cue": "nope"});

    // Missing the permission: `permission` (protocol 2's `forbidden`), before the target check.
    for (corr, action, args, perms) in [
        ("p1", "take", take.clone(), vec!["edit_fields", "rows"]),
        ("p2", "clear", take.clone(), vec!["run_cues"]),
        ("p3", "run_cue", cue.clone(), vec!["take_clear"]),
        (
            "p4",
            "update_fields",
            edit("x"),
            vec!["take_clear", "watch_preview"],
        ),
        // Unknown codes are ignored.
        ("p5", "take", take.clone(), vec!["take_everything"]),
    ] {
        let r = e.command_p(corr, action, args, &perms, s());
        assert_eq!(code(&r), "permission", "{corr}: {r}");
        assert_eq!(r["message"], "You don't have permission to do that.");
    }
    assert!(!e.on_air("l1"));

    // A role next to permissions changes nothing: protocol 2 reads the permissions.
    let r = e.command_by(
        "p6",
        "take",
        take.clone(),
        json!({"id": null, "name": ACTOR_NAME, "role": "operator", "permissions": []}),
        s(),
    );
    assert_eq!(code(&r), "permission");

    // With it: done.
    assert_eq!(
        code(&e.command_p("ok1", "take", take.clone(), &["take_clear"], s())),
        "ok"
    );
    wait_until("on air", || e.on_air("l1"));
    assert_eq!(
        code(&e.command_p("ok2", "run_cue", cue, &["run_cues"], s())),
        "not_found"
    );

    // On air: edit_fields alone isn't enough; edit_live_fields is (and brings edit_fields).
    let r = e.command_p("e1", "update_fields", edit("Typo"), &["edit_fields"], s());
    assert_eq!(code(&r), "on_air");
    let r = e.command_p(
        "e2",
        "update_fields",
        edit("Ingrid"),
        &["edit_live_fields"],
        s(),
    );
    assert_eq!(code(&r), "ok", "{r}");

    // Off air, edit_fields is enough.
    assert_eq!(
        code(&e.command_p("c1", "clear", take, &["take_clear"], s())),
        "ok"
    );
    wait_until("off air", || !e.on_air("l1"));
    wait_until("gone", || !e.st.live().visible(&e.screen, "l1"));
    let r = e.command_p("e3", "update_fields", edit("Kari"), &["edit_fields"], s());
    assert_eq!(code(&r), "ok", "{r}");
}

#[test]
fn the_lock_still_applies_with_permissions() {
    let e = env();
    v2(&e, &["permissions"]);
    e.live();
    e.st.remote_lock(true).unwrap();
    let r = e.command_p(
        "l1",
        "take",
        json!({"layer": "l1"}),
        &["take_clear"],
        e.seq(),
    );
    assert_eq!(code(&r), "locked");
}

#[test]
fn a_protocol_2_crew_reaches_the_view_with_permissions_and_watching() {
    let e = env();
    v2(&e, &["permissions"]);
    e.live();
    e.push(json!({"t": "crew", "connected": [{
        "id": "0193aaaa-0000-7000-8000-000000000001", "name": "Kari N.", "since_ms": 5,
        "permissions": ["take_clear", "edit_fields"],
        "watching": [{"screen": "s1", "kind": "program"}]
    }]}));
    wait_until("crew", || e.st.remote_view().crew.len() == 1);
    let m = &e.st.remote_view().crew[0];
    assert_eq!(
        m.permissions.as_deref(),
        Some(&["take_clear".to_string(), "edit_fields".to_string()][..])
    );
    assert_eq!(m.watching.len(), 1);
    assert!(m.role.is_empty());
    let ui = serde_json::to_value(m).unwrap();
    assert!(
        ui.get("anonymous").is_none() && ui.get("sinceMs").is_some(),
        "{ui}"
    );
}

fn requests(e: &Env) -> Vec<(String, String, String, Value)> {
    e.sh.api.lock().unwrap().clone()
}

fn answer(e: &Env, status: u16, body: Value) {
    e.sh.api_answers.lock().unwrap().push((status, body));
}

fn pending(id: &str, perms: &[&str]) -> Value {
    json!({"id": id, "status": "pending", "email": "kari@example.com", "name": null,
        "permissions": perms, "invited_at": "2026-10-27T10:00:00Z", "joined_at": null,
        "expires_at": "2026-11-03T10:00:00Z"})
}

#[test]
fn collaborators_are_listed_invited_changed_and_removed() {
    let e = env();
    v2(&e, &["permissions", "invites"]);
    e.live();
    let pid = e.st.project_remote_id().unwrap();
    let base = format!("/v1/control/projects/{pid}/collaborators");
    let bearer = format!("Bearer {SESSION}");

    let list = e.rt.block_on(e.st.remote_collaborators()).unwrap();
    assert_eq!((list.limit.max, list.collaborators.len()), (5, 0));
    assert_eq!(
        requests(&e)[0],
        ("GET".into(), base.clone(), bearer.clone(), Value::Null)
    );

    // Invite: the email trimmed, the codes known-only, in order, without duplicates.
    let id = "0193bbbb-0000-7000-8000-000000000009";
    answer(&e, 201, pending(id, &["edit_fields", "rows"]));
    let codes = ["rows", "edit_fields", "future_code", "rows"].map(String::from);
    let c =
        e.rt.block_on(e.st.remote_invite("  kari@example.com ", &codes))
            .unwrap();
    assert_eq!((c.status.as_str(), c.name.as_deref()), ("pending", None));
    let (m, p, auth, body) = requests(&e)[1].clone();
    assert_eq!(
        (m.as_str(), p.as_str(), auth),
        ("POST", base.as_str(), bearer.clone())
    );
    assert_eq!(
        body,
        json!({"email": "kari@example.com", "permissions": ["edit_fields", "rows"]})
    );
    // camelCase to the UI.
    let ui = serde_json::to_value(&c).unwrap();
    assert_eq!(ui["invitedAt"], "2026-10-27T10:00:00Z");
    assert_eq!(ui["expiresAt"], "2026-11-03T10:00:00Z");

    // Change permissions: edit_live_fields brings edit_fields.
    answer(&e, 200, pending(id, &["edit_fields", "edit_live_fields"]));
    e.rt.block_on(e.st.remote_collaborator_set(id, &["edit_live_fields".to_string()]))
        .unwrap();
    let (m, p, _, body) = requests(&e)[2].clone();
    assert_eq!((m.as_str(), p), ("PATCH", format!("{base}/{id}")));
    assert_eq!(
        body,
        json!({"permissions": ["edit_fields", "edit_live_fields"]})
    );

    // Remove.
    answer(&e, 204, Value::Null);
    e.rt.block_on(e.st.remote_collaborator_remove(id)).unwrap();
    let (m, p, _, _) = requests(&e)[3].clone();
    assert_eq!((m.as_str(), p), ("DELETE", format!("{base}/{id}")));

    // The server's refusals reach the UI with their code and details.
    answer(
        &e,
        409,
        json!({"error": {"code": "collaborator_limit", "message": "Limit", "details": {"max": 5}}}),
    );
    match e.rt.block_on(e.st.remote_invite("jonas@example.com", &[])) {
        Err(AppError::Cloud {
            api_code, details, ..
        }) => {
            assert_eq!(api_code, "collaborator_limit");
            assert_eq!(details, Some(json!({"max": 5})));
        }
        other => panic!("{other:?}"),
    }

    // Checked here first: no request for a bad email or an unsafe id.
    let n = requests(&e).len();
    assert!(matches!(
        e.rt.block_on(e.st.remote_invite("kari", &[])),
        Err(AppError::Invalid(_))
    ));
    assert!(matches!(
        e.rt.block_on(e.st.remote_collaborator_remove("../me")),
        Err(AppError::Invalid(_))
    ));
    assert_eq!(requests(&e).len(), n);
}

#[test]
fn inviting_is_pro_work_but_removing_and_listing_are_not() {
    let e = env();
    v2(&e, &["permissions", "invites"]);
    e.live();
    e.st.cloud_service()
        .set_test_entitlements(&["core.app"], true);
    assert!(e
        .rt
        .block_on(e.st.remote_invite("kari@example.com", &[]))
        .is_err());
    assert!(e
        .rt
        .block_on(e.st.remote_collaborator_set("0193bbbb", &[]))
        .is_err());
    assert!(requests(&e).is_empty());
    e.rt.block_on(e.st.remote_collaborators()).unwrap();
    answer(&e, 204, Value::Null);
    e.rt.block_on(e.st.remote_collaborator_remove("0193bbbb"))
        .unwrap();
    assert_eq!(requests(&e).len(), 2);
}

#[test]
fn a_project_never_switched_on_has_no_collaborators_to_manage() {
    let e = env();
    assert!(e.st.project_remote_id().is_none());
    assert!(matches!(
        e.rt.block_on(e.st.remote_collaborators()),
        Err(AppError::Invalid(_))
    ));
    assert!(requests(&e).is_empty());
}
