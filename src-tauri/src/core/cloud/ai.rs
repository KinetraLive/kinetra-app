//! AI template generation through the Kinetra backend (`docs/API.md` "AI"). The core gates (`pro.ai`, full access: Reduced mode blocks new generations),
//! checks the input against the backend's limits before sending, starts a job, polls it with
//! backoff on one tokio task, and imports the result as a normal template folder (validated and
//! hardened like any save). `ai-job` events carry progress; the user keeps working meanwhile.
//!
//! Before import a result is tidied ([`tidy_package`]): its HTML and CSS pretty-printed
//! (`template_format`; models answer in one long line), and field defaults that are only the
//! field's label or key (the model's placeholder, which showed as huge overlapping text) cleared:
//! `0` for number-like fields ([`number_like`]), empty for the rest. A tweak's name gets
//! " (tweaked)" at most once ([`tweaked_name`]).

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use kinetra_license::entitlements::PRO_AI;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::task::AbortHandle;

use super::api::{AiQuota, CallError};
use crate::core::events::names;
use crate::core::model::TEMPLATE_FONTS;
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

/// `KINETRA_AI_MAX_INPUT_CHARS` (API.md): description and instruction, in characters.
pub const AI_MAX_INPUT_CHARS: usize = 2000;
pub const AI_MAX_COLORS: usize = 4;
pub const AI_MAX_FONTS: usize = 2;
/// API.md `template_type` hints.
pub const AI_TEMPLATE_TYPES: &[&str] = &[
    "lower_third",
    "full_screen",
    "title",
    "bug",
    "ticker",
    "scoreboard",
    "countdown",
    "schedule",
    "list",
    "other",
];
/// Whether a field holds a number: type `number`, or a number word in the key or label of a
/// text field (`fieldvals::number_words`).
pub fn number_like(field_type: &str, key: &str, label: &str) -> bool {
    match field_type {
        "number" => true,
        "text" | "multiline" => crate::core::fieldvals::number_words(key, label),
        _ => false,
    }
}

/// A tweaked template's name: the original with " (tweaked)" once (earlier suffixes dropped),
/// at most 100 characters.
pub fn tweaked_name(name: &str) -> String {
    const SUFFIX: &str = " (tweaked)";
    let mut base = name.trim();
    while let Some(b) = base.strip_suffix(SUFFIX) {
        base = b.trim_end();
    }
    let room = 100 - SUFFIX.chars().count();
    let base: String = base.chars().take(room).collect();
    format!("{}{SUFFIX}", base.trim_end())
}

/// Tidies an AI result before import (see the module docs): formatted HTML and CSS, and
/// placeholder defaults (the label or key) cleared.
pub fn tidy_package(package: &mut Value) {
    use crate::core::template_format::{format_css, format_html};
    if let Some(files) = package.get_mut("files").and_then(Value::as_object_mut) {
        for v in files.values_mut() {
            for (k, fmt) in [
                ("html", format_html as fn(&str) -> String),
                ("css", format_css),
            ] {
                if let Some(text) = v.get(k).and_then(Value::as_str) {
                    let formatted = fmt(text);
                    v[k] = Value::String(formatted);
                }
            }
        }
    }
    let Some(fields) = package
        .pointer_mut("/manifest/fields")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for f in fields {
        let get = |k: &str| {
            f.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let (key, label, ty, default) = (get("key"), get("label"), get("type"), get("default"));
        if !matches!(ty.as_str(), "text" | "multiline" | "number") || default.is_empty() {
            continue;
        }
        let placeholder = default.eq_ignore_ascii_case(&label)
            || default.eq_ignore_ascii_case(&key)
            || default.eq_ignore_ascii_case(&format!("[{label}]"));
        let bad_number = ty == "number" && default.parse::<f64>().is_err();
        if placeholder || bad_number {
            let value = if number_like(&ty, &key, &label) {
                "0"
            } else {
                ""
            };
            f["default"] = Value::String(value.to_string());
        }
    }
}

/// Jobs expire after `KINETRA_AI_JOB_TTL` (1 h); polling stops then.
const JOB_MAX: Duration = Duration::from_secs(3600);
const POLL_MIN: Duration = Duration::from_millis(500);
const POLL_OFFLINE_MAX: Duration = Duration::from_secs(30);

/// "Generate template" input (the UI sends this; the core checks it).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiGenerateInput {
    pub prompt: String,
    pub template_type: Option<String>,
    pub brand_colors: Vec<String>,
    pub fonts: Vec<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AiTask {
    Generate,
    Tweak,
}

/// A job as the UI sees it (`ai-job` events, `ai_jobs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiJobView {
    pub id: String,
    pub task: AiTask,
    /// pending | succeeded | failed | canceled
    pub status: &'static str,
    /// The API's error code when failed (`ai_declined`, `ai_output_invalid`, …, or `import`).
    pub code: Option<String>,
    pub message: Option<String>,
    /// The imported template (succeeded).
    pub template_id: Option<String>,
    /// What was asked (the description or instruction), for the job chip.
    pub label: String,
}

