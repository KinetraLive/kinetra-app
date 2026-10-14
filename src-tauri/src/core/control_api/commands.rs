//! Control API commands (`docs/CONTROL_API.md` "Commands"): argument checks against the current
//! tree, then the paths MIDI, OSC and Control use (`run_control`, `set_layer_fields`,
//! `media_control`, `apply_studio_op`, `set_timer`, the UI state), so Studio mode, Live edit,
//! PANIC and Automation off apply exactly as for those controls.

use serde::Serialize;
use serde_json::Value;

use crate::core::control_actions::ControlInput;
use crate::core::flow_engine::RestRunRefusal;
use crate::core::model::{ControlTarget, MediaAction, StudioOp, TimerAction, ValueTarget};
use crate::core::protocol::{Fields, TakeTransition, TakeTransitionKind};
use crate::core::remote::proto::{VALUES_MAX, VALUE_MAX};
use crate::core::remote::state::split_local_key;
use crate::core::state::AppState;
use crate::core::template_safety;
use crate::error::AppError;

/// Why a command didn't run: a code from the protocol and a sentence for the operator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Refused {
    pub code: &'static str,
    pub message: String,
}

fn refuse(code: &'static str, message: impl Into<String>) -> Refused {
    Refused {
        code,
        message: message.into(),
    }
}

fn invalid(message: impl Into<String>) -> Refused {
    refuse("invalid", message)
}

/// Optional commands this version has (`welcome.features`).
pub const FEATURES: &[&str] = &["studio_take", "automation", "timer", "run_flow"];

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, Refused> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid(format!("Missing \"{key}\".")))
}

/// A layer key's screen, layer and node in the tree.
fn layer_of<'a>(
    tree: &'a Value,
    args: &'a Value,
) -> Result<(&'a str, &'a str, &'a Value), Refused> {
    let key = str_arg(args, "layer")?;
    let (screen, layer) =
        split_local_key(key).ok_or_else(|| invalid("\"layer\" is <screen id>.<layer id>."))?;
    let node = tree
        .pointer(&format!("/layers/{key}"))
        .ok_or_else(|| refuse("not_found", "That graphic doesn't exist."))?;
    Ok((screen, layer, node))
}

fn press(st: &AppState, id: &str) -> Result<(), Refused> {
    st.run_control(
        &ControlTarget::Command {
            command_id: id.to_string(),
        },
        ControlInput::Press,
    )
    .map_err(|e| map_error(&e))
}

/// A refusal from a core path. Only `invalid` and `busy` pass the core's (operator-facing) text
/// on; anything else is logged and answered generically.
pub fn map_error(e: &AppError) -> Refused {
    match e {
        AppError::NotFound(_) => refuse("not_found", "That doesn't exist any more."),
        AppError::Invalid(m) => invalid(m.clone()),
        AppError::Busy(m) => refuse("busy", m.clone()),
        AppError::NoProject => refuse("not_found", "No project is open."),
        e => {
            tracing::warn!("control API command failed: {e}");
            refuse("failed", "Something went wrong in Kinetra.")
        }
    }
}

