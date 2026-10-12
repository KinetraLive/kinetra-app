use serde_json::{json, Value};

use super::*;

fn cols(keys: &[&str]) -> Vec<SourceColumn> {
    keys.iter()
        .map(|k| SourceColumn {
            key: k.to_string(),
            label: k.to_uppercase(),
            kind: Default::default(),
        })
        .collect()
}

fn row(pairs: &[(&str, &str)]) -> Fields {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn doc(id: &str, rows: Vec<Fields>, index: u32) -> SourceDoc {
    SourceDoc {
        id: id.into(),
        meta: SourceMeta {
            name: "Guests".into(),
            kind: SourceKind::Text {
                columns: cols(&["name", "title"]),
                rows,
                index,
                auto: None,
            },
            updated_at: String::new(),
        },
    }
}

fn index_of(d: &SourceDoc) -> u32 {
    let SourceKind::Text { index, .. } = &d.meta.kind else {
        panic!("not a text source")
    };
    *index
}

#[test]
fn create_save_get_round_trip_and_file_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    let created = create(p, "  Guests ").unwrap();
    assert_eq!(created.meta.name, "Guests");
    assert_eq!(created.id.len(), 10);
    let SourceKind::Text {
        columns,
        rows,
        index,
        ..
    } = &created.meta.kind
    else {
        panic!("not a text source")
    };
    assert_eq!(
        columns,
        &[SourceColumn {
            key: "text".into(),
            label: "Text".into(),
            kind: Default::default(),
        }]
    );
    assert!(rows.is_empty());
    assert_eq!(*index, 0);
    assert!(!created.meta.updated_at.is_empty());

    let mut d = doc(
        &created.id,
        vec![
            row(&[("name", "Ada"), ("title", "Analyst")]),
            row(&[("name", "Grace")]),
        ],
        1,
    );
    d.meta.updated_at = created.meta.updated_at.clone();
    let saved = save(p, d.clone(), false).unwrap();
    assert_eq!(get(p, &created.id).unwrap(), saved);

    let text = fs::read_to_string(p.join("sources").join(format!("{}.json", created.id))).unwrap();
    assert!(text.trim_start().starts_with("{\n  \"schemaVersion\": 1,"));
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["kind"], "text");
    assert_eq!(v["name"], "Guests");
    assert_eq!(v["index"], 1);
    assert_eq!(v["columns"][0], json!({"key": "name", "label": "NAME"}));
    assert_eq!(v["rows"][1], json!({"name": "Grace"}));
    assert!(v.get("id").is_none(), "the id is the file name");

    // Unchanged: not written, updatedAt kept.
    let again = save(p, saved.clone(), false).unwrap();
    assert_eq!(again.meta.updated_at, saved.meta.updated_at);
    // `create: false` needs the file.
    assert!(matches!(
        save(p, doc("nosuch0001", Vec::new(), 0), false),
        Err(SourceError::NotFound(_))
    ));

    // Summary: current row with every column, row count, index.
    let s = summary(&saved, vec!["scr".into()], &now());
    assert_eq!(s.row_count, 2);
    assert_eq!(s.index, 1);
    assert_eq!(s.current, row(&[("name", "Grace"), ("title", "")]));
    assert_eq!(s.used_by, ["scr"]);
    assert_eq!(s.kind, SourceKindName::Text);

    let copy = duplicate(p, saved.clone(), "Copy").unwrap();
    assert_ne!(copy.id, saved.id);
    assert_eq!(copy.meta.kind, saved.meta.kind);
    delete(p, &saved.id).unwrap();
    assert!(matches!(get(p, &saved.id), Err(SourceError::NotFound(_))));
    assert!(matches!(
        delete(p, &saved.id),
        Err(SourceError::NotFound(_))
    ));
}

#[test]
fn every_validation_rule() {
    let ok = doc("good000001", vec![row(&[("name", "A")])], 0);
    assert!(validate(ok.clone()).is_ok());
    let bad = |f: &dyn Fn(&mut SourceDoc), what: &str| {
        let mut d = ok.clone();
        f(&mut d);
        assert!(
            matches!(validate(d), Err(SourceError::Invalid(_))),
            "{what} must be rejected"
        );
    };
    bad(&|d| d.id = "../x".into(), "bad id");
    bad(&|d| d.meta.name = "  ".into(), "empty name");
    bad(&|d| d.meta.name = "x".repeat(101), "long name");
    bad(&|d| kind(d).0.clear(), "no columns");
    bad(
        &|d| {
            *kind(d).0 = cols(&[
                "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q",
            ])
        },
        "17 columns",
    );
    bad(&|d| kind(d).0[0].key = "Name".into(), "bad key");
    bad(&|d| kind(d).0[1].key = "name".into(), "duplicate key");
    bad(&|d| kind(d).0[0].label = " ".into(), "empty label");
    bad(&|d| kind(d).0[0].label = "x".repeat(101), "long label");
    bad(
        &|d| kind(d).1.push(row(&[("ghost", "x")])),
        "cell for an unknown column",
    );
    bad(
        &|d| {
            kind(d)
                .1
                .push(row(&[("name", &"x".repeat(SOURCE_CELL_MAX + 1))]))
        },
        "long cell",
    );
    bad(
        &|d| *kind(d).1 = vec![Fields::new(); SOURCE_ROWS_MAX + 1],
        "too many rows",
    );

    // Trimmed, index clamped.
    let mut d = ok.clone();
    d.meta.name = " Show ".into();
    kind(&mut d).0[0].label = " Name ".into();
    let SourceKind::Text { index, .. } = &mut d.meta.kind else {
        panic!("not a text source")
    };
    *index = 7;
    let v = validate(d).unwrap();
    assert_eq!(v.meta.name, "Show");
    let SourceKind::Text { columns, .. } = &v.meta.kind else {
        panic!("not a text source")
    };
    assert_eq!(columns[0].label, "Name");
    assert_eq!(index_of(&v), 0, "one row: clamped to 0");
    // Exactly the limits pass.
    let mut d = ok.clone();
    *kind(&mut d).1 = vec![row(&[("name", &"x".repeat(SOURCE_CELL_MAX))]); SOURCE_ROWS_MAX];
    assert!(validate(d).is_ok());
}

