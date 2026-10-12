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

/// The API base for this process: [`API_BASE`], or `KINETRA_API_BASE` in debug builds (the
/// e2e mock server). Release builds can't be pointed elsewhere.
pub fn api_base() -> String {
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("KINETRA_API_BASE") {
        if !v.is_empty() {
            return v.trim_end_matches('/').to_string();
        }
    }
    API_BASE.to_string()
}
