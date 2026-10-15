//! Which cloud project a folder is: copies get their own cloud ID, moved folders keep theirs,
//! Kinetra's Move keeps it at the destination; registration in the background; the two quiet
//! questions (in the trash, another computer's backup) and their answers. Through `AppState`
//! against the local mock.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use super::tests::{err, ok, project_json, quota_json, PID};
use super::tests_service::{env, wait_until, Env, ACCOUNT};
use crate::core::model::{CloudAnswer, CloudQuestion, ProjectFile};
use crate::core::{persist, project};

fn new_project(parent: &Path, name: &str, cloud_id: Option<&str>) -> PathBuf {
    let (dir, mut file) = project::create(parent, name).unwrap();
    file.cloud_id = cloud_id.map(str::to_string);
    persist::save(&dir, &file).unwrap();
    dir
}

fn cloud_id_on_disk(dir: &Path) -> Option<String> {
    persist::load_strict::<ProjectFile>(dir).unwrap().cloud_id
}

/// `cloud.json` as the service holds it (written to disk on the blocking pool; the slice 2
/// tests check the file itself).
fn cloud_json(e: &Env) -> serde_json::Value {
    serde_json::to_value(e.st.cloud_file()).unwrap()
}

fn record(e: &Env, cid: &str) -> serde_json::Value {
    cloud_json(e)["accounts"][ACCOUNT]["projects"][cid].clone()
}

const CID: &str = "0199e2a0-cccc-7000-8000-00000000000c";

/// Signed in with Pro and an `ok` quota, without registering anything yet.
fn ready(e: &Env) {
    e.signed_in();
    e.pro(true);
    e.mock
        .on("GET /v1/cloud/quota", vec![ok(200, quota_json("ok"))]);
    e.refresh(false).unwrap();
}

fn registers(e: &Env) -> Vec<serde_json::Value> {
    e.mock
        .calls_to("POST /v1/cloud/projects")
        .iter()
        .map(|c| c.json())
        .collect()
}

// ---- Identity ----------------------------------------------------------------------------------

#[test]
fn a_copied_folder_gets_its_own_cloud_id_and_the_original_keeps_its_own() {
    let e = env();
    let shows = e.tmp.path().join("shows");
    std::fs::create_dir_all(&shows).unwrap();
    let a = new_project(&shows, "Election", Some(CID));
    e.st.open_project(&a).unwrap();
    assert_eq!(cloud_json(&e)["folders"][CID], a.to_string_lossy().as_ref());
    // Copied in Explorer: same project.json.
    let b = shows.join("Election copy");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::copy(a.join("project.json"), b.join("project.json")).unwrap();
    e.st.open_project(&b).unwrap();
    e.st.flush_all();
    let b_id = cloud_id_on_disk(&b).expect("the copy has a cloud id");
    assert_ne!(b_id, CID);
    assert!(super::api::is_id(&b_id), "{b_id}");
    // The original is untouched and still recognized as itself.
    assert_eq!(cloud_id_on_disk(&a).as_deref(), Some(CID));
    e.st.open_project(&a).unwrap();
    e.st.flush_all();
    assert_eq!(cloud_id_on_disk(&a).as_deref(), Some(CID));
    let folders = &cloud_json(&e)["folders"];
    assert_eq!(folders[CID], a.to_string_lossy().as_ref());
    assert_eq!(folders[b_id.as_str()], b.to_string_lossy().as_ref());
}

#[test]
fn a_folder_moved_outside_kinetra_keeps_its_cloud_id() {
    let e = env();
    let shows = e.tmp.path().join("shows");
    std::fs::create_dir_all(&shows).unwrap();
    let a = new_project(&shows, "Gala", Some(CID));
    e.st.open_project(&a).unwrap();
    e.st.close_project();
    let moved = shows.join("Gala 2026");
    std::fs::rename(&a, &moved).unwrap();
    e.st.open_project(&moved).unwrap();
    e.st.flush_all();
    assert_eq!(cloud_id_on_disk(&moved).as_deref(), Some(CID));
    assert_eq!(
        cloud_json(&e)["folders"][CID],
        moved.to_string_lossy().as_ref()
    );
}

#[test]
fn kinetras_move_keeps_the_cloud_id_at_the_destination() {
    let e = env();
    let shows = e.tmp.path().join("shows");
    std::fs::create_dir_all(&shows).unwrap();
    let a = new_project(&shows, "Gala", Some(CID));
    e.st.open_project(&a).unwrap();
    // "Move project" without deleting the original: the open project switches to the copy.
    let dest = shows.join("Gala on the NAS");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::copy(a.join("project.json"), dest.join("project.json")).unwrap();
    e.st.relocate_project(&a, dest.clone()).unwrap();
    e.st.flush_all();
    assert_eq!(cloud_id_on_disk(&dest).as_deref(), Some(CID));
    assert_eq!(
        cloud_json(&e)["folders"][CID],
        dest.to_string_lossy().as_ref()
    );
    // The folder left behind is now the copy.
    e.st.open_project(&a).unwrap();
    e.st.flush_all();
    assert_ne!(cloud_id_on_disk(&a).as_deref(), Some(CID));
}

