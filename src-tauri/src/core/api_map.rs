//! API sources: the settings' validation and the mapping of a response body into table
//! rows (the data table's model: column key → cell text).
//!
//! - **JSON:** `mapping.rows` is a JSON Pointer (RFC 6901, `""` = the root) to an array (one row
//!   per element) or an object (one row). Each column's `path` is a pointer *within* the row
//!   (`""` = the row itself). Strings are taken as is, numbers and booleans as their text, `null`
//!   and missing values as `""`, arrays and objects as compact JSON. **JSON sent as text:** a
//!   pointer goes on into a string that holds a JSON object or list (a game sending
//!   `"match": "{\"teams\": …}"`), and the rows pointer may name such a string; a pointer that
//!   ends at the string takes its text as is (`lookup`, `Unpacked`).
//! - **CSV:** our own small RFC 4180 parser: quoted fields, `""` escapes, CRLF or LF, a UTF-8
//!   BOM. The delimiter is `,` `;` or tab, detected from the first line (outside quotes) with
//!   `auto`. With `header`, the first record names the columns and a column's `path` is a header
//!   name; without, `path` is the 0-based column index.
//! - **Limits:** the body is at most `API_RESPONSE_MAX_BYTES` (the poller enforces it while
//!   reading); at most `SOURCE_ROWS_MAX` rows are kept (the rest is dropped with a status
//!   message), cells are cut to `SOURCE_CELL_MAX` bytes (at a character boundary).
//!
//! Pure functions: the poller runs them on the blocking pool.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use serde_json::Value;

use crate::core::model::{
    ApiFormat, ApiMapping, ApiMethod, ApiPoll, ApiRequest, CsvDelimiter, SourceColumn,
    API_BODY_MAX, API_HEADERS_MAX, API_HEADER_VALUE_MAX, API_POLL_MAX_MS, API_POLL_MIN_MS,
    API_URL_MAX, SOURCE_CELL_MAX, SOURCE_COLUMNS_MAX, SOURCE_ROWS_MAX,
};
use crate::core::protocol::Fields;
use crate::core::templates::{is_id, is_key};

/// Longest header name and mapping path, in bytes.
pub const NAME_MAX: usize = 256;
pub const PATH_MAX: usize = 512;
/// Longest column label, in characters (as for data tables).
const LABEL_MAX: usize = 100;

// ---- Validation ------------------------------------------------------------------------------

/// A JSON Pointer (RFC 6901): `""` or `/`-separated tokens where `~` is followed by `0` or `1`.
pub fn is_pointer(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if !s.starts_with('/') || s.len() > PATH_MAX {
        return false;
    }
    let b = s.as_bytes();
    b.iter()
        .enumerate()
        .all(|(i, c)| *c != b'~' || matches!(b.get(i + 1), Some(b'0' | b'1')))
}

/// An HTTP header name (RFC 9110 `token`).
pub fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= NAME_MAX
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// An http(s) URL (or empty: not configured yet).
pub fn check_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Ok(());
    }
    if url.len() > API_URL_MAX {
        return Err(format!(
            "The URL is too long (at most {API_URL_MAX} characters)"
        ));
    }
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err("The URL must start with http:// or https://".into());
    }
    match reqwest::Url::parse(url) {
        Ok(u) if u.host_str().is_some_and(|h| !h.is_empty()) => Ok(()),
        _ => Err("The URL is not valid".into()),
    }
}