#[derive(Default)]
pub(crate) struct AiJobs {
    jobs: Mutex<HashMap<String, (AiJobView, Option<AbortHandle>)>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn invalid(m: impl Into<String>) -> AppError {
    AppError::Invalid(m.into())
}

/// The request body for `/v1/ai/templates/generate`, or why the input can't be sent (the
/// backend's own rules, checked first so nothing is spent on a bad request).
pub fn generate_body(input: &AiGenerateInput, machine: &str) -> AppResult<Value> {
    let prompt = input.prompt.trim();
    if prompt.is_empty() {
        return Err(invalid("Describe the template you want."));
    }
    if prompt.chars().count() > AI_MAX_INPUT_CHARS {
        return Err(invalid(format!(
            "The description is too long: at most {AI_MAX_INPUT_CHARS} characters."
        )));
    }
    let mut body = json!({ "machine": machine, "prompt": prompt });
    if let Some(t) = input.template_type.as_deref().filter(|t| !t.is_empty()) {
        if !AI_TEMPLATE_TYPES.contains(&t) {
            return Err(invalid("Unknown template type."));
        }
        body["template_type"] = json!(t);
    }
    if !input.brand_colors.is_empty() {
        if input.brand_colors.len() > AI_MAX_COLORS {
            return Err(invalid(format!("At most {AI_MAX_COLORS} brand colors.")));
        }
        for c in &input.brand_colors {
            let ok =
                c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|h| h.is_ascii_hexdigit());
            if !ok {
                return Err(invalid(format!("{c} isn't a color like #c8102e.")));
            }
        }
        body["brand_colors"] = json!(input
            .brand_colors
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .collect::<Vec<_>>());
    }
    if !input.fonts.is_empty() {
        if input.fonts.len() > AI_MAX_FONTS {
            return Err(invalid(format!("At most {AI_MAX_FONTS} fonts.")));
        }
        for f in &input.fonts {
            if !TEMPLATE_FONTS.iter().any(|(n, _, _)| n == f) {
                return Err(invalid(format!("{f} isn't one of Kinetra's fonts.")));
            }
        }
        body["fonts"] = json!(input.fonts);
    }
    match (input.width, input.height) {
        (Some(w), Some(h)) => {
            if !(16..=8192).contains(&w) || !(16..=8192).contains(&h) {
                return Err(invalid("The size must be 16 to 8192 pixels on each side."));
            }
            body["size"] = json!({ "width": w, "height": h });
        }
        (None, None) => {}
        _ => return Err(invalid("Give both width and height, or neither.")),
    }
    Ok(body)
}

/// A job id is safe to put into a URL path (the backend's ids are UUIDs).
fn valid_job_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

impl AppState {
    fn ai_jobs(&self) -> &AiJobs {
        self.cloud_service().ai_jobs()
    }

    /// Remaining AI quota (`GET /v1/ai/quota`).
    pub async fn ai_quota(&self) -> AppResult<AiQuota> {
        let (api, session, machine) = self.cloud_session_parts()?;
        Ok(api.ai_quota(&session, &machine).await?)
    }

