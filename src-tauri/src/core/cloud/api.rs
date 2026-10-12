//! Typed calls to the Kinetra API. Every request, response and error code here is in
//! `docs/API.md`; nothing is guessed. Bodies are never logged (they hold tokens); only the
//! method, path, status and `x-request-id` are, at debug level.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// An error answer (`{"error": {code, message, details}}`), or no usable answer.
#[derive(Debug, Clone, PartialEq)]
pub enum CallError {
    /// The server answered with an error body.
    Api {
        status: u16,
        code: String,
        message: String,
        details: Option<Value>,
        /// `Retry-After` in seconds (429).
        retry_after: Option<u64>,
    },
    /// No answer, a timeout, a 5xx without an error body, or a body that doesn't parse:
    /// "offline". Never treated as a license decision.
    Offline,
}

impl CallError {
    pub fn code(&self) -> Option<&str> {
        match self {
            CallError::Api { code, .. } => Some(code),
            CallError::Offline => None,
        }
    }

    /// `503 service_unavailable` / `license_service_unavailable` count as offline: keep the
    /// current state, retry later.
    pub fn is_offline(&self) -> bool {
        match self {
            CallError::Offline => true,
            CallError::Api { status, code, .. } => {
                *status == 503
                    && (code == "service_unavailable" || code == "license_service_unavailable")
            }
        }
    }
}

impl From<CallError> for crate::error::AppError {
    fn from(e: CallError) -> Self {
        match e {
            CallError::Offline => crate::error::AppError::Offline,
            CallError::Api {
                code,
                message,
                details,
                ..
            } => crate::error::AppError::Cloud {
                api_code: code,
                message,
                details,
            },
        }
    }
}

// ---- Shapes (docs/API.md) -------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct User {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DeviceToken {
    pub session_token: String,
    pub expires_at: String,
    pub user: User,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct Activation {
    pub id: String,
    #[serde(default)]
    pub machine_name: Option<String>,
    #[serde(default)]
    pub slot: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub last_refreshed_at: Option<String>,
    #[serde(default)]
    pub this_machine: Option<bool>,
}

/// `License` (display data; the signed token is what counts).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct License {
    pub id: String,
    pub plan: String,
    pub kind: String,
    pub status: String,
    #[serde(default)]
    pub entitlements: Vec<String>,
    #[serde(default)]
    pub seats: u32,
    #[serde(default)]
    pub seats_used: u32,
    #[serde(default)]
    pub backup_used: bool,
    #[serde(default)]
    pub starts_at: Option<String>,
    #[serde(default)]
    pub ends_at: Option<String>,
    #[serde(default)]
    pub updates_until: Option<String>,
    #[serde(default)]
    pub revoked_at: Option<String>,
    #[serde(default)]
    pub beta_program_id: Option<String>,
    #[serde(default)]
    pub activations: Vec<Activation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct UpgradeBlocked {
    pub license_id: String,
    pub plan: String,
    pub reason: String,
    #[serde(default)]
    pub activations: Vec<Activation>,
}

