//! Inviting and managing collaborators from the app (protocol 2 §9): the owner's own show, found
//! by the project's remote control ID (`project.json` `id`), through `api.kinetra.live` with the
//! session. The same records as the control site's Collaborators page; the server sends the
//! invite email and applies permission changes to open connections at once.
//!
//! Only the owner reaches a show (anyone else, or a project never registered: `404 not_found`).
//! Inviting and changing permissions are new Pro work (`pro.control`); listing and removing
//! aren't gated, so a lapsed owner can still take people off a show. Emails go to the server and
//! nowhere else: never logged, never written.

use super::perms::normalize;
use super::PRO_CONTROL;
use crate::core::cloud::api::{CollaboratorWire, CollaboratorsWire};
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

/// Longest email the app sends (RFC 5321's path limit).
pub const EMAIL_MAX: usize = 254;

/// The email as sent: trimmed, one `@` with something on both sides, no spaces.
pub fn check_email(email: &str) -> Result<String, String> {
    let e = email.trim();
    let ok = (3..=EMAIL_MAX).contains(&e.chars().count())
        && !e.chars().any(char::is_whitespace)
        && e.split('@').count() == 2
        && e.split('@').all(|p| !p.is_empty())
        && e.rsplit('@').next().is_some_and(|d| d.contains('.'));
    if ok {
        Ok(e.to_string())
    } else {
        Err("Enter an email address like name@example.com.".into())
    }
}

/// A collaborator id from the server before it goes into a path.
fn path_safe(id: &str) -> AppResult<&str> {
    let ok = (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(id)
    } else {
        Err(AppError::Invalid("That collaborator doesn't exist.".into()))
    }
}

impl AppState {
    /// The session, the API and the project's show id (a UUID) for a collaborators call.
    fn collaborators_target(&self) -> AppResult<(crate::core::cloud::api::Api, String, String)> {
        if !self.remote_available() {
            return Err(AppError::Invalid(
                "Remote control isn't available in this version of Kinetra yet.".into(),
            ));
        }
        self.project_dir()?;
        let project_id = self
            .project_remote_id()
            .filter(|id| super::ids::is_uuid(id))
            .ok_or_else(|| {
                AppError::Invalid("Turn on remote control once to invite collaborators.".into())
            })?;
        let (api, session, _) = self.cloud_session_parts()?;
        Ok((api, session, project_id))
    }

    /// The show's collaborators (joined first, then pending) and the plan's limit.
    pub async fn remote_collaborators(&self) -> AppResult<CollaboratorsWire> {
        let (api, s, project) = self.collaborators_target()?;
        Ok(api.collaborators_list(&s, &project).await?)
    }

    /// Invites `email` with `permissions` (unknown codes dropped); the server sends the email.
    pub async fn remote_invite(
        &self,
        email: &str,
        permissions: &[String],
    ) -> AppResult<CollaboratorWire> {
        self.require_new_pro_work(PRO_CONTROL)?;
        let email = check_email(email).map_err(AppError::Invalid)?;
        let (api, s, project) = self.collaborators_target()?;
        Ok(api
            .collaborator_invite(&s, &project, &email, &normalize(permissions))
            .await?)
    }

    /// Changes a pending or joined collaborator's permissions (live on their open connections).
    pub async fn remote_collaborator_set(
        &self,
        id: &str,
        permissions: &[String],
    ) -> AppResult<CollaboratorWire> {
        self.require_new_pro_work(PRO_CONTROL)?;
        let id = path_safe(id)?;
        let (api, s, project) = self.collaborators_target()?;
        Ok(api
            .collaborator_patch(&s, &project, id, &normalize(permissions))
            .await?)
    }

    /// Revokes a pending invite, or removes a joined collaborator (disconnected at once).
    pub async fn remote_collaborator_remove(&self, id: &str) -> AppResult<()> {
        let id = path_safe(id)?;
        let (api, s, project) = self.collaborators_target()?;
        Ok(api.collaborator_delete(&s, &project, id).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_trimmed_and_checked() {
        assert_eq!(
            check_email("  kari@example.com ").unwrap(),
            "kari@example.com"
        );
        for bad in [
            "",
            "kari",
            "kari@",
            "@example.com",
            "a@b@c.no",
            "kari @x.no",
            "kari@local",
        ] {
            assert!(check_email(bad).is_err(), "{bad}");
        }
        assert!(check_email(&format!("{}@x.no", "a".repeat(260))).is_err());
    }

    #[test]
    fn ids_must_be_path_safe() {
        assert!(path_safe("0193aaaa-0000-7000-8000-000000000001").is_ok());
        for bad in ["", "../x", "a/b", "a?b", &"a".repeat(65)] {
            assert!(path_safe(bad).is_err(), "{bad}");
        }
    }
}
