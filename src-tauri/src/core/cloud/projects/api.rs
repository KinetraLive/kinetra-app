//! `/v1/cloud/*` calls (docs/CLOUD_PROJECTS.md §4–§10) on the session client. Ids from the
//! server go into paths only when they look like UUIDs ([`is_id`]); bodies are never logged
//! (manifests hold file paths and hashes).

use std::time::Duration;

use serde_json::{json, Value};

use super::wire::{
    BlobRef, Checked, CloudProject, Committed, Downloads, IfTrashed, ManifestView, NewVersion,
    PartDone, PartReq, PartUrl, Quota, Registered, UploadStatus, Version, VersionPage,
    VersionPatch,
};
use crate::core::cloud::api::{Api, CallError};

/// At most this many blobs per `blobs/check` or `blobs/download` call.
pub const BLOBS_PER_CALL: usize = 1000;
/// At most this many part URLs per `uploads/{id}/parts` call.
pub const PARTS_PER_CALL: usize = 100;
/// A commit body may be up to 8 MB: give it longer than the usual 15 s.
const COMMIT_TIMEOUT: Duration = Duration::from_secs(120);

/// A server id (UUID, any version) that is safe in a path.
pub fn is_id(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

fn id(s: &str) -> Result<&str, CallError> {
    if is_id(s) {
        Ok(s)
    } else {
        Err(CallError::Api {
            status: 400,
            code: "bad_request".into(),
            message: "Not a cloud id".into(),
            details: None,
            retry_after: None,
        })
    }
}

impl Api {
    pub async fn cloud_quota(&self, session: &str) -> Result<Quota, CallError> {
        self.get("/v1/cloud/quota", Some(session)).await
    }

    /// The account's projects, most recently updated first (with the trash on request).
    pub async fn cloud_projects(
        &self,
        session: &str,
        trashed: bool,
    ) -> Result<Vec<CloudProject>, CallError> {
        #[derive(serde::Deserialize)]
        struct L {
            projects: Vec<CloudProject>,
        }
        let path = if trashed {
            "/v1/cloud/projects?trashed=include"
        } else {
            "/v1/cloud/projects"
        };
        self.get::<L>(path, Some(session)).await.map(|l| l.projects)
    }

    /// Registers a project, or gets it (`201` new, `200` existing).
    pub async fn cloud_register(
        &self,
        session: &str,
        app_project_id: &str,
        name: &str,
        if_trashed: IfTrashed,
    ) -> Result<Registered, CallError> {
        let (status, project) = self
            .send_with::<CloudProject>(
                reqwest::Method::POST,
                "/v1/cloud/projects",
                Some(session),
                Some(json!({ "app_project_id": app_project_id, "name": name,
                             "if_trashed": if_trashed })),
                &[],
                None,
            )
            .await?;
        Ok(Registered {
            project: project.ok_or(CallError::Offline)?,
            created: status == 201,
        })
    }

    pub async fn cloud_project(&self, session: &str, pid: &str) -> Result<CloudProject, CallError> {
        self.get(&format!("/v1/cloud/projects/{}", id(pid)?), Some(session))
            .await
    }

    pub async fn cloud_rename(
        &self,
        session: &str,
        pid: &str,
        name: &str,
    ) -> Result<CloudProject, CallError> {
        self.send(
            reqwest::Method::PATCH,
            &format!("/v1/cloud/projects/{}", id(pid)?),
            Some(session),
            Some(json!({ "name": name })),
        )
        .await?
        .ok_or(CallError::Offline)
    }

    /// Moves a project to the trash.
    pub async fn cloud_trash(&self, session: &str, pid: &str) -> Result<CloudProject, CallError> {
        self.send(
            reqwest::Method::DELETE,
            &format!("/v1/cloud/projects/{}", id(pid)?),
            Some(session),
            None,
        )
        .await?
        .ok_or(CallError::Offline)
    }

    /// Restores a project from the trash.
    pub async fn cloud_untrash(&self, session: &str, pid: &str) -> Result<CloudProject, CallError> {
        self.post(
            &format!("/v1/cloud/projects/{}/restore", id(pid)?),
            Some(session),
            json!({}),
        )
        .await
    }

    pub async fn cloud_versions(
        &self,
        session: &str,
        pid: &str,
        before: Option<&str>,
        limit: u32,
    ) -> Result<VersionPage, CallError> {
        let mut path = format!("/v1/cloud/projects/{}/versions?limit={limit}", id(pid)?);
        if let Some(b) = before {
            path.push_str(&format!("&before={}", id(b)?));
        }
        self.get(&path, Some(session)).await
    }

    pub async fn cloud_manifest(
        &self,
        session: &str,
        pid: &str,
        vid: &str,
    ) -> Result<ManifestView, CallError> {
        self.get(
            &format!(
                "/v1/cloud/projects/{}/versions/{}/manifest",
                id(pid)?,
                id(vid)?
            ),
            Some(session),
        )
        .await
    }

    pub async fn cloud_patch_version(
        &self,
        session: &str,
        pid: &str,
        vid: &str,
        patch: &VersionPatch,
    ) -> Result<Version, CallError> {
        self.send(
            reqwest::Method::PATCH,
            &format!("/v1/cloud/projects/{}/versions/{}", id(pid)?, id(vid)?),
            Some(session),
            Some(serde_json::to_value(patch).map_err(|_| CallError::Offline)?),
        )
        .await?
        .ok_or(CallError::Offline)
    }

    pub async fn cloud_delete_version(
        &self,
        session: &str,
        pid: &str,
        vid: &str,
    ) -> Result<(), CallError> {
        self.send::<Value>(
            reqwest::Method::DELETE,
            &format!("/v1/cloud/projects/{}/versions/{}", id(pid)?, id(vid)?),
            Some(session),
            None,
        )
        .await
        .map(drop)
    }

    /// Commits a version. `key` is the `Idempotency-Key`: the same for every retry of one
    /// commit.
    pub async fn cloud_commit(
        &self,
        session: &str,
        pid: &str,
        key: &str,
        version: &NewVersion,
    ) -> Result<Committed, CallError> {
        let body = serde_json::to_value(version).map_err(|_| CallError::Offline)?;
        self.send_with(
            reqwest::Method::POST,
            &format!("/v1/cloud/projects/{}/versions", id(pid)?),
            Some(session),
            Some(body),
            &[("Idempotency-Key", key.to_string())],
            Some(COMMIT_TIMEOUT),
        )
        .await?
        .1
        .ok_or(CallError::Offline)
    }

    /// Which blobs the server still needs (≤ [`BLOBS_PER_CALL`]), with upload instructions.
    pub async fn cloud_check(
        &self,
        session: &str,
        blobs: &[BlobRef],
    ) -> Result<Checked, CallError> {
        self.post(
            "/v1/cloud/blobs/check",
            Some(session),
            json!({ "blobs": blobs }),
        )
        .await
    }

    /// Part URLs (≤ [`PARTS_PER_CALL`]) for a multipart upload.
    pub async fn cloud_part_urls(
        &self,
        session: &str,
        upload_id: &str,
        parts: &[PartReq],
    ) -> Result<Vec<PartUrl>, CallError> {
        #[derive(serde::Deserialize)]
        struct P {
            parts: Vec<PartUrl>,
        }
        self.post::<P>(
            &format!("/v1/cloud/uploads/{}/parts", id(upload_id)?),
            Some(session),
            json!({ "parts": parts }),
        )
        .await
        .map(|p| p.parts)
    }

    /// What storage already has of a multipart upload (to resume).
    pub async fn cloud_upload_status(
        &self,
        session: &str,
        upload_id: &str,
    ) -> Result<UploadStatus, CallError> {
        self.get(
            &format!("/v1/cloud/uploads/{}", id(upload_id)?),
            Some(session),
        )
        .await
    }

    /// Completes a multipart upload with every part, in order.
    pub async fn cloud_complete(
        &self,
        session: &str,
        upload_id: &str,
        parts: &[PartDone],
    ) -> Result<(), CallError> {
        self.post::<Value>(
            &format!("/v1/cloud/uploads/{}/complete", id(upload_id)?),
            Some(session),
            json!({ "parts": parts }),
        )
        .await
        .map(drop)
    }

    pub async fn cloud_abort_upload(
        &self,
        session: &str,
        upload_id: &str,
    ) -> Result<(), CallError> {
        self.send::<Value>(
            reqwest::Method::DELETE,
            &format!("/v1/cloud/uploads/{}", id(upload_id)?),
            Some(session),
            None,
        )
        .await
        .map(drop)
    }

    /// Download URLs for stored blobs (≤ [`BLOBS_PER_CALL`]).
    pub async fn cloud_downloads(
        &self,
        session: &str,
        sha256s: &[String],
    ) -> Result<Downloads, CallError> {
        self.post(
            "/v1/cloud/blobs/download",
            Some(session),
            json!({ "sha256s": sha256s }),
        )
        .await
    }
}
