//! Remote commands (`docs/CONTROL_PROTOCOL.md` §8): every check in the protocol's order (first
//! failure wins), then execution through the paths local actions use: Take / Clear / GO go
//! through `run_control` exactly like a MIDI button or the palette (Studio mode cues into
//! Preview), field edits through `set_layer_fields` (live rules, On take staging, Studio diff,
//! coalesced save). A data editor's edit is applied only while the layer isn't shown, checked
//! under the hub's lock together with the change.
//!
//! The duplicate check (step 1) and the result's timing are the connection's
//! (`core::remote::conn`); everything here is synchronous and may block briefly (the screen
//! lock): run it on the blocking pool.

use serde_json::{json, Map, Value};

use super::proto::{clip, Command, MESSAGE_MAX, VALUES_MAX};
use super::state::{Model, RemoteAsset, CUE_LIST_ID};
use crate::core::control_actions::ControlInput;
use crate::core::model::ControlTarget;
use crate::core::protocol::Fields;
use crate::core::state::AppState;
use crate::core::template_safety;
use crate::error::AppError;

/// A refusal or failure: the protocol's `code`, the message the sender sees, `details`.
#[derive(Debug, Clone, PartialEq)]
pub struct Refused {
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

fn refuse(code: &'static str, message: impl Into<String>) -> Refused {
    Refused {
        code,
        message: clip(&message.into(), MESSAGE_MAX),
        details: None,
    }
}

fn refuse_field(code: &'static str, message: impl Into<String>, field: &str) -> Refused {
    Refused {
        details: Some(json!({ "field": field })),
        ..refuse(code, message)
    }
}

/// The parsed, well-formed command.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Take { layer: String },
    Clear { layer: String },
    UpdateFields { layer: String, values: Fields },
    RunCue { list: String, cue: String },
}

impl Action {
    pub fn layer(&self) -> Option<&str> {
        match self {
            Self::Take { layer } | Self::Clear { layer } | Self::UpdateFields { layer, .. } => {
                Some(layer)
            }
            Self::RunCue { .. } => None,
        }
    }
}

/// Role as the server forwarded it (§9.2; the owner is always an operator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Viewer,
    DataEditor,
    Operator,
}

fn role_of(s: &str) -> Option<Role> {
    match s {
        "viewer" => Some(Role::Viewer),
        "data_editor" => Some(Role::DataEditor),
        "operator" | "owner" => Some(Role::Operator),
        _ => None,
    }
}

fn app_id(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    ((1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    .then(|| s.to_string())
}

/// Step 3: the action and its arguments.
pub fn parse(cmd: &Command) -> Result<Action, Refused> {
    let args = cmd.args.as_object();
    let arg = |k: &str| args.and_then(|a| a.get(k)).and_then(app_id);
    let bad = || refuse("invalid", "This command is incomplete.");
    match cmd.action.as_str() {
        "take" => Ok(Action::Take {
            layer: arg("layer").ok_or_else(bad)?,
        }),
        "clear" => Ok(Action::Clear {
            layer: arg("layer").ok_or_else(bad)?,
        }),
        "update_fields" => {
            let layer = arg("layer").ok_or_else(bad)?;
            let values = args
                .and_then(|a| a.get("values"))
                .and_then(Value::as_object)
                .filter(|v| (1..=VALUES_MAX).contains(&v.len()))
                .ok_or_else(bad)?;
            let mut out = Fields::new();
            for (k, v) in values {
                let s = v.as_str().ok_or_else(bad)?;
                out.insert(k.clone(), s.to_string());
            }
            Ok(Action::UpdateFields { layer, values: out })
        }
        "run_cue" => Ok(Action::RunCue {
            list: arg("cue_list").ok_or_else(bad)?,
            cue: arg("cue").ok_or_else(bad)?,
        }),
        _ => Err(refuse(
            "unsupported",
            "This version of Kinetra can't do that remotely.",
        )),
    }
}

/// What the checks read besides the app: the switch, the lock, the last tree sent (with its
/// history) and the assets in it.
pub struct Ctx<'a> {
    pub enabled: bool,
    pub take_locked: bool,
    pub model: &'a Model,
    pub assets: &'a [RemoteAsset],
}

/// What an accepted command did, for the activity list.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub action: Action,
    /// The layer's or cue's name as the tree has it.
    pub target: String,
    /// The screen of a layer command.
    pub screen: Option<String>,
}