/// `POST /v1/license/activate` and `/refresh`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Activated {
    pub token: String,
    pub server_time: i64,
    pub activation_id: String,
    pub slot: String,
    pub license: License,
    #[serde(default)]
    pub upgrade_blocked: Option<UpgradeBlocked>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Licenses {
    pub licenses: Vec<License>,
    pub server_time: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SlotReq {
    Main,
    Backup,
}

// ---- AI (docs/API.md "AI") ---------------------------------------------------------------------

/** One task's quota (`generate_template` / `tweak_template`). Free has `period: "total"`. */
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct AiTaskQuota {
    pub used: u32,
    pub limit: u32,
    pub period: String,
    #[serde(default)]
    pub resets_at: Option<i64>,
    #[serde(default)]
    pub day_used: Option<u32>,
    #[serde(default)]
    pub day_limit: Option<u32>,
    #[serde(default)]
    pub day_resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct AiQuota {
    pub enabled: bool,
    #[serde(default)]
    pub disabled_for_account: bool,
    #[serde(default)]
    pub plan: String,
    #[serde(default)]
    pub generate_template: Option<AiTaskQuota>,
    #[serde(default)]
    pub tweak_template: Option<AiTaskQuota>,
}

/// `202` when a job starts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AiJobStarted {
    pub job_id: String,
    #[serde(default)]
    pub poll_after_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AiJobError {
    pub code: String,
    #[serde(default)]
    pub message: String,
}

/// `GET /v1/ai/jobs/{id}`: pending, succeeded (with the template package, returned once), or
/// declined / invalid / incomplete / failed (with an error).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AiJobPoll {
    pub status: String,
    #[serde(default)]
    pub poll_after_ms: Option<u64>,
    #[serde(default)]
    pub template: Option<Value>,
    #[serde(default)]
    pub error: Option<AiJobError>,
}

// ---- Client -------------------------------------------------------------------------------

/// The HTTP client for one base URL (`config::api_base()`, or a test server).
#[derive(Clone)]
pub struct Api {
    base: String,
    http: reqwest::Client,
}

impl Api {
    pub fn new(base: String) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("Kinetra/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TIMEOUT)
            .build()
            .map_err(|_| "The HTTP client could not start".to_string())?;
        Ok(Self { base, http })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    async fn send<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        session: Option<&str>,
        body: Option<Value>,
    ) -> Result<Option<T>, CallError> {
        let url = format!("{}{}", self.base, path);
        let mut req = self.http.request(method.clone(), &url);
        if let Some(t) = session {
            req = req.bearer_auth(t);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(|e| {
            tracing::debug!(
                "cloud {method} {path}: {}",
                if e.is_timeout() {
                    "timeout"
                } else {
                    "no connection"
                }
            );
            CallError::Offline
        })?;
        let status = resp.status().as_u16();
        let rid = resp
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-")
            .to_string();
        tracing::debug!("cloud {method} {path} → {status} (x-request-id {rid})");
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let bytes = resp.bytes().await.map_err(|_| CallError::Offline)?;
        if (200..300).contains(&status) {
            if status == 204 || bytes.is_empty() {
                return Ok(None);
            }
            return serde_json::from_slice(&bytes).map(Some).map_err(|_| {
                tracing::debug!(
                    "cloud {method} {path}: the answer didn't parse (x-request-id {rid})"
                );
                CallError::Offline
            });
        }
        Err(error_body(status, retry_after, &bytes))
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        session: Option<&str>,
        body: Value,
    ) -> Result<T, CallError> {
        self.send(reqwest::Method::POST, path, session, Some(body))
            .await?
            .ok_or(CallError::Offline)
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        session: Option<&str>,
    ) -> Result<T, CallError> {
        self.send(reqwest::Method::GET, path, session, None)
            .await?
            .ok_or(CallError::Offline)
    }

    // ---- Sign-in ----

    pub async fn device_start(&self, device_name: Option<&str>) -> Result<DeviceStart, CallError> {
        self.post(
            "/v1/auth/device",
            None,
            json!({ "device_name": device_name }),
        )
        .await
    }

    pub async fn device_token(&self, device_code: &str) -> Result<DeviceToken, CallError> {
        self.post(
            "/v1/auth/device/token",
            None,
            json!({ "device_code": device_code }),
        )
        .await
    }

    pub async fn logout(&self, session: &str) -> Result<(), CallError> {
        self.send::<Value>(
            reqwest::Method::POST,
            "/v1/auth/logout",
            Some(session),
            None,
        )
        .await
        .map(drop)
    }

    pub async fn me(&self, session: &str) -> Result<User, CallError> {
        self.get("/v1/me", Some(session)).await
    }

    // ---- Licensing ----

    pub async fn keys(&self) -> Result<kinetra_license::SignedKeySet, CallError> {
        self.get("/v1/license/keys", None).await
    }

    pub async fn revocations(&self) -> Result<String, CallError> {
        #[derive(Deserialize)]
        struct R {
            revocations: String,
        }
        self.get::<R>("/v1/license/revocations", None)
            .await
            .map(|r| r.revocations)
    }

    pub async fn licenses(&self, session: &str, machine: &str) -> Result<Licenses, CallError> {
        self.get(&format!("/v1/license?machine={machine}"), Some(session))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn activate(
        &self,
        session: &str,
        machine: &str,
        machine_name: &str,
        slot: SlotReq,
        license_id: Option<&str>,
        nonce: &str,
        timestamp: i64,
    ) -> Result<Activated, CallError> {
        self.post(
            "/v1/license/activate",
            Some(session),
            json!({ "machine": machine, "machine_name": machine_name, "slot": slot,
                    "license_id": license_id, "nonce": nonce, "timestamp": timestamp }),
        )
        .await
    }

    pub async fn refresh(
        &self,
        session: &str,
        activation_id: &str,
        machine: &str,
        nonce: &str,
        timestamp: i64,
    ) -> Result<Activated, CallError> {
        self.post(
            "/v1/license/refresh",
            Some(session),
            json!({ "activation_id": activation_id, "machine": machine, "nonce": nonce, "timestamp": timestamp }),
        )
        .await
    }

    pub async fn deactivate(&self, session: &str, activation_id: &str) -> Result<(), CallError> {
        self.send::<Value>(
            reqwest::Method::POST,
            "/v1/license/deactivate",
            Some(session),
            Some(json!({ "activation_id": activation_id })),
        )
        .await
        .map(drop)
    }

    pub async fn redeem(&self, session: &str, key: &str) -> Result<License, CallError> {
        self.post("/v1/license/redeem", Some(session), json!({ "key": key }))
            .await
    }

    // ---- AI ----

    pub async fn ai_quota(&self, session: &str, machine: &str) -> Result<AiQuota, CallError> {
        self.get(&format!("/v1/ai/quota?machine={machine}"), Some(session))
            .await
    }

    /// `POST /v1/ai/templates/generate` with the already-validated body.
    pub async fn ai_generate(&self, session: &str, body: Value) -> Result<AiJobStarted, CallError> {
        self.post("/v1/ai/templates/generate", Some(session), body)
            .await
    }

    /// `POST /v1/ai/templates/tweak`.
    pub async fn ai_tweak(&self, session: &str, body: Value) -> Result<AiJobStarted, CallError> {
        self.post("/v1/ai/templates/tweak", Some(session), body)
            .await
    }

    pub async fn ai_job(
        &self,
        session: &str,
        job_id: &str,
        machine: &str,
    ) -> Result<AiJobPoll, CallError> {
        self.get(
            &format!("/v1/ai/jobs/{job_id}?machine={machine}"),
            Some(session),
        )
        .await
    }

    pub async fn beta_status(
        &self,
        session: Option<&str>,
        program: &str,
    ) -> Result<String, CallError> {
        #[derive(Deserialize)]
        struct S {
            status: String,
        }
        self.get::<S>(&format!("/v1/beta/status?program={program}"), session)
            .await
            .map(|s| s.status)
    }

    // ---- Polls (docs/API.md "Polls") ----
    // Ids go into paths: callers pass only ids that passed `polls::is_poll_id`.

    pub async fn polls_quota(&self, session: &str) -> Result<PollQuota, CallError> {
        self.get("/v1/polls/quota", Some(session)).await
    }

    pub async fn polls_list(&self, session: &str) -> Result<Vec<PollWire>, CallError> {
        #[derive(Deserialize)]
        struct L {
            polls: Vec<PollWire>,
        }
        self.get::<L>("/v1/polls", Some(session))
            .await
            .map(|l| l.polls)
    }

    pub async fn poll_create(&self, session: &str, body: Value) -> Result<PollWire, CallError> {
        self.post("/v1/polls", Some(session), body).await
    }

    pub async fn poll_get(&self, session: &str, id: &str) -> Result<PollWire, CallError> {
        self.get(&format!("/v1/polls/{id}"), Some(session)).await
    }

    pub async fn poll_patch(
        &self,
        session: &str,
        id: &str,
        body: Value,
    ) -> Result<PollWire, CallError> {
        self.send(
            reqwest::Method::PATCH,
            &format!("/v1/polls/{id}"),
            Some(session),
            Some(body),
        )
        .await?
        .ok_or(CallError::Offline)
    }

    /// `open`, `close` or `reset`.
    pub async fn poll_action(
        &self,
        session: &str,
        id: &str,
        action: &str,
    ) -> Result<PollWire, CallError> {
        self.post(
            &format!("/v1/polls/{id}/{action}"),
            Some(session),
            json!({}),
        )
        .await
    }

    pub async fn poll_delete(&self, session: &str, id: &str) -> Result<(), CallError> {
        self.send::<Value>(
            reqwest::Method::DELETE,
            &format!("/v1/polls/{id}"),
            Some(session),
            None,
        )
        .await
        .map(|_| ())
    }

    /// Opens the live results stream (`text/event-stream`) with `client` (one without a total
    /// timeout). `Ok` is a 200 whose body is the stream; errors before it are the usual JSON.
    pub async fn poll_events(
        &self,
        client: &reqwest::Client,
        session: &str,
        id: &str,
    ) -> Result<reqwest::Response, CallError> {
        let path = format!("/v1/polls/{id}/events");
        let resp = client
            .get(format!("{}{path}", self.base))
            .bearer_auth(session)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(|_| CallError::Offline)?;
        let status = resp.status().as_u16();
        tracing::debug!("cloud GET {path} → {status} (stream)");
        if status == 200 {
            return Ok(resp);
        }
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let bytes = resp.bytes().await.map_err(|_| CallError::Offline)?;
        Err(error_body(status, retry_after, &bytes))
    }
}

/// An error answer's `{"error": {…}}`, or `Offline` when the body isn't one.
fn error_body(status: u16, retry_after: Option<u64>, bytes: &[u8]) -> CallError {
    #[derive(Deserialize)]
    struct Envelope {
        error: ErrBody,
    }
    #[derive(Deserialize)]
    struct ErrBody {
        code: String,
        #[serde(default)]
        message: String,
        #[serde(default)]
        details: Option<Value>,
    }
    match serde_json::from_slice::<Envelope>(bytes) {
        Ok(e) => CallError::Api {
            status,
            code: e.error.code,
            message: e.error.message,
            details: e.error.details,
            retry_after,
        },
        Err(_) => CallError::Offline,
    }
}

// ---- Polls --------------------------------------------------------------------------------------

pub use crate::core::model::PollQuota;

/// The poll object as the server sends it (docs/API.md "Poll object"). Unknown fields are
/// ignored (new ones may be added).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PollWire {
    pub id: String,
    pub code: String,
    pub url: String,
    pub question: String,
    pub options: Vec<PollWireOption>,
    pub status: String,
    #[serde(default)]
    pub show_results: bool,
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    #[serde(default)]
    pub total_votes: u64,
    #[serde(default)]
    pub max_votes: Option<u64>,
    #[serde(default)]
    pub vote_limit_reached: bool,
    #[serde(default)]
    pub round: u32,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub opened_at: Option<String>,
    #[serde(default)]
    pub closes_at: Option<String>,
    #[serde(default)]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub delete_after: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PollWireOption {
    pub position: u32,
    pub label: String,
    #[serde(default)]
    pub votes: u64,
}

impl From<PollWire> for crate::core::model::PollData {
    fn from(w: PollWire) -> Self {
        use crate::core::model::{PollOption, PollStatus};
        let mut options: Vec<PollOption> = w
            .options
            .into_iter()
            .map(|o| PollOption {
                position: o.position,
                label: o.label,
                votes: o.votes,
            })
            .collect();
        options.sort_by_key(|o| o.position);
        Self {
            id: w.id,
            code: w.code,
            url: w.url,
            question: w.question,
            options,
            status: match w.status.as_str() {
                "open" => PollStatus::Open,
                "closed" => PollStatus::Closed,
                _ => PollStatus::Draft,
            },
            show_results: w.show_results,
            duration_seconds: w.duration_seconds,
            total_votes: w.total_votes,
            max_votes: w.max_votes,
            vote_limit_reached: w.vote_limit_reached,
            round: w.round,
            created_at: w.created_at,
            opened_at: w.opened_at,
            closes_at: w.closes_at,
            closed_at: w.closed_at,
            delete_after: w.delete_after,
            expires_at: w.expires_at,
        }
    }
}
