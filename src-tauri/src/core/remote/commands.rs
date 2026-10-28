//! Remote commands (`docs/CONTROL_PROTOCOL.md` §8; protocol 2 §5): every check in the protocol's
//! order (first failure wins; step 4 is the actor's permissions, [`super::perms`]), then
//! execution through the paths local actions use: Take / Clear / GO go through `run_control`
//! exactly like a MIDI button or the palette (Studio mode cues into Preview), field edits through
//! `set_layer_fields` (live rules, On take staging, Studio diff, coalesced save). An edit by
//! someone without `edit_live_fields` is applied only while the layer isn't shown, checked under
//! the hub's lock together with the change. Protocol 2 adds timers (`set_timer`), variants
//! (`set_layer_variant`), rows (`step_source`) and Studio cue / take (Control's own Studio
//! paths), each only when the server announced its feature.
//!
//! The duplicate check (step 1) and the result's timing are the connection's
//! (`core::remote::conn`); everything here is synchronous and may block briefly (the screen
//! lock): run it on the blocking pool.

use serde_json::{json, Map, Value};

use super::perms::{
    BUTTON_PANEL, EDIT_FIELDS, EDIT_LIVE_FIELDS, ROWS, RUN_CUES, STUDIO, TAKE_CLEAR, TIMERS,
    VARIANTS,
};
use super::proto::{clip, Command, MESSAGE_MAX, VALUES_MAX};
use super::state::{local_layer_id, row_ids, Model, Parts, RemoteAsset, CUE_LIST_ID};
use crate::core::control_actions::ControlInput;
use crate::core::model::{ControlTarget, SourceStep, StudioOp, TimerAction};
use crate::core::protocol::{Fields, TakeTransition, TakeTransitionKind, VariantTransition};
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

const STALE: &str = "Changed on the show machine. Check and try again.";

/// `timer`'s `op` (protocol 2 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerOp {
    Start,
    Pause,
    Reset,
}

/// The parsed, well-formed command.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Take {
        layer: String,
    },
    Clear {
        layer: String,
    },
    UpdateFields {
        layer: String,
        values: Fields,
    },
    RunCue {
        list: String,
        cue: String,
    },
    Timer {
        layer: String,
        field: String,
        op: TimerOp,
    },
    SetVariant {
        layer: String,
        variant: String,
    },
    SelectRow {
        source: String,
        row: String,
    },
    StepRow {
        source: String,
        delta: i32,
    },
    StudioCue {
        screen: String,
        layer: String,
    },
    StudioUncue {
        screen: String,
        layer: String,
    },
    StudioTake {
        screen: String,
        cut: bool,
    },
    PressButton {
        button: String,
    },
}

impl Action {
    pub fn layer(&self) -> Option<&str> {
        match self {
            Self::Take { layer }
            | Self::Clear { layer }
            | Self::UpdateFields { layer, .. }
            | Self::Timer { layer, .. }
            | Self::SetVariant { layer, .. }
            | Self::StudioCue { layer, .. }
            | Self::StudioUncue { layer, .. } => Some(layer),
            Self::RunCue { .. }
            | Self::SelectRow { .. }
            | Self::StepRow { .. }
            | Self::StudioTake { .. }
            | Self::PressButton { .. } => None,
        }
    }

    /// The `welcome.features` entry a protocol 2 action needs (protocol 1 actions: none).
    fn feature(&self) -> Option<&'static str> {
        match self {
            Self::Take { .. }
            | Self::Clear { .. }
            | Self::UpdateFields { .. }
            | Self::RunCue { .. } => None,
            Self::Timer { .. } => Some("timers"),
            Self::SetVariant { .. } => Some("variants"),
            Self::SelectRow { .. } | Self::StepRow { .. } => Some("rows"),
            Self::StudioCue { .. } | Self::StudioUncue { .. } | Self::StudioTake { .. } => {
                Some("studio")
            }
            Self::PressButton { .. } => Some("panel"),
        }
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
    let text = |k: &str| args.and_then(|a| a.get(k)).and_then(Value::as_str);
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
        "timer" => Ok(Action::Timer {
            layer: arg("layer").ok_or_else(bad)?,
            field: arg("field").ok_or_else(bad)?,
            op: match text("op") {
                Some("start") => TimerOp::Start,
                Some("pause") => TimerOp::Pause,
                Some("reset") => TimerOp::Reset,
                _ => return Err(bad()),
            },
        }),
        "set_variant" => Ok(Action::SetVariant {
            layer: arg("layer").ok_or_else(bad)?,
            variant: arg("variant").ok_or_else(bad)?,
        }),
        "select_row" => Ok(Action::SelectRow {
            source: arg("source").ok_or_else(bad)?,
            row: arg("row").ok_or_else(bad)?,
        }),
        "step_row" => Ok(Action::StepRow {
            source: arg("source").ok_or_else(bad)?,
            delta: match args.and_then(|a| a.get("delta")).and_then(Value::as_i64) {
                Some(1) => 1,
                Some(-1) => -1,
                _ => return Err(bad()),
            },
        }),
        "studio_cue" | "studio_uncue" => {
            let screen = arg("screen").ok_or_else(bad)?;
            let layer = arg("layer").ok_or_else(bad)?;
            Ok(if cmd.action == "studio_cue" {
                Action::StudioCue { screen, layer }
            } else {
                Action::StudioUncue { screen, layer }
            })
        }
        "studio_take" => Ok(Action::StudioTake {
            screen: arg("screen").ok_or_else(bad)?,
            cut: match text("mode") {
                Some("take") => false,
                Some("cut") => true,
                _ => return Err(bad()),
            },
        }),
        "press_button" => Ok(Action::PressButton {
            button: arg("button").ok_or_else(bad)?,
        }),
        _ => Err(unsupported()),
    }
}