// ---- Registration -------------------------------------------------------------------------------

#[test]
fn opening_registers_in_the_background_with_a_new_cloud_id() {
    let e = env();
    ready(&e);
    e.mock.on(
        "POST /v1/cloud/projects",
        vec![ok(201, project_json("set-by-test", "Election night"))],
    );
    let shows = e.tmp.path().join("shows");
    std::fs::create_dir_all(&shows).unwrap();
    let a = new_project(&shows, "Election night", None);
    e.st.open_project(&a).unwrap();
    wait_until("registered", || {
        e.st.cloud_view().project.is_some_and(|p| p.registered)
    });
    e.st.flush_all();
    let cid = cloud_id_on_disk(&a).expect("a cloud id was written");
    let bodies = registers(&e);
    assert_eq!(bodies.len(), 1);
    assert_eq!(
        bodies[0],
        json!({"app_project_id": cid, "name": "Election night", "if_trashed": "error"})
    );
    assert_eq!(record(&e, &cid)["serverId"], PID);
    let p = e.st.cloud_view().project.unwrap();
    assert!(p.backup && !p.notice_seen && p.question.is_none());
    // Opening it again doesn't register again.
    e.st.open_project(&a).unwrap();
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(registers(&e).len(), 1);
}

#[test]
fn nothing_registers_for_free_lapsed_or_backup_off() {
    let e = env();
    e.signed_in();
    e.pro(false);
    let shows = e.tmp.path().join("shows");
    std::fs::create_dir_all(&shows).unwrap();
    let a = new_project(&shows, "Free show", None);
    e.st.open_project(&a).unwrap();
    std::thread::sleep(Duration::from_millis(80));
    assert!(e.mock.calls().is_empty(), "{:?}", e.mock.calls());
    // Pro, but the project's backup is off: still nothing.
    e.pro(true);
    e.mock
        .on("GET /v1/cloud/quota", vec![ok(200, quota_json("lapsed"))]);
    e.refresh(false).unwrap();
    std::thread::sleep(Duration::from_millis(80));
    assert!(registers(&e).is_empty(), "lapsed accounts don't register");
    e.mock
        .on("GET /v1/cloud/quota", vec![ok(200, quota_json("ok"))]);
    e.st.cloud_set_backup(false).unwrap();
    e.refresh(false).unwrap();
    std::thread::sleep(Duration::from_millis(80));
    assert!(registers(&e).is_empty());
    assert!(!e.st.cloud_view().project.unwrap().backup);
    // Turning it on registers.
    e.mock.on(
        "POST /v1/cloud/projects",
        vec![ok(201, project_json("x", "Free show"))],
    );
    e.st.cloud_set_backup(true).unwrap();
    wait_until("registered", || registers(&e).len() == 1);
}

#[test]
fn new_projects_follow_the_computers_default() {
    let e = env();
    ready(&e);
    let mut prefs = e.st.prefs();
    prefs.cloud.new_projects = false;
    e.st.set_prefs(prefs);
    let shows = e.tmp.path().join("shows");
    std::fs::create_dir_all(&shows).unwrap();
    let a = new_project(&shows, "Quiet", None);
    e.st.open_project(&a).unwrap();
    std::thread::sleep(Duration::from_millis(80));
    assert!(registers(&e).is_empty());
    assert!(!e.st.cloud_view().project.unwrap().backup);
    e.st.cloud_notice_seen().unwrap();
    assert!(e.st.cloud_view().project.unwrap().notice_seen);
}

// ---- Questions ----------------------------------------------------------------------------------

fn other_computers_project(app_id: &str) -> serde_json::Value {
    let mut p = project_json(app_id, "Gala");
    p["head"] = json!({"id": "0199e2a0-dddd-7000-8000-00000000000d", "project_id": PID,
        "created_at": "2026-10-15T14:02:00Z", "machine_name": "STUDIO-PC", "app_version": "0.1.0",
        "is_head": true, "file_count": 3, "logical_bytes": 10, "new_bytes": 10});
    p["versions"] = json!(1);
    p
}

#[test]
fn another_computers_backup_asks_once_and_continue_here_adopts_it() {
    let e = env();
    ready(&e);
    let shows = e.tmp.path().join("shows");
    std::fs::create_dir_all(&shows).unwrap();
    // A folder copied from another computer: it has a cloud id this computer never recorded.
    let a = new_project(&shows, "Gala", Some(CID));
    e.mock.on(
        "POST /v1/cloud/projects",
        vec![ok(200, other_computers_project(CID))],
    );
    e.st.open_project(&a).unwrap();
    wait_until("asked", || {
        e.st.cloud_view()
            .project
            .is_some_and(|p| p.question.is_some())
    });
    let p = e.st.cloud_view().project.unwrap();
    assert_eq!(
        p.question,
        Some(CloudQuestion::OtherComputer {
            machine: Some("STUDIO-PC".into()),
            at: Some("2026-10-15T14:02:00Z".into())
        })
    );
    assert!(!p.registered);
    // The UI gets it tagged.
    let sent = e.rec.of("cloud-changed").pop().unwrap();
    assert_eq!(sent["project"]["question"]["kind"], "otherComputer");
    e.rt.block_on(e.st.cloud_answer(CloudAnswer::ContinueHere))
        .unwrap();
    let p = e.st.cloud_view().project.unwrap();
    assert!(p.registered && p.question.is_none());
    let r = record(&e, CID);
    assert_eq!(r["serverId"], PID);
    // Carries on from that head: the next backup's base is the other computer's version.
    assert_eq!(r["head"], "0199e2a0-dddd-7000-8000-00000000000d");
}