/// Whether a layer is shown (on air or animating out) or cued in on Preview.
fn shown(st: &AppState, screen: &str, layer: &str) -> bool {
    st.live().visible(screen, layer) || {
        let cued = st.live().is_studio()
            && st.live().preview_view(screen).ok().is_some_and(|p| {
                p.cues
                    .iter()
                    .any(|c| c.layer_id == layer && c.visible == Some(true))
            });
        cued
    }
}

/// Steps 2–9 of §8 for one command, then executes it. Step 1 (duplicates) is the caller's.
pub fn run(st: &AppState, ctx: &Ctx<'_>, cmd: &Command) -> Result<Done, Refused> {
    // 2. Remote control still on.
    if !ctx.enabled {
        return Err(refuse(
            "disabled",
            "Remote control is off on the show machine.",
        ));
    }
    // 3. Known action, well-formed arguments.
    let action = parse(cmd)?;
    let role = role_of(&cmd.actor.role);
    let layer_path = action.layer().map(|l| format!("/layers/{l}"));
    let layer_node = layer_path.as_deref().and_then(|p| ctx.model.at(p));
    let screen = layer_node
        .and_then(|l| l["screen"].as_str())
        .map(str::to_string);
    let is_shown = |s: &Option<String>, l: &str| s.as_deref().is_some_and(|s| shown(st, s, l));

    // 4. Role.
    match (&action, role) {
        (_, Some(Role::Operator)) => {}
        (Action::UpdateFields { layer, .. }, Some(Role::DataEditor)) => {
            if is_shown(&screen, layer) {
                return Err(refuse(
                    "on_air",
                    "This graphic is on air. An operator can change it.",
                ));
            }
        }
        _ => return Err(refuse("forbidden", "Your role can't do that.")),
    }

    // 5. Lock remote take: anything that changes what's on air (an edit of an on-air layer;
    // a layer only cued in Preview isn't on air).
    let changes_air = match &action {
        Action::Take { .. } | Action::Clear { .. } | Action::RunCue { .. } => true,
        Action::UpdateFields { layer, .. } => screen
            .as_deref()
            .is_some_and(|s| st.live().visible(s, layer)),
    };
    if ctx.take_locked && changes_air {
        return Err(refuse(
            "locked",
            "Remote take is locked on the show machine.",
        ));
    }

    // 6. Target.
    let target = match &action {
        Action::RunCue { list, cue } => {
            let node = ctx
                .model
                .at(&format!("/cue_lists/{list}/cues/{cue}"))
                .ok_or_else(|| refuse("not_found", "That cue doesn't exist any more."))?;
            name_in(node)
        }
        _ => {
            let node = layer_node
                .ok_or_else(|| refuse("not_found", "That layer doesn't exist any more."))?;
            if node["kind"] == "media" {
                return Err(refuse(
                    "read_only",
                    "Videos and images can only be taken on the show machine.",
                ));
            }
            if node["remote_editable"] != Value::Bool(true) {
                return Err(refuse("read_only", "This layer can't be changed remotely."));
            }
            name_in(node)
        }
    };
    if let (Action::UpdateFields { values, .. }, Some(node)) = (&action, layer_node) {
        check_fields(ctx, node, values)?;
    }

    // 7. Stale.
    let seen = cmd.seen_seq;
    match &action {
        Action::Take { layer } | Action::Clear { layer } => {
            if ctx
                .model
                .changed_since(&format!("/layers/{layer}/on_air"), seen)
            {
                return Err(refuse(
                    "stale",
                    "Changed on the show machine. Check and try again.",
                ));
            }
        }
        Action::UpdateFields { layer, values } => {
            let stale: Vec<&String> = values
                .keys()
                .filter(|k| {
                    ctx.model
                        .changed_since(&format!("/layers/{layer}/values/{k}"), seen)
                })
                .collect();
            if !stale.is_empty() {
                return Err(Refused {
                    details: Some(json!({ "fields": stale })),
                    ..refuse("stale", "Changed on the show machine. Check and try again.")
                });
            }
        }
        Action::RunCue { list, .. } => {
            let moved = ["current", "next"].iter().any(|k| {
                ctx.model
                    .changed_since(&format!("/cue_lists/{list}/{k}"), seen)
            });
            if moved {
                return Err(refuse(
                    "stale",
                    "The cue list moved on. Check the next cue and try again.",
                ));
            }
        }
    }

    // 8–9. Validation as local, then the same path as local.
    let screen_id = screen.clone().unwrap_or_default();
    let result = match &action {
        Action::Take { layer } => press(st, &format!("live.take:{screen_id}/{layer}")),
        Action::Clear { layer } => press(st, &format!("live.clear:{screen_id}/{layer}")),
        Action::RunCue { list, cue } => {
            if list != CUE_LIST_ID {
                return Err(refuse("not_found", "That cue list doesn't exist."));
            }
            press(st, &format!("cue.go:{cue}"))
        }
        Action::UpdateFields { layer, values } => {
            if role == Some(Role::DataEditor) {
                match st.set_layer_fields_unless_shown(&screen_id, layer, values.clone()) {
                    Ok(Some(_)) => Ok(()),
                    Ok(None) => {
                        return Err(refuse(
                            "on_air",
                            "This graphic is on air. An operator can change it.",
                        ))
                    }
                    Err(e) => Err(e),
                }
            } else {
                st.set_layer_fields(&screen_id, layer, values.clone())
                    .map(drop)
            }
        }
    };
    match result {
        Ok(()) => Ok(Done {
            action,
            target,
            screen,
        }),
        Err(e) => Err(map_error(&e, &action)),
    }
}

