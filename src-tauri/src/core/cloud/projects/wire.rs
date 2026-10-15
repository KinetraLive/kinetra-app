//! What the server sends and takes for cloud projects (docs/CLOUD_PROJECTS.md §4–§10). Unknown
//! response fields are ignored (§14: additive changes keep `/v1`); fields the contract may
//! leave out default.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ---- Quota (§4) ---------------------------------------------------------------------------

/// The server sends snake_case (`not_in_plan`); the UI gets camelCase (`notInPlan`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
pub enum QuotaState {
    #[default]
    Ok,
    Warning,
    Full,
    Lapsed,
    NotInPlan,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Count {
    pub used: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct UploadToday {
    pub used_bytes: u64,
    pub limit_bytes: u64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Lapse {
    pub since: Option<String>,
    pub purge_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct RetentionChange {
    pub retention_days: u64,
    pub from: Option<String>,
}

/// `pause` | `limit` (§2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OnAir {
    #[default]
    Pause,
    Limit,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ClientDefaults {
    pub backup_interval_s: u64,
    #[serde(deserialize_with = "on_air_or_pause")]
    pub on_air: OnAir,
    pub on_air_limit_kbps: u32,
    pub multipart_threshold_bytes: u64,
    pub part_size_bytes: u64,
}

impl Default for ClientDefaults {
    /// The contract's defaults (§4), for a quota that leaves them out.
    fn default() -> Self {
        Self {
            backup_interval_s: 300,
            on_air: OnAir::Pause,
            on_air_limit_kbps: 2000,
            multipart_threshold_bytes: 16 * 1024 * 1024,
            part_size_bytes: 16 * 1024 * 1024,
        }
    }
}

/// An unknown `on_air` value means the safe one: pause.
fn on_air_or_pause<'de, D: serde::Deserializer<'de>>(d: D) -> Result<OnAir, D::Error> {
    Ok(match String::deserialize(d)?.as_str() {
        "limit" => OnAir::Limit,
        _ => OnAir::Pause,
    })
}

/// `GET /v1/cloud/quota`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Quota {
    pub plan: Option<String>,
    pub state: QuotaState,
    pub used_bytes: u64,
    pub pending_bytes: u64,
    pub limit_bytes: u64,
    pub percent: u64,
    pub warn_percent: u64,
    pub projects: Count,
    pub max_file_bytes: u64,
    pub retention_days: u64,
    pub kept_per_project: u64,
    pub upload_today: UploadToday,
    pub lapse: Option<Lapse>,
    pub retention_change: Option<RetentionChange>,
    pub client: ClientDefaults,
}

// ---- Projects and versions (§5, §7, §10) ----------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VersionConflict {
    pub with_version_id: String,
    #[serde(default)]
    pub resolved_at: Option<String>,
}

/// A version object (§7).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub created_at: String,
    #[serde(default)]
    pub base_version_id: Option<String>,
    #[serde(default)]
    pub restored_from: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub machine_name: Option<String>,
    #[serde(default)]
    pub app_version: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub kept: bool,
    #[serde(default)]
    pub is_head: bool,
    #[serde(default)]
    pub damaged: bool,
    #[serde(default)]
    pub file_count: u64,
    #[serde(default)]
    pub logical_bytes: u64,
    #[serde(default)]
    pub new_bytes: u64,
    #[serde(default)]
    pub conflict: Option<VersionConflict>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// A project object (§5).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CloudProject {
    pub id: String,
    pub app_project_id: String,
    pub name: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub head: Option<Version>,
    #[serde(default)]
    pub versions: u64,
    #[serde(default)]
    pub kept: u64,
    #[serde(default)]
    pub conflict: bool,
    #[serde(default)]
    pub trashed_at: Option<String>,
    #[serde(default)]
    pub purge_at: Option<String>,
}

/// `if_trashed` of a register (§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IfTrashed {
    #[default]
    Error,
    Restore,
    New,
}

/// `POST /v1/cloud/projects`: the project and whether it was created (`201`) or found (`200`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    pub project: CloudProject,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VersionPage {
    pub versions: Vec<Version>,
    #[serde(default)]
    pub next: Option<String>,
}

/// `PATCH …/versions/{vid}` (any of them).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct VersionPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kept: Option<bool>,
    /// `Some(None)` clears the label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_resolved: Option<bool>,
}

// ---- Manifest (§8) ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub modified_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ManifestView {
    pub version: Version,
    pub manifest: Manifest,
}

/// `reason` of a commit (§7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Auto,
    Close,
    Manual,
}

/// `POST /v1/cloud/projects/{id}/versions` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewVersion {
    pub base_version_id: Option<String>,
    pub restored_from: Option<String>,
    pub reason: Reason,
    pub label: Option<String>,
    pub activation_id: String,
    pub app_version: String,
    pub manifest: Manifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CommitConflict {
    pub with_version: Version,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Committed {
    pub version: Version,
    #[serde(default)]
    pub conflict: Option<CommitConflict>,
}

// ---- Blobs (§6, §10) -------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlobRef {
    pub sha256: String,
    pub size: u64,
}

/// A presigned request: the URL and exactly the headers to send (§3.1). The URL is a secret
/// until it expires: no `Debug` (it would end up in logs); see `r2::redact`.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Presigned {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

impl std::fmt::Debug for Presigned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Presigned")
            .field("url", &super::r2::redact(&self.url))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Upload {
    Put {
        #[serde(flatten)]
        put: Presigned,
    },
    Multipart {
        upload_id: String,
        part_size: u64,
        part_count: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MissingBlob {
    pub sha256: String,
    pub size: u64,
    pub upload: Upload,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Checked {
    pub missing: Vec<MissingBlob>,
    #[serde(default)]
    pub reserved_until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartReq {
    pub number: u32,
    /// MD5 of the part, 32 lowercase hex.
    pub md5: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PartUrl {
    pub number: u32,
    #[serde(flatten)]
    pub put: Presigned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartDone {
    pub number: u32,
    pub etag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UploadStatus {
    pub sha256: String,
    pub size: u64,
    pub part_size: u64,
    pub part_count: u32,
    #[serde(default)]
    pub parts_done: Vec<PartDone>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DownloadUrl {
    pub sha256: String,
    #[serde(flatten)]
    pub get: Presigned,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Downloads {
    pub urls: Vec<DownloadUrl>,
    #[serde(default)]
    pub missing: Vec<String>,
}
