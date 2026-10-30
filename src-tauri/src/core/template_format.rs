//! Pretty-printing template HTML and CSS: AI results (one long line) are formatted before they
//! are saved, and the template code editor's "Format HTML and CSS" runs the same code
//! (`format_template_code`).
//!
//! Conservative on purpose, so the rendered graphic never changes:
//! - **HTML:** tags (with their attributes, bindings and `{{…}}`) are kept exactly as written;
//!   only whitespace between them changes. An element whose content is only text and inline
//!   elements stays on one line (adding a line break there could add a visible space). Lines are
//!   broken only between block-level elements, where whitespace never shows; text and inline
//!   runs keep their own whitespace, collapsed to single spaces as HTML renders it. `<pre>`,
//!   `<textarea>` and `<style>` content and comments are kept as they are. Indentation: two
//!   spaces per level.
//! - **CSS:** one declaration per line, `prop: value`, rules and at-rules (nested ones too)
//!   indented by their braces, a blank line between top-level blocks. Strings and comments are
//!   kept as they are.
//!
//! Formatting twice gives the same text, and a formatted template has the same tags, text and
//! declarations as before (tested, over every built-in preset too).

/// Elements that never contain anything.
const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Elements whose content is kept byte for byte.
const RAW: [&str; 4] = ["pre", "textarea", "style", "script"];

/// Block-level elements: line breaks around them never show.
const BLOCK: [&str; 47] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "dd",
    "details",
    "dialog",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "li",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
    "svg",
    "g",
    "defs",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok<'a> {
    /// `<name …>` (the whole tag as written; `self_closing` for `/>`).
    Open {
        name: String,
        raw: &'a str,
        self_closing: bool,
    },
    Close {
        name: String,
        raw: &'a str,
    },
    Text(&'a str),
    /// Comments, doctypes, processing instructions: kept as written.
    Other(&'a str),
}

fn tag_name(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ':')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// The end of a tag starting at `start` (`<`), skipping quoted attribute values.
fn tag_end(s: &str, start: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = start + 1;
    let mut quote = 0u8;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

fn tokenize(s: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    let b = s.as_bytes();
    while i < b.len() {
        if b[i] != b'<' {
            let next = s[i..].find('<').map_or(s.len(), |n| i + n);
            out.push(Tok::Text(&s[i..next]));
            i = next;
            continue;
        }
        let rest = &s[i..];
        if rest.starts_with("<!--") {
            let end = rest.find("-->").map_or(s.len(), |n| i + n + 3);
            out.push(Tok::Other(&s[i..end]));
            i = end;
            continue;
        }
        let Some(end) = tag_end(s, i) else {
            // A stray `<`: text up to the end.
            out.push(Tok::Text(&s[i..]));
            break;
        };
        let raw = &s[i..end];
        if let Some(inner) = raw.strip_prefix("</") {
            out.push(Tok::Close {
                name: tag_name(inner),
                raw,
            });
        } else if raw.starts_with("<!") || raw.starts_with("<?") {
            out.push(Tok::Other(raw));
        } else {
            let name = tag_name(&raw[1..]);
            if name.is_empty() {
                out.push(Tok::Text(raw));
                i = end;
                continue;
            }
            let self_closing = raw.ends_with("/>");
            out.push(Tok::Open {
                name: name.clone(),
                raw,
                self_closing,
            });
            if RAW.contains(&name.as_str()) && !self_closing {
                // Raw content up to the matching close tag, kept as written.
                let close = format!("</{name}");
                let lower = s[end..].to_ascii_lowercase();
                let stop = lower.find(&close).map_or(s.len(), |n| end + n);
                if stop > end {
                    out.push(Tok::Other(&s[end..stop]));
                }
                i = stop;
                continue;
            }
        }
        i = end;
    }
    out
}

/// A parsed node.
#[derive(Debug)]
enum Node<'a> {
    Element {
        name: String,
        open: &'a str,
        close: Option<&'a str>,
        children: Vec<Node<'a>>,
    },
    Text(&'a str),
    Other(&'a str),
}

/// Builds a tree; unmatched close tags are kept as text, unclosed elements end at their parent.
fn tree<'a>(toks: &[Tok<'a>]) -> Vec<Node<'a>> {
    fn build<'a>(toks: &[Tok<'a>], i: &mut usize, until: Option<&str>) -> Vec<Node<'a>> {
        let mut out = Vec::new();
        while *i < toks.len() {
            match &toks[*i] {
                Tok::Text(t) => {
                    out.push(Node::Text(t));
                    *i += 1;
                }
                Tok::Other(t) => {
                    out.push(Node::Other(t));
                    *i += 1;
                }
                Tok::Close { name, raw } => {
                    if until == Some(name.as_str()) {
                        return out;
                    }
                    if until.is_some() && closes_ancestor(toks, *i) {
                        // A close tag for an ancestor: the parent takes it.
                        return out;
                    }
                    out.push(Node::Text(raw));
                    *i += 1;
                }
                Tok::Open {
                    name,
                    raw,
                    self_closing,
                } => {
                    *i += 1;
                    if *self_closing || VOID.contains(&name.as_str()) {
                        out.push(Node::Element {
                            name: name.clone(),
                            open: raw,
                            close: None,
                            children: Vec::new(),
                        });
                        continue;
                    }
                    let children = build(toks, i, Some(name));
                    let close = match toks.get(*i) {
                        Some(Tok::Close { name: n, raw }) if n == name => {
                            *i += 1;
                            Some(*raw)
                        }
                        _ => None,
                    };
                    out.push(Node::Element {
                        name: name.clone(),
                        open: raw,
                        close,
                        children,
                    });
                }
            }
        }
        out
    }
    let mut i = 0;
    let mut out = build(toks, &mut i, None);
    // Anything left (close tags with no element) stays as text.
    while i < toks.len() {
        if let Tok::Close { raw, .. } = &toks[i] {
            out.push(Node::Text(raw));
        }
        i += 1;
    }
    out
}

