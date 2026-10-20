//! Debug builds only: `KINETRA_DEV_PROFILE=<name>` makes this PC act as another computer, for
//! testing what needs two (cloud backup conflicts, seat limits) on one machine. The profile
//! changes everything that tells computers apart: the machine fingerprint (a hash of the OS
//! machine id plus the profile), the computer name ("STUDIO-PC (b)"), the keychain service
//! ("Kinetra dev-b": its own sign-in and license) and the config and log folders (a `dev-b`
//! subfolder: its own prefs, recents, `license.json` and `cloud.json`).
//!
//! [`dev_profile`] doesn't exist in release builds: `examples/release_guard.rs` calls it and
//! `scripts/check-release-overrides.mjs` proves it fails to compile with `--release`. Everything
//! else goes through [`active`], which is `None` there.

/// The profile name from `KINETRA_DEV_PROFILE` (`[a-z0-9]{1,16}`; anything else is ignored).
#[cfg(debug_assertions)]
pub fn dev_profile() -> Option<&'static str> {
    static P: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        std::env::var("KINETRA_DEV_PROFILE")
            .ok()
            .filter(|p| is_profile_name(p))
    })
    .as_deref()
}

/// The active dev profile; always `None` in release builds.
pub fn active() -> Option<&'static str> {
    #[cfg(debug_assertions)]
    {
        dev_profile()
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

#[cfg_attr(not(any(debug_assertions, test)), allow(dead_code))]
fn is_profile_name(p: &str) -> bool {
    (1..=16).contains(&p.len())
        && p.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// The OS machine id as this profile sees it.
pub fn machine_id(os_id: String, profile: Option<&str>) -> String {
    match profile {
        Some(p) => format!("{os_id}#dev-{p}"),
        None => os_id,
    }
}

/// The computer name as this profile shows it.
pub fn machine_name(name: String, profile: Option<&str>) -> String {
    match profile {
        Some(p) => format!("{name} ({p})"),
        None => name,
    }
}

/// The keychain service as this profile uses it.
pub fn keychain_service(base: &str, profile: Option<&str>) -> String {
    match profile {
        Some(p) => format!("{base} dev-{p}"),
        None => base.to_string(),
    }
}

/// A config or log folder as this profile uses it.
pub fn dir(base: std::path::PathBuf, profile: Option<&str>) -> std::path::PathBuf {
    match profile {
        Some(p) => base.join(format!("dev-{p}")),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_profile_changes_every_identity() {
        let p = Some("b");
        assert_eq!(machine_id("GUID".into(), p), "GUID#dev-b");
        assert_eq!(machine_id("GUID".into(), None), "GUID");
        assert_ne!(
            kinetra_license::machine::fingerprint(&machine_id("GUID".into(), p)),
            kinetra_license::machine::fingerprint("GUID")
        );
        assert_eq!(machine_name("STUDIO".into(), p), "STUDIO (b)");
        assert_eq!(keychain_service("Kinetra", p), "Kinetra dev-b");
        assert_eq!(keychain_service("Kinetra", None), "Kinetra");
        assert_eq!(dir(PathBuf::from("c"), p), PathBuf::from("c").join("dev-b"));
        assert_eq!(dir(PathBuf::from("c"), None), PathBuf::from("c"));
    }

    #[test]
    fn only_simple_names_are_profiles() {
        assert!(is_profile_name("b"));
        assert!(is_profile_name("pc2"));
        for bad in [
            "",
            "B",
            "a/b",
            "..",
            "a b",
            "seventeen-chars-x",
            "abcdefghijklmnopq",
        ] {
            assert!(!is_profile_name(bad), "{bad}");
        }
    }
}
