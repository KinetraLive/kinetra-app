//! What this computer remembers about cloud backup: `cloud.json` in the app config dir. Never
//! in the project folder, which is backed up and copied: a copy must not carry the server's
//! ids (CLOUD_PROJECTS.md §2.4). Not secret (ids and folder paths only).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::persist::Store;

/// `cloud.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CloudFile {
    /// Account id → what this computer knows of it.
    pub accounts: BTreeMap<String, AccountRecord>,
    /// cloudId → the folder that holds that project on this computer (absolute path). Shared
    /// by every account: it's about folders, not backups.
    pub folders: BTreeMap<String, String>,
}

impl Store for CloudFile {
    const FILE: &'static str = "cloud.json";
    const VERSION: u32 = 1;
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountRecord {
    /// The server answered with cloud backup for this account here (any state but
    /// `not_in_plan`): it may be asked again without `pro.cloud_sync`, so a lapsed plan can
    /// still list and download its backups.
    pub known: bool,
    /// cloudId → this project's backup on this computer.
    pub projects: BTreeMap<String, ProjectRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectRecord {
    /// The server's project id (after registering).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    /// The head this computer last knew (`base_version_id` of the next commit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    /// Backup on / off for this project; `None` = `CloudPrefs.newProjects`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<bool>,
    /// The one-time "Backing up to the cloud" notice was shown.
    pub notice_seen: bool,
    /// RFC 3339 of the last backup that finished here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_at: Option<String>,
    /// Changes since the last backup that aren't in the cloud yet (kept across restarts: the next
    /// open backs them up).
    pub pending: bool,
    /// The user OK'd a first backup over 1 GB (asked once, CLOUD_PROJECTS_APP_PLAN decision 5).
    pub big_ok: bool,
    /// The `Idempotency-Key` of a commit not confirmed yet (reused until it is).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_commit: Option<String>,
    /// Multipart uploads in progress, by blob SHA-256, so a restart resumes them.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub uploads: BTreeMap<String, MultipartRecord>,
}

/// A multipart upload in progress (CLOUD_PROJECTS.md §6): the parts storage already has.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MultipartRecord {
    pub upload_id: String,
    pub part_size: u64,
    pub part_count: u32,
    pub parts: Vec<super::wire::PartDone>,
}

/// How an opened folder relates to the folder recorded for its cloudId.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Identity {
    /// This is the recorded folder (or none was recorded: this one becomes it).
    Same,
    /// The recorded folder is gone or holds another project: this folder was moved there. The
    /// record follows it.
    Moved,
    /// The recorded folder still holds a project with the same cloudId: this folder is a copy
    /// and needs its own cloudId.
    Copy,
}

/// Decides [`Identity`] for `here`, given the recorded folder and what that folder's
/// `project.json` says its cloudId is (`None`: missing or unreadable).
pub fn identity(
    recorded: Option<&Path>,
    here: &Path,
    cloud_id: &str,
    id_at: impl Fn(&Path) -> Option<String>,
) -> Identity {
    match recorded {
        None => Identity::Same,
        Some(r) if same_folder(r, here) => Identity::Same,
        Some(r) => match id_at(r) {
            Some(id) if id == cloud_id => Identity::Copy,
            _ => Identity::Moved,
        },
    }
}

/// Whether two absolute folder paths are the same folder (Windows: ignoring case and the
/// separator style).
pub fn same_folder(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        let norm = |p: &Path| {
            p.to_string_lossy()
                .replace('/', "\\")
                .trim_end_matches('\\')
                .to_lowercase()
        };
        norm(a) == norm(b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn copies_moves_and_the_same_folder() {
        let here = PathBuf::from("/shows/b");
        let at = |id: Option<&'static str>| move |_: &Path| id.map(str::to_string);
        assert_eq!(identity(None, &here, "x", at(None)), Identity::Same);
        assert_eq!(
            identity(Some(Path::new("/shows/b")), &here, "x", at(Some("x"))),
            Identity::Same
        );
        // The recorded folder still has the same id: this one is the copy.
        assert_eq!(
            identity(Some(Path::new("/shows/a")), &here, "x", at(Some("x"))),
            Identity::Copy
        );
        // Gone, or another project now: this folder moved.
        assert_eq!(
            identity(Some(Path::new("/shows/a")), &here, "x", at(None)),
            Identity::Moved
        );
        assert_eq!(
            identity(Some(Path::new("/shows/a")), &here, "x", at(Some("y"))),
            Identity::Moved
        );
    }

    #[test]
    #[cfg(windows)]
    fn windows_folders_compare_ignoring_case_and_separators() {
        assert!(same_folder(
            Path::new(r"C:\Shows\Election"),
            Path::new("c:/shows/election/")
        ));
        assert!(!same_folder(
            Path::new(r"C:\Shows\Election"),
            Path::new(r"C:\Shows\Election 2")
        ));
    }
}
