//! Frames from a screen (Windows): the renderer page captured on the GPU, converted to the
//! output's pixel format at a fixed frame rate, and handed to senders (NDI now, OMT later).
//! Nothing here exists until an output asks for it: no device, thread or WebView2 at rest.
//!
//! Ported from the output-engine bench (branch `omt-output`, `bench/output-engine/`), which
//! measured this pipeline for 30 minutes at 1080p60 with 0 missed ticks. Plan: kinetra-private
//! `docs/NDI_OUTPUT_PLAN.md`.

pub mod capture;
pub mod clock;
pub mod convert;
pub mod gpu;
pub mod jpeg;
pub mod source;

pub use capture::Page;
pub use gpu::Adapter;
pub use source::{FrameSource, Outlet, SourceConfig};

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("graphics device: {0}")]
    Gpu(String),
    #[error("page capture: {0}")]
    WebView(String),
}
