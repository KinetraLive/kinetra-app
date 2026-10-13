//! The error type every command returns. It serializes to `{ code, message }` so the UI can branch
//! on `code` and show `message` as-is.
//!
//! Each `core` module has its own `thiserror` enum and converts into `AppError` with `From`.

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("This folder is not a Kinetra project: {0}")]
    NotAProject(String),
    #[error("The folder is not empty: {0}")]
    NotEmpty(String),
    #[error("This file was made by a newer version of Kinetra. Update Kinetra to open it.")]
    NewerVersion { found: u32, supported: u32 },
    #[error("{0}")]
    Invalid(String),
    #[error("No project is open")]
    NoProject,
    #[error("Canceled")]
    Cancelled,
    #[error("{0}")]
    Busy(String),
    /// Checking for or downloading an update failed (the real error is in the log).
    #[error("Couldn't check for updates. Try again later.")]
    UpdateCheck,
    /// Installing an update while anything is on air.
    #[error("Take everything off air before installing.")]
    OnAir,
    /// Installing an update while a display output shows something (same code as `OnAir`).
    #[error("Turn off the outputs before installing.")]
    OutputsOn,
    /// A Pro feature without a license that grants it (decided in the core, never the UI).
    #[error("This needs Kinetra Pro.")]
    ProRequired,
    /// Pro is in reduced mode (offline too long): existing Pro work keeps running, new Pro work
    /// waits until Kinetra connects.
    #[error("Kinetra needs to connect to check your license before starting new Pro work.")]
    ProLimited,
    /// The Kinetra server can't be reached (network, timeout, 5xx without a body, 503).
    #[error("Kinetra can't reach the server. Check the connection and try again.")]
    Offline,
    /// An error answer from the Kinetra API (`docs/API.md`): its `code`, human `message` and
    /// optional `details`, passed to the UI as `apiCode` / `message` / `details`.
    #[error("{message}")]
    Cloud {
        api_code: String,
        message: String,
        details: Option<serde_json::Value>,
    },
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Io(_) => "io",
            AppError::NotFound(_) => "notFound",
            AppError::NotAProject(_) => "notAProject",
            AppError::NotEmpty(_) => "notEmpty",
            AppError::NewerVersion { .. } => "newerVersion",
            AppError::Invalid(_) => "invalid",
            AppError::NoProject => "noProject",
            AppError::Cancelled => "cancelled",
            AppError::Busy(_) => "busy",
            AppError::UpdateCheck => "updateCheck",
            AppError::OnAir | AppError::OutputsOn => "onAir",
            AppError::ProRequired => "proRequired",
            AppError::ProLimited => "proLimited",
            AppError::Offline => "offline",
            AppError::Cloud { .. } => "cloud",
            AppError::Internal(_) => "internal",
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Invalid(e.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("AppError", 4)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("message", &self.to_string())?;
        if let AppError::Cloud {
            api_code, details, ..
        } = self
        {
            st.serialize_field("apiCode", api_code)?;
            if let Some(d) = details {
                st.serialize_field("details", d)?;
            }
        }
        st.end()
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_code_and_message() {
        let json = serde_json::to_value(AppError::NewerVersion {
            found: 3,
            supported: 1,
        })
        .unwrap();
        assert_eq!(json["code"], "newerVersion");
        assert!(json["message"].as_str().unwrap().contains("newer version"));
    }
}
