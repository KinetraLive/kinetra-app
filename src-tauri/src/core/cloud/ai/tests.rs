//! AI input checks (what the core refuses before anything is sent or spent).

use super::*;

const M: &str = "857a78b7a23b14cd718a9cbf9b90b0577225543ddcce5ed21854a4985f25ee27";

fn input(prompt: &str) -> AiGenerateInput {
    AiGenerateInput {
        prompt: prompt.into(),
        ..AiGenerateInput::default()
    }
}

#[test]
fn a_minimal_request_has_machine_and_prompt_only() {
    let b = generate_body(&input("  Lower third for a football match "), M).unwrap();
    assert_eq!(
        b,
        json!({"machine": M, "prompt": "Lower third for a football match"})
    );
}

#[test]
fn every_optional_field_is_checked() {
    let full = AiGenerateInput {
        prompt: "Score bug".into(),
        template_type: Some("scoreboard".into()),
        brand_colors: vec!["#C8102E".into(), "#ffffff".into()],
        fonts: vec!["Barlow Condensed".into(), "Barlow".into()],
        width: Some(960),
        height: Some(200),
    };
    let b = generate_body(&full, M).unwrap();
    assert_eq!(b["brand_colors"], json!(["#c8102e", "#ffffff"]));
    assert_eq!(b["size"], json!({"width": 960, "height": 200}));
    assert_eq!(b["template_type"], "scoreboard");

    let bad = |f: &dyn Fn(&mut AiGenerateInput)| {
        let mut i = full.clone();
        f(&mut i);
        generate_body(&i, M).unwrap_err().to_string()
    };
    assert!(bad(&|i| i.prompt = "   ".into()).contains("Describe"));
    assert!(bad(&|i| i.prompt = "x".repeat(AI_MAX_INPUT_CHARS + 1)).contains("2000"));
    // 2000 characters (not bytes) are fine, also with multi-byte letters.
    let mut ok = full.clone();
    ok.prompt = "ø".repeat(AI_MAX_INPUT_CHARS);
    assert!(generate_body(&ok, M).is_ok());
    assert!(bad(&|i| i.template_type = Some("poster".into())).contains("type"));
    assert!(bad(&|i| i.brand_colors = vec!["#fff".into(); 5]).contains("4"));
    assert!(bad(&|i| i.brand_colors = vec!["red".into()]).contains("isn't a color"));
    assert!(bad(&|i| i.fonts = vec!["Comic Sans".into()]).contains("fonts"));
    assert!(bad(&|i| i.fonts = vec!["Barlow".into(); 3]).contains("2"));
    assert!(bad(&|i| i.width = Some(8)).contains("16 to 8192"));
    assert!(bad(&|i| i.height = None).contains("both"));
}

#[test]
fn job_ids_must_be_path_safe() {
    assert!(valid_job_id("01a12259-0000-7000-8000-000000000001"));
    assert!(!valid_job_id("../me"));
    assert!(!valid_job_id("a?b"));
    assert!(!valid_job_id(""));
}

#[test]
fn a_tweak_adds_the_suffix_once() {
    assert_eq!(
        tweaked_name("Match upper third"),
        "Match upper third (tweaked)"
    );
    assert_eq!(
        tweaked_name("Match upper third (tweaked) (tweaked) (tweaked)"),
        "Match upper third (tweaked)"
    );
    assert_eq!(tweaked_name("  Bug (tweaked)  "), "Bug (tweaked)");
    let long = "x".repeat(120);
    let n = tweaked_name(&long);
    assert_eq!(n.chars().count(), 100);
    assert!(n.ends_with(" (tweaked)"));
}

