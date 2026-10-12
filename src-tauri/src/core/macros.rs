//! Legacy macros in `<project>/macros.json` = `{ "schemaVersion": 1, "macros": [...] }`. A macro is
//! now a Cue flow: on open, `core::flows` migrates the file ([`read_legacy`]) and rewrites it
//! as `{ "schemaVersion": 2, "migratedTo": "flows" }` ([`write_migrated_marker`]). Read
//! through `persist` (versioned, atomic writes, a corrupt file is set aside, a newer one is never
//! overwritten). Part of the backup config set.
//!
//! Validation ([`validate`]) checks shapes and limits only: steps may point at screens, layers or
//! sources that no longer exist (they are skipped when the macro runs).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::core::model::{Macro, MacroStep, MACROS_MAX, MACRO_STEPS_MAX, MACRO_WAIT_MAX_MS};
use crate::core::persist::Store;
use crate::core::screens::VALUE_MAX;
use crate::core::templates::{is_id, is_key};
use crate::error::AppError;

/// Longest macro name, in characters.
pub const MACRO_NAME_MAX: usize = 100;

/// `macros.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MacrosFile {
    pub macros: Vec<Macro>,
}

impl Store for MacrosFile {
    const FILE: &'static str = "macros.json";
    const VERSION: u32 = 1;
}

/// `schemaVersion` of `macros.json` once its macros became flows.
pub const MIGRATED_VERSION: u32 = 2;

/// What `macros.json` holds, for the migration to flows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy {
    /// No `macros.json`.
    None,
    /// A `schemaVersion: 1` file (possibly with no macros): to migrate.
    Macros(Vec<Macro>),
    /// Already migrated (the marker), or from a newer Kinetra: ignored, never written.
    Done,
    /// Unreadable or not the shape of a macros file: left alone.
    Unreadable,
}

/// Reads `macros.json` for the migration (nothing on disk changes).
pub fn read_legacy(dir: &std::path::Path) -> Legacy {
    use crate::core::persist::PersistError;
    match crate::core::persist::load_strict::<MacrosFile>(dir) {
        Ok(f) => Legacy::Macros(f.macros),
        Err(PersistError::Missing(_)) => Legacy::None,
        Err(PersistError::NewerVersion { .. }) => Legacy::Done,
        Err(e) => {
            tracing::warn!("macros.json: {e}; not migrated");
            Legacy::Unreadable
        }
    }
}

/// Rewrites `macros.json` as `{ "schemaVersion": 2, "migratedTo": "flows" }` (atomic).
pub fn write_migrated_marker(dir: &std::path::Path) -> std::io::Result<()> {
    crate::core::fsx::write_json_atomic(
        &crate::core::persist::path_of::<MacrosFile>(dir),
        &serde_json::json!({ "schemaVersion": MIGRATED_VERSION, "migratedTo": "flows" }),
    )
}

fn invalid(msg: impl Into<String>) -> AppError {
    AppError::Invalid(msg.into())
}

fn check_id(what: &str, id: &str) -> Result<(), AppError> {
    if is_id(id) {
        Ok(())
    } else {
        Err(invalid(format!("Invalid {what} id: {id}")))
    }
}

fn validate_step(m: &str, i: usize, step: &MacroStep) -> Result<(), AppError> {
    let at = |e: AppError| match e {
        AppError::Invalid(msg) => invalid(format!("Macro \"{m}\", step {}: {msg}", i + 1)),
        e => e,
    };
    match step {
        MacroStep::Take {
            screen_id,
            layer_id,
        }
        | MacroStep::Clear {
            screen_id,
            layer_id,
        } => {
            check_id("screen", screen_id).map_err(at)?;
            check_id("layer", layer_id).map_err(at)?;
        }
        MacroStep::ClearScreen { screen_id } => check_id("screen", screen_id).map_err(at)?,
        MacroStep::StepSource { source_id, .. } => check_id("source", source_id).map_err(at)?,
        MacroStep::SetFields {
            screen_id,
            layer_id,
            fields,
        } => {
            check_id("screen", screen_id).map_err(at)?;
            check_id("layer", layer_id).map_err(at)?;
            if fields.is_empty() {
                return Err(at(invalid("no field values to set")));
            }
            for (k, v) in fields {
                if !is_key(k) {
                    return Err(at(invalid(format!("invalid field key \"{k}\""))));
                }
                if v.len() > VALUE_MAX {
                    return Err(at(invalid(format!(
                        "field \"{k}\" is too long (at most {VALUE_MAX} bytes)"
                    ))));
                }
            }
        }
        MacroStep::Wait { ms } => {
            if *ms > MACRO_WAIT_MAX_MS {
                return Err(at(invalid(format!(
                    "a wait is at most {} seconds",
                    MACRO_WAIT_MAX_MS / 1000
                ))));
            }
        }
    }
    Ok(())
}

