//! UUID v7 strings (RFC 9562) for the project's show ID and correlation IDs: a 48-bit Unix
//! millisecond timestamp, then random bits from the OS. No uuid crate.

use crate::core::fsx::now_epoch_ms;

/// A new UUID v7, lower-case with hyphens (`0193…-…`).
pub fn uuid_v7() -> String {
    let mut b = [0u8; 16];
    // A failing OS RNG leaves zeros: still unique enough with the timestamp for corr IDs, and
    // the show ID is only ever made once per project.
    if getrandom::fill(&mut b[6..]).is_err() {
        tracing::warn!("OS random numbers unavailable for an id");
    }
    let ms = now_epoch_ms();
    b[..6].copy_from_slice(&ms.to_be_bytes()[2..8]);
    b[6] = (b[6] & 0x0f) | 0x70;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Whether `s` is a UUID (any version) as the protocol uses them: 36 characters, hex and hyphens
/// in the 8-4-4-4-12 layout.
pub fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// A nonce for `hello`: 32 URL-safe characters from the OS RNG.
pub fn nonce() -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut b = [0u8; 32];
    if getrandom::fill(&mut b).is_err() {
        tracing::warn!("OS random numbers unavailable for a nonce");
    }
    b.iter()
        .map(|x| ABC[usize::from(*x) % 64] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_v7_has_the_version_variant_and_layout() {
        let a = uuid_v7();
        let b = uuid_v7();
        // No value in the message: assertion output is logged, and ids count as secrets to scanners.
        assert!(is_uuid(&a), "not in the 8-4-4-4-12 hex layout");
        assert_ne!(a, b);
        assert_eq!(&a[14..15], "7");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"));
        assert!(!is_uuid("0193-x"));
    }

    #[test]
    fn nonces_are_url_safe_and_fresh() {
        let n = nonce();
        assert_eq!(n.len(), 32);
        assert!(n
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(n, nonce());
    }
}
