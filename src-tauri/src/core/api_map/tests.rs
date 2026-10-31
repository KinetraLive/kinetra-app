//! Mapping (JSON pointers, CSV), limits and settings validation of API sources.

use std::time::Instant;

use serde_json::json;

use super::*;
use crate::core::model::{ApiCsv, ApiHeader, ApiMappingColumn, ColumnKind};

fn col(key: &str, path: &str) -> ApiMappingColumn {
    ApiMappingColumn {
        key: key.into(),
        label: key.to_uppercase(),
        kind: ColumnKind::Text,
        path: path.into(),
    }
}

fn mapping(rows: &str, cols: &[(&str, &str)]) -> ApiMapping {
    ApiMapping {
        rows: rows.into(),
        columns: cols.iter().map(|(k, p)| col(k, p)).collect(),
        csv: ApiCsv::default(),
    }
}

fn csv_mapping(header: bool, delimiter: CsvDelimiter, cols: &[(&str, &str)]) -> ApiMapping {
    ApiMapping {
        rows: String::new(),
        columns: cols.iter().map(|(k, p)| col(k, p)).collect(),
        csv: ApiCsv { header, delimiter },
    }
}

fn cells(m: &Mapped, key: &str) -> Vec<String> {
    m.rows.iter().map(|r| r[key].clone()).collect()
}

// ---- Pointers and JSON ---------------------------------------------------------------------

#[test]
fn pointer_syntax() {
    for ok in ["", "/", "/a", "/a/0/b", "/a~0b", "/a~1b", "/ spaced"] {
        assert!(is_pointer(ok), "{ok}");
    }
    for bad in ["a", "a/b", "/a~", "/a~2", "~0"] {
        assert!(!is_pointer(bad), "{bad}");
    }
    assert!(!is_pointer(&format!("/{}", "x".repeat(PATH_MAX))));
}

#[test]
fn json_array_rows_with_nested_paths_and_types() {
    let body = json!({
        "data": { "items": [
            { "name": "Ada", "score": 12, "live": true, "team": { "city": "Oslo" },
              "tags": ["a", "b"], "note": null, "a/b": "slash", "t~x": "tilde" },
            { "name": "Grace", "score": 7.5, "live": false, "team": {} },
        ]}
    });
    let m = map_json(
        body.to_string().as_bytes(),
        &mapping(
            "/data/items",
            &[
                ("name", "/name"),
                ("score", "/score"),
                ("live", "/live"),
                ("city", "/team/city"),
                ("tags", "/tags"),
                ("team", "/team"),
                ("note", "/note"),
                ("missing", "/nope"),
                ("slash", "/a~1b"),
                ("tilde", "/t~0x"),
            ],
        ),
    )
    .unwrap();
    assert_eq!(m.total, 2);
    assert_eq!(cells(&m, "name"), ["Ada", "Grace"]);
    assert_eq!(cells(&m, "score"), ["12", "7.5"]);
    assert_eq!(cells(&m, "live"), ["true", "false"]);
    assert_eq!(cells(&m, "city"), ["Oslo", ""]);
    assert_eq!(cells(&m, "tags"), ["[\"a\",\"b\"]", ""]);
    assert_eq!(cells(&m, "team"), ["{\"city\":\"Oslo\"}", "{}"]);
    assert_eq!(cells(&m, "note"), ["", ""]);
    assert_eq!(cells(&m, "missing"), ["", ""]);
    assert_eq!(cells(&m, "slash"), ["slash", ""]);
    assert_eq!(cells(&m, "tilde"), ["tilde", ""]);
    // Every row has every column.
    assert!(m.rows.iter().all(|r| r.len() == 10));
    assert_eq!(m.note(), None);
}

