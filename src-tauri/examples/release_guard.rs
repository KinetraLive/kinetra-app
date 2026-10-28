//! Release guard: proves the debug-only test overrides don't exist in release builds.
//!
//! This example calls every override that must never ship: the entitlement override the e2e
//! fixture uses to grant Pro (`Cloud::set_test_entitlements`) and its planted license token
//! (`Cloud::set_test_license`), the template sandbox bypass
//! (`preview::load_unchecked_for_tests`) and the "act as another computer" profile
//! (`devprofile::dev_profile`), the extra trusted keys (`config::extra_trusted_keys`) and the
//! corrupt-download hook (`restore::corrupt_downloads`). It compiles in debug builds (`cargo clippy
//! --all-targets` builds it) and must FAIL to compile with `--release`, with exactly one
//! "no method / function" error per override. `scripts/check-release-overrides.mjs` runs
//! `cargo check --release --example release_guard` and fails CI otherwise: if an override were
//! compiled into a release build (a cfg widened, `debug-assertions = true` in the release
//! profile), this example would compile.
//!
//! Never run: only compiled.

use std::sync::Arc;

use kinetra_lib::core::devprofile;
use kinetra_lib::core::events::NullSink;
use kinetra_lib::core::live::{TEST_LAYER_ID, TEST_SCREEN_ID};
use kinetra_lib::core::live_hub::LiveHub;
use kinetra_lib::core::preview;
use kinetra_lib::core::state::AppState;

fn main() {
    let dir = std::env::temp_dir().join("kinetra-release-guard");
    let Ok(st) = AppState::load(dir, Arc::new(NullSink)) else {
        return;
    };
    st.cloud_service()
        .set_test_entitlements(&["core.app", "pro.control"], true);
    st.cloud_service().set_test_license(Some("token"), None);
    let _ = devprofile::dev_profile();
    let _ = kinetra_lib::core::cloud::config::extra_trusted_keys();
    let _ = kinetra_lib::core::cloud::projects::restore::corrupt_downloads();
    let hub = LiveHub::with_test_screen();
    let _ = hub.update_layer::<kinetra_lib::core::live::LiveError>(
        TEST_SCREEN_ID,
        TEST_LAYER_ID,
        |layer| Ok(preview::load_unchecked_for_tests(layer, "<div></div>", "")),
    );
}
