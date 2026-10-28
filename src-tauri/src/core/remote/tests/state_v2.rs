//! Protocol 2 state parts (§4) against the mock relay: variants, data source rows and the
//! Studio take scope, each only when the server announces its feature.

use super::*;
use crate::core::model::{ColumnKind, LiveEdit, SourceColumn, SourceKind, SourceStep, UiState};
use crate::core::protocol::VariantTransition;

/// A protocol 2 mock announcing `features`, with a text source "Speakers" (three rows) whose
/// `name` column feeds `l1`'s `title`. Returns the source id.
pub(super) fn setup(features: &'static [&'static str]) -> (Env, String) {
    let e = env();
    e.cfg(|c| {
        c.protocols = &["kinetra-control.v2", "kinetra-control.v1"];
        c.features = features;
    });
    let mut doc = e.st.create_source("Speakers").unwrap();
    let col = |key: &str, kind| SourceColumn {
        key: key.into(),
        label: key.into(),
        kind,
    };
    let row = |name: &str, role: &str| {
        Fields::from([
            ("name".to_string(), name.to_string()),
            ("photo".to_string(), "/assets/ingrid.png".to_string()),
            ("role".to_string(), role.to_string()),
        ])
    };
    doc.meta.kind = SourceKind::Text {
        columns: vec![
            col("name", ColumnKind::Text),
            col("photo", ColumnKind::Image),
            col("role", ColumnKind::Text),
        ],
        rows: vec![
            row("Ingrid Vale", "Host"),
            row("Kari Nordmann", "Guest"),
            row("", ""),
        ],
        index: 0,
        auto: None,
    };
    let sid = e.st.save_source(doc).unwrap().id;
    let mut screen = e.st.screens().doc(&e.screen).unwrap();
    for l in &mut screen.meta.layers {
        if let ScreenLayerKind::Template { bindings, .. } = &mut l.kind {
            for b in bindings.values_mut() {
                b.source_id = sid.clone();
                b.column = "name".into();
            }
        }
    }
    e.st.save_screen(screen).unwrap();
    (e, sid)
}

pub(super) const ALL: &[&str] = &["permissions", "variants", "rows", "studio", "timers"];

#[test]
fn v2_parts_are_in_the_tree_when_announced() {
    let (e, sid) = setup(ALL);
    let tree = e.live();
    state::check_refs(&tree).unwrap();

    // Variants: the template's looks, and the one each layer shows.
    let l1 = &tree["layers"]["l1"];
    let t = &tree["templates"][l1["template"].as_str().unwrap()];
    let variants = t["variants"].as_array().unwrap();
    assert!(variants.len() > 1, "{t}");
    assert!(variants
        .iter()
        .all(|v| v["id"].is_string() && v["name"].is_string()));
    assert!(variants.iter().any(|v| v["id"] == l1["variant"]));
    assert!(l1.get("variant_next").is_none());

    // Rows: the bound source with every row, labels from the first two text columns.
    assert_eq!(l1["sources"], json!([sid]));
    assert_eq!(tree["layers"]["l2"]["sources"], json!([]));
    assert_eq!(tree["source_order"], json!([sid]));
    let s = &tree["sources"][&sid];
    assert_eq!(s["name"], "Speakers");
    assert_eq!(s["truncated"], false);
    let labels: Vec<&str> = s["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        labels,
        ["Ingrid Vale · Host", "Kari Nordmann · Guest", "Row 3"]
    );
    let ids: Vec<Value> = s["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].clone())
        .collect();
    assert_eq!(s["current"], ids[0]);

    // Studio: take is per screen.
    assert_eq!(tree["studio"], json!({"take_scope": "screen"}));

    // A step moves `current`; the ids stay the rows' own.
    e.st.step_source(&sid, SourceStep::By { delta: 1 }).unwrap();
    wait_until("current", || e.tree()["sources"][&sid]["current"] == ids[1]);
    let after: Vec<Value> = e.tree()["sources"][&sid]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].clone())
        .collect();
    assert_eq!(after, ids);
}

#[test]
fn unannounced_parts_never_reach_the_server() {
    for (protocols, features) in [
        (
            &["kinetra-control.v2", "kinetra-control.v1"][..],
            &["permissions"][..],
        ),
        (&["kinetra-control.v1"][..], ALL),
    ] {
        let (e, _) = setup(features);
        e.cfg(|c| c.protocols = protocols);
        let tree = e.live();
        assert!(tree.get("sources").is_none(), "{tree}");
        assert!(tree.get("source_order").is_none());
        assert!(tree.get("studio").is_none());
        for l in tree["layers"].as_object().unwrap().values() {
            for k in ["variant", "variant_next", "sources"] {
                assert!(l.get(k).is_none(), "{k} in {l}");
            }
        }
        for t in tree["templates"].as_object().unwrap().values() {
            assert!(t.get("variants").is_none(), "{t}");
        }
    }
}

#[test]
fn a_variant_staged_for_the_next_take_is_variant_next() {
    let (e, _) = setup(ALL);
    let mut ui: UiState = e.st.ui_state().unwrap();
    ui.control.live_edit = LiveEdit::OnTake;
    e.st.set_ui_state(ui).unwrap();
    let tree = e.live();
    let l1 = tree["layers"]["l1"].clone();
    let shown = l1["variant"].as_str().unwrap().to_string();
    let other = tree["templates"][l1["template"].as_str().unwrap()]["variants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["id"].as_str().unwrap().to_string())
        .find(|v| *v != shown)
        .unwrap();

    // Off air: changed at once.
    e.st.set_layer_variant(&e.screen, "l1", &other, VariantTransition::Crossfade)
        .unwrap();
    wait_until("variant", || e.tree()["layers"]["l1"]["variant"] == other);
    assert!(e.tree()["layers"]["l1"].get("variant_next").is_none());

    // On air under "On take": the output keeps its look, the change waits.
    e.st.live().show(&e.screen, "l1").unwrap();
    e.st.set_layer_variant(&e.screen, "l1", &shown, VariantTransition::Crossfade)
        .unwrap();
    wait_until("variant_next", || {
        e.tree()["layers"]["l1"]["variant_next"] == shown
    });
    assert_eq!(e.tree()["layers"]["l1"]["variant"], other);
    state::check_refs(&e.tree()).unwrap();

    // The take applies it.
    e.st.live().show(&e.screen, "l1").unwrap();
    wait_until("taken", || {
        let l = &e.tree()["layers"]["l1"];
        l["variant"] == shown && l.get("variant_next").is_none()
    });
}