#[test]
fn number_like_fields_are_found_by_type_key_or_label() {
    assert!(number_like("number", "anything", "Anything"));
    assert!(number_like("text", "blue-score", "Blue series score"));
    assert!(number_like("text", "p1", "Player 1 boost"));
    assert!(number_like("text", "goals_home", "Home"));
    assert!(number_like("multiline", "x", "Win %"));
    assert!(!number_like("text", "name", "Name"));
    assert!(!number_like("text", "team", "Blue team"));
    // Images, colors and lists never are.
    assert!(!number_like("image", "score-logo", "Score logo"));
    assert!(!number_like("list", "scores", "Scores"));
}

/// (default, sample) of every field after tidying.
fn split_of(p: &Value) -> Vec<(String, Option<String>)> {
    p["manifest"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["default"].as_str().unwrap().to_string(),
                f.get("sample").and_then(Value::as_str).map(str::to_string),
            )
        })
        .collect()
}

#[test]
fn tidying_formats_the_code_and_moves_realistic_values_to_samples() {
    let mut p = json!({
        "manifest": {"fields": [
            {"key": "name", "label": "Name", "type": "text", "default": "Name"},
            {"key": "team", "label": "Team", "type": "text", "default": "[Team]"},
            {"key": "score", "label": "Score", "type": "text", "default": "score"},
            {"key": "boost", "label": "Boost", "type": "number", "default": "lots"},
            {"key": "home", "label": "Home", "type": "text", "default": "Rangers"},
            {"key": "goals", "label": "Goals", "type": "number", "default": "3"},
            {"key": "accent", "label": "Accent", "type": "color", "default": "#c8102e", "sample": "x"}
        ]},
        "files": {"default": {"html": "<div class=\"a\"><div>{{name}}</div></div>", "css": ".a{color:red}"}}
    });
    tidy_package(&mut p);
    let o = |d: &str, s: Option<&str>| (d.to_string(), s.map(str::to_string));
    // New layers start empty (0 for numbers); the realistic values are preview samples;
    // placeholders are neither; other types keep their default and get no sample.
    assert_eq!(
        split_of(&p),
        [
            o("", None),
            o("", None),
            o("0", None),
            o("0", None),
            o("", Some("Rangers")),
            o("0", Some("3")),
            o("#c8102e", None),
        ]
    );
    assert_eq!(
        p["files"]["default"]["html"],
        "<div class=\"a\">
  <div>{{name}}</div>
</div>
"
    );
    assert_eq!(
        p["files"]["default"]["css"],
        ".a {
  color: red;
}
"
    );
}

/// A server that sends samples itself: its defaults are the real ones (placeholders still go).
#[test]
fn a_result_with_samples_keeps_its_defaults() {
    let mut p = json!({"manifest": {"fields": [
        {"key": "title", "label": "Title", "type": "text", "default": "BREAKING", "sample": "Storm warning"},
        {"key": "score", "label": "Score", "type": "number", "default": "0", "sample": "2"},
        {"key": "name", "label": "Name", "type": "text", "default": "Name", "sample": "Name"},
        {"key": "pct", "label": "Pct", "type": "number", "default": "0", "sample": "lots"}
    ]}});
    tidy_package(&mut p);
    let o = |d: &str, s: Option<&str>| (d.to_string(), s.map(str::to_string));
    assert_eq!(
        split_of(&p),
        [
            o("BREAKING", Some("Storm warning")),
            o("0", Some("2")),
            o("", None),
            o("0", None),
        ]
    );
}

/// A tweak sends the samples as defaults (the API's format has no sample yet).
#[test]
fn a_tweak_sends_samples_as_defaults() {
    let mut p = json!({"manifest": {"fields": [
        {"key": "home", "label": "Home", "type": "text", "default": "", "sample": "Rangers"},
        {"key": "title", "label": "Title", "type": "text", "default": "LIVE"}
    ]}});
    fold_samples(&mut p);
    assert_eq!(
        p["manifest"]["fields"],
        json!([
            {"key": "home", "label": "Home", "type": "text", "default": "Rangers"},
            {"key": "title", "label": "Title", "type": "text", "default": "LIVE"}
        ])
    );
}
