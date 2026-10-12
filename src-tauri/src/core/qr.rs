//! QR codes for poll voting links, made locally (no external service): `qrcodegen` builds the
//! matrix, we draw it as one SVG path on a white square with the 4-module quiet zone, black
//! modules (never tinted: contrast is what makes it readable from the back of a room), and hand it
//! out as a `data:image/svg+xml` URL, which the renderer accepts in image fields.
//!
//! Error correction: Quartile when the text fits in version 3 (a short voting URL does: big
//! modules, survives a dirty projector), else Medium at whatever version it needs.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use qrcodegen::{QrCode, QrCodeEcc, QrSegment, Version};

/// Modules of white around the code (the spec's minimum).
const QUIET: i32 = 4;
/// Cached codes (a project uses a handful of polls).
const CACHE_MAX: usize = 64;

/// The QR matrix for `text`: rows of dark (`true`) modules, without the quiet zone.
pub fn matrix(text: &str) -> Option<Vec<Vec<bool>>> {
    let segs = QrSegment::make_segments(text);
    let qr = QrCode::encode_segments_advanced(
        &segs,
        QrCodeEcc::Quartile,
        Version::new(1),
        Version::new(3),
        None,
        true,
    )
    .or_else(|_| QrCode::encode_text(text, QrCodeEcc::Medium))
    .ok()?;
    let n = qr.size();
    Some(
        (0..n)
            .map(|y| (0..n).map(|x| qr.get_module(x, y)).collect())
            .collect(),
    )
}

/// The code as an SVG document: a white square (quiet zone included) and one black path, one
/// `h` run per horizontal stretch of dark modules.
pub fn svg(text: &str) -> Option<String> {
    let m = matrix(text)?;
    let n = m.len() as i32;
    let side = n + 2 * QUIET;
    let mut path = String::with_capacity((n * n) as usize);
    for (y, row) in m.iter().enumerate() {
        let mut x = 0;
        while x < row.len() {
            if !row[x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < row.len() && row[x] {
                x += 1;
            }
            let run = x - start;
            path.push_str(&format!(
                "M{},{}h{run}v1h-{run}z",
                start as i32 + QUIET,
                y as i32 + QUIET
            ));
        }
    }
    Some(format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {side} {side}' \
         shape-rendering='crispEdges'><rect width='{side}' height='{side}' fill='#fff'/>\
         <path fill='#000' d='{path}'/></svg>"
    ))
}

/// `svg` as a `data:image/svg+xml,…` URL (percent-encoded where URLs need it), cached per text.
/// Empty when the text can't be encoded (never for a voting URL).
pub fn data_url(text: &str) -> String {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(hit) = cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(text)
    {
        return hit.clone();
    }
    let Some(svg) = svg(text) else {
        return String::new();
    };
    let mut url = String::with_capacity(svg.len() + 40);
    url.push_str("data:image/svg+xml,");
    for c in svg.chars() {
        match c {
            '%' => url.push_str("%25"),
            '#' => url.push_str("%23"),
            '<' => url.push_str("%3C"),
            '>' => url.push_str("%3E"),
            ' ' => url.push_str("%20"),
            '"' => url.push_str("%22"),
            c => url.push(c),
        }
    }
    let mut g = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if g.len() >= CACHE_MAX {
        g.clear();
    }
    g.insert(text.to_string(), url.clone());
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://poll.kinetra.live/K7P2QX";

    #[test]
    fn a_voting_url_fits_version_3_at_quartile() {
        let m = matrix(URL).unwrap();
        // Version 3 is 29 × 29 modules: big enough to read from far away.
        assert_eq!(m.len(), 29);
        assert!(m.iter().all(|r| r.len() == 29));
        // The three finder patterns: dark corners.
        assert!(m[0][0] && m[0][28] && m[28][0]);
    }

    #[test]
    fn longer_text_still_encodes() {
        let long = format!("{URL}?lang=nb&something=very-long-to-overflow-version-3");
        let m = matrix(&long).unwrap();
        assert!(m.len() > 29);
    }

    #[test]
    fn svg_has_a_white_quiet_zone_and_black_modules() {
        let s = svg(URL).unwrap();
        assert!(
            s.contains("viewBox='0 0 37 37'"),
            "29 + 2 × 4 quiet modules"
        );
        assert!(s.contains("fill='#fff'") && s.contains("fill='#000'"));
        assert!(s.starts_with("<svg") && s.ends_with("</svg>"));
        // The first dark run starts inside the quiet zone's edge.
        assert!(
            s.contains("d='M4,4h7v1h-7z"),
            "top-left finder row: 7 dark modules"
        );
    }

    #[test]
    fn data_url_is_safe_for_an_image_field_and_cached() {
        let u = data_url(URL);
        assert!(u.starts_with("data:image/svg+xml,"));
        assert!(!u.contains(['<', '>', '#', ' ', '"']));
        assert!(
            u.len() < 6 * 1024,
            "small enough to send once: {} bytes",
            u.len()
        );
        assert_eq!(data_url(URL), u);
    }
}