/// Whether the close tag at `at` matches an element still open before it.
fn closes_ancestor(toks: &[Tok<'_>], at: usize) -> bool {
    let Tok::Close { name, .. } = &toks[at] else {
        return false;
    };
    let mut depth: Vec<&str> = Vec::new();
    for t in &toks[..at] {
        match t {
            Tok::Open {
                name: n,
                self_closing,
                ..
            } if !*self_closing && !VOID.contains(&n.as_str()) => depth.push(n),
            Tok::Close { name: n, .. } => {
                if let Some(p) = depth.iter().rposition(|d| d == n) {
                    depth.truncate(p);
                }
            }
            _ => {}
        }
    }
    depth.iter().any(|d| d == name)
}

fn is_block(n: &Node<'_>) -> bool {
    matches!(n, Node::Element { name, .. } if BLOCK.contains(&name.as_str()))
}

fn is_raw(n: &Node<'_>) -> bool {
    matches!(n, Node::Element { name, .. } if RAW.contains(&name.as_str()))
}

/// Whitespace runs as one space (how HTML renders text outside `<pre>`).
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

/// A node on one line (inline content: whitespace collapsed, nothing added).
fn inline(n: &Node<'_>, out: &mut String) {
    match n {
        Node::Text(t) => out.push_str(&collapse(t)),
        Node::Other(t) => out.push_str(t),
        Node::Element {
            open,
            close,
            children,
            ..
        } => {
            out.push_str(open);
            if is_raw(n) {
                for c in children {
                    if let Node::Other(t) | Node::Text(t) = c {
                        out.push_str(t);
                    } else {
                        inline(c, out);
                    }
                }
            } else {
                for c in children {
                    inline(c, out);
                }
            }
            if let Some(c) = close {
                out.push_str(c);
            }
        }
    }
}

/// Whether an element's children may go on lines of their own: it has a block child and every
/// text between children is whitespace only (so breaking lines adds no visible space).
fn breaks_children(children: &[Node<'_>]) -> bool {
    children.iter().any(is_block)
        && children.iter().all(|c| match c {
            Node::Text(t) => t.trim().is_empty(),
            Node::Element { .. } => is_block(c),
            Node::Other(_) => true,
        })
}

fn block(nodes: &[Node<'_>], depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    for n in nodes {
        match n {
            Node::Text(t) if t.trim().is_empty() => {}
            Node::Element {
                open,
                close,
                children,
                ..
            } if !is_raw(n) && breaks_children(children) => {
                out.push_str(&pad);
                out.push_str(open);
                out.push('\n');
                block(children, depth + 1, out);
                if let Some(c) = close {
                    out.push_str(&pad);
                    out.push_str(c);
                    out.push('\n');
                }
            }
            _ => {
                let mut line = String::new();
                inline(n, &mut line);
                out.push_str(&pad);
                out.push_str(line.trim());
                out.push('\n');
            }
        }
    }
}

/// Formats template HTML (see the module docs). Content that isn't only blocks at the top level
/// is formatted as far as it safely can be.
pub fn format_html(html: &str) -> String {
    let nodes = tree(&tokenize(html));
    let top_breaks = nodes.iter().all(|c| match c {
        Node::Text(t) => t.trim().is_empty(),
        _ => true,
    });
    let mut out = String::new();
    if top_breaks {
        block(&nodes, 0, &mut out);
    } else {
        for n in &nodes {
            inline(n, &mut out);
        }
        out = out.trim().to_string();
        out.push('\n');
    }
    out
}

// ---- CSS ---------------------------------------------------------------------------------------

/// Formats template CSS (see the module docs).
pub fn format_css(css: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut seg = String::new();
    let b: Vec<char> = css.chars().collect();
    let mut i = 0;
    let pad = |d: usize| "  ".repeat(d);
    let flush_decl = |seg: &mut String, out: &mut String, depth: usize| {
        let s = collapse(seg).trim().to_string();
        seg.clear();
        if s.is_empty() {
            return;
        }
        out.push_str(&pad(depth));
        out.push_str(&declaration(&s));
        out.push_str(";\n");
    };
    while i < b.len() {
        let c = b[i];
        match c {
            '/' if b.get(i + 1) == Some(&'*') => {
                // A comment, kept as written, on its own line.
                let start = i;
                i += 2;
                while i < b.len() && !(b[i] == '*' && b.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                let comment: String = b[start..i].iter().collect();
                if !seg.trim().is_empty() {
                    seg.push_str(&comment);
                } else {
                    out.push_str(&pad(depth));
                    out.push_str(&comment);
                    out.push('\n');
                }
                continue;
            }
            '"' | '\'' => {
                let start = i;
                i += 1;
                while i < b.len() && b[i] != c {
                    if b[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i = (i + 1).min(b.len());
                seg.extend(&b[start..i]);
                continue;
            }
            '(' => {
                // Parentheses (media queries, functions) as written, up to the matching one.
                let start = i;
                let mut level = 0;
                while i < b.len() {
                    match b[i] {
                        '(' => level += 1,
                        ')' => {
                            level -= 1;
                            if level == 0 {
                                i += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                seg.extend(&b[start..i]);
                continue;
            }
            '{' => {
                let selector = collapse(&seg).trim().to_string();
                seg.clear();
                out.push_str(&pad(depth));
                out.push_str(&selector);
                out.push_str(" {\n");
                depth += 1;
            }
            ';' => flush_decl(&mut seg, &mut out, depth),
            '}' => {
                flush_decl(&mut seg, &mut out, depth);
                depth = depth.saturating_sub(1);
                out.push_str(&pad(depth));
                out.push_str("}\n");
                if depth == 0 {
                    out.push('\n');
                }
            }
            _ => seg.push(c),
        }
        i += 1;
    }
    let rest = collapse(&seg).trim().to_string();
    if !rest.is_empty() {
        out.push_str(&rest);
        out.push('\n');
    }
    let trimmed = out.trim_end();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

/// `prop:value` → `prop: value` (the first colon outside parentheses); custom properties keep
/// their value as written after one space.
fn declaration(s: &str) -> String {
    let mut level = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => level += 1,
            ')' => level -= 1,
            ':' if level == 0 => {
                let (p, v) = (s[..i].trim(), s[i + 1..].trim());
                return if v.is_empty() {
                    format!("{p}:")
                } else {
                    format!("{p}: {v}")
                };
            }
            _ => {}
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests;