/// Hard checks (nothing is saved when one fails): at most `MACROS_MAX` macros, unique path-safe
/// ids, names trimmed to 1..=100 characters, at most `MACRO_STEPS_MAX` steps, path-safe target
/// ids, waits up to `MACRO_WAIT_MAX_MS`. Returns the list with names trimmed.
pub fn validate(mut macros: Vec<Macro>) -> Result<Vec<Macro>, AppError> {
    if macros.len() > MACROS_MAX {
        return Err(invalid(format!(
            "Too many macros ({}, at most {MACROS_MAX})",
            macros.len()
        )));
    }
    let mut ids = HashSet::new();
    for m in &mut macros {
        check_id("macro", &m.id)?;
        if !ids.insert(m.id.clone()) {
            return Err(invalid(format!("Duplicate macro id: {}", m.id)));
        }
        let name = m.name.trim();
        if name.is_empty() || name.chars().count() > MACRO_NAME_MAX {
            return Err(invalid(format!(
                "Macro names must be 1 to {MACRO_NAME_MAX} characters"
            )));
        }
        m.name = name.to_string();
        if m.steps.len() > MACRO_STEPS_MAX {
            return Err(invalid(format!(
                "Macro \"{}\" has too many steps ({}, at most {MACRO_STEPS_MAX})",
                m.name,
                m.steps.len()
            )));
        }
        for (i, s) in m.steps.iter().enumerate() {
            validate_step(&m.name, i, s)?;
        }
    }
    Ok(macros)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::SourceStep;
    use crate::core::persist;

    fn mac(id: &str, name: &str, steps: Vec<MacroStep>) -> Macro {
        Macro {
            id: id.into(),
            name: name.into(),
            steps,
        }
    }

    fn take() -> MacroStep {
        MacroStep::Take {
            screen_id: "s1".into(),
            layer_id: "l1".into(),
        }
    }

    #[test]
    fn validates_shapes_and_limits() {
        let ok = validate(vec![mac("m1", "  Open  ", vec![take()])]).unwrap();
        assert_eq!(ok[0].name, "Open");
        let bad: Vec<Vec<Macro>> = vec![
            vec![mac("m1", "A", vec![]), mac("m1", "B", vec![])],
            vec![mac("../x", "A", vec![])],
            vec![mac("m1", "   ", vec![])],
            vec![mac("m1", &"x".repeat(101), vec![])],
            vec![mac("m1", "A", vec![take(); MACRO_STEPS_MAX + 1])],
            vec![mac(
                "m1",
                "A",
                vec![MacroStep::Wait {
                    ms: MACRO_WAIT_MAX_MS + 1,
                }],
            )],
            vec![mac(
                "m1",
                "A",
                vec![MacroStep::ClearScreen {
                    screen_id: "a/b".into(),
                }],
            )],
            vec![mac(
                "m1",
                "A",
                vec![MacroStep::SetFields {
                    screen_id: "s".into(),
                    layer_id: "l".into(),
                    fields: [("Bad".to_string(), "x".to_string())].into(),
                }],
            )],
            (0..=MACROS_MAX)
                .map(|i| mac(&format!("m{i}"), "A", vec![]))
                .collect(),
        ];
        for (i, list) in bad.into_iter().enumerate() {
            assert!(
                matches!(validate(list), Err(AppError::Invalid(_))),
                "case {i}"
            );
        }
        assert!(validate(vec![mac(
            "m1",
            "A",
            vec![
                MacroStep::Wait {
                    ms: MACRO_WAIT_MAX_MS
                };
                MACRO_STEPS_MAX
            ]
        )])
        .is_ok());
    }

    #[test]
    fn file_round_trip_and_shape() {
        let dir = tempfile::tempdir().unwrap();
        let file = MacrosFile {
            macros: vec![mac(
                "m1",
                "Open",
                vec![
                    take(),
                    MacroStep::Wait { ms: 500 },
                    MacroStep::StepSource {
                        source_id: "src".into(),
                        step: SourceStep::By { delta: 1 },
                    },
                ],
            )],
        };
        persist::save(dir.path(), &file).unwrap();
        let text = std::fs::read_to_string(dir.path().join("macros.json")).unwrap();
        assert!(text.trim_start().starts_with("{\n  \"schemaVersion\": 1"));
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v["macros"][0]["steps"][0],
            serde_json::json!({"do": "take", "screenId": "s1", "layerId": "l1"})
        );
        assert_eq!(
            v["macros"][0]["steps"][1],
            serde_json::json!({"do": "wait", "ms": 500})
        );
        let (back, rec) = persist::load_or_recover::<MacrosFile>(dir.path()).unwrap();
        assert!(rec.is_none());
        assert_eq!(back, file);
    }
}
