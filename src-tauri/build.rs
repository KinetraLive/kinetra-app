fn main() {
    // The renderer bundle is embedded with rust-embed, which needs the folder to exist at compile
    // time. Create it empty when the renderer hasn't been built (e.g. `cargo test` on a fresh
    // checkout); `/screen/:id` then answers 503 until `pnpm --dir renderer build` runs.
    let dist = std::path::Path::new("../renderer/dist");
    if !dist.exists() {
        let _ = std::fs::create_dir_all(dist);
    }
    // The style library's index (`presets/library.json`) is compiled in; an empty library when
    // it doesn't exist yet. Copied through OUT_DIR so a missing file never breaks the build.
    println!("cargo:rerun-if-changed=../presets/library.json");
    if let Ok(out) = std::env::var("OUT_DIR") {
        let text = std::fs::read_to_string("../presets/library.json")
            .unwrap_or_else(|_| "{\"styles\":[],\"presets\":[]}".to_string());
        let _ = std::fs::write(std::path::Path::new(&out).join("library.json"), text);
    }
    build_info();
    tauri_build::build()
}

/// `KINETRA_CHANNEL` (stable | canary, default stable; anything else fails the build) and
/// `KINETRA_COMMIT` (short git hash, "unknown" outside a checkout) for `app_info`.
fn build_info() {
    println!("cargo:rerun-if-env-changed=KINETRA_CHANNEL");
    let channel = std::env::var("KINETRA_CHANNEL").unwrap_or_default();
    let channel = match channel.trim() {
        "" | "stable" => "stable",
        "canary" => "canary",
        other => panic!("KINETRA_CHANNEL must be \"stable\" or \"canary\", not {other:?}"),
    };
    println!("cargo:rustc-env=KINETRA_CHANNEL={channel}");

    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=KINETRA_COMMIT={commit}");

    // The build's release date for licensing (perpetual licenses cover builds released on or
    // before `updates_until`): HEAD's commit time, so one commit always gives one date;
    // SOURCE_DATE_EPOCH wins when set; 0 outside a checkout.
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let release = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .or_else(|| {
            std::process::Command::new("git")
                .args(["log", "-1", "--format=%ct", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
        })
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    println!("cargo:rustc-env=KINETRA_RELEASE_DATE={release}");

    // Beta builds embed their program slug (set by CI); empty = not a beta build.
    println!("cargo:rerun-if-env-changed=KINETRA_BETA_PROGRAM");
    let beta = std::env::var("KINETRA_BETA_PROGRAM").unwrap_or_default();
    let beta = beta.trim();
    assert!(
        beta.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
        "KINETRA_BETA_PROGRAM must be a slug (a-z, 0-9, -), not {beta:?}"
    );
    println!("cargo:rustc-env=KINETRA_BETA_PROGRAM={beta}");

    // Rerun only when HEAD moves: HEAD itself (a checkout) and the ref it points to (a commit),
    // plus packed-refs (a ref that was packed). Never the whole .git folder.
    if let Some(git_dir) = git_dir() {
        let head = git_dir.join("HEAD");
        println!("cargo:rerun-if-changed={}", head.display());
        if let Ok(text) = std::fs::read_to_string(&head) {
            if let Some(r) = text.trim().strip_prefix("ref: ") {
                // A worktree keeps its HEAD here but shared refs in the common dir.
                let common = std::fs::read_to_string(git_dir.join("commondir"))
                    .map(|c| git_dir.join(c.trim()))
                    .unwrap_or_else(|_| git_dir.clone());
                for base in [&git_dir, &common] {
                    let file = base.join(r);
                    if file.exists() {
                        println!("cargo:rerun-if-changed={}", file.display());
                    }
                }
                let packed = common.join("packed-refs");
                if packed.exists() {
                    println!("cargo:rerun-if-changed={}", packed.display());
                }
            }
        }
    }
}

/// The repository's git dir (`../.git`, or where a `.git` file points for a worktree).
fn git_dir() -> Option<std::path::PathBuf> {
    let dot_git = std::path::Path::new("..").join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    let dir = text.trim().strip_prefix("gitdir: ")?;
    let dir = std::path::PathBuf::from(dir);
    Some(if dir.is_absolute() {
        dir
    } else {
        std::path::Path::new("..").join(dir)
    })
}
