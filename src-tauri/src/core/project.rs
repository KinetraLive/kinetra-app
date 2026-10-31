//! Project folders on disk: classify, initialize, create and open. No in-memory state here; see
//! `state` for the open project.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::fsx;
use crate::core::model::{FolderKind, ProjectFile, ProjectSettings, FOREIGN_ENTRY_CAP};
use crate::core::persist::{self, PersistError};
use crate::error::AppError;

/// Subfolders every project has.
pub const SUBDIRS: [&str; 5] = ["screens", "templates", "sources", "chat-themes", "assets"];

/// Name of the backups folder inside a project (ignored by git).
pub const BACKUPS_DIR: &str = ".backups";

/// What a project's `.gitignore` ignores: the backups and the copies of web images
/// (`web_images`, which also adds its line to older projects on the first download).
const GITIGNORE_LINES: [&str; 2] = [".backups/", ".web-images/"];

/// Files the OS drops into folders; they do not make a folder "non-empty".
const OS_LITTER: [&str; 3] = ["desktop.ini", "Thumbs.db", ".DS_Store"];

pub const NAME_MAX_CHARS: usize = 100;
const SLUG_MAX_CHARS: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("Not found: {}", fsx::display_path(.0))]
    NotFound(PathBuf),
    #[error("Not a folder: {}", fsx::display_path(.0))]
    NotADirectory(PathBuf),
    #[error("This folder is not a Kinetra project: {}", fsx::display_path(.0))]
    NotAProject(PathBuf),
    #[error("This folder is already a Kinetra project: {}", fsx::display_path(.0))]
    AlreadyAProject(PathBuf),
    #[error("The folder is not empty: {}", fsx::display_path(.0))]
    NotEmpty(PathBuf),
    #[error("{0}")]
    Invalid(String),
    #[error("{}: {source}", fsx::display_path(path))]
    Io { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Persist(#[from] PersistError),
}

impl From<ProjectError> for AppError {
    fn from(e: ProjectError) -> Self {
        match e {
            ProjectError::NotFound(p) => AppError::NotFound(fsx::display_path(&p)),
            ProjectError::NotAProject(p) => AppError::NotAProject(fsx::display_path(&p)),
            ProjectError::NotEmpty(p) => AppError::NotEmpty(fsx::display_path(&p)),
            ProjectError::NotADirectory(_)
            | ProjectError::AlreadyAProject(_)
            | ProjectError::Invalid(_) => AppError::Invalid(e.to_string()),
            ProjectError::Io {
                ref path,
                ref source,
            } => AppError::Io(io::Error::new(
                source.kind(),
                format!("{}: {source}", fsx::display_path(path)),
            )),
            ProjectError::Persist(p) => p.into(),
        }
    }
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> ProjectError + '_ {
    move |source| ProjectError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Fails with `NotFound` / `NotADirectory` unless `path` is an existing directory.
fn require_dir(path: &Path) -> Result<(), ProjectError> {
    match fs::metadata(path) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(ProjectError::NotADirectory(path.to_path_buf())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Err(ProjectError::NotFound(path.to_path_buf()))
        }
        Err(e) => Err(io_err(path)(e)),
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| fsx::display_path(path))
}

/// Trims and checks a project name (1..=100 chars).
pub fn validate_name(name: &str) -> Result<String, ProjectError> {
    let name = name.trim();
    let len = name.chars().count();
    if len == 0 {
        return Err(ProjectError::Invalid("Project name cannot be empty".into()));
    }
    if len > NAME_MAX_CHARS {
        return Err(ProjectError::Invalid(format!(
            "Project name is too long (at most {NAME_MAX_CHARS} characters)"
        )));
    }
    Ok(name.to_string())
}

/// Inspects a folder without writing anything. Never recurses; counts at most
/// `FOREIGN_ENTRY_CAP` entries.
pub fn classify(path: &Path) -> Result<FolderKind, ProjectError> {
    require_dir(path)?;
    if persist::path_of::<ProjectFile>(path).exists() {
        // An unreadable project.json is still a project; `open` reports the real error.
        let name = persist::load_strict::<ProjectFile>(path)
            .ok()
            .map(|f| f.name)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| folder_name(path));
        return Ok(FolderKind::Project { name });
    }
    let mut entries: u32 = 0;
    for entry in fs::read_dir(path).map_err(io_err(path))? {
        let entry = entry.map_err(io_err(path))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if OS_LITTER.iter().any(|l| l.eq_ignore_ascii_case(&name)) {
            continue;
        }
        entries += 1;
        if entries >= FOREIGN_ENTRY_CAP {
            break;
        }
    }
    Ok(if entries == 0 {
        FolderKind::Empty
    } else {
        FolderKind::NonEmptyForeign { entries }
    })
}

