//! Settings export/import: one JSON file holding app prefs plus, when a project is open, its
//! settings, UI state and MIDI mappings (`controls`). Secrets are never part of it (see the key-set test).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::core::fsx;
use crate::core::model::{
    BackupReason, ControlsDoc, FieldChange, ImportPreview, Prefs, ProjectSettings, SectionDiff,
    SettingsSection, UiState, LEGACY_SETTINGS_EXPORT_KINDS, SETTINGS_EXPORT_KIND,
};
use crate::core::state::{clamp_prefs, clamp_ui_state, validate_project_settings, AppState};
use crate::error::{AppError, AppResult};

pub const SCHEMA_VERSION: u32 = 1;
/// Larger files are refused before they are read.
pub const MAX_IMPORT_BYTES: u64 = 1024 * 1024;
const NOT_SETTINGS: &str = "This is not a Kinetra settings file";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile<'a> {
    kind: &'static str,
    schema_version: u32,
    exported_at: String,
    app_version: &'static str,
    prefs: &'a Prefs,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<&'a ProjectSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ui_state: Option<&'a UiState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    controls: Option<&'a ControlsDoc>,
}

/// The open project's part of the settings: settings, UI state and MIDI mappings.
type ProjectPart<'a> = (&'a ProjectSettings, &'a UiState, &'a ControlsDoc);

/// The export document. `project` is the open project's settings, UI state and mappings, if any.
pub fn export_value(prefs: &Prefs, project: Option<ProjectPart<'_>>) -> AppResult<Value> {
    let file = ExportFile {
        kind: SETTINGS_EXPORT_KIND,
        schema_version: SCHEMA_VERSION,
        exported_at: fsx::now_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION"),
        prefs,
        project: project.map(|p| p.0),
        ui_state: project.map(|p| p.1),
        controls: project.map(|p| p.2),
    };
    Ok(serde_json::to_value(&file)?)
}

type ProjectOwned = (ProjectSettings, UiState, ControlsDoc);

/// In-memory settings of `state`: prefs plus the open project's settings, UI state and mappings.
fn current(state: &AppState) -> (Prefs, Option<ProjectOwned>) {
    let project = state.current_project().and_then(|p| {
        let ui = state.ui_state().ok()?;
        let controls = state.get_controls().ok()?;
        Some((p.settings, ui, controls))
    });
    (state.prefs(), project)
}

fn borrowed(p: &Option<ProjectOwned>) -> Option<ProjectPart<'_>> {
    p.as_ref().map(|(s, u, c)| (s, u, c))
}

/// Writes the export file to `path` atomically. Reads only memory, then writes one file.
pub fn export(state: &AppState, path: &Path) -> AppResult<()> {
    let (prefs, project) = current(state);
    let value = export_value(&prefs, borrowed(&project))?;
    fsx::write_json_atomic(path, &value)?;
    Ok(())
}

/// A parsed import file. Prefs and UI state are already clamped.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportFile {
    pub exported_at: String,
    pub app_version: String,
    pub prefs: Option<Prefs>,
    pub project: Option<ProjectSettings>,
    pub ui_state: Option<UiState>,
    pub controls: Option<ControlsDoc>,
}

impl ImportFile {
    fn has(&self, s: SettingsSection) -> bool {
        match s {
            SettingsSection::Prefs => self.prefs.is_some(),
            SettingsSection::Project => self.project.is_some(),
            SettingsSection::UiState => self.ui_state.is_some(),
            SettingsSection::Controls => self.controls.is_some(),
        }
    }
}

/// Reads and parses `path`; files over `MAX_IMPORT_BYTES` are refused without being read.
pub fn read_file(path: &Path) -> AppResult<ImportFile> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(AppError::NotFound(fsx::display_path(path)))
        }
        Err(e) => return Err(e.into()),
    };
    if !meta.is_file() || meta.len() > MAX_IMPORT_BYTES {
        return Err(AppError::Invalid(format!(
            "{NOT_SETTINGS} (it is larger than 1 MB)"
        )));
    }
    parse(&fs::read(path)?)
}

