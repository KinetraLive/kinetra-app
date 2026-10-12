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
