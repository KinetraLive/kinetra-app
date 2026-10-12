//! Versioned JSON stores on disk.
//!
//! On disk a store is its struct's JSON object with `"schemaVersion": N` as the first key. Loading
//! parses to a `serde_json::Value` (only for the duration of the load), checks the version,
//! migrates older files, then deserializes into the struct. Writes go through
//! [`fsx::write_json_atomic`].
//!
//! Coalescing of writes (dirty flags + the flusher thread) lives in `state`.

use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::core::fsx;
use crate::core::model::{Prefs, ProjectFile, RecentProject, UiState, RECENTS_CAP};
use crate::error::AppError;

const VERSION_KEY: &str = "schemaVersion";

/// A JSON file with a schema version.
pub trait Store: Serialize + DeserializeOwned + Default {
    /// File name inside the store's directory.
    const FILE: &'static str;
    /// Current schema version written by this build.
    const VERSION: u32;

    /// Upgrades `v` (with `schemaVersion` already removed) from schema `from` all the way to
    /// `VERSION`. Only called when `from < VERSION`. Default: no migrations, return `v`.
    fn migrate(v: Value, from: u32) -> Result<Value, PersistError> {
        let _ = from;
        Ok(v)
    }

    /// Fix-ups applied after every successful load (e.g. resetting fields that are not
    /// meaningful on disk). Default: nothing.
    fn after_load(&mut self) {}
}

#[derive(Debug, thiserror::Error)]
pub enum PersistError {
    #[error("{} does not exist", .0.display())]
    Missing(PathBuf),
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{} is not valid: {msg}", path.display())]
    Corrupt { path: PathBuf, msg: String },
    #[error("file has schema {found}, this version supports {supported}")]
    NewerVersion { found: u32, supported: u32 },
    #[error("migration failed: {0}")]
    Migration(String),
}

impl From<PersistError> for AppError {
    fn from(e: PersistError) -> Self {
        match e {
            PersistError::Missing(p) => AppError::NotFound(fsx::display_path(&p)),
            PersistError::Io { path, source } => AppError::Io(io::Error::new(
                source.kind(),
                format!("{}: {source}", fsx::display_path(&path)),
            )),
            PersistError::Corrupt { .. } | PersistError::Migration(_) => {
                AppError::Invalid(e.to_string())
            }
            PersistError::NewerVersion { found, supported } => {
                AppError::NewerVersion { found, supported }
            }
        }
    }
}

/// A corrupt store file was moved aside and defaults are used instead. Emitted to the UI as
/// `store-recovered` (see [`Recovered::payload`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered {
    /// Path of the original file.
    pub file: String,
    /// Path the corrupt file was renamed to.
    pub quarantined_as: String,
}

impl Recovered {
    /// `{ "file": ..., "quarantinedAs": ... }`
    pub fn payload(&self) -> Value {
        serde_json::json!({ "file": self.file, "quarantinedAs": self.quarantined_as })
    }
}

#[derive(Serialize)]
struct Versioned<'a, S> {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(flatten)]
    inner: &'a S,
}

/// Path of store `S` inside `dir`.
pub fn path_of<S: Store>(dir: &Path) -> PathBuf {
    dir.join(S::FILE)
}

/// Writes `value` to `<dir>/<S::FILE>` atomically, with `schemaVersion` first.
pub fn save<S: Store>(dir: &Path, value: &S) -> Result<(), PersistError> {
    let path = path_of::<S>(dir);
    let versioned = Versioned {
        schema_version: S::VERSION,
        inner: value,
    };
    fsx::write_json_atomic(&path, &versioned).map_err(|source| PersistError::Io { path, source })
}