fn section<T: DeserializeOwned>(obj: &mut Map<String, Value>, key: &str) -> AppResult<Option<T>> {
    match obj.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v)
            .map(Some)
            .map_err(|e| AppError::Invalid(format!("The \"{key}\" section is not valid: {e}"))),
    }
}

/// Validates `kind` and `schemaVersion` and parses each section with the app's own types
/// (unknown fields ignored, missing fields defaulted).
pub fn parse(bytes: &[u8]) -> AppResult<ImportFile> {
    let v: Value =
        serde_json::from_slice(bytes).map_err(|_| AppError::Invalid(NOT_SETTINGS.into()))?;
    let Value::Object(mut obj) = v else {
        return Err(AppError::Invalid(NOT_SETTINGS.into()));
    };
    let kind = obj.get("kind").and_then(Value::as_str);
    let known = kind == Some(SETTINGS_EXPORT_KIND)
        || kind.is_some_and(|k| LEGACY_SETTINGS_EXPORT_KINDS.contains(&k));
    if !known {
        return Err(AppError::Invalid(NOT_SETTINGS.into()));
    }
    let version = obj
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .filter(|v| *v >= 1)
        .ok_or_else(|| AppError::Invalid(NOT_SETTINGS.to_string()))?;
    if version > u64::from(SCHEMA_VERSION) {
        return Err(AppError::NewerVersion {
            found: u32::try_from(version).unwrap_or(u32::MAX),
            supported: SCHEMA_VERSION,
        });
    }
    let text = |obj: &Map<String, Value>, k: &str| {
        obj.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Ok(ImportFile {
        exported_at: text(&obj, "exportedAt"),
        app_version: text(&obj, "appVersion"),
        prefs: section::<Prefs>(&mut obj, "prefs")?.map(clamp_prefs),
        project: section(&mut obj, "project")?,
        ui_state: section::<UiState>(&mut obj, "uiState")?.map(clamp_ui_state),
        controls: section(&mut obj, "controls")?,
    })
}

/// Leaves of `v` keyed by dotted camelCase path. Arrays and scalars are leaves.
fn flatten(v: &Value, prefix: &str, out: &mut BTreeMap<String, Value>) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(child, &key, out);
            }
        }
        other => {
            out.insert(prefix.to_string(), other.clone());
        }
    }
}

/// Every leaf that differs between `from` and `to`, sorted by key. Absent leaves are `null`.
pub fn diff<T: Serialize>(from: &T, to: &T) -> AppResult<Vec<FieldChange>> {
    let mut a = BTreeMap::new();
    let mut b = BTreeMap::new();
    flatten(&serde_json::to_value(from)?, "", &mut a);
    flatten(&serde_json::to_value(to)?, "", &mut b);
    let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
    keys.sort();
    keys.dedup();
    Ok(keys
        .into_iter()
        .filter_map(|k| {
            let x = a.get(k).cloned().unwrap_or(Value::Null);
            let y = b.get(k).cloned().unwrap_or(Value::Null);
            (x != y).then(|| FieldChange {
                key: k.clone(),
                from: x,
                to: y,
            })
        })
        .collect())
}

/// Per-section changes `file` would make. `project`/`uiState` are left out when no project is
/// open.
pub fn preview_against(
    prefs: &Prefs,
    project: Option<ProjectPart<'_>>,
    file: &ImportFile,
) -> AppResult<ImportPreview> {
    let mut sections = Vec::new();
    if let Some(p) = &file.prefs {
        sections.push(SectionDiff {
            section: SettingsSection::Prefs,
            changes: diff(prefs, p)?,
        });
    }
    if let Some((settings, ui, controls)) = project {
        if let Some(p) = &file.project {
            sections.push(SectionDiff {
                section: SettingsSection::Project,
                changes: diff(settings, p)?,
            });
        }
        if let Some(u) = &file.ui_state {
            sections.push(SectionDiff {
                section: SettingsSection::UiState,
                changes: diff(ui, u)?,
            });
        }
        if let Some(c) = &file.controls {
            sections.push(SectionDiff {
                section: SettingsSection::Controls,
                changes: diff(controls, c)?,
            });
        }
    }
    Ok(ImportPreview {
        exported_at: file.exported_at.clone(),
        app_version: file.app_version.clone(),
        sections,
    })
}