/// Hard checks of an API source's settings; normalizes in place: the URL is trimmed, a secret
/// header carries no value, `stored` is cleared (server-set, never saved), labels are trimmed.
pub fn validate_settings(
    request: &mut ApiRequest,
    format: ApiFormat,
    mapping: &mut ApiMapping,
    poll: &ApiPoll,
) -> Result<(), String> {
    request.url = request.url.trim().to_string();
    check_url(&request.url)?;
    if request.headers.len() > API_HEADERS_MAX {
        return Err(format!("At most {API_HEADERS_MAX} headers"));
    }
    for h in &mut request.headers {
        h.name = h.name.trim().to_string();
        if !is_token(&h.name) {
            return Err(format!(
                "Invalid header name \"{}\": use letters, digits and !#$%&'*+-.^_`|~",
                h.name
            ));
        }
        h.stored = None;
        if let Some(id) = &h.secret {
            if !is_id(id) {
                return Err(format!(
                    "Header {}: the secret is not valid; enter the value again",
                    h.name
                ));
            }
            h.value = None;
        }
        if let Some(v) = &h.value {
            if v.len() > API_HEADER_VALUE_MAX {
                return Err(format!(
                    "Header {}: the value is too long (at most {} KB)",
                    h.name,
                    API_HEADER_VALUE_MAX / 1024
                ));
            }
            if v.chars().any(|c| c.is_control() && c != '\t') {
                return Err(format!("Header {}: the value has a line break", h.name));
            }
        }
    }
    if let Some(body) = &request.body {
        if body.len() > API_BODY_MAX {
            return Err(format!(
                "The request body is too long (at most {} KB)",
                API_BODY_MAX / 1024
            ));
        }
    }
    if request.method == ApiMethod::Get {
        request.body = None;
    }
    if !(API_POLL_MIN_MS..=API_POLL_MAX_MS).contains(&poll.interval_ms) {
        return Err(format!(
            "The poll interval must be between {} second and {} minutes",
            API_POLL_MIN_MS / 1000,
            API_POLL_MAX_MS / 60_000
        ));
    }
    validate_mapping(format, mapping)
}

/// Hard checks of a mapping (API and live data sources); labels are trimmed in place.
pub fn validate_mapping(format: ApiFormat, mapping: &mut ApiMapping) -> Result<(), String> {
    if format == ApiFormat::Json && !is_pointer(&mapping.rows) {
        return Err(format!(
            "Rows: \"{}\" is not a JSON Pointer (e.g. /items, or empty for the whole response)",
            mapping.rows
        ));
    }
    if mapping.columns.len() > SOURCE_COLUMNS_MAX {
        return Err(format!("At most {SOURCE_COLUMNS_MAX} columns"));
    }
    let mut keys = HashSet::new();
    for c in &mut mapping.columns {
        if !is_key(&c.key) {
            return Err(format!(
                "Invalid column key \"{}\": use a-z, 0-9, _ and -, starting with a letter",
                c.key
            ));
        }
        if !keys.insert(c.key.clone()) {
            return Err(format!("Duplicate column key: {}", c.key));
        }
        let label = c.label.trim();
        if label.is_empty() || label.chars().count() > LABEL_MAX {
            return Err(format!("Column labels must be 1 to {LABEL_MAX} characters"));
        }
        c.label = label.to_string();
        let ok = match format {
            ApiFormat::Json => is_pointer(&c.path),
            ApiFormat::Csv if mapping.csv.header => c.path.len() <= PATH_MAX,
            ApiFormat::Csv => c.path.parse::<u16>().is_ok(),
        };
        if !ok {
            return Err(match format {
                ApiFormat::Json => format!(
                    "Column {}: \"{}\" is not a JSON Pointer (e.g. /name)",
                    c.key, c.path
                ),
                ApiFormat::Csv => format!(
                    "Column {}: without a header row, the field is a column number (0, 1, ...)",
                    c.key
                ),
            });
        }
    }
    Ok(())
}

/// The table columns of a mapping.
pub fn columns_of(mapping: &ApiMapping) -> Vec<SourceColumn> {
    mapping
        .columns
        .iter()
        .map(|c| SourceColumn {
            key: c.key.clone(),
            label: c.label.clone(),
            kind: c.kind,
        })
        .collect()
}

/// Cuts `s` to at most `SOURCE_CELL_MAX` bytes, at a character boundary.
pub fn cut_cell(mut s: String) -> String {
    if s.len() > SOURCE_CELL_MAX {
        let mut end = SOURCE_CELL_MAX;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

/// Cached rows after a mapping change: unknown keys dropped, cells cut, at most
/// `SOURCE_ROWS_MAX` rows (the cache is server-made; it never fails validation).
pub fn sanitize_rows(columns: &[SourceColumn], rows: &mut Vec<Fields>) {
    rows.truncate(SOURCE_ROWS_MAX);
    let keys: HashSet<&str> = columns.iter().map(|c| c.key.as_str()).collect();
    for row in rows.iter_mut() {
        row.retain(|k, _| keys.contains(k.as_str()));
        for v in row.values_mut() {
            if v.len() > SOURCE_CELL_MAX {
                *v = cut_cell(std::mem::take(v));
            }
        }
    }
}

// ---- Mapping ---------------------------------------------------------------------------------

/// Mapped rows and how many the response had.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mapped {
    pub rows: Vec<Fields>,
    /// Rows in the response (more than `rows.len()` when cut to `SOURCE_ROWS_MAX`).
    pub total: usize,
    /// CSV: the header names (or "0".."n-1" without a header row).
    pub csv_columns: Option<Vec<String>>,
}

impl Mapped {
    /// The status note when rows were dropped.
    pub fn note(&self) -> Option<String> {
        (self.total > self.rows.len()).then(|| {
            format!(
                "Showing the first {} of {} rows",
                self.rows.len(),
                self.total
            )
        })
    }
}

/// A JSON value as cell text: strings as is, numbers and booleans as text, null as "", arrays
/// and objects as compact JSON.
pub fn json_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(_) | Value::Object(_) => v.to_string(),
    }
}

