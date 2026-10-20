//! Which cloud project a folder is (CLOUD_PROJECTS.md §2.4, §5; the app plan's decision 1).
//!
//! A project's cloud ID is `project.json` `cloudId` (a UUID v7, `app_project_id`), separate
//! from remote control's `id` (which a copied folder keeps on purpose). It's written the first
//! time the project backs up, and `cloud.json` records which folder holds it on this computer.
//!
//! - **Copies get their own ID.** Opening a folder whose cloudId is recorded for another folder
//!   that still holds it means this one is a copy: it gets a new cloudId at once (nothing is
//!   asked). A folder whose recorded one is gone was moved: the record follows it. Kinetra's
//!   own Move keeps the ID at the destination.
//! - **Another computer's backup.** Registering a project the server already has, with
//!   versions, that this computer has no record of (a folder copied from another computer)
//!   asks once, quietly: continue that backup here, back up as a separate project, or not here.
//! - **In the trash** (`409 project_in_trash`): restore it, start a new cloud backup, or turn
//!   backup off for this project.
//!
//! Registration (`POST /v1/cloud/projects`) is metadata only and runs in the background when
//! the project opens, backup is switched on, or the quota first allows it. Nothing here
//! touches anything on air or opens a modal; questions wait in the view.

use std::path::{Path, PathBuf};
use std::sync::{MutexGuard, PoisonError};

use super::local::{identity, Identity, ProjectRecord};
use super::service::PRO_CLOUD;
use super::wire::{CloudProject, IfTrashed, QuotaState, VersionPatch};
use crate::core::cloud::api::CallError;
use crate::core::model::{
    CloudAccess, CloudAnswer, CloudConflictView, CloudProjectView, CloudQuestion, ProjectFile,
};
use crate::core::persist;
use crate::core::remote::ids::uuid_v7;
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

/// The open project as cloud backup sees it (in the service's state).
#[derive(Debug, Default)]
pub(super) struct OpenProject {
    pub dir: PathBuf,
    pub question: Option<CloudQuestion>,
    /// The server's project behind an `OtherComputer` question (for Continue here).
    pub pending: Option<CloudProject>,
    pub issue: Option<String>,
    /// A registration is running.
    pub registering: bool,
    /// The backup running now, and how the last one ended (see `backup`).
    pub backup: super::backup::BackupState,
    /// Its cloud id (when it has one), for the change hook (which can't read `project.json`).
    pub cloud_id: Option<String>,
    /// When the next automatic backup runs (see `schedule`).
    pub sched: super::schedule::Sched,
}

/// A project name as the server takes it: no control characters, 1–200 characters.
pub fn server_name(name: &str) -> String {
    let clean: String = name.chars().filter(|c| !c.is_control()).take(200).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "Project".into()
    } else {
        clean.to_string()
    }
}

/// The cloudId in `dir`'s `project.json`, if readable.
pub(super) fn cloud_id_at(dir: &Path) -> Option<String> {
    persist::load_strict::<ProjectFile>(dir).ok()?.cloud_id
}

