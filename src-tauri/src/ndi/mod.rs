//! NDI® output: sends frames from `crate::frames` as NDI sources, through the user's installed
//! NDI Runtime (never bundled). Nothing is loaded or running until an NDI output is on.
//! Plan and licensing: kinetra-private `docs/NDI_OUTPUT_PLAN.md`.
//!
//! NDI® is a registered trademark of Vizrt NDI AB.

pub mod ffi;
pub mod recv;
pub mod runtime;
pub mod sender;

pub use runtime::Runtime;
pub use sender::{NdiSender, SendMode, SenderStats};

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum NdiError {
    #[error("the NDI Runtime isn't installed")]
    RuntimeMissing,
    #[error("the NDI Runtime couldn't be loaded: {0}")]
    Load(String),
    #[error("the NDI Runtime has no {0} (too old?)")]
    Symbol(&'static str),
    #[error("this processor can't run NDI")]
    CpuUnsupported,
    #[error("source names can't contain a null character")]
    Name,
    #[error("the NDI source couldn't be created")]
    SendCreate,
    #[error("NDI receive: {0}")]
    Find(String),
}