/// Body bytes as text: UTF-8 (invalid sequences replaced), without a BOM.
pub fn body_text(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8_lossy(bytes)
}

/// Parses a JSON body.
pub fn parse_json(bytes: &[u8]) -> Result<Value, String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    serde_json::from_slice(bytes).map_err(|e| format!("The response is not valid JSON: {e}"))
}

/// A JSON object or list sent as text: `s` parsed when it holds one, else None.
pub fn unpack(s: &str) -> Option<Value> {
    let t = s.trim_start();
    if !t.starts_with(['{', '[']) {
        return None;
    }
    serde_json::from_str::<Value>(t)
        .ok()
        .filter(|v| v.is_object() || v.is_array())
}

/// The strings of one document that held JSON, parsed once each (every column of a row often
/// reads the same one). Keyed by the text's address, which stays put while the document and
/// this cache live; use one per document.
#[derive(Default)]
pub struct Unpacked(HashMap<usize, Option<Rc<Value>>>);

impl Unpacked {
    fn get(&mut self, s: &str) -> Option<Rc<Value>> {
        self.0
            .entry(s.as_ptr() as usize)
            .or_insert_with(|| unpack(s).map(Rc::new))
            .clone()
    }
}

/// The value at a JSON Pointer, going on into strings that hold JSON (see the module docs).
pub fn lookup<'a>(v: &'a Value, ptr: &str, cache: &mut Unpacked) -> Option<Cow<'a, Value>> {
    if let Some(x) = v.pointer(ptr) {
        return Some(Cow::Borrowed(x));
    }
    // The longest prefix that leads somewhere: the pointer can only go on if that is text.
    let mut end = ptr.len();
    while let Some(i) = ptr[..end].rfind('/') {
        match v.pointer(&ptr[..i]) {
            Some(Value::String(s)) => {
                let inner = cache.get(s)?;
                return lookup(&inner, &ptr[i..], cache).map(|c| Cow::Owned(c.into_owned()));
            }
            Some(_) => return None,
            None => end = i,
        }
    }
    None
}

/// Maps a parsed JSON document (see the module docs).
pub fn map_json_value(doc: &Value, mapping: &ApiMapping) -> Result<Mapped, String> {
    let mut cache = Unpacked::default();
    let mut at = lookup(doc, &mapping.rows, &mut cache).ok_or_else(|| {
        format!(
            "Nothing at the rows pointer \"{}\" in the response",
            mapping.rows
        )
    })?;
    let inner = match at.as_ref() {
        Value::String(s) => cache.get(s),
        _ => None,
    };
    if let Some(v) = inner {
        at = Cow::Owned(Rc::unwrap_or_clone(v));
    }
    let items: &[Value] = match at.as_ref() {
        Value::Array(a) => a,
        Value::Object(_) => std::slice::from_ref(at.as_ref()),
        _ => {
            return Err(format!(
                "The rows pointer \"{}\" points to a value, not a list or an object",
                mapping.rows
            ))
        }
    };
    let rows = items
        .iter()
        .take(SOURCE_ROWS_MAX)
        .map(|item| {
            mapping
                .columns
                .iter()
                .map(|c| {
                    let v = lookup(item, &c.path, &mut cache)
                        .map(|v| json_text(&v))
                        .unwrap_or_default();
                    (c.key.clone(), cut_cell(v))
                })
                .collect()
        })
        .collect();
    Ok(Mapped {
        rows,
        total: items.len(),
        csv_columns: None,
    })
}

