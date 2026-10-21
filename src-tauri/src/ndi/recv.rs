//! An NDI® receiver, for measurements and local end-to-end tests only (`examples/ndi_probe.rs`):
//! finds a source by name and hands out its video frames.

use std::ffi::{c_int, CStr, CString};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::ffi::*;
use super::runtime::Runtime;
use super::NdiError;

pub struct Receiver {
    runtime: Arc<Runtime>,
    inst: RecvInstance,
    /// The source's full name, `MACHINE (name)`.
    pub source: String,
}

// SAFETY: an NDI receiver may be used from any thread (one call at a time here).
unsafe impl Send for Receiver {}

/// A received video frame; freed on drop.
pub struct Video<'a> {
    recv: &'a Receiver,
    pub frame: VideoFrameV2,
}

impl Video<'_> {
    /// The frame's bytes (`line_stride × yres`, plus the alpha plane for UYVA).
    pub fn data(&self) -> &[u8] {
        let f = &self.frame;
        let plane = f.line_stride_in_bytes.max(0) as usize * f.yres.max(0) as usize;
        let len = if f.fourcc == FOURCC_UYVA {
            plane + (f.xres * f.yres).max(0) as usize
        } else {
            plane
        };
        if f.p_data.is_null() {
            return &[];
        }
        // SAFETY: NDI's frame holds this many bytes until it's freed (on drop).
        unsafe { std::slice::from_raw_parts(f.p_data, len) }
    }
}

impl Drop for Video<'_> {
    fn drop(&mut self) {
        unsafe { (self.recv.runtime.api.recv_free_video_v2)(self.recv.inst, &self.frame) };
    }
}

impl Receiver {
    /// Connects to the first source whose name ends with `(<name>)`, waiting up to `timeout`.
    /// `fastest` asks for UYVA/UYVY as sent; otherwise BGRA/BGRX.
    pub fn connect(
        runtime: Arc<Runtime>,
        name: &str,
        fastest: bool,
        timeout: Duration,
    ) -> Result<Self, NdiError> {
        let api = runtime.api;
        let create = FindCreate {
            show_local_sources: true,
            p_groups: std::ptr::null(),
            p_extra_ips: std::ptr::null(),
        };
        let find = unsafe { (api.find_create_v2)(&create) };
        if find.is_null() {
            return Err(NdiError::Find("finder".into()));
        }
        let suffix = format!("({name})");
        let deadline = Instant::now() + timeout;
        let found = loop {
            unsafe { (api.find_wait_for_sources)(find, 200) };
            let mut n = 0u32;
            let list = unsafe { (api.find_get_current_sources)(find, &mut n) };
            let hit = (0..n as usize).find_map(|i| {
                // SAFETY: `n` sources, valid until the next call on the finder.
                let s = unsafe { *list.add(i) };
                let full = unsafe { CStr::from_ptr(s.p_ndi_name) }
                    .to_string_lossy()
                    .into_owned();
                full.ends_with(&suffix).then_some(full)
            });
            if let Some(full) = hit {
                break Some(full);
            }
            if Instant::now() > deadline {
                break None;
            }
        };
        unsafe { (api.find_destroy)(find) };
        let source = found.ok_or_else(|| NdiError::Find(format!("no source named {name}")))?;
        let c_source = CString::new(source.clone()).map_err(|_| NdiError::Name)?;
        let c_recv = c"Kinetra probe";
        let create = RecvCreateV3 {
            source_to_connect_to: Source {
                p_ndi_name: c_source.as_ptr(),
                p_url_address: std::ptr::null(),
            },
            color_format: if fastest {
                RECV_COLOR_FASTEST
            } else {
                RECV_COLOR_BGRX_BGRA
            },
            bandwidth: RECV_BANDWIDTH_HIGHEST,
            allow_video_fields: false,
            p_ndi_recv_name: c_recv.as_ptr(),
        };
        let inst = unsafe { (api.recv_create_v3)(&create) };
        if inst.is_null() {
            return Err(NdiError::Find("receiver".into()));
        }
        Ok(Self {
            runtime,
            inst,
            source,
        })
    }

    /// The next video frame within `timeout_ms`; `Err` when the connection is lost.
    pub fn video(&self, timeout_ms: u32) -> Result<Option<Video<'_>>, NdiError> {
        let mut frame = VideoFrameV2::default();
        let kind: c_int = unsafe {
            (self.runtime.api.recv_capture_v2)(
                self.inst,
                &mut frame,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                timeout_ms,
            )
        };
        match kind {
            FRAME_TYPE_VIDEO => Ok(Some(Video { recv: self, frame })),
            FRAME_TYPE_ERROR => Err(NdiError::Find("connection lost".into())),
            _ => Ok(None),
        }
    }

    /// (frames received, frames dropped) as NDI counts them.
    pub fn performance(&self) -> (i64, i64) {
        let (mut total, mut dropped) = (RecvPerformance::default(), RecvPerformance::default());
        unsafe { (self.runtime.api.recv_get_performance)(self.inst, &mut total, &mut dropped) };
        (total.video_frames, dropped.video_frames)
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        unsafe { (self.runtime.api.recv_destroy)(self.inst) };
    }
}
