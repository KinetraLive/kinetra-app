use std::path::PathBuf;

use super::*;

/// The tags and text of HTML, whitespace aside (what rendering depends on).
fn html_content(html: &str) -> Vec<String> {
    tokenize(html)
        .into_iter()
        .filter_map(|t| match t {
            Tok::Open { raw, .. } | Tok::Close { raw, .. } => Some(raw.to_string()),
            Tok::Other(t) => Some(t.to_string()),
            Tok::Text(t) => {
                let c = collapse(t).trim().to_string();
                (!c.is_empty()).then_some(c)
            }
        })
        .collect()
}

/// CSS without whitespace and without the last `;` of a block (what formatting may change).
fn css_content(css: &str) -> String {
    let s: String = css.chars().filter(|c| !c.is_whitespace()).collect();
    s.replace(";}", "}")
}

#[test]
fn formats_an_ai_one_liner() {
    let html = r#"<div class="lt" data-state="in"><div class="plate"><div class="name" data-fit>{{name}}</div><div class="role">{{role}}</div></div><div class="bar"></div></div>"#;
    assert_eq!(
        format_html(html),
        r#"<div class="lt" data-state="in">
  <div class="plate">
    <div class="name" data-fit>{{name}}</div>
    <div class="role">{{role}}</div>
  </div>
  <div class="bar"></div>
</div>
"#
    );
    let css = ":host{--accent:#c8102e}.lt{position:absolute;inset:0;font:600 48px/1.1 var(--font-heading)}.lt[data-state=in] .plate{animation:in .4s cubic-bezier(.2,0,0,1) both}@keyframes in{from{opacity:0;transform:translateY(20px)}to{opacity:1}}";
    assert_eq!(
        format_css(css),
        ":host {
  --accent: #c8102e;
}

.lt {
  position: absolute;
  inset: 0;
  font: 600 48px/1.1 var(--font-heading);
}

.lt[data-state=in] .plate {
  animation: in .4s cubic-bezier(.2,0,0,1) both;
}

@keyframes in {
  from {
    opacity: 0;
    transform: translateY(20px);
  }
  to {
    opacity: 1;
  }
}
"
    );
}

#[test]
fn inline_content_and_whitespace_sensitive_siblings_stay_on_one_line() {
    // Text with inline elements: a line break inside would add a visible space.
    let html = r#"<div class="score"><span class="home">{{home}}</span><span class="sep">–</span><span class="away">{{away}}</span></div>"#;
    assert_eq!(format_html(html), format!("{html}\n"));
    // Mixed text and blocks: kept as written (whitespace collapsed only).
    let mixed = "<div>Now:   <b>{{title}}</b>\n  <div>{{sub}}</div></div>";
    assert_eq!(
        format_html(mixed),
        "<div>Now: <b>{{title}}</b> <div>{{sub}}</div></div>\n"
    );
    // Lists, attributes with `>` in quotes, comments.
    let list = r#"<ul data-each="rows" data-limit="5"><li data-name="{{.status}}" title="a > b">{{.title}}</li></ul><!-- end -->"#;
    assert_eq!(
        format_html(list),
        "<ul data-each=\"rows\" data-limit=\"5\">\n  <li data-name=\"{{.status}}\" title=\"a > b\">{{.title}}</li>\n</ul>\n<!-- end -->\n"
    );
}

#[test]
fn raw_elements_strings_and_comments_are_kept() {
    let pre = "<div><pre>  a\n   b</pre><div>x</div></div>";
    assert_eq!(
        format_html(pre),
        "<div>\n  <pre>  a\n   b</pre>\n  <div>x</div>\n</div>\n"
    );
    let css = r#".a{content:"; } {";/* keep: this; */color:red}"#;
    let f = format_css(css);
    assert!(f.contains(r#"content: "; } {";"#), "{f}");
    assert!(f.contains("/* keep: this; */"), "{f}");
    assert!(f.contains("color: red;"), "{f}");
    // Empty stays empty.
    assert_eq!(format_css("  \n"), "");
    assert_eq!(format_html(""), "");
}

#[test]
fn formatting_is_idempotent_and_keeps_the_content() {
    let samples = [
        r#"<div class="a"><div class="b">{{x}}</div><img src="{{logo}}" alt=""><br/></div>"#,
        "<section><h1>{{t}}</h1><p>Some <em>text</em>   here</p></section>",
    ];
    for h in samples {
        let once = format_html(h);
        assert_eq!(format_html(&once), once, "{h}");
        assert_eq!(html_content(&once), html_content(h), "{h}");
    }
}

/// Every built-in preset look: formatting twice gives the same, and the tags, text and
/// declarations are those of the original (the renderer sees the same graphic). The formatted
/// look still passes the template checks.
#[test]
fn every_preset_formats_without_changing_its_content() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../presets/templates");
    let mut looks = 0;
    for entry in walkdir::WalkDir::new(&root) {
        let entry = entry.unwrap();
        let p = entry.path();
        let ext = p.extension().and_then(|e| e.to_str());
        let text = match ext {
            Some("html") | Some("css") => std::fs::read_to_string(p).unwrap(),
            _ => continue,
        };
        if ext == Some("html") {
            let f = format_html(&text);
            assert_eq!(format_html(&f), f, "not idempotent: {}", p.display());
            assert_eq!(
                html_content(&f),
                html_content(&text),
                "content changed: {}",
                p.display()
            );
            let problems = crate::core::template_safety::check_html(&f);
            assert!(problems.is_empty(), "{}: {problems:?}", p.display());
            looks += 1;
        } else {
            let f = format_css(&text);
            assert_eq!(format_css(&f), f, "not idempotent: {}", p.display());
            assert_eq!(
                css_content(&f),
                css_content(&text),
                "content changed: {}",
                p.display()
            );
        }
    }
    assert!(looks > 30, "found {looks} looks");
}
