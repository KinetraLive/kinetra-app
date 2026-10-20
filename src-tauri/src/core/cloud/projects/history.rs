//! The history and the account's cloud projects (CLOUD_PROJECTS.md §5, §10): the open project's
//! versions (newest first, a page at a time; keep, label, delete) and every project the account
//! backs up (rename, move to the trash, restore from it). Reading and deleting work while the plan
//! has lapsed; the server refuses the rest then (`cloud_read_only`) and says so. Nothing here
//! touches local files: downloading and restoring come with restore.

use super::wire::{CloudProject, Version, VersionPatch};
use crate::core::cloud::api::CallError;
use crate::core::model::{
    CloudAccess, CloudProjectSummary, CloudQuestion, CloudVersionPage, CloudVersionView,
};
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

/// A page of the history at most.
pub const VERSIONS_PER_PAGE: u32 = 50;
/// A version label: ≤ 100 characters (CLOUD_PROJECTS.md §7).
pub const LABEL_MAX: usize = 100;

fn version_view(v: Version) -> CloudVersionView {
    CloudVersionView {
        conflict: v.conflict.as_ref().is_some_and(|c| c.resolved_at.is_none()),
        id: v.id,
        created_at: v.created_at,
        machine: v.machine_name,
        label: v.label,
        kept: v.kept,
        is_head: v.is_head,
        damaged: v.damaged,
        file_count: v.file_count,
        logical_bytes: v.logical_bytes,
        new_bytes: v.new_bytes,
        reason: v.reason,
        expires_at: v.expires_at,
    }
}

/// A label as stored: trimmed, no control characters, ≤ 100 characters; empty = none.
pub fn clean_label(label: Option<&str>) -> AppResult<Option<String>> {
    let Some(l) = label else { return Ok(None) };
    let l: String = l.chars().filter(|c| !c.is_control()).collect();
    let l = l.trim();
    if l.is_empty() {
        return Ok(None);
    }
    if l.chars().count() > LABEL_MAX {
        return Err(AppError::Invalid(format!(
            "A label can be at most {LABEL_MAX} characters."
        )));
    }
    Ok(Some(l.to_string()))
}

/// The account's plan limit, in words.
fn limit_message(e: &CallError) -> Option<String> {
    let CallError::Api { code, details, .. } = e else {
        return None;
    };
    match code.as_str() {
        "too_many_kept" => {
            let n = details
                .as_ref()
                .and_then(|d| d.get("limit"))
                .and_then(|v| v.as_u64());
            Some(match n {
                Some(1) => "Your plan keeps up to 1 version per project.".into(),
                Some(n) => format!("Your plan keeps up to {n} versions per project."),
                None => "You're keeping as many versions as your plan allows.".into(),
            })
        }
        "cannot_delete_head" => Some(
            "The newest version can't be deleted. Move the project's backup to the trash instead."
                .into(),
        ),
        _ => None,
    }
}

fn friendly(e: CallError) -> AppError {
    match limit_message(&e) {
        Some(message) => AppError::Invalid(message),
        None => e.into(),
    }
}

impl AppState {
    /// The open project's server id; an error when it isn't backed up yet.
    fn cloud_open_server_id(&self) -> AppResult<String> {
        self.cloud_record()
            .and_then(|r| r.server_id)
            .ok_or_else(|| AppError::Invalid("This project has no cloud backup yet.".into()))
    }

    pub(super) fn cloud_history_access(&self) -> AppResult<()> {
        match self.cloud_access() {
            CloudAccess::Available => Ok(()),
            CloudAccess::SignedOut => Err(AppError::Cloud {
                api_code: "unauthorized".into(),
                message: "Sign in first.".into(),
                details: None,
            }),
            CloudAccess::Free => Err(AppError::ProRequired),
        }
    }

    /// A page of the open project's history, newest first (`before`: the `next` of the last
    /// page).
    pub async fn cloud_versions(&self, before: Option<String>) -> AppResult<CloudVersionPage> {
        self.cloud_history_access()?;
        let pid = self.cloud_open_server_id()?;
        let (api, session, _) = self.cloud_session_parts()?;
        let page = api
            .cloud_versions(&session, &pid, before.as_deref(), VERSIONS_PER_PAGE)
            .await?;
        Ok(CloudVersionPage {
            versions: page.versions.into_iter().map(version_view).collect(),
            next: page.next,
        })
    }

    async fn cloud_patch(
        &self,
        version_id: &str,
        patch: VersionPatch,
    ) -> AppResult<CloudVersionView> {
        self.cloud_history_access()?;
        let pid = self.cloud_open_server_id()?;
        let (api, session, _) = self.cloud_session_parts()?;
        let v = api
            .cloud_patch_version(&session, &pid, version_id, &patch)
            .await
            .map_err(friendly)?;
        Ok(version_view(v))
    }