pub fn map_json(bytes: &[u8], mapping: &ApiMapping) -> Result<Mapped, String> {
    map_json_value(&parse_json(bytes)?, mapping)
}

/// The delimiter of `auto`: the most frequent of `,` `;` tab in the first line (outside
/// quotes); `,` on a tie or none.
pub fn detect_delimiter(text: &str) -> char {
    let mut counts = [0usize; 3];
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' | '\r' if !quoted => break,
            ',' if !quoted => counts[0] += 1,
            ';' if !quoted => counts[1] += 1,
            '\t' if !quoted => counts[2] += 1,
            _ => {}
        }
    }
    let best = [',', ';', '\t']
        .into_iter()
        .zip(counts)
        .fold((',', 0), |acc, (c, n)| if n > acc.1 { (c, n) } else { acc });
    best.0
}

/// RFC 4180 records. Stops after `max_records` (the rest is only counted). Returns the records
/// and the total number of records.
pub fn parse_csv(text: &str, delim: char, max_records: usize) -> (Vec<Vec<String>>, usize) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut records = Vec::new();
    let mut total = 0usize;
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    // Whether the current record has anything (so a trailing newline adds no empty record).
    let mut started = false;
    let mut chars = text.chars().peekable();
    let mut finish = |record: &mut Vec<String>, field: &mut String, total: &mut usize| {
        record.push(std::mem::take(field));
        *total += 1;
        let r = std::mem::take(record);
        if records.len() < max_records {
            records.push(r);
        }
    };
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                quoted = true;
                started = true;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                if started || !field.is_empty() || !record.is_empty() {
                    finish(&mut record, &mut field, &mut total);
                }
                started = false;
            }
            '\n' => {
                if started || !field.is_empty() || !record.is_empty() {
                    finish(&mut record, &mut field, &mut total);
                }
                started = false;
            }
            c if c == delim => {
                record.push(std::mem::take(&mut field));
                started = true;
            }
            c => {
                field.push(c);
                started = true;
            }
        }
    }
    if started || !field.is_empty() || !record.is_empty() {
        finish(&mut record, &mut field, &mut total);
    }
    (records, total)
}

fn delimiter_of(d: CsvDelimiter, text: &str) -> char {
    match d {
        CsvDelimiter::Auto => detect_delimiter(text.strip_prefix('\u{feff}').unwrap_or(text)),
        CsvDelimiter::Comma => ',',
        CsvDelimiter::Semicolon => ';',
        CsvDelimiter::Tab => '\t',
    }
}

/// Maps a CSV body (see the module docs).
pub fn map_csv(bytes: &[u8], mapping: &ApiMapping) -> Result<Mapped, String> {
    let text = body_text(bytes);
    let delim = delimiter_of(mapping.csv.delimiter, &text);
    let header = mapping.csv.header;
    let (mut records, total) = parse_csv(&text, delim, SOURCE_ROWS_MAX + usize::from(header));
    let (names, total) = if header {
        if records.is_empty() {
            (Vec::new(), 0)
        } else {
            let names: Vec<String> = records
                .remove(0)
                .into_iter()
                .map(|n| n.trim().to_string())
                .collect();
            (names, total - 1)
        }
    } else {
        let width = records.iter().map(Vec::len).max().unwrap_or(0);
        ((0..width).map(|i| i.to_string()).collect(), total)
    };
    // Column → field index.
    let index: Vec<Option<usize>> = mapping
        .columns
        .iter()
        .map(|c| {
            if header {
                names.iter().position(|n| *n == c.path.trim())
            } else {
                c.path.parse::<usize>().ok()
            }
        })
        .collect();
    let rows = records
        .iter()
        .map(|r| {
            mapping
                .columns
                .iter()
                .zip(&index)
                .map(|(c, i)| {
                    let v = i.and_then(|i| r.get(i)).cloned().unwrap_or_default();
                    (c.key.clone(), cut_cell(v))
                })
                .collect()
        })
        .collect();
    Ok(Mapped {
        rows,
        total,
        csv_columns: Some(names),
    })
}

/// Maps a response body by format.
pub fn map_body(format: ApiFormat, bytes: &[u8], mapping: &ApiMapping) -> Result<Mapped, String> {
    match format {
        ApiFormat::Json => map_json(bytes, mapping),
        ApiFormat::Csv => map_csv(bytes, mapping),
    }
}

#[cfg(test)]
mod tests;
