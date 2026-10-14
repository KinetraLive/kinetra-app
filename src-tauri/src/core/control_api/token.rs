//! The Control API token: `kt_` + 43 characters of base64url (32 bytes from the OS RNG), kept
//! in the keychain (one per machine; canary builds use their own keychain service).

/// Keychain id (`secret:control-api-token`).
pub const SECRET_ID: &str = "control-api-token";

const PREFIX: &str = "kt_";

/// A new random token; `None` without a secure random source.
pub fn generate() -> Option<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).ok()?;
    Some(format!("{PREFIX}{}", base64url(&bytes)))
}

/// Whether `s` looks like a token we made (a stored value that doesn't is replaced).
pub fn is_well_formed(s: &str) -> bool {
    s.strip_prefix(PREFIX).is_some_and(|b| {
        b.len() == 43
            && b.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    })
}

/// Compares in time that depends only on the lengths, never on where they differ.
pub fn matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    let mut diff = u8::from(a.len() != b.len());
    for (i, x) in a.iter().enumerate() {
        diff |= x ^ b.get(i).copied().unwrap_or(!x);
    }
    diff == 0
}

/// The token in an `Authorization` header value (`Bearer <token>`, scheme in any case).
pub fn bearer(header: &str) -> Option<&str> {
    let (scheme, rest) = header.trim().split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| rest.trim())
        .filter(|t| !t.is_empty())
}

/// RFC 4648 §5 without padding.
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_random_and_well_formed() {
        let a = generate().unwrap();
        let b = generate().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 46);
        assert!(is_well_formed(&a), "{a}");
        assert!(!is_well_formed("kt_short"));
        assert!(!is_well_formed(&a.replace("kt_", "xx_")));
    }

    #[test]
    fn base64url_matches_the_rfc() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foob"), "Zm9vYg");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn compare_and_bearer() {
        assert!(matches("kt_abc", "kt_abc"));
        assert!(!matches("kt_abc", "kt_abd"));
        assert!(!matches("kt_abc", "kt_ab"));
        assert!(!matches("kt_abc", "kt_abcd"));
        assert!(!matches("kt_abc", ""));
        assert_eq!(bearer("Bearer kt_x"), Some("kt_x"));
        assert_eq!(bearer("bearer  kt_x "), Some("kt_x"));
        assert_eq!(bearer("Basic kt_x"), None);
        assert_eq!(bearer("Bearer "), None);
        assert_eq!(bearer("kt_x"), None);
    }
}
