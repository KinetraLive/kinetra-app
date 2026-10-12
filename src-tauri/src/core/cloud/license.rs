//! Pure licensing decisions: what a stored token allows now, and when a change of access may
//! take effect. No I/O, no clock (callers pass `now`), so every state is unit-tested.
//!
//! - [`evaluate_token`]: token → `verify_license` (signature, key status, machine) →
//!   `kinetra_license::evaluate` (revocation, clock tampering, coverage, expiry, offline
//!   grace). No token or an invalid one → `core.app` only (`evaluate::no_license`).
//! - [`settle`]: the on-air rule. Gaining entitlements applies at once; losing one waits
//!   while anything is on air and applies once nothing is.

use kinetra_license::evaluate::{self, Access, EvalInput, Status};
use kinetra_license::revocation::RevocationList;
use kinetra_license::{KeySet, LicenseClaims};

/// The license state the core works from (display data for the UI is built from it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eval {
    /// `None` = no usable token (none stored, unverifiable, other machine).
    pub status: Option<Status>,
    pub access: Access,
    /// Always contains the baseline (`core.app`).
    pub entitlements: Vec<String>,
    /// The verified claims, when there is a token.
    pub claims: Option<LicenseClaims>,
    pub full_until: i64,
    pub reduced_until: i64,
    /// "Valid for N more days without connection" (0 unless in full access).
    pub offline_days_left: i64,
}

impl Eval {
    /// What having no license means: `core.app` only.
    pub fn none() -> Self {
        Self {
            status: None,
            access: Access::Free,
            entitlements: evaluate::no_license(),
            claims: None,
            full_until: 0,
            reduced_until: 0,
            offline_days_left: 0,
        }
    }
}

/// Inputs besides the token.
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    pub keys: &'a KeySet,
    pub machine: &'a str,
    pub now: i64,
    /// Highest server time ever seen (file, keychain, responses; the token's `iat` is added
    /// by `evaluate`).
    pub last_server_time: i64,
    pub release_date: i64,
    pub revocations: Option<&'a RevocationList>,
}

/// What the stored token allows right now (see the module docs).
pub fn evaluate_token(token: Option<&str>, cx: &Context<'_>) -> Eval {
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return Eval::none();
    };
    let claims = match kinetra_license::verify_license(token, cx.keys, cx.machine) {
        Ok(v) => v.claims,
        Err(e) => {
            tracing::info!("license token not usable: {e}");
            return Eval::none();
        }
    };
    let revoked = cx
        .revocations
        .is_some_and(|r| r.revokes(&claims.lid, &claims.aid));
    let ev = evaluate::evaluate(
        &claims,
        &EvalInput {
            now: cx.now,
            last_server_time: cx.last_server_time,
            app_release_date: cx.release_date,
            revoked,
        },
    );
    Eval {
        offline_days_left: evaluate::offline_days_left(&ev, cx.now),
        status: Some(ev.status),
        access: ev.access,
        entitlements: ev.entitlements,
        claims: Some(claims),
        full_until: ev.full_until,
        reduced_until: ev.reduced_until,
    }
}

/// The on-air rule. `effective` is what gates commands now, `latest` what the newest
/// evaluation says. Returns the new effective set; it equals `latest` unless something is on
/// air and `latest` lacks an entitlement `effective` has: then gains apply and losses wait.
pub fn settle(effective: &[String], latest: &[String], on_air: bool) -> Vec<String> {
    if !on_air {
        return latest.to_vec();
    }
    let mut e: Vec<String> = effective.iter().chain(latest).cloned().collect();
    e.sort();
    e.dedup();
    e
}

/// Whether a loss of access is waiting for the show to end.
pub fn pending(effective: &[String], latest: &[String]) -> bool {
    effective.iter().any(|e| !latest.contains(e))
}

/// A random URL-safe nonce (32 chars) for activate/refresh, from the OS RNG.
pub fn nonce() -> Result<String, String> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "No secure random source".to_string())?;
    Ok(bytes
        .iter()
        .map(|b| ALPHABET[usize::from(b & 63)] as char)
        .collect())
}

/// Reads the server time out of a `clock_skew` error's `details.server_time`.
pub fn skew_server_time(details: Option<&serde_json::Value>) -> Option<i64> {
    details?.get("server_time")?.as_i64()
}

#[cfg(test)]
mod tests;