#[test]
fn another_computers_backup_can_become_a_separate_project() {
    let e = env();
    ready(&e);
    let shows = e.tmp.path().join("shows");
    std::fs::create_dir_all(&shows).unwrap();
    let a = new_project(&shows, "Gala", Some(CID));
    e.mock.on(
        "POST /v1/cloud/projects",
        vec![
            ok(200, other_computers_project(CID)),
            ok(201, project_json("new", "Gala")),
        ],
    );
    e.st.open_project(&a).unwrap();
    wait_until("asked", || {
        e.st.cloud_view()
            .project
            .is_some_and(|p| p.question.is_some())
    });
    e.rt.block_on(e.st.cloud_answer(CloudAnswer::Separate))
        .unwrap();
    e.st.flush_all();
    let new_id = cloud_id_on_disk(&a).unwrap();
    assert_ne!(new_id, CID);
    let bodies = registers(&e);
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[1]["app_project_id"], new_id);
    assert!(e.st.cloud_view().project.unwrap().registered);
}

#[test]
fn a_project_in_the_trash_offers_restore_start_new_or_off() {
    let e = env();
    ready(&e);
    let shows = e.tmp.path().join("shows");
    std::fs::create_dir_all(&shows).unwrap();
    let a = new_project(&shows, "Gala", Some(CID));
    let trash = err(
        409,
        "project_in_trash",
        Some(json!({"project_id": PID, "purge_at": "2026-10-22T10:00:00Z"})),
    );
    e.mock.on(
        "POST /v1/cloud/projects",
        vec![trash.clone(), ok(200, project_json(CID, "Gala"))],
    );
    e.st.open_project(&a).unwrap();
    wait_until("asked", || {
        e.st.cloud_view()
            .project
            .is_some_and(|p| p.question.is_some())
    });
    assert_eq!(
        e.st.cloud_view().project.unwrap().question,
        Some(CloudQuestion::InTrash {
            purge_at: Some("2026-10-22T10:00:00Z".into())
        })
    );
    e.rt.block_on(e.st.cloud_answer(CloudAnswer::Restore))
        .unwrap();
    assert_eq!(registers(&e)[1]["if_trashed"], "restore");
    let p = e.st.cloud_view().project.unwrap();
    assert!(p.registered && p.question.is_none());

    // Another project in the trash: Turn off.
    let b = new_project(&shows, "Old", Some("0199e2a0-eeee-7000-8000-00000000000e"));
    e.mock.on("POST /v1/cloud/projects", vec![trash]);
    e.st.open_project(&b).unwrap();
    wait_until("asked", || {
        e.st.cloud_view()
            .project
            .is_some_and(|p| p.question.is_some())
    });
    e.rt.block_on(e.st.cloud_answer(CloudAnswer::TurnOff))
        .unwrap();
    let p = e.st.cloud_view().project.unwrap();
    assert!(!p.backup && p.question.is_none());
    // With no question waiting, an answer does nothing.
    assert!(e
        .rt
        .block_on(e.st.cloud_answer(CloudAnswer::ContinueHere))
        .is_ok()); // no question: nothing to do
}

#[test]
fn the_plan_limit_is_an_issue_not_a_question() {
    let e = env();
    ready(&e);
    e.mock.on(
        "POST /v1/cloud/projects",
        vec![err(409, "too_many_projects", None)],
    );
    let shows = e.tmp.path().join("shows");
    std::fs::create_dir_all(&shows).unwrap();
    let a = new_project(&shows, "One too many", None);
    e.st.open_project(&a).unwrap();
    wait_until("issue", || {
        e.st.cloud_view().project.is_some_and(|p| p.issue.is_some())
    });
    let p = e.st.cloud_view().project.unwrap();
    assert!(p.question.is_none() && !p.registered);
}

#[test]
fn closing_the_project_drops_it_from_the_view() {
    let e = env();
    ready(&e);
    let shows = e.tmp.path().join("shows");
    std::fs::create_dir_all(&shows).unwrap();
    e.st.cloud_projects().set_test_retry(None);
    let a = new_project(&shows, "Gala", None);
    e.mock.on(
        "POST /v1/cloud/projects",
        vec![ok(201, project_json("x", "Gala"))],
    );
    e.st.open_project(&a).unwrap();
    assert!(e.st.cloud_view().project.is_some());
    e.st.close_project();
    assert!(e.st.cloud_view().project.is_none());
}