pub fn preview(state: &AppState, file: &ImportFile) -> AppResult<ImportPreview> {
    let (prefs, project) = current(state);
    preview_against(&prefs, borrowed(&project), file)
}

/// Applies the chosen `sections` of `file`. Everything is validated before anything changes;
/// a `pre-import` snapshot is taken when a project is open; the result is flushed to disk.
pub fn apply(state: &AppState, file: &ImportFile, sections: &[SettingsSection]) -> AppResult<()> {
    state.ensure_not_moving()?;
    if sections.is_empty() {
        return Err(AppError::Invalid(
            "Choose at least one section to import".into(),
        ));
    }
    let project_open = state.current_project().is_some();
    for s in sections {
        if !file.has(*s) {
            return Err(AppError::Invalid(format!(
                "The settings file has no {} section",
                section_name(*s)
            )));
        }
        if *s != SettingsSection::Prefs && !project_open {
            return Err(AppError::NoProject);
        }
    }
    let want = |s: SettingsSection| sections.contains(&s);
    let prefs = file.prefs.clone().filter(|_| want(SettingsSection::Prefs));
    let project = file
        .project
        .clone()
        .filter(|_| want(SettingsSection::Project));
    let ui = file
        .ui_state
        .clone()
        .filter(|_| want(SettingsSection::UiState));
    let controls = file
        .controls
        .clone()
        .filter(|_| want(SettingsSection::Controls));
    if let Some(p) = &project {
        validate_project_settings(p)?;
    }
    if let Some(c) = &controls {
        crate::core::controls::validate(c)?;
    }

    if project_open {
        state.create_snapshot(BackupReason::PreImport)?;
    }
    if let Some(p) = prefs {
        state.set_prefs(p);
    }
    if let Some(p) = project {
        state.set_project_settings(p)?;
    }
    if let Some(u) = ui {
        state.set_ui_state(u)?;
    }
    if let Some(c) = controls {
        state.save_controls(c)?;
    }
    state.flush_all();
    Ok(())
}

