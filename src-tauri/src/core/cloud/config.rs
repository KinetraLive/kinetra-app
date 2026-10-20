//! The one place for Kinetra's backend URLs (`docs/API.md`).

/// The API (`/v1/…`).
pub const API_BASE: &str = "https://api.kinetra.live";
/// The account site (people sign in and approve devices there).
pub const ACCOUNT_BASE: &str = "https://account.kinetra.live";
/// "Manage account".
pub const ACCOUNT_URL: &str = "https://account.kinetra.live/account";
/// "Upgrade to Pro".
pub const PRICING_URL: &str = "https://kinetra.live/pricing";
/// "Download the stable version" (a beta or canary build after its beta ended).
pub const DOWNLOAD_URL: &str = "https://kinetra.live/download";

/// Beta and canary builds stop when their beta ends (stable builds fall back to Free).
pub fn prerelease_build() -> bool {
    beta_program().is_some() || env!("KINETRA_CHANNEL") == "canary"
}

/// The public key set this build trusts (`KeySet::from_trusted_json`). Updates from
/// `GET /v1/license/keys` are applied only when a key in here (or one it vouched for) signed
/// them.
pub const TRUSTED_KEYS: &str = include_str!("../../../license/trusted-keys.json");

/// This build's release date (unix seconds; HEAD's commit time, see `build.rs`), for
/// perpetual licenses' `updates_until`.
pub fn release_date() -> i64 {
    env!("KINETRA_RELEASE_DATE").parse().unwrap_or(0)
}

/// The beta program this build belongs to (`KINETRA_BETA_PROGRAM` at build time), if any.
pub fn beta_program() -> Option<&'static str> {
    Some(env!("KINETRA_BETA_PROGRAM")).filter(|s| !s.is_empty())
}

/// Debug builds only: more trusted public keys from the file `KINETRA_TRUSTED_KEYS` names (e.g.
/// `src-tauri/license/dev-trusted-keys.json`, the dev API's key), added to the embedded ones so a
/// debug build can use the dev API's licenses. Doesn't exist in release builds (`release_guard`,
/// `scripts/check-release-overrides.mjs`): release builds trust only [`TRUSTED_KEYS`].
#[cfg(debug_assertions)]
pub fn extra_trusted_keys() -> Option<String> {
    let path = std::env::var("KINETRA_TRUSTED_KEYS")
        .ok()
        .filter(|p| !p.is_empty())?;
    match std::fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("KINETRA_TRUSTED_KEYS not read: {e}");
            None
        }
    }
}

/// The trusted keys as JSON: [`TRUSTED_KEYS`], plus [`extra_trusted_keys`] in debug builds.
pub fn trusted_keys() -> String {
    #[cfg(debug_assertions)]
    if let Some(extra) = extra_trusted_keys() {
        return merge_keys(TRUSTED_KEYS, &extra);
    }
    TRUSTED_KEYS.to_string()
}

/// `base` with the `keys` of `extra` appended (`base` unchanged if `extra` doesn't parse).
#[cfg_attr(not(any(debug_assertions, test)), allow(dead_code))]
fn merge_keys(base: &str, extra: &str) -> String {
    let (Ok(mut b), Ok(e)) = (
        serde_json::from_str::<serde_json::Value>(base),
        serde_json::from_str::<serde_json::Value>(extra),
    ) else {
        return base.to_string();
    };
    if let (Some(keys), Some(more)) = (b["keys"].as_array_mut(), e["keys"].as_array()) {
        keys.extend(more.iter().cloned());
    }
    b.to_string()
}

/// `Kinetra/<version> (<os>)`, the User-Agent of every API request (CLOUD_PROJECTS.md §3.1).
pub fn user_agent() -> String {
    let os = match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        other => other,
    };
    format!("Kinetra/{} ({os})", env!("CARGO_PKG_VERSION"))
}

/// The API base for this process: [`API_BASE`], or `KINETRA_API_BASE` in debug builds (the dev
/// API, the e2e and test mock servers). Release builds can't be pointed elsewhere. An override
/// must be `https://`, or `http://` to this computer (the mocks): the session token never goes
/// over plain HTTP to another machine; anything else is ignored.
pub fn api_base() -> String {
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("KINETRA_API_BASE") {
        let v = v.trim_end_matches('/');
        if !v.is_empty() {
            if safe_base(v) {
                return v.to_string();
            }
            tracing::warn!(
                "KINETRA_API_BASE ignored: it must be https:// or http:// to this computer"
            );
        }
    }
    API_BASE.to_string()
}

/// `https://…`, or `http://` to the loopback (`127.0.0.1`, `localhost`, `[::1]`, any port).
#[cfg_attr(not(any(debug_assertions, test)), allow(dead_code))]
fn safe_base(base: &str) -> bool {
    let lower = base.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return true;
    }
    let Some(rest) = lower.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_api_override_is_https_or_this_computer() {
        for ok in [
            "https://vps-b17d613f.tail33aa62.ts.net:8445",
            "HTTPS://api.example",
            "http://127.0.0.1:51234",
            "http://localhost:7000/x",
            "http://[::1]:9",
        ] {
            assert!(super::safe_base(ok), "{ok}");
        }
        for bad in [
            "http://api.kinetra.live",
            "http://192.168.1.20:8445",
            "http://127.0.0.1.evil.example",
            "http://localhost.evil.example:80",
            "ftp://127.0.0.1",
            "127.0.0.1:8080",
        ] {
            assert!(!super::safe_base(bad), "{bad}");
        }
    }

    #[test]
    fn extra_keys_are_added_to_the_embedded_ones() {
        let base = r#"{"iat":1,"keys":[{"kid":"prod","public_key":"a","status":"active"}]}"#;
        let extra = r#"{"keys":[{"kid":"dev","public_key":"b","status":"active"}]}"#;
        let merged: serde_json::Value =
            serde_json::from_str(&super::merge_keys(base, extra)).unwrap();
        let kids: Vec<&str> = merged["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["kid"].as_str().unwrap())
            .collect();
        assert_eq!(kids, ["prod", "dev"]);
        assert_eq!(merged["iat"], 1);
        // Garbage changes nothing.
        assert_eq!(super::merge_keys(base, "nope"), base);
        // The real files parse into one key set with both keys.
        let both = super::merge_keys(
            super::TRUSTED_KEYS,
            include_str!("../../../license/dev-trusted-keys.json"),
        );
        assert!(kinetra_license::KeySet::from_trusted_json(&both).is_ok());
    }
}