    /// Starts "Generate template". Gated in the core: `pro.ai` with full access.
    pub async fn ai_generate(&self, input: AiGenerateInput) -> AppResult<AiJobView> {
        self.require_new_pro_work(PRO_AI)?;
        self.project_dir()?;
        let (api, session, machine) = self.cloud_session_parts()?;
        let body = generate_body(&input, &machine)?;
        let started = api.ai_generate(&session, body).await?;
        Ok(self.ai_track(
            started.job_id,
            started.poll_after_ms,
            AiTask::Generate,
            input.prompt.trim().to_string(),
            None,
        ))
    }

    /// Starts "Tweak with AI" on template `template_id`; the result is imported as a new
    /// template ("<name> (tweaked)", the suffix once: [`tweaked_name`]), the original stays.
    pub async fn ai_tweak(&self, template_id: String, instruction: String) -> AppResult<AiJobView> {
        self.require_new_pro_work(PRO_AI)?;
        let instruction = instruction.trim().to_string();
        if instruction.is_empty() {
            return Err(invalid("Say what to change."));
        }
        if instruction.chars().count() > AI_MAX_INPUT_CHARS {
            return Err(invalid(format!(
                "The instruction is too long: at most {AI_MAX_INPUT_CHARS} characters."
            )));
        }
        let dir = self.project_dir()?;
        let package = crate::core::templates::to_package(&dir, &template_id)?;
        let name = package["manifest"]["name"]
            .as_str()
            .unwrap_or("Template")
            .to_string();
        let (api, session, machine) = self.cloud_session_parts()?;
        let body = json!({ "machine": machine, "template": package, "instruction": instruction });
        let started = api.ai_tweak(&session, body).await?;
        Ok(self.ai_track(
            started.job_id,
            started.poll_after_ms,
            AiTask::Tweak,
            instruction,
            Some(name),
        ))
    }

    /// Stops following a job (the server may still finish it; its result is dropped).
    pub fn ai_cancel(&self, job_id: &str) {
        let view = {
            let mut g = lock(&self.ai_jobs().jobs);
            g.get_mut(job_id).map(|(v, task)| {
                if let Some(t) = task.take() {
                    t.abort();
                }
                if v.status == "pending" {
                    v.status = "canceled";
                }
                v.clone()
            })
        };
        if let Some(v) = view {
            self.emit_ai(&v);
        }
    }

    /// Jobs of this session (newest first), for the UI after a reload.
    pub fn ai_jobs_list(&self) -> Vec<AiJobView> {
        let mut v: Vec<AiJobView> = lock(&self.ai_jobs().jobs)
            .values()
            .map(|(v, _)| v.clone())
            .collect();
        v.reverse();
        v
    }

    fn emit_ai(&self, v: &AiJobView) {
        if let Ok(p) = serde_json::to_value(v) {
            self.events().emit(names::AI_JOB, p);
        }
    }

    fn ai_set(&self, v: AiJobView) {
        if let Some((cur, _)) = lock(&self.ai_jobs().jobs).get_mut(&v.id) {
            *cur = v.clone();
        }
        self.emit_ai(&v);
    }