fn section_name(s: SettingsSection) -> &'static str {
    match s {
        SettingsSection::Prefs => "preferences",
        SettingsSection::Project => "project",
        SettingsSection::UiState => "UI state",
        SettingsSection::Controls => "MIDI mappings",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::backup;
    use crate::core::events::NullSink;
    use crate::core::model::{LiveEdit, Theme};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    fn state(tmp: &Path) -> AppState {
        AppState::load_with_debounce(
            tmp.join("config"),
            Arc::new(NullSink),
            Duration::from_secs(60),
        )
        .unwrap()
    }

    fn customized(st: &AppState, tmp: &Path) -> PathBuf {
        let p = st.create_project(tmp, "Show").unwrap();
        let mut prefs = Prefs {
            theme: Theme::Dark,
            ..Default::default()
        };
        prefs.backup.interval_minutes = 30;
        st.set_prefs(prefs);
        st.set_project_settings(ProjectSettings {
            port: 8123,
            lan_access: true,
            ..Default::default()
        })
        .unwrap();
        let mut ui = UiState {
            sidebar_pinned: true,
            ..Default::default()
        };
        ui.control.live_edit = LiveEdit::Immediate;
        ui.control.hotkeys.insert("main/layer1".into(), "F1".into());
        ui.control.hidden_screen_ids = vec!["a".into()];
        st.set_ui_state(ui).unwrap();
        PathBuf::from(p.path)
    }

    type Snapshot = (Prefs, ProjectSettings, UiState);

    fn snapshot(st: &AppState) -> Snapshot {
        (
            st.prefs(),
            st.current_project().unwrap().settings,
            st.ui_state().unwrap(),
        )
    }

    fn reset_all(st: &AppState) {
        st.set_prefs(Prefs::default());
        st.set_project_settings(ProjectSettings::default()).unwrap();
        st.set_ui_state(UiState::default()).unwrap();
    }

    const ALL: [SettingsSection; 4] = [
        SettingsSection::Prefs,
        SettingsSection::Project,
        SettingsSection::UiState,
        SettingsSection::Controls,
    ];

    #[test]
    fn export_import_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        customized(&st, tmp.path());
        let before = snapshot(&st);
        let file = tmp.path().join("settings.json");
        export(&st, &file).unwrap();

        reset_all(&st);
        assert_ne!(snapshot(&st), before);
        let parsed = read_file(&file).unwrap();
        apply(&st, &parsed, &ALL).unwrap();
        assert_eq!(snapshot(&st), before);
        // A second preview shows no changes.
        let pv = preview(&st, &parsed).unwrap();
        assert_eq!(pv.sections.len(), 4);
        assert!(pv.sections.iter().all(|s| s.changes.is_empty()));
    }

    #[test]
    fn export_has_exact_key_set() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let keys = |st: &AppState| {
            let file = tmp.path().join("out.json");
            export(st, &file).unwrap();
            let v: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
            let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
            k.sort();
            (k, v)
        };
        let (k, v) = keys(&st);
        assert_eq!(
            k,
            ["appVersion", "exportedAt", "kind", "prefs", "schemaVersion"]
        );
        assert_eq!(v["kind"], SETTINGS_EXPORT_KIND);
        assert_eq!(v["schemaVersion"], 1);
        customized(&st, tmp.path());
        let (k, v) = keys(&st);
        assert_eq!(
            k,
            [
                "appVersion",
                "controls",
                "exportedAt",
                "kind",
                "prefs",
                "project",
                "schemaVersion",
                "uiState"
            ]
        );
        let mut pk: Vec<String> = v["prefs"].as_object().unwrap().keys().cloned().collect();
        pk.sort();
        assert_eq!(pk, ["backup", "keybindings", "theme", "uiMode"]);
    }

    #[test]
    fn rejects_bad_files() {
        let tmp = tempfile::tempdir().unwrap();
        let wrong = br#"{"kind":"something-else","schemaVersion":1,"prefs":{}}"#;
        assert!(matches!(parse(wrong), Err(AppError::Invalid(_))));
        assert!(matches!(
            parse(br#"{"schemaVersion":1}"#),
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(parse(b"not json"), Err(AppError::Invalid(_))));
        let newer = br#"{"kind":"kinetra/settings","schemaVersion":2,"prefs":{}}"#;
        assert!(matches!(
            parse(newer),
            Err(AppError::NewerVersion {
                found: 2,
                supported: 1
            })
        ));
        let bad_section =
            br#"{"kind":"kinetra/settings","schemaVersion":1,"prefs":{"theme":"purple"}}"#;
        assert!(matches!(parse(bad_section), Err(AppError::Invalid(_))));

        let big = tmp.path().join("big.json");
        let mut text = String::from(r#"{"kind":"kinetra/settings","schemaVersion":1,"pad":""#);
        text.push_str(&"x".repeat(MAX_IMPORT_BYTES as usize));
        text.push_str("\"}");
        fs::write(&big, text).unwrap();
        assert!(matches!(read_file(&big), Err(AppError::Invalid(_))));
        assert!(matches!(
            read_file(&tmp.path().join("missing.json")),
            Err(AppError::NotFound(_))
        ));
    }

    #[test]
    fn parse_defaults_missing_and_ignores_unknown_fields() {
        let text = br#"{"kind":"kinetra/settings","schemaVersion":1,
            "prefs":{"theme":"light","futureThing":1,"backup":{"intervalMinutes":9999}}}"#;
        let f = parse(text).unwrap();
        let p = f.prefs.unwrap();
        assert_eq!(p.theme, Theme::Light);
        assert_eq!(p.backup.interval_minutes, 240);
        assert_eq!(p.backup.retention, 10);
        assert!(f.project.is_none() && f.ui_state.is_none());
    }

    #[test]
    fn preview_lists_changed_leaves() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        customized(&st, tmp.path());
        let file = tmp.path().join("settings.json");
        export(&st, &file).unwrap();
        let parsed = read_file(&file).unwrap();
        reset_all(&st);

        let pv = preview(&st, &parsed).unwrap();
        let keys = |s: SettingsSection| -> Vec<String> {
            pv.sections
                .iter()
                .find(|d| d.section == s)
                .unwrap()
                .changes
                .iter()
                .map(|c| c.key.clone())
                .collect()
        };
        assert_eq!(
            keys(SettingsSection::Prefs),
            ["backup.intervalMinutes", "theme"]
        );
        assert_eq!(keys(SettingsSection::Project), ["lanAccess", "port"]);
        assert_eq!(
            keys(SettingsSection::UiState),
            [
                "control.hiddenScreenIds",
                "control.hotkeys.main/layer1",
                "control.liveEdit",
                "sidebarPinned"
            ]
        );
        let ui = &pv.sections[2].changes;
        assert_eq!(ui[1].from, Value::Null);
        assert_eq!(ui[1].to, "F1");
        assert_eq!(ui[0].to, serde_json::json!(["a"]));
        let prefs = &pv.sections[0].changes;
        assert_eq!(prefs[0].from, 15);
        assert_eq!(prefs[0].to, 30);

        // No project open: project/uiState are left out.
        st.close_project();
        let pv = preview(&st, &parsed).unwrap();
        assert_eq!(pv.sections.len(), 1);
        assert_eq!(pv.sections[0].section, SettingsSection::Prefs);
    }

    #[test]
    fn bad_project_section_fails_before_any_change() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let dir = customized(&st, tmp.path());
        let text = br#"{"kind":"kinetra/settings","schemaVersion":1,
            "prefs":{"theme":"light"},"project":{"port":80}}"#;
        let f = parse(text).unwrap();
        let before = snapshot(&st);
        let n_backups = backup::list(&dir).unwrap().len();
        let err = apply(&st, &f, &[SettingsSection::Prefs, SettingsSection::Project]).unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
        assert_eq!(snapshot(&st), before);
        assert_eq!(
            backup::list(&dir)
                .unwrap()
                .iter()
                .filter(|b| b.reason == BackupReason::PreImport)
                .count(),
            0
        );
        assert!(backup::list(&dir).unwrap().len() <= n_backups + 1);
    }

    #[test]
    fn partial_apply_and_section_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let st = state(tmp.path());
        let dir = customized(&st, tmp.path());
        let file = tmp.path().join("settings.json");
        export(&st, &file).unwrap();
        let parsed = read_file(&file).unwrap();
        reset_all(&st);

        apply(&st, &parsed, &[SettingsSection::UiState]).unwrap();
        assert_eq!(st.prefs(), Prefs::default());
        assert_eq!(
            st.current_project().unwrap().settings,
            ProjectSettings::default()
        );
        assert!(st.ui_state().unwrap().sidebar_pinned);
        // Flushed to disk.
        let saved: UiState = crate::core::persist::load_strict(&dir).unwrap();
        assert!(saved.sidebar_pinned);
        assert!(backup::list(&dir)
            .unwrap()
            .iter()
            .any(|b| b.reason == BackupReason::PreImport));

        // Section not in the file.
        let only_prefs =
            parse(br#"{"kind":"kinetra/settings","schemaVersion":1,"prefs":{}}"#).unwrap();
        assert!(matches!(
            apply(&st, &only_prefs, &[SettingsSection::Project]),
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            apply(&st, &parsed, &[]),
            Err(AppError::Invalid(_))
        ));
        // Project sections with no project open.
        st.close_project();
        assert!(matches!(
            apply(&st, &parsed, &[SettingsSection::UiState]),
            Err(AppError::NoProject)
        ));
        apply(&st, &parsed, &[SettingsSection::Prefs]).unwrap();
        assert_eq!(st.prefs().theme, Theme::Dark);
    }

    #[test]
    fn exports_from_before_the_rename_still_import() {
        let file = parse(
            br#"{"kind":"broadcast-app/settings","schemaVersion":1,"prefs":{"theme":"dark"}}"#,
        )
        .unwrap();
        assert!(file.prefs.is_some());
        assert!(parse(br#"{"kind":"someone-else/settings","schemaVersion":1}"#).is_err());
    }
}