/// Makes `path` (created if missing) a project: the five subfolders, a `.gitignore` that ignores
/// `.backups/` and `.web-images/`, and `project.json` (written last). Refuses if `project.json` exists. Does not
/// check emptiness; `classify` is the guard and the opt-in decision belongs to the UI.
pub fn init_in(path: &Path, name: &str) -> Result<ProjectFile, ProjectError> {
    let name = validate_name(name)?;
    fs::create_dir_all(path).map_err(io_err(path))?;
    if persist::path_of::<ProjectFile>(path).exists() {
        return Err(ProjectError::AlreadyAProject(path.to_path_buf()));
    }
    ensure_subdirs(path)?;
    ensure_gitignore(path)?;
    let file = ProjectFile {
        name,
        created_at: fsx::now_rfc3339(),
        settings: ProjectSettings::default(),
        id: None,
        cloud_id: None,
    };
    persist::save(path, &file)?;
    Ok(file)
}

/// Creates `<parent>/<slug(name)>` and initializes it. An existing empty folder with that name is
/// reused; a non-empty one is `NotEmpty`. The project keeps the original (trimmed) `name`.
pub fn create(parent: &Path, name: &str) -> Result<(PathBuf, ProjectFile), ProjectError> {
    let name = validate_name(name)?;
    require_dir(parent)?;
    let dir = parent.join(slug(&name));
    match fs::metadata(&dir) {
        Ok(m) if m.is_dir() => {
            if classify(&dir)? != FolderKind::Empty {
                return Err(ProjectError::NotEmpty(dir));
            }
        }
        Ok(_) => return Err(ProjectError::NotEmpty(dir)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&dir).map_err(io_err(&dir))?;
        }
        Err(e) => return Err(io_err(&dir)(e)),
    }
    let file = init_in(&dir, &name)?;
    Ok((dir, file))
}

/// Reads an existing project. Recreates any missing subfolders (users delete empty folders).
pub fn open(path: &Path) -> Result<ProjectFile, ProjectError> {
    require_dir(path)?;
    let file = match persist::load_strict::<ProjectFile>(path) {
        Ok(f) => f,
        Err(PersistError::Missing(_)) => return Err(ProjectError::NotAProject(path.to_path_buf())),
        Err(e) => return Err(e.into()),
    };
    ensure_subdirs(path)?;
    Ok(file)
}

fn ensure_subdirs(path: &Path) -> Result<(), ProjectError> {
    for sub in SUBDIRS {
        let p = path.join(sub);
        fs::create_dir_all(&p).map_err(io_err(&p))?;
    }
    Ok(())
}

fn ensure_gitignore(path: &Path) -> Result<(), ProjectError> {
    GITIGNORE_LINES
        .iter()
        .try_for_each(|line| ensure_ignored(path, line))
}

/// Adds `line` (a folder, `name/`) to the project's `.gitignore` unless it is there in some form
/// (`name`, `/name/`, `/name`); creates the file if missing.
pub fn ensure_ignored(path: &Path, line: &str) -> Result<(), ProjectError> {
    let p = path.join(".gitignore");
    let bare = line.trim_end_matches('/');
    match fs::read_to_string(&p) {
        Ok(existing) => {
            let has = existing.lines().any(|l| {
                let l = l.trim();
                l.strip_prefix('/').unwrap_or(l).trim_end_matches('/') == bare
            });
            if has {
                return Ok(());
            }
            let mut text = existing;
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(line);
            text.push('\n');
            fsx::write_bytes_atomic(&p, text.as_bytes()).map_err(io_err(&p))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fsx::write_bytes_atomic(&p, format!("{line}\n").as_bytes()).map_err(io_err(&p))
        }
        Err(e) => Err(io_err(&p)(e)),
    }
}