#[test]
fn json_object_is_one_row_and_root_arrays_work() {
    let one = json!({ "match": { "home": "Rosenborg", "goals": 2 } });
    let m = map_json(
        one.to_string().as_bytes(),
        &mapping("/match", &[("home", "/home"), ("goals", "/goals")]),
    )
    .unwrap();
    assert_eq!(m.rows.len(), 1);
    assert_eq!(m.rows[0]["home"], "Rosenborg");
    assert_eq!(m.rows[0]["goals"], "2");
    // The root array of strings: each row is a string, the column path "" is the row itself.
    let m = map_json(br#"["a","b",3]"#, &mapping("", &[("v", "")])).unwrap();
    assert_eq!(cells(&m, "v"), ["a", "b", "3"]);
    // With a BOM.
    let m = map_json(b"\xEF\xBB\xBF[1]", &mapping("", &[("v", "")])).unwrap();
    assert_eq!(cells(&m, "v"), ["1"]);
}

#[test]
fn json_sent_as_text_is_read_through() {
    // A game sends its match data as JSON inside a text field (twice over for the score).
    let teams = json!([
        { "name": "Harbor Hawks", "players": [{ "name": "Ada" }, { "name": "Grace" }] },
        { "name": "Summit Rangers", "players": [{ "name": "Linus" }] },
    ]);
    let score = json!({ "home": 2, "away": 1 }).to_string();
    let match_data = json!({ "teams": teams, "score": score, "note": "{not json" }).to_string();
    let body = json!({ "type": "match", "data": match_data }).to_string();

    // Rows inside the text, columns inside those rows.
    let m = map_json(
        body.as_bytes(),
        &mapping(
            "/data/teams",
            &[
                ("team", "/name"),
                ("first", "/players/0/name"),
                ("n", "/players"),
            ],
        ),
    )
    .unwrap();
    assert_eq!(cells(&m, "team"), ["Harbor Hawks", "Summit Rangers"]);
    assert_eq!(cells(&m, "first"), ["Ada", "Linus"]);
    assert!(m.rows[1]["n"].starts_with("[{"));

    // One row: the text itself, text nested in text, and text that isn't JSON.
    let m = map_json(
        body.as_bytes(),
        &mapping(
            "/data",
            &[
                ("home", "/score/home"),
                ("score", "/score"),
                ("note", "/note"),
                ("bad", "/note/x"),
            ],
        ),
    )
    .unwrap();
    assert_eq!(m.rows[0]["home"], "2");
    assert_eq!(
        m.rows[0]["score"], score,
        "a pointer ending at the text takes it as is"
    );
    assert_eq!(m.rows[0]["note"], "{not json");
    assert_eq!(m.rows[0]["bad"], "");

    // The rows pointer may name the text; the whole row is still its JSON.
    let m = map_json(
        body.as_bytes(),
        &mapping("/data", &[("t", "/teams/1/name")]),
    )
    .unwrap();
    assert_eq!(cells(&m, "t"), ["Summit Rangers"]);
    // A path that goes nowhere, and one through a number, find nothing.
    let doc = json!({ "a": 5, "s": "[1, 2]" });
    let mut cache = Unpacked::default();
    assert!(lookup(&doc, "/a/b", &mut cache).is_none());
    assert!(lookup(&doc, "/x/y", &mut cache).is_none());
    assert_eq!(
        lookup(&doc, "/s/1", &mut cache).unwrap().as_ref(),
        &json!(2)
    );
    assert!(lookup(&doc, "/s/2", &mut cache).is_none());
}

#[test]
fn json_errors() {
    let body = br#"{"items": 5, "list": []}"#;
    let e = map_json(body, &mapping("/nope", &[("a", "/a")])).unwrap_err();
    assert!(e.contains("Nothing at"), "{e}");
    let e = map_json(body, &mapping("/items", &[("a", "/a")])).unwrap_err();
    assert!(e.contains("not a list"), "{e}");
    let e = map_json(b"{not json", &mapping("", &[])).unwrap_err();
    assert!(e.contains("not valid JSON"), "{e}");
    let m = map_json(body, &mapping("/list", &[("a", "/a")])).unwrap();
    assert!(m.rows.is_empty());
}

// ---- CSV -----------------------------------------------------------------------------------

#[test]
fn csv_quotes_escapes_newlines_and_bom() {
    let text = "\u{feff}name,quote,city\r\n\"Lovelace, Ada\",\"She said \"\"hi\"\"\",London\r\nGrace,\"two\nlines\",\r\n\"\",x,y\n";
    let (recs, total) = parse_csv(text, ',', 100);
    assert_eq!(total, 4);
    assert_eq!(recs[0], ["name", "quote", "city"]);
    assert_eq!(recs[1], ["Lovelace, Ada", "She said \"hi\"", "London"]);
    assert_eq!(recs[2], ["Grace", "two\nlines", ""]);
    assert_eq!(recs[3], ["", "x", "y"]);
    let m = map_csv(
        text.as_bytes(),
        &csv_mapping(
            true,
            CsvDelimiter::Auto,
            &[("n", "name"), ("c", " city "), ("q", "quote")],
        ),
    )
    .unwrap();
    assert_eq!(m.total, 3);
    assert_eq!(cells(&m, "n"), ["Lovelace, Ada", "Grace", ""]);
    assert_eq!(cells(&m, "c"), ["London", "", "y"]);
    assert_eq!(m.csv_columns.as_deref().unwrap(), ["name", "quote", "city"]);
}

#[test]
fn csv_edge_cases() {
    // No trailing newline; empty trailing fields; blank lines are skipped.
    let (recs, total) = parse_csv("a,b\n\n1,\n2,3", ',', 100);
    assert_eq!(total, 3);
    assert_eq!(recs, [vec!["a", "b"], vec!["1", ""], vec!["2", "3"]]);
    // A lone CR ends a record too.
    let (recs, _) = parse_csv("a\rb\r", ',', 100);
    assert_eq!(recs, [vec!["a"], vec!["b"]]);
    // A quote inside an unquoted field is literal.
    let (recs, _) = parse_csv("5\" screen,x", ',', 100);
    assert_eq!(recs, [vec!["5\" screen", "x"]]);
    // Only the first `max` records are kept, the rest counted.
    let (recs, total) = parse_csv("1\n2\n3\n", ',', 2);
    assert_eq!((recs.len(), total), (2, 3));
    // Empty input.
    assert_eq!(parse_csv("", ',', 10), (Vec::<Vec<String>>::new(), 0));
}

#[test]
fn csv_delimiter_detection_and_index_columns() {
    assert_eq!(detect_delimiter("a;b;c\n1,2"), ';');
    assert_eq!(detect_delimiter("a\tb\tc"), '\t');
    assert_eq!(detect_delimiter("\"a;b\",c"), ',');
    assert_eq!(detect_delimiter("single"), ',');
    let body = "Ada;12\nGrace;7\n";
    let m = map_csv(
        body.as_bytes(),
        &csv_mapping(
            false,
            CsvDelimiter::Auto,
            &[("name", "0"), ("score", "1"), ("x", "5")],
        ),
    )
    .unwrap();
    assert_eq!(cells(&m, "name"), ["Ada", "Grace"]);
    assert_eq!(cells(&m, "score"), ["12", "7"]);
    assert_eq!(cells(&m, "x"), ["", ""]);
    assert_eq!(m.csv_columns.as_deref().unwrap(), ["0", "1"]);
    // An explicit delimiter wins over detection.
    let m = map_csv(
        b"a;b,c\n1;2,3",
        &csv_mapping(true, CsvDelimiter::Comma, &[("x", "a;b")]),
    )
    .unwrap();
    assert_eq!(cells(&m, "x"), ["1;2"]);
    let m = map_csv(
        b"a\tb\n1\t2",
        &csv_mapping(true, CsvDelimiter::Tab, &[("x", "b")]),
    )
    .unwrap();
    assert_eq!(cells(&m, "x"), ["2"]);
}

// ---- Limits --------------------------------------------------------------------------------

#[test]
fn rows_beyond_the_limit_are_dropped_with_a_note() {
    let items: Vec<_> = (0..1200).map(|i| json!({ "n": i })).collect();
    let m = map_json(
        serde_json::to_vec(&items).unwrap().as_slice(),
        &mapping("", &[("n", "/n")]),
    )
    .unwrap();
    assert_eq!(m.rows.len(), SOURCE_ROWS_MAX);
    assert_eq!(m.total, 1200);
    assert_eq!(m.note().unwrap(), "Showing the first 1000 of 1200 rows");
    let csv: String = std::iter::once("n\n".to_string())
        .chain((0..1500).map(|i| format!("{i}\n")))
        .collect();
    let m = map_csv(
        csv.as_bytes(),
        &csv_mapping(true, CsvDelimiter::Auto, &[("n", "n")]),
    )
    .unwrap();
    assert_eq!((m.rows.len(), m.total), (1000, 1500));
}

#[test]
fn cells_are_cut_at_a_character_boundary() {
    let long = "é".repeat(3000); // 6000 bytes
    let m = map_json(
        json!([{ "v": long }]).to_string().as_bytes(),
        &mapping("", &[("v", "/v")]),
    )
    .unwrap();
    let v = &m.rows[0]["v"];
    assert!(v.len() <= SOURCE_CELL_MAX && v.len() >= SOURCE_CELL_MAX - 1);
    assert!(v.chars().all(|c| c == 'é'));
    let mut rows = vec![[("v".to_string(), long), ("gone".to_string(), "x".into())]
        .into_iter()
        .collect::<Fields>()];
    sanitize_rows(&columns_of(&mapping("", &[("v", "/v")])), &mut rows);
    assert!(rows[0]["v"].len() <= SOURCE_CELL_MAX);
    assert!(!rows[0].contains_key("gone"));
}

#[test]
fn mapping_1000_rows_by_16_columns_is_fast() {
    let cols: Vec<(String, String)> = (0..16)
        .map(|i| (format!("c{i}"), format!("/f{i}")))
        .collect();
    let items: Vec<_> = (0..1000)
        .map(|r| {
            let mut o = serde_json::Map::new();
            for i in 0..16 {
                o.insert(format!("f{i}"), json!(format!("row {r} field {i}")));
            }
            serde_json::Value::Object(o)
        })
        .collect();
    let body = serde_json::to_vec(&json!({ "items": items })).unwrap();
    let refs: Vec<(&str, &str)> = cols.iter().map(|(k, p)| (k.as_str(), p.as_str())).collect();
    let m = mapping("/items", &refs);
    // Warm up, then the best of a few runs.
    map_json(&body, &m).unwrap();
    let mut best = f64::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        let out = map_json(&body, &m).unwrap();
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(out.rows.len(), 1000);
    }
    let csv: String = std::iter::once(
        (0..16)
            .map(|i| format!("f{i}"))
            .collect::<Vec<_>>()
            .join(",")
            + "\n",
    )
    .chain((0..1000).map(|r| {
        (0..16)
            .map(|i| format!("\"row {r}, field {i}\""))
            .collect::<Vec<_>>()
            .join(",")
            + "\n"
    }))
    .collect();
    let cm = csv_mapping(
        true,
        CsvDelimiter::Auto,
        &cols
            .iter()
            .map(|(k, p)| (k.as_str(), &p[1..]))
            .collect::<Vec<_>>(),
    );
    let mut best_csv = f64::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        let out = map_csv(csv.as_bytes(), &cm).unwrap();
        best_csv = best_csv.min(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(out.rows[999]["c15"], "row 999, field 15");
    }
    println!(
        "API mapping 1000 rows x 16 columns: JSON {best:.2} ms, CSV {best_csv:.2} ms ({} build)",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    let bound = if cfg!(debug_assertions) { 400.0 } else { 20.0 };
    assert!(
        best <= bound && best_csv <= bound,
        "JSON {best} ms, CSV {best_csv} ms"
    );
}

// ---- Settings validation -------------------------------------------------------------------

fn request(url: &str) -> ApiRequest {
    ApiRequest {
        url: url.into(),
        method: ApiMethod::Get,
        headers: Vec::new(),
        body: None,
    }
}

fn check(req: &mut ApiRequest, m: &mut ApiMapping, poll: &ApiPoll) -> Result<(), String> {
    validate_settings(req, ApiFormat::Json, m, poll)
}

#[test]
fn settings_validation() {
    let poll = ApiPoll::default();
    let mut m = mapping("/items", &[("name", "/name")]);
    assert!(check(
        &mut request(" https://api.example.com/x?a=1 "),
        &mut m,
        &poll
    )
    .is_ok());
    assert!(
        check(&mut request(""), &mut m, &poll).is_ok(),
        "not configured yet"
    );
    for bad in [
        "ftp://x",
        "file:///c:/x",
        "javascript:alert(1)",
        "http://",
        "example.com",
    ] {
        assert!(check(&mut request(bad), &mut m, &poll).is_err(), "{bad}");
    }
    let long = format!("https://x.com/{}", "a".repeat(API_URL_MAX));
    assert!(check(&mut request(&long), &mut m, &poll).is_err());
    // Interval.
    let fast = ApiPoll {
        interval_ms: 999,
        ..ApiPoll::default()
    };
    assert!(check(&mut request(""), &mut m, &fast)
        .unwrap_err()
        .contains("interval"));
    // Headers.
    let mut r = request("https://x.com");
    r.headers = vec![ApiHeader {
        name: "Bad Name".into(),
        value: Some("v".into()),
        ..Default::default()
    }];
    assert!(check(&mut r, &mut m, &poll).is_err());
    r.headers = vec![ApiHeader {
        name: "X-Key".into(),
        value: Some("a\nb".into()),
        ..Default::default()
    }];
    assert!(check(&mut r, &mut m, &poll).is_err());
    r.headers = vec![ApiHeader {
        name: "Authorization".into(),
        value: Some("leftover".into()),
        secret: Some("abcDEF_123-xyz".into()),
        stored: Some(true),
    }];
    check(&mut r, &mut m, &poll).unwrap();
    assert_eq!(r.headers[0].value, None, "a secret header carries no value");
    assert_eq!(r.headers[0].stored, None, "stored is never saved");
    r.headers[0].secret = Some("../x".into());
    assert!(check(&mut r, &mut m, &poll).is_err());
    r.headers = (0..=API_HEADERS_MAX)
        .map(|i| ApiHeader {
            name: format!("X-{i}"),
            ..Default::default()
        })
        .collect();
    assert!(check(&mut r, &mut m, &poll).is_err());
    // Body: dropped for GET, limited for POST.
    let mut r = request("https://x.com");
    r.body = Some("{}".into());
    check(&mut r, &mut m, &poll).unwrap();
    assert_eq!(r.body, None);
    r.method = ApiMethod::Post;
    r.body = Some("x".repeat(API_BODY_MAX + 1));
    assert!(check(&mut r, &mut m, &poll).is_err());
    // Mapping.
    let mut r = request("");
    assert!(check(&mut r, &mut mapping("items", &[]), &poll).is_err());
    assert!(check(&mut r, &mut mapping("", &[("name", "name")]), &poll).is_err());
    assert!(check(&mut r, &mut mapping("", &[("a", "/a"), ("a", "/b")]), &poll).is_err());
    assert!(check(&mut r, &mut mapping("", &[("Bad", "/a")]), &poll).is_err());
    let many: Vec<(String, String)> = (0..17)
        .map(|i| (format!("c{i}"), "/x".to_string()))
        .collect();
    let refs: Vec<(&str, &str)> = many.iter().map(|(k, p)| (k.as_str(), p.as_str())).collect();
    assert!(check(&mut r, &mut mapping("", &refs), &poll).is_err());
    // CSV: header names, or column numbers without a header row.
    let mut cm = csv_mapping(true, CsvDelimiter::Auto, &[("a", "Team name")]);
    assert!(validate_settings(&mut r, ApiFormat::Csv, &mut cm, &poll).is_ok());
    let mut cm = csv_mapping(false, CsvDelimiter::Auto, &[("a", "x")]);
    assert!(validate_settings(&mut r, ApiFormat::Csv, &mut cm, &poll).is_err());
    let mut cm = csv_mapping(false, CsvDelimiter::Auto, &[("a", "2")]);
    assert!(validate_settings(&mut r, ApiFormat::Csv, &mut cm, &poll).is_ok());
}

#[test]
fn header_tokens() {
    assert!(is_token("X-Api-Key"));
    assert!(is_token("authorization"));
    assert!(!is_token(""));
    assert!(!is_token("X Key"));
    assert!(!is_token("X:Key"));
    assert!(!is_token("Ключ"));
}