/// Decodes a store from raw bytes (version check, migration, deserialization). `path` is only
/// used in error messages.
pub fn decode<S: Store>(path: &Path, bytes: &[u8]) -> Result<S, PersistError> {
    let corrupt = |msg: String| PersistError::Corrupt {
        path: path.to_path_buf(),
        msg,
    };
    let v: Value = serde_json::from_slice(bytes).map_err(|e| corrupt(e.to_string()))?;
    let Value::Object(mut obj) = v else {
        return Err(corrupt("expected a JSON object".into()));
    };
    let found = match obj.remove(VERSION_KEY) {
        None => 1,
        Some(n) => n
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n >= 1)
            .ok_or_else(|| corrupt(format!("bad {VERSION_KEY}: {n}")))?,
    };
    if found > S::VERSION {
        return Err(PersistError::NewerVersion {
            found,
            supported: S::VERSION,
        });
    }
    let mut v = Value::Object(obj);
    if found < S::VERSION {
        v = S::migrate(v, found)?;
    }
    let mut s: S = serde_json::from_value(v).map_err(|e| corrupt(e.to_string()))?;
    s.after_load();
    Ok(s)
}

/// Loads `<dir>/<S::FILE>`; any problem (including a missing file) is an error and nothing on
/// disk is changed. Used for `project.json`.
pub fn load_strict<S: Store>(dir: &Path) -> Result<S, PersistError> {
    let path = path_of::<S>(dir);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(PersistError::Missing(path)),
        Err(source) => return Err(PersistError::Io { path, source }),
    };
    decode(&path, &bytes)
}

/// Loads `<dir>/<S::FILE>`, falling back to defaults:
/// - missing file: `S::default()`;
/// - unparseable, wrong-shape or unmigratable file: renamed to `<FILE>.corrupt-<timestamp>`,
///   defaults returned with `Some(Recovered)`;
/// - newer `schemaVersion`: `Err(NewerVersion)`; the file is left alone (never overwrite it);
/// - other I/O failures (the read, or the quarantine rename): `Err(Io)`.
pub fn load_or_recover<S: Store>(dir: &Path) -> Result<(S, Option<Recovered>), PersistError> {
    match load_strict::<S>(dir) {
        Ok(s) => Ok((s, None)),
        Err(PersistError::Missing(_)) => Ok((S::default(), None)),
        Err(e @ (PersistError::Corrupt { .. } | PersistError::Migration(_))) => {
            let path = path_of::<S>(dir);
            tracing::warn!("{e}; setting {} aside", path.display());
            let recovered = quarantine(&path)?;
            Ok((S::default(), Some(recovered)))
        }
        Err(e) => Err(e),
    }
}

/// Renames a corrupt file to `<file>.corrupt-<timestamp>`.
pub(crate) fn quarantine(path: &Path) -> Result<Recovered, PersistError> {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".corrupt-{}", fsx::now_compact()));
    let mut target = path.with_file_name(&name);
    if target.exists() {
        name.push(format!("-{}", nanoid::nanoid!(4)));
        target = path.with_file_name(name);
    }
    std::fs::rename(path, &target).map_err(|source| PersistError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(Recovered {
        file: fsx::display_path(path),
        quarantined_as: fsx::display_path(&target),
    })
}

// ---- Stores --------------------------------------------------------------------------------

impl Store for Prefs {
    const FILE: &'static str = "prefs.json";
    const VERSION: u32 = 1;
}

/// `recents.json` in the app config dir.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RecentsFile {
    pub projects: Vec<RecentProject>,
}

impl Store for RecentsFile {
    const FILE: &'static str = "recents.json";
    const VERSION: u32 = 1;

    fn after_load(&mut self) {
        // `missing` is only ever filled in by the deferred existence check.
        for p in &mut self.projects {
            p.missing = false;
        }
        self.projects.truncate(RECENTS_CAP);
    }
}

impl Store for UiState {
    const FILE: &'static str = "ui-state.json";
    const VERSION: u32 = 1;
}

impl Store for ProjectFile {
    const FILE: &'static str = "project.json";
    const VERSION: u32 = 1;
}

