//! Where one sender takes its frames from (every platform; the sources are per platform).

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

use crate::core::frame_out::{Frame, Mailbox};

/// Where one sender takes its frames from.
pub struct Outlet {
    pub mailbox: Mailbox<Arc<Frame>>,
    /// Frames replaced before this outlet's sender took them.
    pub dropped: AtomicU64,
    /// The sender wants frames (an NDI sender: a receiver is connected). While no outlet wants
    /// any, the source skips the GPU conversion and read-back and hands out nothing.
    pub wanted: AtomicBool,
}

impl Default for Outlet {
    fn default() -> Self {
        Self {
            mailbox: Mailbox::default(),
            dropped: AtomicU64::new(0),
            wanted: AtomicBool::new(true),
        }
    }
}