/// A field value as text (strings; numbers and booleans as written).
fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// `set_fields`'s values: 1–64 keys the template has, not bound, not clock / timer; image and
/// video values from the media library (or empty); html within the template rules.
fn check_values(tree: &Value, node: &Value, values: &Value) -> Result<Fields, Refused> {
    let obj = values
        .as_object()
        .ok_or_else(|| invalid("\"values\" is an object of field → text."))?;
    if obj.is_empty() || obj.len() > VALUES_MAX {
        return Err(invalid(format!("Send 1–{VALUES_MAX} values.")));
    }
    let template = node["template"].as_str().unwrap_or_default();
    let defs = tree
        .pointer(&format!("/templates/{template}/fields"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let bound: Vec<&str> = node["bound"]
        .as_array()
        .map(|b| b.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let assets: Vec<&str> = tree["assets"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x["path"].as_str()).collect())
        .unwrap_or_default();
    let mut out = Fields::new();
    for (k, v) in obj {
        let v = value_text(v).ok_or_else(|| invalid(format!("\"{k}\" must be text.")))?;
        let def = defs
            .iter()
            .find(|f| f["key"].as_str() == Some(k))
            .ok_or_else(|| invalid(format!("This graphic has no field \"{k}\".")))?;
        if bound.contains(&k.as_str()) {
            return Err(refuse(
                "bound",
                format!("\"{k}\" comes from a data source."),
            ));
        }
        match def["type"].as_str().unwrap_or_default() {
            "clock" => return Err(refuse("read_only", format!("\"{k}\" is a clock."))),
            "timer" => {
                return Err(refuse(
                    "read_only",
                    format!("\"{k}\" is a timer: use the timer action."),
                ))
            }
            "image" | "video" if !v.is_empty() && !assets.contains(&v.as_str()) => {
                return Err(invalid(format!(
                    "\"{k}\" takes a file from the media library (/assets/<name>)."
                )))
            }
            "html" if !template_safety::check_html(&v).is_empty() => {
                return Err(invalid(format!(
                    "\"{k}\" has markup Kinetra doesn't allow."
                )))
            }
            _ => {}
        }
        out.insert(k.clone(), v);
    }
    Ok(out)
}

/// Runs one command against `tree` (the current state). Never blocks on the network; may block
/// briefly on the screen store (call from the blocking pool).
pub fn run(st: &AppState, tree: &Value, action: &str, args: &Value) -> Result<(), Refused> {
    match action {
        "take" | "clear" | "toggle" | "update" => {
            let (s, l, _) = layer_of(tree, args)?;
            press(st, &format!("live.{action}:{s}/{l}"))
        }
        "set_fields" => {
            let (s, l, node) = layer_of(tree, args)?;
            if node["kind"] == "media" {
                return Err(refuse("read_only", "A media layer has no fields."));
            }
            let take = match args.get("take") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err(invalid("\"take\" is true or false.")),
            };
            let values = check_values(tree, node, args.get("values").unwrap_or(&Value::Null))?;
            st.set_layer_fields(s, l, values)
                .map_err(|e| map_error(&e))?;
            if take {
                press(st, &format!("live.take:{s}/{l}"))?;
            }
            Ok(())
        }
        "run_cue" => {
            let cue = str_arg(args, "cue")?;
            if tree
                .pointer(&format!("/cue_lists/cues/cues/{cue}"))
                .is_none()
            {
                return Err(refuse("not_found", "That cue doesn't exist."));
            }
            press(st, &format!("cue.go:{cue}"))
        }
        "run_flow" => {
            let flow = str_arg(args, "flow")?;
            let value = match args.get("value") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    value_text(v)
                        .filter(|t| t.len() <= VALUE_MAX)
                        .ok_or_else(|| invalid("\"value\" is text (at most 16 KB)."))?,
                ),
            };
            st.run_flow_rest(flow, value).map_err(|r| match r {
                RestRunRefusal::NotFound => {
                    refuse("not_found", "No flow with an API call trigger has that id.")
                }
                RestRunRefusal::Disarmed => refuse("disarmed", "That flow is disarmed."),
                RestRunRefusal::TooOften => refuse("rate_limited", "That flow runs too often."),
                RestRunRefusal::Unavailable => {
                    refuse("failed", "Flows can't run right now. Restart Kinetra.")
                }
            })
        }
        "go_next" => {
            let next = tree["cue_lists"]["cues"]["next"]
                .as_str()
                .ok_or_else(|| refuse("not_found", "There's no next cue."))?;
            press(st, &format!("cue.go:{next}"))
        }
        "clear_screen" => {
            let screen = str_arg(args, "screen")?;
            if tree["screens"].get(screen).is_none() {
                return Err(refuse("not_found", "That screen doesn't exist."));
            }
            press(st, &format!("live.clearScreen:{screen}"))
        }
        "clear_all" => press(st, "live.clearAll"),
        "panic" => press(st, "live.panic"),
        "source_step" => {
            let source = str_arg(args, "source")?;
            if tree["sources"].get(source).is_none() {
                return Err(refuse("not_found", "That source doesn't exist."));
            }
            let dir = match str_arg(args, "dir")? {
                "next" => "next",
                "prev" => "prev",
                _ => return Err(invalid("\"dir\" is next or prev.")),
            };
            press(st, &format!("source.{dir}:{source}"))
        }
        "media" => {
            let (s, l, node) = layer_of(tree, args)?;
            if node["kind"] != "media" {
                return Err(invalid("That isn't a media layer."));
            }
            let action = match str_arg(args, "op")? {
                "play" => MediaAction::Play,
                "pause" => MediaAction::Pause,
                "stop" => MediaAction::Stop,
                "restart" => MediaAction::Restart,
                "seek" => MediaAction::Seek,
                _ => return Err(invalid("\"op\" is play, pause, stop, restart or seek.")),
            };
            let ms = match action {
                MediaAction::Seek => {
                    let sec = args
                        .get("sec")
                        .and_then(Value::as_f64)
                        .filter(|v| v.is_finite() && *v >= 0.0)
                        .ok_or_else(|| invalid("\"sec\" is a number of seconds."))?;
                    Some((sec * 1000.0).round() as u64)
                }
                _ => None,
            };
            st.live()
                .media_control(s, l, action, ms)
                .map(drop)
                .map_err(|e| map_error(&e.into()))
        }
        "set_level" => {
            let (s, l, node) = layer_of(tree, args)?;
            let target = match str_arg(args, "target")? {
                "opacity" => ValueTarget::Opacity,
                t => match t.strip_prefix("var:") {
                    Some(key) if node["kind"] == "template" => ValueTarget::Var { key: key.into() },
                    _ => return Err(invalid("\"target\" is opacity or var:<key>.")),
                },
            };
            let value = args
                .get("value")
                .and_then(Value::as_f64)
                .filter(|v| (0.0..=1.0).contains(v))
                .ok_or_else(|| invalid("\"value\" is a number from 0 to 1."))?;
            st.run_control(
                &ControlTarget::Value {
                    screen_id: s.into(),
                    layer_id: l.into(),
                    target,
                },
                ControlInput::Level(value as f32),
            )
            .map_err(|e| map_error(&e))
        }
        "studio_take" => {
            let screen = str_arg(args, "screen")?;
            if tree["screens"].get(screen).is_none() {
                return Err(refuse("not_found", "That screen doesn't exist."));
            }
            if !st.live().is_studio() {
                return Err(refuse("not_studio", "Studio mode is off."));
            }
            let transition = match args.get("transition").and_then(Value::as_str) {
                None | Some("auto") => None,
                Some("cut") => Some(TakeTransition {
                    kind: TakeTransitionKind::Cut,
                    ..TakeTransition::default()
                }),
                Some(_) => return Err(invalid("\"transition\" is cut or auto.")),
            };
            st.apply_studio_op(screen, StudioOp::Take { transition })
                .map(drop)
                .map_err(|e| map_error(&e))
        }
        "timer" => {
            let (s, l, node) = layer_of(tree, args)?;
            if node["kind"] == "media" {
                return Err(invalid("A media layer has no timers."));
            }
            let field = str_arg(args, "field")?;
            let action = match str_arg(args, "op")? {
                "start" => TimerAction::Start,
                "pause" => TimerAction::Pause,
                "reset" => TimerAction::Reset,
                _ => return Err(invalid("\"op\" is start, pause or reset.")),
            };
            st.set_timer(s, l, field, action)
                .map(drop)
                .map_err(|e| map_error(&e))
        }
        "automation" => {
            let mut ui = st.ui_state().map_err(|e| map_error(&e))?;
            ui.control.automation_off = match args.get("off") {
                Some(Value::Bool(b)) => *b,
                Some(Value::String(s)) if s == "toggle" => !ui.control.automation_off,
                _ => return Err(invalid("\"off\" is true, false or \"toggle\".")),
            };
            st.set_ui_state(ui).map(drop).map_err(|e| map_error(&e))
        }
        _ => Err(invalid(format!("Unknown action: {action}"))),
    }
}