    /// Registers a started job and polls it on one task until it ends.
    fn ai_track(
        &self,
        job_id: String,
        poll_ms: Option<u64>,
        task: AiTask,
        label: String,
        tweak_of: Option<String>,
    ) -> AiJobView {
        let mut label = label;
        if label.chars().count() > 80 {
            label = label.chars().take(79).collect::<String>() + "…";
        }
        let view = AiJobView {
            id: job_id.clone(),
            task,
            status: "pending",
            code: None,
            message: None,
            template_id: None,
            label,
        };
        lock(&self.ai_jobs().jobs).insert(job_id.clone(), (view.clone(), None));
        self.emit_ai(&view);
        if !valid_job_id(&job_id) {
            self.ai_set(AiJobView {
                status: "failed",
                code: Some("bad_job".into()),
                message: Some("The server answered with an unusable job.".into()),
                ..view.clone()
            });
            return view;
        }
        let weak = self.downgrade();
        let base = view.clone();
        let fut = async move {
            let mut wait = Duration::from_millis(poll_ms.unwrap_or(2000)).max(POLL_MIN);
            let mut offline_wait = Duration::from_secs(2);
            let deadline = tokio::time::Instant::now() + JOB_MAX;
            loop {
                tokio::time::sleep(wait).await;
                let Some(st) = weak.upgrade() else { return };
                let Ok((api, session, machine)) = st.cloud_session_parts() else {
                    st.ai_set(AiJobView {
                        status: "failed",
                        code: Some("unauthorized".into()),
                        message: None,
                        ..base.clone()
                    });
                    return;
                };
                let next = match api.ai_job(&session, &job_id, &machine).await {
                    Ok(p) if p.status == "pending" => {
                        wait = Duration::from_millis(p.poll_after_ms.unwrap_or(2000)).max(POLL_MIN);
                        offline_wait = Duration::from_secs(2);
                        None
                    }
                    Ok(p) if p.status == "succeeded" => {
                        Some(st.ai_import(&base, p.template, tweak_of.as_deref()))
                    }
                    Ok(p) => {
                        let e = p.error.unwrap_or(super::api::AiJobError {
                            code: "ai_unavailable".into(),
                            message: String::new(),
                        });
                        Some(AiJobView {
                            status: "failed",
                            code: Some(e.code),
                            message: Some(e.message),
                            ..base.clone()
                        })
                    }
                    Err(CallError::Offline) => {
                        wait = offline_wait;
                        offline_wait = (offline_wait * 2).min(POLL_OFFLINE_MAX);
                        None
                    }
                    Err(CallError::Api {
                        code,
                        message,
                        retry_after,
                        ..
                    }) if code == "rate_limited" => {
                        let _ = message;
                        wait = Duration::from_secs(retry_after.unwrap_or(5));
                        None
                    }
                    Err(CallError::Api { code, message, .. }) => Some(AiJobView {
                        status: "failed",
                        code: Some(code),
                        message: Some(message),
                        ..base.clone()
                    }),
                };
                if let Some(v) = next {
                    st.ai_set(v);
                    if let Some((_, t)) = lock(&st.ai_jobs().jobs).get_mut(&job_id) {
                        *t = None;
                    }
                    return;
                }
                if tokio::time::Instant::now() >= deadline {
                    st.ai_set(AiJobView {
                        status: "failed",
                        code: Some("ai_job_gone".into()),
                        message: None,
                        ..base.clone()
                    });
                    return;
                }
            }
        };
        let handle = match self.live().runtime_handle() {
            Some(rt) => rt.spawn(fut),
            None => tokio::spawn(fut),
        };
        if let Some((_, t)) = lock(&self.ai_jobs().jobs).get_mut(&view.id) {
            *t = Some(handle.abort_handle());
        }
        view
    }

    /// Imports a finished job's package into the open project.
    fn ai_import(
        &self,
        base: &AiJobView,
        template: Option<Value>,
        tweak_of: Option<&str>,
    ) -> AiJobView {
        let fail = |m: String| AiJobView {
            status: "failed",
            code: Some("import".into()),
            message: Some(m),
            ..base.clone()
        };
        let Some(mut package) = template else {
            return fail("The result had no template.".into());
        };
        tidy_package(&mut package);
        let dir = match self.project_dir() {
            Ok(d) => d,
            Err(_) => return fail("Open a project to add the template.".into()),
        };
        let name = tweak_of.map(tweaked_name);
        match self
            .templates()
            .import_package(&dir, &package, name.as_deref(), tweak_of.is_some())
        {
            Ok(doc) => AiJobView {
                status: "succeeded",
                template_id: Some(doc.id),
                ..base.clone()
            },
            Err(e) => fail(format!("The result couldn't be added: {e}")),
        }
    }
}

#[cfg(test)]
mod tests;