const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Folder name for a project name: lowercase, whitespace to `-`, only letters, digits, `-`, `_`
/// and `.` kept (which drops everything Windows forbids), repeated `-` collapsed, no leading or
/// trailing `.`/`-`/space, Windows reserved device names suffixed with `-project`, and
/// `project` if nothing is left.
pub fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        let mapped = if c.is_whitespace() || c == '-' {
            '-'
        } else if c.is_alphanumeric() || c == '_' || c == '.' {
            c
        } else {
            continue;
        };
        if mapped == '-' && out.ends_with('-') {
            continue;
        }
        out.push(mapped);
    }
    let trim = |s: &str| {
        s.trim_matches(|c: char| c == '-' || c == '.' || c == ' ')
            .to_string()
    };
    let mut s = trim(&out);
    if s.chars().count() > SLUG_MAX_CHARS {
        s = trim(&s.chars().take(SLUG_MAX_CHARS).collect::<String>());
    }
    if s.is_empty() {
        return "project".into();
    }
    let (stem, rest) = match s.find('.') {
        Some(i) => s.split_at(i),
        None => (s.as_str(), ""),
    };
    if RESERVED.contains(&stem) {
        return format!("{stem}-project{rest}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_kinds() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(classify(dir.path()).unwrap(), FolderKind::Empty);

        fs::write(dir.path().join("Thumbs.db"), "").unwrap();
        fs::write(dir.path().join("desktop.ini"), "").unwrap();
        fs::write(dir.path().join(".DS_Store"), "").unwrap();
        assert_eq!(classify(dir.path()).unwrap(), FolderKind::Empty);

        fs::write(dir.path().join("notes.txt"), "").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("deep.txt"), "").unwrap();
        assert_eq!(
            classify(dir.path()).unwrap(),
            FolderKind::NonEmptyForeign { entries: 2 }
        );

        let p = dir.path().join("proj");
        init_in(&p, "Show").unwrap();
        assert_eq!(
            classify(&p).unwrap(),
            FolderKind::Project {
                name: "Show".into()
            }
        );

        fs::write(p.join("project.json"), "broken").unwrap();
        assert_eq!(
            classify(&p).unwrap(),
            FolderKind::Project {
                name: "proj".into()
            }
        );

        assert!(matches!(
            classify(&dir.path().join("nope")),
            Err(ProjectError::NotFound(_))
        ));
    }

    #[test]
    fn classify_caps_entry_count() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..1500 {
            fs::File::create(dir.path().join(format!("f{i}"))).unwrap();
        }
        assert_eq!(
            classify(dir.path()).unwrap(),
            FolderKind::NonEmptyForeign {
                entries: FOREIGN_ENTRY_CAP
            }
        );
    }

    #[test]
    fn init_creates_layout_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let file = init_in(dir.path(), "  My Show  ").unwrap();
        assert_eq!(file.name, "My Show");
        for sub in SUBDIRS {
            assert!(dir.path().join(sub).is_dir(), "{sub}");
        }
        assert_eq!(
            fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
            ".backups/\n.web-images/\n"
        );
        let text = fs::read_to_string(dir.path().join("project.json")).unwrap();
        assert!(text.contains("\"schemaVersion\": 1"));
        assert_eq!(open(dir.path()).unwrap(), file);
    }

    #[test]
    fn init_refuses_existing_project() {
        let dir = tempfile::tempdir().unwrap();
        init_in(dir.path(), "A").unwrap();
        assert!(matches!(
            init_in(dir.path(), "B"),
            Err(ProjectError::AlreadyAProject(_))
        ));
        assert_eq!(open(dir.path()).unwrap().name, "A");
    }

    #[test]
    fn init_appends_to_existing_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "node_modules").unwrap();
        init_in(dir.path(), "A").unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
            "node_modules\n.backups/\n.web-images/\n"
        );

        let dir2 = tempfile::tempdir().unwrap();
        fs::write(
            dir2.path().join(".gitignore"),
            "x\n/.backups\n.web-images\n",
        )
        .unwrap();
        init_in(dir2.path(), "A").unwrap();
        assert_eq!(
            fs::read_to_string(dir2.path().join(".gitignore")).unwrap(),
            "x\n/.backups\n.web-images\n"
        );
    }

    #[test]
    fn open_requires_project_json_and_restores_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            open(dir.path()),
            Err(ProjectError::NotAProject(_))
        ));
        init_in(dir.path(), "A").unwrap();
        fs::remove_dir(dir.path().join("assets")).unwrap();
        open(dir.path()).unwrap();
        assert!(dir.path().join("assets").is_dir());
    }

    #[test]
    fn open_newer_project_is_newer_version() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("project.json"),
            r#"{"schemaVersion": 9, "name": "x", "createdAt": "y"}"#,
        )
        .unwrap();
        let err = AppError::from(open(dir.path()).unwrap_err());
        assert!(matches!(err, AppError::NewerVersion { found: 9, .. }));
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("My Show!"), "my-show");
        assert_eq!(slug("CON"), "con-project");
        assert_eq!(slug("lpt9"), "lpt9-project");
        assert_eq!(slug("a<>:\"/\\|?*b"), "ab");
        assert_eq!(slug("  trailing dots... "), "trailing-dots");
        assert_eq!(slug("a  -  b"), "a-b");
        assert_eq!(slug("!!!"), "project");
        assert_eq!(slug("Nyheter Ø"), "nyheter-ø");
    }

    #[test]
    fn create_cases() {
        let dir = tempfile::tempdir().unwrap();
        let (p, f) = create(dir.path(), "My Show!").unwrap();
        assert_eq!(p, dir.path().join("my-show"));
        assert_eq!(f.name, "My Show!");
        assert_eq!(open(&p).unwrap().name, "My Show!");

        let (p, _) = create(dir.path(), "CON").unwrap();
        assert_eq!(p, dir.path().join("con-project"));

        assert!(matches!(
            create(dir.path(), "  "),
            Err(ProjectError::Invalid(_))
        ));
        assert!(matches!(
            create(dir.path(), &"x".repeat(101)),
            Err(ProjectError::Invalid(_))
        ));

        // Existing non-empty slug dir.
        fs::create_dir(dir.path().join("busy")).unwrap();
        fs::write(dir.path().join("busy").join("a.txt"), "").unwrap();
        assert!(matches!(
            create(dir.path(), "Busy"),
            Err(ProjectError::NotEmpty(_))
        ));
        // Existing project with that slug is not empty either.
        assert!(matches!(
            create(dir.path(), "my show"),
            Err(ProjectError::NotEmpty(_))
        ));
        // Existing empty slug dir is reused.
        fs::create_dir(dir.path().join("empty")).unwrap();
        let (p, _) = create(dir.path(), "Empty").unwrap();
        assert_eq!(p, dir.path().join("empty"));
    }
}