fn lock<T>(m: &std::sync::Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl AppState {
    fn cloud_open(&self) -> MutexGuard<'_, super::service::Svc> {
        lock(&self.cloud_projects().inner)
    }

    // ---- Open, close, move ----

    /// A project opened (or reloaded): settle its identity (copies get their own cloudId,
    /// moved folders update the record), then register in the background when due. Touches
    /// the disk: call from a blocking context.
    pub(crate) fn cloud_project_opened(&self) {
        // A build without cloud backup leaves project.json's cloudId and cloud.json alone.
        if !self.cloud_available() {
            return;
        }
        let Some((dir, cid)) = self.project_cloud_id() else {
            return;
        };
        // Another project was open (switching without a close): it closes now.
        let other = self
            .cloud_open()
            .project
            .as_ref()
            .is_some_and(|p| p.dir != dir);
        if other {
            self.cloud_schedule_closing();
        }
        // A reload of the same project keeps its running backup and schedule.
        let same = self
            .cloud_open()
            .project
            .as_ref()
            .is_some_and(|p| p.dir == dir);
        if !same {
            self.cloud_open().project = Some(OpenProject {
                dir: dir.clone(),
                ..Default::default()
            });
        }
        if let Some(cid) = cid {
            let recorded = self.cloud_file().folders.get(&cid).map(PathBuf::from);
            match identity(recorded.as_deref(), &dir, &cid, cloud_id_at) {
                Identity::Same | Identity::Moved => self.cloud_record_folder(&cid, &dir),
                Identity::Copy => {
                    tracing::info!("a copy of a backed-up project: it gets its own cloud backup");
                    if let Err(e) = self.cloud_new_id(&dir) {
                        tracing::warn!("cloud id not written: {e}");
                    }
                }
            }
        }
        let cid = self.project_cloud_id().and_then(|(_, c)| c);
        if let Some(p) = self.cloud_open().project.as_mut() {
            p.cloud_id = cid;
        }
        self.emit_cloud();
        self.cloud_maybe_register();
        self.cloud_load_conflict();
        if !same {
            self.cloud_schedule_opened();
        }
    }

    /// The project closed: its backup stops (uploaded blobs stay); with changes not in the cloud
    /// a background backup of the folder starts (`schedule`).
    pub(crate) fn cloud_project_closed(&self) {
        self.cloud_schedule_closing();
        self.cloud_open().project = None;
        self.emit_cloud();
    }

    /// The open project moved to another folder (Kinetra's Move): the cloudId stays with it.
    pub(crate) fn cloud_project_relocated(&self) {
        let Some((dir, cid)) = self.project_cloud_id() else {
            return;
        };
        if let Some(p) = self.cloud_open().project.as_mut() {
            p.dir = dir.clone();
        }
        if let Some(cid) = cid {
            self.cloud_record_folder(&cid, &dir);
        }
    }

    pub(super) fn cloud_record_folder(&self, cid: &str, dir: &Path) {
        let path = dir.to_string_lossy().into_owned();
        self.cloud_file_update(|f| {
            f.folders.insert(cid.to_string(), path);
        });
    }

    /// Gives the open project at `dir` a new cloudId (a copy, or "separate project").
    fn cloud_new_id(&self, dir: &Path) -> AppResult<String> {
        let id = uuid_v7();
        self.set_project_cloud_id(dir, id.clone())?;
        self.cloud_record_folder(&id, dir);
        if let Some(p) = self.cloud_open().project.as_mut().filter(|p| p.dir == dir) {
            p.cloud_id = Some(id.clone());
        }
        Ok(id)
    }

    /// The open project's cloudId, made (and written) when it has none.
    pub(super) fn cloud_ensure_id(&self) -> AppResult<(PathBuf, String)> {
        let (dir, _) = self.project_cloud_id().ok_or(AppError::NoProject)?;
        let (id, made) = self.ensure_project_cloud_id(&dir, uuid_v7())?;
        if made {
            self.cloud_record_folder(&id, &dir);
            if let Some(p) = self.cloud_open().project.as_mut().filter(|p| p.dir == dir) {
                p.cloud_id = Some(id.clone());
            }
        }
        Ok((dir, id))
    }

    // ---- Records ----

    /// This account's record of the open project, if any.
    pub(super) fn cloud_record(&self) -> Option<ProjectRecord> {
        let account = self.cloud_account()?;
        let (_, cid) = self.project_cloud_id()?;
        self.cloud_file()
            .accounts
            .get(&account)?
            .projects
            .get(&cid?)
            .cloned()
    }

    /// This account's record of project `cloud_id` (open or not).
    pub(super) fn cloud_record_of(&self, cloud_id: &str) -> Option<ProjectRecord> {
        let account = self.cloud_account()?;
        self.cloud_file()
            .accounts
            .get(&account)?
            .projects
            .get(cloud_id)
            .cloned()
    }

    /// Changes this account's record of project `cloud_id` (no-op signed out).
    pub(super) fn cloud_record_update_of(
        &self,
        cloud_id: &str,
        f: impl FnOnce(&mut ProjectRecord),
    ) {
        let Some(account) = self.cloud_account() else {
            return;
        };
        self.cloud_file_update(|file| {
            f(file
                .accounts
                .entry(account)
                .or_default()
                .projects
                .entry(cloud_id.to_string())
                .or_default());
        });
    }

    /// Changes this account's record of the open project (making the cloudId if needed).
    fn cloud_record_update(&self, f: impl FnOnce(&mut ProjectRecord)) -> AppResult<()> {
        let account = self.cloud_account().ok_or_else(|| AppError::Cloud {
            api_code: "unauthorized".into(),
            message: "Sign in first.".into(),
            details: None,
        })?;
        let (_, cid) = self.cloud_ensure_id()?;
        self.cloud_file_update(|file| {
            f(file
                .accounts
                .entry(account)
                .or_default()
                .projects
                .entry(cid)
                .or_default());
        });
        Ok(())
    }

    /// Backup is on for the open project: its own switch, else "Back up new projects".
    pub(super) fn cloud_backup_on(&self) -> bool {
        self.cloud_record()
            .and_then(|r| r.backup)
            .unwrap_or(self.prefs().cloud.new_projects)
    }

    /// The open project's part of the view.
    pub(super) fn cloud_project_view(&self, access: CloudAccess) -> Option<CloudProjectView> {
        if matches!(access, CloudAccess::SignedOut | CloudAccess::Unavailable) {
            return None;
        }
        let (question, issue, b) = {
            let s = self.cloud_open();
            let p = s.project.as_ref()?;
            (p.question.clone(), p.issue.clone(), p.backup.clone())
        };
        let record = self.cloud_record();
        Some(CloudProjectView {
            backup: self.cloud_backup_on(),
            registered: record.as_ref().is_some_and(|r| r.server_id.is_some()),
            server_id: record.as_ref().and_then(|r| r.server_id.clone()),
            notice_seen: record.as_ref().is_some_and(|r| r.notice_seen),
            question,
            issue,
            run: b.run,
            last_backup_at: record.as_ref().and_then(|r| r.last_at.clone()),
            report: b.report,
            skipped: b.skipped,
            error: b.error,
            conflict: b.conflict,
        })
    }

    // ---- Registration ----

    /// Whether the open project should register now (see the module docs).
    fn cloud_should_register(&self) -> bool {
        if self.cloud_access() != CloudAccess::Available
            || self.require_new_pro_work(PRO_CLOUD).is_err()
            || !self.cloud_writable()
        {
            return false;
        }
        {
            let s = self.cloud_open();
            match s.project.as_ref() {
                Some(p) if p.question.is_none() && !p.registering => {}
                _ => return false,
            }
        }
        self.cloud_backup_on() && !self.cloud_record().is_some_and(|r| r.server_id.is_some())
    }

    /// The last quota allows new backups (ok, warning, full: a full account still commits
    /// versions that add no bytes).
    pub(super) fn cloud_writable(&self) -> bool {
        matches!(
            self.cloud_quota_state(),
            Some(QuotaState::Ok | QuotaState::Warning | QuotaState::Full)
        )
    }

    /// Registers the open project in the background when due.
    pub(crate) fn cloud_maybe_register(&self) {
        if !self.cloud_should_register() {
            return;
        }
        let Some(rt) = self.live().runtime_handle() else {
            return;
        };
        let weak = self.downgrade();
        rt.spawn(async move {
            if let Some(st) = weak.upgrade() {
                if let Err(e) = st.cloud_register(IfTrashed::Error).await {
                    tracing::debug!("cloud register: {e}");
                }
            }
        });
    }

    /// `POST /v1/cloud/projects` for the open project; the answer becomes the record, a
    /// question, or an issue.
    pub(crate) async fn cloud_register(&self, if_trashed: IfTrashed) -> AppResult<()> {
        let (dir, cid) = self.cloud_ensure_id()?;
        {
            let mut s = self.cloud_open();
            let Some(p) = s.project.as_mut().filter(|p| p.dir == dir) else {
                return Err(AppError::NoProject);
            };
            if p.registering {
                return Ok(());
            }
            p.registering = true;
        }
        let result = self.cloud_register_inner(&dir, &cid, if_trashed).await;
        if let Some(p) = self.cloud_open().project.as_mut().filter(|p| p.dir == dir) {
            p.registering = false;
        }
        self.emit_cloud();
        result
    }

    async fn cloud_register_inner(
        &self,
        dir: &Path,
        cid: &str,
        if_trashed: IfTrashed,
    ) -> AppResult<()> {
        let (api, session, _) = self.cloud_session_parts()?;
        let name = server_name(&self.current_project().ok_or(AppError::NoProject)?.name);
        let known_here = self.cloud_record().is_some_and(|r| r.server_id.is_some());
        let answer = api.cloud_register(&session, cid, &name, if_trashed).await;
        let still_open = || {
            self.project_cloud_id()
                .is_some_and(|(d, c)| d == dir && c.as_deref() == Some(cid))
        };
        if !still_open() {
            return Ok(());
        }
        match answer {
            Ok(r) => {
                let other_head = (!r.created && !known_here && if_trashed == IfTrashed::Error)
                    .then(|| r.project.head.clone())
                    .flatten();
                if let Some(head) = other_head {
                    self.cloud_set_question(
                        dir,
                        Some(CloudQuestion::OtherComputer {
                            machine: head.machine_name,
                            at: Some(head.created_at),
                        }),
                        Some(r.project),
                    );
                } else {
                    let (id, head) = (r.project.id.clone(), r.project.head.map(|h| h.id));
                    self.cloud_record_update(|rec| {
                        rec.server_id = Some(id);
                        if r.created {
                            rec.head = None;
                        } else if rec.head.is_none() {
                            rec.head = head;
                        }
                    })?;
                    self.cloud_set_question(dir, None, None);
                }
                Ok(())
            }
            Err(CallError::Api { code, details, .. }) if code == "project_in_trash" => {
                let purge_at = details
                    .as_ref()
                    .and_then(|d| d.get("purge_at"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                self.cloud_set_question(dir, Some(CloudQuestion::InTrash { purge_at }), None);
                Ok(())
            }
            Err(CallError::Api { code, .. }) if code == "too_many_projects" => {
                self.cloud_set_issue(
                    dir,
                    "You have as many projects in the cloud as your plan allows.",
                );
                Ok(())
            }
            Err(e @ CallError::Api { .. })
                if matches!(e.code(), Some("cloud_read_only" | "cloud_not_in_plan")) =>
            {
                // The plan changed: the quota says how.
                let _ = self.cloud_refresh(false).await;
                Err(e.into())
            }
            Err(e) => Err(e.into()),
        }
    }

    fn cloud_set_question(
        &self,
        dir: &Path,
        question: Option<CloudQuestion>,
        pending: Option<CloudProject>,
    ) {
        if let Some(p) = self.cloud_open().project.as_mut().filter(|p| p.dir == dir) {
            p.question = question;
            p.pending = pending;
            p.issue = None;
        }
    }

    /// Shows `question` for the open project at `dir`.
    pub(super) fn cloud_set_question_pub(&self, dir: &Path, question: CloudQuestion) {
        self.cloud_set_question(dir, Some(question), None);
        self.emit_cloud();
    }

    /// Clears the open project's question.
    pub(super) fn cloud_clear_question(&self, dir: &Path) {
        self.cloud_set_question(dir, None, None);
        self.emit_cloud();
    }

    fn cloud_set_issue(&self, dir: &Path, issue: &str) {
        if let Some(p) = self.cloud_open().project.as_mut().filter(|p| p.dir == dir) {
            p.issue = Some(issue.to_string());
        }
    }

    /// The next commit's base (the head this computer knows) and `restored_from` (a version
    /// restored over the project here since the last backup, CLOUD_PROJECTS.md §7, §10).
    pub(super) fn cloud_commit_base(&self, cloud_id: &str) -> (Option<String>, Option<String>) {
        let r = self.cloud_record_of(cloud_id).unwrap_or_default();
        (r.head, r.restored_from)
    }

    /// The key of a commit not confirmed yet.
    pub(super) fn cloud_pending_commit(&self, cloud_id: &str) -> Option<String> {
        self.cloud_record_of(cloud_id)
            .and_then(|r| r.pending_commit)
    }

    pub(super) fn cloud_set_pending_commit(&self, cloud_id: &str, key: Option<String>) {
        self.cloud_record_update_of(cloud_id, |r| r.pending_commit = key);
    }

    /// Reads an unresolved conflict of the open project from the server (once per open; quiet when
    /// it can't), so the notice survives a restart.
    pub(super) fn cloud_load_conflict(&self) {
        let Some(server_id) = self.cloud_record().and_then(|r| r.server_id) else {
            return;
        };
        if self.cloud_access() != CloudAccess::Available {
            return;
        }
        let Some(rt) = self.live().runtime_handle() else {
            return;
        };
        let Some((dir, _)) = self.project_cloud_id() else {
            return;
        };
        let weak = self.downgrade();
        rt.spawn(async move {
            let Some(st) = weak.upgrade() else { return };
            let Ok((api, session, _)) = st.cloud_session_parts() else {
                return;
            };
            let Ok(p) = api.cloud_project(&session, &server_id).await else {
                return;
            };
            if !p.conflict {
                return;
            }
            let Ok(page) = api.cloud_versions(&session, &server_id, None, 50).await else {
                return;
            };
            let open = page
                .versions
                .iter()
                .find(|v| v.conflict.as_ref().is_some_and(|c| c.resolved_at.is_none()));
            let Some(v) = open else { return };
            let other_id = v.conflict.as_ref().map(|c| c.with_version_id.clone());
            let other = page
                .versions
                .iter()
                .find(|x| Some(&x.id) == other_id.as_ref());
            let view = CloudConflictView {
                machine: other.and_then(|o| o.machine_name.clone()),
                at: other.map(|o| o.created_at.clone()),
                version_id: v.id.clone(),
                other_version_id: other_id,
            };
            if let Some(p) = lock(&st.cloud_projects().inner)
                .project
                .as_mut()
                .filter(|p| p.dir == dir)
            {
                p.backup.conflict.get_or_insert(view);
            }
            st.emit_cloud();
        });
    }

    /// "Dismiss" on the conflict notice: both versions stay; the conflict is resolved (§9).
    pub async fn cloud_conflict_dismiss(&self) -> AppResult<()> {
        self.cloud_build_check()?;
        let (dir, version_id) = {
            let s = self.cloud_open();
            let p = s.project.as_ref().ok_or(AppError::NoProject)?;
            let Some(c) = p.backup.conflict.as_ref() else {
                return Ok(());
            };
            (p.dir.clone(), c.version_id.clone())
        };
        let server_id = self
            .cloud_record()
            .and_then(|r| r.server_id)
            .ok_or(AppError::NoProject)?;
        let (api, session, _) = self.cloud_session_parts()?;
        let patch = VersionPatch {
            conflict_resolved: Some(true),
            ..Default::default()
        };
        match api
            .cloud_patch_version(&session, &server_id, &version_id, &patch)
            .await
        {
            Ok(_) => {}
            // Gone (deleted, or resolved by a later backup): nothing left to dismiss.
            Err(CallError::Api { status: 404, .. }) => {}
            Err(e) => return Err(e.into()),
        }
        if let Some(p) = self.cloud_open().project.as_mut().filter(|p| p.dir == dir) {
            p.backup.conflict = None;
        }
        self.emit_cloud();
        Ok(())
    }

    /// A commit went through: its version is the head this computer knows.
    pub(super) fn cloud_record_committed(&self, cloud_id: &str, head: &str, at: &str) {
        let Some(account) = self.cloud_account() else {
            return;
        };
        self.cloud_file_update(|f| {
            let r = f
                .accounts
                .entry(account)
                .or_default()
                .projects
                .entry(cloud_id.to_string())
                .or_default();
            r.head = Some(head.to_string());
            r.last_at = Some(at.to_string());
            r.restored_from = None;
        });
    }

    // ---- Commands ----

    /// Backup on / off for the open project (its own switch; on registers when due).
    pub fn cloud_set_backup(&self, on: bool) -> AppResult<()> {
        self.cloud_build_check()?;
        self.cloud_record_update(|r| r.backup = Some(on))?;
        if !on {
            if let Some(p) = self.cloud_open().project.as_mut() {
                p.question = None;
                p.pending = None;
                p.issue = None;
            }
        }
        self.emit_cloud();
        self.cloud_maybe_register();
        Ok(())
    }

    /// The one-time notice was shown.
    pub fn cloud_notice_seen(&self) -> AppResult<()> {
        self.cloud_build_check()?;
        self.cloud_record_update(|r| r.notice_seen = true)?;
        self.emit_cloud();
        Ok(())
    }

    /// The user's answer to the open project's question.
    pub async fn cloud_answer(&self, answer: CloudAnswer) -> AppResult<()> {
        self.cloud_build_check()?;
        let (dir, question, pending) = {
            let s = self.cloud_open();
            let p = s.project.as_ref().ok_or(AppError::NoProject)?;
            (p.dir.clone(), p.question.clone(), p.pending.clone())
        };
        let Some(question) = question else {
            return Ok(());
        };
        match (question, answer) {
            (_, CloudAnswer::TurnOff) => self.cloud_set_backup(false),
            (CloudQuestion::InTrash { .. }, CloudAnswer::Restore) => {
                self.cloud_set_question(&dir, None, None);
                self.cloud_register(IfTrashed::Restore).await
            }
            (CloudQuestion::InTrash { .. }, CloudAnswer::StartNew) => {
                self.cloud_set_question(&dir, None, None);
                self.cloud_register(IfTrashed::New).await
            }
            (CloudQuestion::OtherComputer { .. }, CloudAnswer::ContinueHere) => {
                let project = pending.ok_or(AppError::NoProject)?;
                let head = project.head.map(|h| h.id);
                self.cloud_record_update(|r| {
                    r.server_id = Some(project.id);
                    r.head = head;
                })?;
                self.cloud_set_question(&dir, None, None);
                self.emit_cloud();
                Ok(())
            }
            (CloudQuestion::FirstBackup { .. }, CloudAnswer::BackUp) => {
                self.cloud_record_update(|r| r.big_ok = true)?;
                self.cloud_set_question(&dir, None, None);
                self.emit_cloud();
                self.cloud_backup_start(super::wire::Reason::Auto)
            }
            (CloudQuestion::OtherComputer { .. }, CloudAnswer::Separate) => {
                self.cloud_set_question(&dir, None, None);
                self.cloud_new_id(&dir)?;
                self.cloud_register(IfTrashed::Error).await
            }
            _ => Err(AppError::Invalid(
                "That answer doesn't fit the question".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn server_names_are_clean_and_short() {
        assert_eq!(
            super::server_name("  Election\u{7} night "),
            "Election night"
        );
        assert_eq!(super::server_name("\n"), "Project");
        assert_eq!(super::server_name(&"x".repeat(300)).chars().count(), 200);
    }
}
