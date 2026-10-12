//! Template hardening (defense in depth next to the renderer's CSP): what `docs/TEMPLATE_FORMAT.md`
//! §7/§9 never allows, refused on every save and import (AI results included):
//!
//! - HTML: `<script>`, `<iframe>`, `<frame>`, `<frameset>`, `<object>`, `<embed>`, `<applet>`,
//!   `<link>`, `<meta>`, `<base>`, `<form>`, `<portal>`; `on*` handler attributes; `srcdoc`,
//!   `formaction`; `javascript:` / `vbscript:` / `data:text/html` URLs; external URLs (`http(s)://`
//!   or `//host`) in attributes (images come through `image` fields from `/assets/…`).
//! - CSS (template CSS, `<style>` elements and `style` attributes): `url(`, `@import`,
//!   `@font-face`, `expression(`, `-moz-binding`, `behavior:`.
//!
//! The scan is conservative (lower-cased text, a small tag tokenizer): it may refuse odd but
//! harmless markup, never let a listed construct through.

/// What a template may not contain: one entry per kind of problem, operator-readable.
pub fn check_html(html: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |s: String| {
        if !out.contains(&s) {
            out.push(s);
        }
    };
    let lower = html.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut i = 0;
    while let Some(off) = lower[i..].find('<') {
        let start = i + off;
        if lower[start..].starts_with("<!--") {
            i = lower[start..]
                .find("-->")
                .map_or(b.len(), |e| start + e + 3);
            continue;
        }
        let mut j = start + 1;
        if j < b.len() && b[j] == b'/' {
            j += 1;
        }
        let name_start = j;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
            j += 1;
        }
        let name = &lower[name_start..j];
        if name.is_empty() {
            i = start + 1;
            continue;
        }
        if BANNED_TAGS.contains(&name) {
            add(format!("<{name}> elements"));
        }
        // Attributes up to the closing '>' (quotes respected).
        let mut quote: Option<u8> = None;
        let tag_start = j;
        while j < b.len() {
            match (quote, b[j]) {
                (Some(q), c) if c == q => quote = None,
                (None, b'"' | b'\'') => quote = Some(b[j]),
                (None, b'>') => break,
                _ => {}
            }
            j += 1;
        }
        let attrs = &lower[tag_start..j.min(b.len())];
        for (attr, value) in attributes(attrs) {
            if attr.starts_with("on") && attr.len() > 2 {
                add("event handler attributes (on…)".into());
            }
            if attr == "srcdoc" || attr == "formaction" {
                add(format!("the {attr} attribute"));
            }
            let v = value.trim();
            let v_compact: String = v
                .chars()
                .filter(|c| !c.is_whitespace() && !c.is_control())
                .collect();
            if v_compact.starts_with("javascript:")
                || v_compact.starts_with("vbscript:")
                || v_compact.starts_with("data:text/html")
            {
                add("script URLs".into());
            }
            if v.starts_with("http://") || v.starts_with("https://") || v.starts_with("//") {
                add("links to other sites (import images into Kinetra instead)".into());
            }
            if attr == "style" {
                for p in check_css(v) {
                    add(p);
                }
            }
        }
        // A <style> element: its CSS too.
        if name == "style" && lower.as_bytes().get(name_start - 1) != Some(&b'/') {
            if let Some(end) = lower[j..].find("</style") {
                for p in check_css(&lower[j + 1..j + end]) {
                    add(p);
                }
            }
        }
        i = j.max(start + 1);
    }
    out
}

const BANNED_TAGS: &[&str] = &[
    "script", "iframe", "frame", "frameset", "object", "embed", "applet", "link", "meta", "base",
    "form", "portal",
];

/// `name="value"` pairs of a tag's attribute text (lower case).
fn attributes(s: &str) -> Vec<(String, String)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let ns = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        let name = s[ns..i].to_string();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let vs = i + 1;
                i = vs;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                value = s[vs..i.min(b.len())].to_string();
                i += 1;
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = s[vs..i].to_string();
            }
        }
        if !name.is_empty() {
            out.push((name, value));
        }
        if i == ns {
            i += 1;
        }
    }
    out
}

/// What template CSS may not contain (see the module docs).
pub fn check_css(css: &str) -> Vec<String> {
    let lower = css.to_ascii_lowercase();
    // Comments can hide nothing: they are part of the scanned text (`u/**/rl(` is still
    // refused because the CSS parser wouldn't read it as url( either way, but `@im/**/port`
    // can't form an at-rule).
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::new();
    for (needle, what) in [
        ("url(", "url() (images come through image fields)"),
        ("@import", "@import"),
        ("@font-face", "@font-face (use the curated fonts)"),
        ("expression(", "CSS expressions"),
        ("-moz-binding", "CSS bindings"),
        ("behavior:", "CSS behaviors"),
    ] {
        if compact.contains(needle) {
            out.push(what.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_scripts_handlers_frames_and_external_urls() {
        let bad = [
            ("<script>alert(1)</script>", "<script>"),
            ("<div><SCRIPT src=x></SCRIPT></div>", "<script>"),
            ("<img src=x onerror=alert(1)>", "on…"),
            ("<div ONCLICK = \"x()\">", "on…"),
            ("<a href=\"  JaVaScRiPt:alert(1)\">", "script URLs"),
            ("<a href=\"java\tscript:x\">", "script URLs"),
            ("<iframe src=\"/x\"></iframe>", "<iframe>"),
            ("<object data=x></object>", "<object>"),
            ("<embed src=x>", "<embed>"),
            ("<link rel=stylesheet href=x>", "<link>"),
            ("<meta http-equiv=refresh content=0>", "<meta>"),
            ("<base href=\"//evil\">", "<base>"),
            ("<img src=\"https://evil.example/x.png\">", "other sites"),
            ("<img src=//evil.example/x.png>", "other sites"),
            ("<div style=\"background:url(https://evil/x)\">", "url()"),
            ("<style>@import 'x.css';</style>", "@import"),
            ("<iframe srcdoc=\"<b>\">", "srcdoc"),
        ];
        for (html, want) in bad {
            let found = check_html(html);
            assert!(found.iter().any(|p| p.contains(want)), "{html}: {found:?}");
        }
    }

    #[test]
    fn allows_ordinary_template_markup() {
        let ok = r#"<div class="lt" data-state="in"><img src="{{logo}}" alt="" decoding="async">
            <span data-fit data-fit-lines="2">{{name}}</span><svg viewBox="0 0 10 10"><path d="M0 0h10"/></svg>
            <!-- <script> in a comment is inert --><ul data-each="rows"><li>{{.title}}</li></ul>
            <p style="color: var(--accent)">Done</p></div>"#;
        assert_eq!(check_html(ok), Vec::<String>::new());
        assert!(
            check_css(".a{color:red;animation:in .3s}@keyframes in{from{opacity:0}}").is_empty()
        );
    }

    #[test]
    fn css_rules() {
        assert!(!check_css(".a{background: URL( 'x' )}").is_empty());
        assert!(!check_css("@font-face{font-family:x}").is_empty());
        assert!(!check_css("@IMPORT 'x';").is_empty());
        assert!(!check_css(".a{width:expression(alert(1))}").is_empty());
    }
}