fn name_in(node: &Value) -> String {
    node["name"].as_str().unwrap_or_default().to_string()
}

fn press(st: &AppState, id: &str) -> Result<(), AppError> {
    st.run_control(
        &ControlTarget::Command {
            command_id: id.to_string(),
        },
        ControlInput::Press,
    )
}

/// Step 6 for `update_fields`: keys exist, aren't bound, aren't clock / timer; image and video
/// values are one of the assets sent (or empty); html passes the template rules.
fn check_fields(ctx: &Ctx<'_>, layer: &Value, values: &Fields) -> Result<(), Refused> {
    let template = layer["template"].as_str().unwrap_or_default();
    let fields = ctx
        .model
        .at(&format!("/templates/{template}/fields"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let bound: Vec<&str> = layer["bound"]
        .as_array()
        .map(|b| b.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for (k, v) in values {
        let def = fields
            .iter()
            .find(|f| f["key"] == Value::String(k.clone()))
            .ok_or_else(|| refuse_field("invalid", "This graphic has no such field.", k))?;
        if bound.contains(&k.as_str()) {
            return Err(refuse_field(
                "bound",
                "This field comes from a data source on the show machine.",
                k,
            ));
        }
        match def["type"].as_str().unwrap_or_default() {
            "clock" | "timer" => {
                return Err(refuse_field(
                    "read_only",
                    "This field can't be changed remotely.",
                    k,
                ))
            }
            "image" | "video" => {
                if !v.is_empty() && !ctx.assets.iter().any(|a| a.path == *v) {
                    return Err(refuse_field(
                        "invalid",
                        "Pick a file from the show's media library.",
                        k,
                    ));
                }
            }
            "html" if !template_safety::check_html(v).is_empty() => {
                return Err(refuse_field(
                    "invalid",
                    "This text has markup Kinetra doesn't allow.",
                    k,
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// A refusal from the core path, in the protocol's codes. Messages never carry paths or
/// internals: only `invalid` and `busy` pass the core's (operator-facing) text on.
fn map_error(e: &AppError, action: &Action) -> Refused {
    match e {
        AppError::NotFound(_) => refuse("not_found", "That doesn't exist any more."),
        AppError::Invalid(m) => {
            let field = match action {
                Action::UpdateFields { values, .. } => values
                    .keys()
                    .find(|k| m.contains(&format!("\"{k}\"")))
                    .cloned(),
                _ => None,
            };
            match field {
                Some(f) => refuse_field("invalid", m.clone(), &f),
                None => refuse("invalid", m.clone()),
            }
        }
        AppError::Busy(m) => refuse("busy", m.clone()),
        AppError::NoProject => refuse("disabled", "No show is open on the show machine."),
        _ => refuse("internal", "Something went wrong on the show machine."),
    }
}

/// A plain-text line for the activity list ("Kari N. updated Name on Lower third 2").
pub fn describe(actor: &str, done: &Done, fields: &Map<String, Value>) -> String {
    let who = if actor.trim().is_empty() {
        "Someone"
    } else {
        actor.trim()
    };
    match &done.action {
        Action::Take { .. } => format!("{who} took {}", done.target),
        Action::Clear { .. } => format!("{who} cleared {}", done.target),
        Action::RunCue { .. } => format!("{who} ran {}", done.target),
        Action::UpdateFields { values, .. } => {
            let labels: Vec<String> = values
                .keys()
                .map(|k| {
                    fields
                        .get(k)
                        .and_then(Value::as_str)
                        .unwrap_or(k)
                        .to_string()
                })
                .collect();
            format!("{who} updated {} on {}", labels.join(", "), done.target)
        }
    }
}