fn unsupported() -> Refused {
    refuse(
        "unsupported",
        "This version of Kinetra can't do that remotely.",
    )
}

/// What the checks read besides the app: the switch, the lock, the last tree sent (with its
/// history), the assets in it and the protocol 2 features the server announced.
pub struct Ctx<'a> {
    pub enabled: bool,
    /// The protocol the server picked: a missing permission is `permission` in protocol 2,
    /// `forbidden` in protocol 1.
    pub protocol: u32,
    pub take_locked: bool,
    pub model: &'a Model,
    pub assets: &'a [RemoteAsset],
    pub parts: Parts,
}

impl Ctx<'_> {
    fn announced(&self, feature: &str) -> bool {
        self.protocol >= 2
            && match feature {
                "timers" => self.parts.timers,
                "variants" => self.parts.variants,
                "rows" => self.parts.rows,
                "studio" => self.parts.studio,
                "panel" => self.parts.panel,
                _ => false,
            }
    }
}

/// What an accepted command did, for the activity list.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub action: Action,
    /// The layer's, cue's, source's or screen's name as the tree has it.
    pub target: String,
    /// The screen of a layer command.
    pub screen: Option<String>,
    /// `set_variant`: the variant's name.
    pub detail: Option<String>,
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

/// Whether a layer of the tree (by key) is on air.
fn layer_on_air(st: &AppState, key: &str, node: &Value) -> bool {
    node["screen"]
        .as_str()
        .is_some_and(|s| st.live().visible(s, local_layer_id(key, s)))
}

