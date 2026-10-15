//! Retry timing for cloud backup (CLOUD_PROJECTS.md §3.4): network errors, 5xx and
//! `503 cloud_unavailable` back off from 5 s doubling to 15 min with jitter; a `429` waits
//! `Retry-After`. Pure, so the schedule is tested without waiting.

use std::time::Duration;

use crate::core::cloud::api::CallError;

pub const FIRST: Duration = Duration::from_secs(5);
pub const CAP: Duration = Duration::from_secs(15 * 60);

/// What to do after a failed call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    /// Try again after this long (offline, 5xx, 429).
    After(Duration),
    /// Retrying won't help: the caller handles the error (a refusal, a state).
    No,
}

/// The wait before attempt `fails + 1` (`fails` ≥ 1), with `jitter` in 0..1 taking away up to a
/// fifth (so many computers that went offline together don't come back together).
pub fn delay(fails: u32, jitter: f64) -> Duration {
    let exp = fails.saturating_sub(1).min(16);
    let base = FIRST.saturating_mul(1u32 << exp).min(CAP);
    base.mul_f64(1.0 - 0.2 * jitter.clamp(0.0, 1.0))
}

/// How to retry `err` after `fails` failures in a row (this one included).
pub fn classify(err: &CallError, fails: u32, jitter: f64) -> Retry {
    match err {
        CallError::Offline => Retry::After(delay(fails, jitter)),
        CallError::Api {
            status: 429,
            retry_after,
            ..
        } => Retry::After(
            retry_after
                .map(Duration::from_secs)
                .unwrap_or_else(|| delay(fails, jitter)),
        ),
        CallError::Api { status, .. } if *status >= 500 => Retry::After(delay(fails, jitter)),
        CallError::Api { .. } => Retry::No,
    }
}

/// A jitter value in 0..1 from the OS RNG (0.5 if it fails).
pub fn jitter() -> f64 {
    let mut b = [0u8; 4];
    match getrandom::fill(&mut b) {
        Ok(()) => f64::from(u32::from_le_bytes(b)) / f64::from(u32::MAX),
        Err(_) => 0.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(status: u16, code: &str, retry_after: Option<u64>) -> CallError {
        CallError::Api {
            status,
            code: code.into(),
            message: String::new(),
            details: None,
            retry_after,
        }
    }

    #[test]
    fn doubles_from_five_seconds_to_fifteen_minutes() {
        let secs: Vec<u64> = (1..=10).map(|n| delay(n, 0.0).as_secs()).collect();
        assert_eq!(secs, [5, 10, 20, 40, 80, 160, 320, 640, 900, 900]);
        assert_eq!(delay(1000, 0.0), CAP);
        // Jitter takes off at most a fifth.
        assert_eq!(delay(1, 1.0), Duration::from_secs(4));
        assert!(delay(9, 0.5) < CAP && delay(9, 0.5) >= CAP.mul_f64(0.8));
    }

    #[test]
    fn retries_offline_and_server_errors_but_not_refusals() {
        assert_eq!(classify(&CallError::Offline, 1, 0.0), Retry::After(FIRST));
        assert_eq!(
            classify(&api(503, "cloud_unavailable", None), 2, 0.0),
            Retry::After(Duration::from_secs(10))
        );
        assert_eq!(
            classify(&api(500, "internal", None), 1, 0.0),
            Retry::After(FIRST)
        );
        // 429: Retry-After wins.
        assert_eq!(
            classify(&api(429, "rate_limited", Some(42)), 9, 0.0),
            Retry::After(Duration::from_secs(42))
        );
        for (s, c) in [
            (403, "storage_full"),
            (403, "cloud_not_in_plan"),
            (409, "missing_blobs"),
            (400, "invalid_path"),
            (401, "invalid_token"),
        ] {
            assert_eq!(classify(&api(s, c, None), 1, 0.0), Retry::No, "{c}");
        }
    }

    #[test]
    fn jitter_is_in_range() {
        for _ in 0..100 {
            let j = jitter();
            assert!((0.0..=1.0).contains(&j));
        }
    }
}
