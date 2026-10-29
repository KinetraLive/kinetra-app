//! Frames from a screen: the renderer page captured, converted to the output's pixel format at a
//! fixed frame rate, and handed to senders (NDI now, OMT later). Nothing here exists until an
//! output asks for it: no device, thread or web view at rest.
//!
//! Every platform: the output clock ([`clock`]) and the [`Outlet`] senders take frames from. The
//! source itself is per platform: Windows captures a WebView2 visual on the GPU (`capture`,
//! `convert`, `gpu`); macOS has none yet (kinetra-private `docs/MACOS_PARITY_PLAN.md`).
//!
//! Ported from the output-engine bench (branch `omt-output`, `bench/output-engine/`), which
//! measured this pipeline for 30 minutes at 1080p60 with 0 missed ticks. Plan: kinetra-private
//! `docs/NDI_OUTPUT_PLAN.md`.

#[cfg(windows)]
pub mod capture;
pub mod clock;
#[cfg(windows)]
pub mod convert;
#[cfg(windows)]
pub mod gpu;
#[cfg(windows)]
pub mod jpeg;
pub mod outlet;
#[cfg(windows)]
pub mod source;

#[cfg(windows)]
pub use capture::Page;
#[cfg(windows)]
pub use gpu::Adapter;
pub use outlet::Outlet;
#[cfg(windows)]
pub use source::{FrameSource, SourceConfig};

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("graphics device: {0}")]
    Gpu(String),
    #[error("page capture: {0}")]
    WebView(String),
}