/// Whether a layer fed by `source` (`layers.*.sources`) is on air.
fn source_on_air(st: &AppState, ctx: &Ctx<'_>, source: &str) -> bool {
    ctx.model
        .at("/layers")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .any(|(k, l)| {
            l["sources"]
                .as_array()
                .is_some_and(|s| s.iter().any(|s| s == source))
                && layer_on_air(st, k, l)
        })
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
    // 3. Known action, well-formed arguments; a protocol 2 action only with its feature.
    let action = parse(cmd)?;
    if action.feature().is_some_and(|f| !ctx.announced(f)) {
        return Err(unsupported());
    }
    let perms = cmd.actor.perms();
    let layer_path = action.layer().map(|l| format!("/layers/{l}"));
    let layer_node = layer_path.as_deref().and_then(|p| ctx.model.at(p));
    let screen = layer_node
        .and_then(|l| l["screen"].as_str())
        .map(str::to_string);
    let is_shown = |s: &Option<String>, l: &str| {
        s.as_deref()
            .is_some_and(|s| shown(st, s, local_layer_id(l, s)))
    };
    let layer_live = || {
        screen
            .as_deref()
            .zip(action.layer())
            .is_some_and(|(s, l)| st.live().visible(s, local_layer_id(l, s)))
    };

    // 4. Permission (protocol 2 §2; a protocol 1 role maps to the same set).
    let may_edit_live = perms.has(EDIT_LIVE_FIELDS);
    let needed = match &action {
        Action::Take { .. } | Action::Clear { .. } => TAKE_CLEAR,
        Action::RunCue { .. } => RUN_CUES,
        Action::UpdateFields { .. } => EDIT_FIELDS,
        Action::Timer { .. } => TIMERS,
        Action::SetVariant { .. } => VARIANTS,
        Action::SelectRow { .. } | Action::StepRow { .. } => ROWS,
        Action::StudioCue { .. } | Action::StudioUncue { .. } | Action::StudioTake { .. } => STUDIO,
        // The owner pre-approved the button's actions: only this one is checked.
        Action::PressButton { .. } => BUTTON_PANEL,
    };
    if !perms.has(needed) {
        let code = if ctx.protocol >= 2 {
            "permission"
        } else {
            "forbidden"
        };
        return Err(refuse(code, "You don't have permission to do that."));
    }
    if let Action::UpdateFields { layer, .. } = &action {
        if !may_edit_live && is_shown(&screen, layer) {
            return Err(on_air());
        }
    }

    // 5. Lock remote take: anything that changes what's on air (an edit of an on-air layer;
    // a layer only cued in Preview isn't on air; cueing changes only Preview).
    let changes_air = match &action {
        Action::Take { .. }
        | Action::Clear { .. }
        | Action::RunCue { .. }
        | Action::StudioTake { .. } => true,
        Action::UpdateFields { .. } | Action::Timer { .. } | Action::SetVariant { .. } => {
            layer_live()
        }
        Action::SelectRow { source, .. } | Action::StepRow { source, .. } => {
            source_on_air(st, ctx, source)
        }
        Action::StudioCue { .. } | Action::StudioUncue { .. } => false,
        // The app's own definition decides (protocol 2 §4.3 `on_air`).
        Action::PressButton { button } => st
            .panel_doc()
            .ok()
            .and_then(|d| d.buttons.iter().find(|b| b.id == *button).cloned())
            .is_some_and(|b| st.panel_button_on_air(&b)),
    };
    if ctx.take_locked && changes_air {
        return Err(refuse(
            "locked",
            "Remote take is locked on the show machine.",
        ));
    }

    // 6. Target.
    let mut detail = None;
    let target = match &action {
        Action::RunCue { list, cue } => {
            let node = ctx
                .model
                .at(&format!("/cue_lists/{list}/cues/{cue}"))
                .ok_or_else(|| refuse("not_found", "That cue doesn't exist any more."))?;
            name_in(node)
        }
        Action::SelectRow { source, .. } | Action::StepRow { source, .. } => {
            let node = ctx
                .model
                .at(&format!("/sources/{source}"))
                .ok_or_else(|| refuse("not_found", "That data source isn't available remotely."))?;
            name_in(node)
        }
        Action::StudioTake { screen, .. } => {
            let node = ctx
                .model
                .at(&format!("/screens/{screen}"))
                .ok_or_else(|| refuse("not_found", "That screen doesn't exist any more."))?;
            name_in(node)
        }
        Action::PressButton { button } => st
            .panel_doc()
            .ok()
            .and_then(|d| {
                d.buttons
                    .iter()
                    .find(|b| b.id == *button)
                    .map(|b| b.label.clone())
            })
            .ok_or_else(|| refuse("not_found", "That button doesn't exist any more."))?,
        _ => {
            let node = layer_node
                .ok_or_else(|| refuse("not_found", "That layer doesn't exist any more."))?;
            if let Action::StudioCue { screen: s, .. } | Action::StudioUncue { screen: s, .. } =
                &action
            {
                if node["screen"] != Value::String(s.clone()) {
                    return Err(refuse("not_found", "That layer doesn't exist any more."));
                }
            }
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
    match (&action, layer_node) {
        (Action::UpdateFields { values, .. }, Some(node)) => check_fields(ctx, node, values)?,
        (Action::Timer { field, .. }, Some(node)) => check_timer(ctx, node, field)?,
        (Action::SetVariant { variant, .. }, Some(node)) => {
            let template = node["template"].as_str().unwrap_or_default();
            let v = ctx
                .model
                .at(&format!("/templates/{template}/variants"))
                .and_then(Value::as_array)
                .and_then(|vs| {
                    vs.iter()
                        .find(|v| v["id"] == Value::String(variant.clone()))
                })
                .ok_or_else(|| refuse("not_found", "That look doesn't exist any more."))?;
            detail = Some(name_in(v));
        }
        _ => {}
    }

    // 7. Stale.
    let seen = cmd.seen_seq;
    let changed = |p: String| ctx.model.changed_since(&p, seen);
    let stale = match &action {
        Action::Take { layer } | Action::Clear { layer } => {
            changed(format!("/layers/{layer}/on_air"))
        }
        Action::UpdateFields { layer, values } => {
            let stale: Vec<&String> = values
                .keys()
                .filter(|k| changed(format!("/layers/{layer}/values/{k}")))
                .collect();
            if !stale.is_empty() {
                return Err(Refused {
                    details: Some(json!({ "fields": stale })),
                    ..refuse("stale", STALE)
                });
            }
            false
        }
        Action::RunCue { list, .. } => {
            let moved = ["current", "next"]
                .iter()
                .any(|k| changed(format!("/cue_lists/{list}/{k}")));
            if moved {
                return Err(refuse(
                    "stale",
                    "The cue list moved on. Check the next cue and try again.",
                ));
            }
            false
        }
        Action::Timer { layer, field, .. } => changed(format!("/layers/{layer}/values/{field}")),
        Action::SetVariant { layer, .. } => ["variant", "variant_next"]
            .iter()
            .any(|k| changed(format!("/layers/{layer}/{k}"))),
        Action::SelectRow { source, .. } | Action::StepRow { source, .. } => {
            changed(format!("/sources/{source}/current"))
        }
        Action::StudioCue { screen, .. } | Action::StudioUncue { screen, .. } => {
            changed(format!("/preview/{screen}"))
        }
        Action::StudioTake { screen, .. } => {
            changed(format!("/preview/{screen}"))
                || ctx
                    .model
                    .at(&format!("/screens/{screen}/layers"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .any(|l| changed(format!("/layers/{l}/on_air")))
        }
        // Never stale (protocol 2 §5).
        Action::PressButton { .. } => false,
    };
    if stale {
        return Err(refuse("stale", STALE));
    }

    // 8–9. Validation as local, then the same path as local.
    let studio_action = matches!(
        action,
        Action::StudioCue { .. } | Action::StudioUncue { .. } | Action::StudioTake { .. }
    );
    if studio_action && !st.live().is_studio() {
        return Err(refuse("busy", "Studio mode is off on the show machine."));
    }
    let screen_id = match &action {
        Action::StudioTake { screen, .. } => screen.clone(),
        _ => screen.clone().unwrap_or_default(),
    };
    // The screen's own layer id (a tree key may be screen-qualified: state::layer_key).
    let local = |key: &str| local_layer_id(key, &screen_id).to_string();
    let result = match &action {
        Action::Take { layer } => press(st, &format!("live.take:{screen_id}/{}", local(layer))),
        Action::Clear { layer } => press(st, &format!("live.clear:{screen_id}/{}", local(layer))),
        Action::RunCue { list, cue } => {
            if list != CUE_LIST_ID {
                return Err(refuse("not_found", "That cue list doesn't exist."));
            }
            press(st, &format!("cue.go:{cue}"))
        }
        Action::UpdateFields { layer, values } => {
            if !may_edit_live {
                match st.set_layer_fields_unless_shown(&screen_id, &local(layer), values.clone()) {
                    Ok(Some(_)) => Ok(()),
                    Ok(None) => return Err(on_air()),
                    Err(e) => Err(e),
                }
            } else {
                st.set_layer_fields(&screen_id, &local(layer), values.clone())
                    .map(drop)
            }
        }
        Action::Timer { layer, field, op } => {
            let a = match op {
                TimerOp::Start => TimerAction::Start,
                TimerOp::Pause => TimerAction::Pause,
                TimerOp::Reset => TimerAction::Reset,
            };
            st.set_timer(&screen_id, &local(layer), field, a).map(drop)
        }
        Action::SetVariant { layer, variant } => st
            .set_layer_variant(
                &screen_id,
                &local(layer),
                variant,
                VariantTransition::default(),
            )
            .map(drop),
        Action::SelectRow { source, row } => {
            // The row as the app has it now: one edited since the browser saw it is gone.
            let index = st.sources().doc(source).and_then(|d| {
                let t = d.meta.kind.table()?;
                row_ids(t.columns, t.rows).iter().position(|r| r == row)
            });
            let Some(index) = index.and_then(|i| u32::try_from(i).ok()) else {
                return Err(refuse("not_found", "That row changed on the show machine."));
            };
            st.step_source(source, SourceStep::Index { index })
                .map(drop)
        }
        Action::StepRow { source, delta } => st
            .step_source(source, SourceStep::By { delta: *delta })
            .map(drop),
        Action::StudioCue { layer, .. } => st.studio_cue(&screen_id, &local(layer), true),
        Action::StudioUncue { layer, .. } => st.studio_cue(&screen_id, &local(layer), false),
        Action::StudioTake { cut, .. } => {
            let transition = cut.then(|| TakeTransition {
                kind: TakeTransitionKind::Cut,
                ..TakeTransition::default()
            });
            st.apply_studio_op(&screen_id, StudioOp::Take { transition })
                .map(drop)
        }
        Action::PressButton { button } => {
            return match st.press_panel_button(button) {
                Ok(()) => Ok(Done {
                    action,
                    target,
                    screen: None,
                    detail,
                }),
                Err(f) => Err(Refused {
                    details: (f.steps > 0).then(|| json!({"step": f.step, "steps": f.steps})),
                    ..refuse(f.code, f.message)
                }),
            };
        }
    };
    let screen = match &action {
        Action::StudioTake { screen, .. } => Some(screen.clone()),
        _ => screen,
    };
    match result {
        Ok(()) => Ok(Done {
            action,
            target,
            screen,
            detail,
        }),
        Err(e) => Err(map_error(&e, &action)),
    }
}

/// An edit of a layer on air without `edit_live_fields`.
fn on_air() -> Refused {
    refuse(
        "on_air",
        "This graphic is on air. Someone allowed to edit live graphics can change it.",
    )
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

/// The template's field definitions of a layer node.
fn fields_of(ctx: &Ctx<'_>, layer: &Value) -> Vec<Value> {
    let template = layer["template"].as_str().unwrap_or_default();
    ctx.model
        .at(&format!("/templates/{template}/fields"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn is_bound(layer: &Value, key: &str) -> bool {
    layer["bound"]
        .as_array()
        .is_some_and(|b| b.iter().any(|k| k == key))
}

/// Step 6 for `timer`: a timer field of the layer's template, not bound.
fn check_timer(ctx: &Ctx<'_>, layer: &Value, key: &str) -> Result<(), Refused> {
    let fields = fields_of(ctx, layer);
    if !fields
        .iter()
        .any(|f| f["key"] == key && f["type"] == "timer")
    {
        return Err(refuse_field(
            "invalid",
            "This graphic has no such timer.",
            key,
        ));
    }
    if is_bound(layer, key) {
        return Err(refuse_field(
            "bound",
            "This field comes from a data source on the show machine.",
            key,
        ));
    }
    Ok(())
}

/// Step 6 for `update_fields`: keys exist, aren't bound, aren't clock / timer; image and video
/// values are one of the assets sent (or empty); html passes the template rules.
fn check_fields(ctx: &Ctx<'_>, layer: &Value, values: &Fields) -> Result<(), Refused> {
    let fields = fields_of(ctx, layer);
    for (k, v) in values {
        let def = fields
            .iter()
            .find(|f| f["key"] == Value::String(k.clone()))
            .ok_or_else(|| refuse_field("invalid", "This graphic has no such field.", k))?;
        if is_bound(layer, k) {
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
                Action::Timer { field, .. } => Some(field.clone()),
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

/// A plain-text line for the activity list ("Kari N. updated Name on Lower third 2"). Row
/// labels are show content and never appear here.
pub fn describe(actor: &str, done: &Done, fields: &Map<String, Value>) -> String {
    let who = if actor.trim().is_empty() {
        "Someone"
    } else {
        actor.trim()
    };
    let label = |k: &str| {
        fields
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or(k)
            .to_string()
    };
    let t = &done.target;
    match &done.action {
        Action::Take { .. } => format!("{who} took {t}"),
        Action::Clear { .. } => format!("{who} cleared {t}"),
        Action::RunCue { .. } => format!("{who} ran {t}"),
        Action::UpdateFields { values, .. } => {
            let labels: Vec<String> = values.keys().map(|k| label(k)).collect();
            format!("{who} updated {} on {t}", labels.join(", "))
        }
        Action::Timer { field, op, .. } => {
            let verb = match op {
                TimerOp::Start => "started",
                TimerOp::Pause => "paused",
                TimerOp::Reset => "reset",
            };
            format!("{who} {verb} {} on {t}", label(field))
        }
        Action::SetVariant { .. } => match &done.detail {
            Some(v) => format!("{who} switched {t} to {v}"),
            None => format!("{who} switched the look of {t}"),
        },
        Action::SelectRow { .. } => format!("{who} picked a row of {t}"),
        Action::StepRow { delta, .. } if *delta < 0 => {
            format!("{who} went to the previous row of {t}")
        }
        Action::StepRow { .. } => format!("{who} went to the next row of {t}"),
        Action::StudioCue { .. } => format!("{who} cued {t} in Preview"),
        Action::StudioUncue { .. } => format!("{who} cued {t} out of Preview"),
        Action::StudioTake { cut: true, .. } => format!("{who} cut Preview to Program on {t}"),
        Action::StudioTake { .. } => format!("{who} took Preview to Program on {t}"),
        Action::PressButton { .. } => format!("{who} pressed {t}"),
    }
}