impl Default for ProjectFile {
    fn default() -> Self {
        Self {
            name: String::new(),
            created_at: fsx::now_rfc3339(),
            settings: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Theme;
    use std::fs;

    #[test]
    fn round_trip_with_version_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = Prefs {
            theme: Theme::Dark,
            ..Default::default()
        };
        p.backup.retention = 42;
        save(dir.path(), &p).unwrap();
        let text = fs::read_to_string(dir.path().join("prefs.json")).unwrap();
        assert!(text.starts_with("{\n  \"schemaVersion\": 1,\n"), "{text}");
        assert!(text.ends_with("}\n"));
        let back: Prefs = load_strict(dir.path()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn missing_file_gives_default() {
        let dir = tempfile::tempdir().unwrap();
        let (p, rec) = load_or_recover::<Prefs>(dir.path()).unwrap();
        assert_eq!(p, Prefs::default());
        assert!(rec.is_none());
        assert!(matches!(
            load_strict::<Prefs>(dir.path()),
            Err(PersistError::Missing(_))
        ));
    }

    #[test]
    fn corrupt_file_is_quarantined() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.json");
        fs::write(&path, "{ not json").unwrap();
        let (p, rec) = load_or_recover::<Prefs>(dir.path()).unwrap();
        assert_eq!(p, Prefs::default());
        let rec = rec.expect("recovered");
        assert!(!path.exists());
        assert!(Path::new(&rec.quarantined_as).exists());
        assert!(rec.quarantined_as.contains("prefs.json.corrupt-"));
        assert_eq!(rec.payload()["quarantinedAs"], rec.quarantined_as.as_str());
        assert_eq!(
            fs::read_to_string(&rec.quarantined_as).unwrap(),
            "{ not json"
        );
    }

    #[test]
    fn wrong_shape_is_quarantined() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("prefs.json"), r#"{"theme": 5}"#).unwrap();
        let (_, rec) = load_or_recover::<Prefs>(dir.path()).unwrap();
        assert!(rec.is_some());
        fs::write(dir.path().join("prefs.json"), "[1,2]").unwrap();
        let (_, rec) = load_or_recover::<Prefs>(dir.path()).unwrap();
        assert!(rec.is_some());
    }

    #[test]
    fn strict_load_never_renames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.json");
        fs::write(&path, "garbage").unwrap();
        assert!(matches!(
            load_strict::<ProjectFile>(dir.path()),
            Err(PersistError::Corrupt { .. })
        ));
        assert!(path.exists());
    }

    #[test]
    fn newer_version_is_an_error_and_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prefs.json");
        let text = r#"{"schemaVersion": 7, "theme": "dark", "future": true}"#;
        fs::write(&path, text).unwrap();
        let err = load_or_recover::<Prefs>(dir.path()).unwrap_err();
        assert!(matches!(
            err,
            PersistError::NewerVersion {
                found: 7,
                supported: 1
            }
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(matches!(
            AppError::from(err),
            AppError::NewerVersion { found: 7, .. }
        ));
    }

    #[test]
    fn missing_schema_version_is_v1() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("prefs.json"), r#"{"theme": "light"}"#).unwrap();
        let (p, rec) = load_or_recover::<Prefs>(dir.path()).unwrap();
        assert!(rec.is_none());
        assert_eq!(p.theme, Theme::Light);
    }

    #[test]
    fn recents_missing_flag_reset_on_load() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("recents.json"),
            r#"{"schemaVersion":1,"projects":[{"path":"C:\\a","name":"a","lastOpenedAt":"x","missing":true}]}"#,
        )
        .unwrap();
        let r: RecentsFile = load_strict(dir.path()).unwrap();
        assert!(!r.projects[0].missing);
    }

    #[derive(Debug, Default, Serialize, serde::Deserialize, PartialEq)]
    struct V2 {
        title: String,
    }

    impl Store for V2 {
        const FILE: &'static str = "v2.json";
        const VERSION: u32 = 2;
        fn migrate(mut v: Value, from: u32) -> Result<Value, PersistError> {
            assert_eq!(from, 1);
            let obj = v
                .as_object_mut()
                .ok_or(PersistError::Migration("not an object".into()))?;
            let name = obj.remove("name").unwrap_or(Value::Null);
            obj.insert("title".into(), name);
            Ok(v)
        }
    }

    #[test]
    fn migrates_older_schema() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("v2.json"), r#"{"name":"x"}"#).unwrap();
        let v: V2 = load_strict(dir.path()).unwrap();
        assert_eq!(v.title, "x");
    }
}