    /// Keep a version (retention never deletes it) or let it go.
    pub async fn cloud_version_keep(
        &self,
        version_id: &str,
        kept: bool,
    ) -> AppResult<CloudVersionView> {
        self.cloud_patch(
            version_id,
            VersionPatch {
                kept: Some(kept),
                ..Default::default()
            },
        )
        .await
    }

    /// A version's label (`None` or blank clears it).
    pub async fn cloud_version_label(
        &self,
        version_id: &str,
        label: Option<String>,
    ) -> AppResult<CloudVersionView> {
        let label = clean_label(label.as_deref())?;
        self.cloud_patch(
            version_id,
            VersionPatch {
                label: Some(label),
                ..Default::default()
            },
        )
        .await
    }

    /// Deletes a version (not the newest).
    pub async fn cloud_version_delete(&self, version_id: &str) -> AppResult<()> {
        self.cloud_history_access()?;
        let pid = self.cloud_open_server_id()?;
        let (api, session, _) = self.cloud_session_parts()?;
        api.cloud_delete_version(&session, &pid, version_id)
            .await
            .map_err(friendly)
    }

    // ---- The account's projects ----

    fn summary(&self, p: CloudProject) -> CloudProjectSummary {
        let file = self.cloud_file();
        let here = file
            .folders
            .get(&p.app_project_id)
            .is_some_and(|f| std::path::Path::new(f).is_dir());
        let open = self
            .project_cloud_id()
            .and_then(|(_, c)| c)
            .is_some_and(|c| c == p.app_project_id);
        CloudProjectSummary {
            last_at: p.head.as_ref().map(|h| h.created_at.clone()),
            last_machine: p.head.as_ref().and_then(|h| h.machine_name.clone()),
            id: p.id,
            name: p.name,
            versions: p.versions,
            kept: p.kept,
            conflict: p.conflict,
            trashed_at: p.trashed_at,
            purge_at: p.purge_at,
            open,
            here,
        }
    }

    /// The account's cloud projects, most recently updated first; with the trash when asked.
    pub async fn cloud_projects_list(&self, trashed: bool) -> AppResult<Vec<CloudProjectSummary>> {
        self.cloud_history_access()?;
        let (api, session, _) = self.cloud_session_parts()?;
        let list = api.cloud_projects(&session, trashed).await?;
        Ok(list.into_iter().map(|p| self.summary(p)).collect())
    }

    /// Renames a cloud project (the local project keeps its own name).
    pub async fn cloud_project_rename(
        &self,
        id: &str,
        name: &str,
    ) -> AppResult<CloudProjectSummary> {
        self.cloud_history_access()?;
        let name = super::identity::server_name(name);
        let (api, session, _) = self.cloud_session_parts()?;
        let p = api.cloud_rename(&session, id, &name).await?;
        Ok(self.summary(p))
    }

    /// Moves a cloud project to the trash (deleted for good after 7 days; it still counts
    /// toward storage until then). The open project's backup then asks what to do.
    pub async fn cloud_project_trash(&self, id: &str) -> AppResult<CloudProjectSummary> {
        self.cloud_history_access()?;
        let (api, session, _) = self.cloud_session_parts()?;
        let p = api.cloud_trash(&session, id).await?;
        let summary = self.summary(p);
        if summary.open {
            if let Some((dir, _)) = self.project_cloud_id() {
                self.cloud_set_question_pub(
                    &dir,
                    CloudQuestion::InTrash {
                        purge_at: summary.purge_at.clone(),
                    },
                );
            }
        }
        Ok(summary)
    }

    /// Restores a cloud project from the trash.
    pub async fn cloud_project_untrash(&self, id: &str) -> AppResult<CloudProjectSummary> {
        self.cloud_history_access()?;
        let (api, session, _) = self.cloud_session_parts()?;
        let p = api.cloud_untrash(&session, id).await?;
        let summary = self.summary(p);
        if summary.open {
            if let Some((dir, _)) = self.project_cloud_id() {
                self.cloud_clear_question(&dir);
            }
        }
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_clean_and_short() {
        assert_eq!(clean_label(None).unwrap(), None);
        assert_eq!(clean_label(Some("  ")).unwrap(), None);
        assert_eq!(
            clean_label(Some(" Before\u{7} the show "))
                .unwrap()
                .as_deref(),
            Some("Before the show")
        );
        assert!(clean_label(Some(&"x".repeat(101))).is_err());
        assert!(clean_label(Some(&"x".repeat(100))).is_ok());
    }
}
