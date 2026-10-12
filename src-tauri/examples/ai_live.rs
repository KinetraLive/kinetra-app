//! Live AI check against the real Kinetra API. COSTS MONEY: run by hand only, by a
//! maintainer. Device-flow sign-in → activate → quota → ONE generate → ONE tweak of the result
//! → sign out. Results are imported into a temporary project (never a real one), each checked
//! by the template hardening; secrets stay in memory (never the app's keychain).
//!
//! `cargo run --example ai_live`

use std::sync::Arc;
use std::time::{Duration, Instant};

use kinetra_lib::core::cloud::ai::{AiGenerateInput, AiJobView};
use kinetra_lib::core::cloud::service::SigninView;
use kinetra_lib::core::cloud::{config, machine};
use kinetra_lib::core::events::NullSink;
use kinetra_lib::core::secrets::MemorySecrets;
use kinetra_lib::core::state::AppState;

fn wait_job(st: &AppState, id: &str) -> AiJobView {
    let t = Instant::now();
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if let Some(j) = st.ai_jobs_list().into_iter().find(|j| j.id == id) {
            if j.status != "pending" {
                return j;
            }
        }
        if t.elapsed() > Duration::from_secs(600) {
            panic!("job {id} still pending after 10 minutes");
        }
    }
}

fn report(st: &AppState, label: &str, j: &AiJobView) {
    println!(
        "[{label}] status={} code={:?} message={:?} template={:?}",
        j.status, j.code, j.message, j.template_id
    );
    let Some(id) = &j.template_id else { return };
    let dir = st.project_dir().expect("project");
    let doc = kinetra_lib::core::templates::get(&dir, id).expect("template");
    println!(
        "[{label}] \"{}\": {} fields, {} variables, variants {:?}, design {:?}",
        doc.meta.name,
        doc.meta.fields.len(),
        doc.meta.variables.len(),
        doc.variants
            .iter()
            .map(|v| v.id.as_str())
            .collect::<Vec<_>>(),
        doc.meta.design.map(|d| (d.w, d.h))
    );
    for v in &doc.variants {
        let mut p = kinetra_lib::core::template_safety::check_html(&v.html);
        p.extend(kinetra_lib::core::template_safety::check_css(&v.css));
        println!(
            "[{label}] variant {}: {} B HTML, {} B CSS, hardening problems {:?}",
            v.id,
            v.html.len(),
            v.css.len(),
            p
        );
    }
}

fn main() {
    let tmp = std::env::temp_dir().join(format!("kinetra-ai-live-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("tmp");
    let st = AppState::load(tmp.join("config"), Arc::new(NullSink)).expect("state");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    st.live().set_runtime(rt.handle().clone());
    let machine = machine::fingerprint().expect("MachineGuid");
    st.cloud_service().set_test_env(
        config::API_BASE,
        Arc::new(MemorySecrets::default()),
        &machine,
    );
    st.cloud_init();
    st.create_project(&tmp, "AI live test").expect("project");

    if let SigninView::Waiting { user_code, url, .. } =
        rt.block_on(st.sign_in_start()).expect("device flow")
    {
        println!("APPROVE: open {url} and check the code {user_code}");
    }
    let t = Instant::now();
    while !(st.license_view().signed_in && st.license_view().features.ai) {
        std::thread::sleep(Duration::from_secs(2));
        if t.elapsed() > Duration::from_secs(660) {
            println!("no Pro after sign-in: {:?}", st.license_view().signin);
            return;
        }
    }
    println!("signed in; AI unlocked");
    match rt.block_on(st.ai_quota()) {
        Ok(q) => println!("quota before: {q:?}"),
        Err(e) => println!("quota failed: {e}"),
    }

    let j = rt.block_on(st.ai_generate(AiGenerateInput {
        prompt: "Lower third for a football match: player name and team, bold and clean".into(),
        template_type: Some("lower_third".into()),
        brand_colors: vec!["#c8102e".into(), "#ffffff".into()],
        fonts: vec!["Barlow Condensed".into(), "Barlow".into()],
        width: Some(960),
        height: Some(200),
    }));
    // Every step reports instead of stopping, so the session is always signed out below.
    let done = match j {
        Ok(j) => {
            let d = wait_job(&st, &j.id);
            report(&st, "generate", &d);
            Some(d)
        }
        Err(e) => {
            println!("[generate] refused: {e}");
            None
        }
    };

    if let Some(id) = done.and_then(|d| d.template_id) {
        match rt.block_on(st.ai_tweak(
            id,
            "Make the name plate dark blue and the team name smaller".into(),
        )) {
            Ok(t) => {
                let d = wait_job(&st, &t.id);
                report(&st, "tweak", &d);
            }
            Err(e) => println!("[tweak] refused: {e}"),
        }
    }
    match rt.block_on(st.ai_quota()) {
        Ok(q) => println!("quota after: {q:?}"),
        Err(e) => println!("quota failed: {e}"),
    }
    let _ = rt.block_on(st.sign_out());
    println!("project kept for inspection: {}", tmp.display());
}