#[test]
fn steps_are_clamped_without_wrap() {
    let by = |delta| SourceStep::By { delta };
    let to = |index| SourceStep::Index { index };
    assert_eq!(stepped(0, 3, by(1)), 1);
    assert_eq!(stepped(2, 3, by(1)), 2, "no wrap at the end");
    assert_eq!(stepped(0, 3, by(-1)), 0, "no wrap at the start");
    assert_eq!(stepped(1, 3, by(-5)), 0);
    assert_eq!(stepped(1, 3, by(i32::MAX)), 2);
    assert_eq!(stepped(1, 3, to(0)), 0);
    assert_eq!(stepped(1, 3, to(99)), 2);
    assert_eq!(stepped(0, 0, by(1)), 0, "no rows");
    assert_eq!(stepped(0, 0, to(3)), 0);
    assert_eq!(clamp_index(5, 0), 0);
    assert_eq!(clamp_index(5, 2), 1);
}

#[test]
fn current_rows_for_plans() {
    let d = doc("s000000001", vec![row(&[("name", "Ada")])], 0);
    let c = Current::of(&d, &now());
    assert_eq!(c.columns, ["name", "title"]);
    assert_eq!(c.row, Some(row(&[("name", "Ada"), ("title", "")])));
    let empty = doc("s000000002", Vec::new(), 0);
    assert_eq!(Current::of(&empty, &now()).row, None);
    assert!(current_row(&empty, &now()).is_empty());
}

#[test]
fn newer_files_are_never_overwritten_and_corrupt_ones_set_aside_on_load() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    fs::create_dir_all(p.join("sources")).unwrap();
    let newer = r#"{"schemaVersion": 9, "name": "Future", "kind": "text", "columns": []}"#;
    fs::write(p.join("sources/newer00001.json"), newer).unwrap();
    fs::write(p.join("sources/broken0001.json"), "{ nope").unwrap();
    let good = create(p, "Good").unwrap();

    let d = doc("newer00001", Vec::new(), 0);
    assert!(matches!(
        save(p, d.clone(), false),
        Err(SourceError::Persist(PersistError::NewerVersion { .. }))
    ));
    assert!(!write_index(p, &d).unwrap(), "index write skipped");
    assert_eq!(
        fs::read_to_string(p.join("sources/newer00001.json")).unwrap(),
        newer
    );

    let (docs, recovered) = load_all(p);
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].id, good.id);
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0]
        .quarantined_as
        .contains("broken0001.json.corrupt-"));
    assert!(!p.join("sources/broken0001.json").exists());
    assert!(p.join("sources/newer00001.json").exists(), "left alone");
}

#[test]
fn store_steps_in_memory_and_tracks_dirty() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SourceStore::default();
    assert!(store.take_dirty().is_none());
    let d = doc(
        "s000000001",
        vec![row(&[("name", "A")]), row(&[("name", "B")])],
        0,
    );
    store.install(tmp.path(), vec![d.clone()]);
    assert!(store.step("nope", SourceStep::By { delta: 1 }).is_none());
    let (after, changed) = store.step(&d.id, SourceStep::By { delta: 1 }).unwrap();
    assert!(changed);
    assert_eq!(index_of(&after), 1);
    let (_, changed) = store.step(&d.id, SourceStep::By { delta: 1 }).unwrap();
    assert!(!changed, "clamped: same index");
    assert!(store.is_dirty(&d.id));
    assert_eq!(index_of(&store.doc(&d.id).unwrap()), 1);
    let (dir, docs) = store.take_dirty().unwrap();
    assert_eq!(dir, tmp.path());
    assert_eq!(docs.len(), 1);
    assert!(!store.is_dirty(&d.id));
    store.mark_dirty(&d.id);
    store.put(d.clone());
    assert!(!store.is_dirty(&d.id), "a written document is clean");
    let current = store.current(["s000000001", "missing"]);
    assert_eq!(current.len(), 1);
    store.forget();
    assert!(store.doc(&d.id).is_none());
}

fn kind(d: &mut SourceDoc) -> (&mut Vec<SourceColumn>, &mut Vec<Fields>) {
    let SourceKind::Text { columns, rows, .. } = &mut d.meta.kind else {
        panic!("not a text source")
    };
    (columns, rows)
}

fn now() -> crate::core::localtime::Now {
    crate::core::localtime::Now::at(0, crate::core::localtime::Zone::Fixed(0))
}
