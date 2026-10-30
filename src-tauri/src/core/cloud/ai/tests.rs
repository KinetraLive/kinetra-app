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

#[test]
fn tidying_formats_the_code_and_clears_placeholder_values() {
    let mut p = json!({
        "manifest": {"fields": [
            {"key": "name", "label": "Name", "type": "text", "default": "Name"},
            {"key": "team", "label": "Team", "type": "text", "default": "[Team]"},
            {"key": "score", "label": "Score", "type": "text", "default": "score"},
            {"key": "boost", "label": "Boost", "type": "number", "default": "lots"},
            {"key": "home", "label": "Home", "type": "text", "default": "Rangers"},
            {"key": "goals", "label": "Goals", "type": "number", "default": "3"},
            {"key": "accent", "label": "Accent", "type": "color", "default": "Accent"}
        ]},
        "files": {"default": {"html": "<div class=\"a\"><div>{{name}}</div></div>", "css": ".a{color:red}"}}
    });
    tidy_package(&mut p);
    let d: Vec<&str> = p["manifest"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["default"].as_str().unwrap())
        .collect();
    // Placeholders cleared (numbers to 0), real values kept, other types untouched.
    assert_eq!(d, ["", "", "0", "0", "Rangers", "3", "Accent"]);
    assert_eq!(
        p["files"]["default"]["html"],
        "<div class=\"a\">\n  <div>{{name}}</div>\n</div>\n"
    );
    assert_eq!(p["files"]["default"]["css"], ".a {\n  color: red;\n}\n");
}
