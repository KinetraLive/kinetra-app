//! The NDI® SDK's C types that Kinetra uses, transcribed from the NDI 6 SDK headers
//! (`Processing.NDI.structs.h`, `.Send.h`, `.Recv.h`, `.Find.h`, `.Lib.h`; headers v6.3.0.3 as
//! published with DistroAV, `lib/ndi` at commit 04be005). Those headers carry this license, which
//! applies to these declarations:
//!
//! > Copyright (C) 2023-2026 Vizrt NDI AB. All rights reserved.
//! >
//! > Permission is hereby granted, free of charge, to any person obtaining a copy of this software
//! > and associated documentation files(the "Software"), to deal in the Software without
//! > restriction, including without limitation the rights to use, copy, modify, merge, publish,
//! > distribute, sublicense, and / or sell copies of the Software, and to permit persons to whom the
//! > Software is furnished to do so, subject to the following conditions :
//! >
//! > The above copyright notice and this permission notice shall be included in all copies or
//! > substantial portions of the Software.
//! >
//! > THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
//! > BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
//! > NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! > DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! > OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//!
//! The NDI® SDK and Runtime themselves are not part of Kinetra and are licensed separately by
//! Vizrt NDI AB (http://ndi.link/ndisdk_license). NDI® is a registered trademark of Vizrt NDI AB.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_void};

pub type SendInstance = *mut c_void;
pub type RecvInstance = *mut c_void;
pub type FindInstance = *mut c_void;

const fn fourcc(s: &[u8; 4]) -> u32 {
    (s[0] as u32) | ((s[1] as u32) << 8) | ((s[2] as u32) << 16) | ((s[3] as u32) << 24)
}

pub const FOURCC_UYVY: u32 = fourcc(b"UYVY");
pub const FOURCC_UYVA: u32 = fourcc(b"UYVA");
pub const FOURCC_BGRA: u32 = fourcc(b"BGRA");
pub const FOURCC_BGRX: u32 = fourcc(b"BGRX");

pub const FRAME_FORMAT_PROGRESSIVE: c_int = 1;
pub const FRAME_TYPE_NONE: c_int = 0;
pub const FRAME_TYPE_VIDEO: c_int = 1;
pub const FRAME_TYPE_ERROR: c_int = 4;

/// `NDIlib_send_timecode_synthesize`: let the library stamp the frame.
pub const TIMECODE_SYNTHESIZE: i64 = i64::MAX;

pub const RECV_COLOR_FASTEST: c_int = 100;
pub const RECV_COLOR_BGRX_BGRA: c_int = 0;
pub const RECV_BANDWIDTH_HIGHEST: c_int = 100;

/// `NDIlib_source_t`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Source {
    pub p_ndi_name: *const c_char,
    pub p_url_address: *const c_char,
}

/// `NDIlib_send_create_t`.
#[repr(C)]
pub struct SendCreate {
    pub p_ndi_name: *const c_char,
    pub p_groups: *const c_char,
    pub clock_video: bool,
    pub clock_audio: bool,
}

/// `NDIlib_video_frame_v2_t`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VideoFrameV2 {
    pub xres: c_int,
    pub yres: c_int,
    pub fourcc: u32,
    pub frame_rate_n: c_int,
    pub frame_rate_d: c_int,
    pub picture_aspect_ratio: f32,
    pub frame_format_type: c_int,
    pub timecode: i64,
    pub p_data: *mut u8,
    /// Union with `data_size_in_bytes` (compressed formats only).
    pub line_stride_in_bytes: c_int,
    pub p_metadata: *const c_char,
    pub timestamp: i64,
}

impl Default for VideoFrameV2 {
    fn default() -> Self {
        Self {
            xres: 0,
            yres: 0,
            fourcc: FOURCC_UYVY,
            frame_rate_n: 30000,
            frame_rate_d: 1001,
            picture_aspect_ratio: 0.0,
            frame_format_type: FRAME_FORMAT_PROGRESSIVE,
            timecode: TIMECODE_SYNTHESIZE,
            p_data: std::ptr::null_mut(),
            line_stride_in_bytes: 0,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        }
    }
}

/// `NDIlib_tally_t`.
#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tally {
    pub on_program: bool,
    pub on_preview: bool,
}

/// `NDIlib_find_create_t`.
#[repr(C)]
pub struct FindCreate {
    pub show_local_sources: bool,
    pub p_groups: *const c_char,
    pub p_extra_ips: *const c_char,
}

/// `NDIlib_recv_create_v3_t`.
#[repr(C)]
pub struct RecvCreateV3 {
    pub source_to_connect_to: Source,
    pub color_format: c_int,
    pub bandwidth: c_int,
    pub allow_video_fields: bool,
    pub p_ndi_recv_name: *const c_char,
}

/// `NDIlib_recv_performance_t`.
#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct RecvPerformance {
    pub video_frames: i64,
    pub audio_frames: i64,
    pub metadata_frames: i64,
}

pub type Initialize = unsafe extern "C" fn() -> bool;
pub type Destroy = unsafe extern "C" fn();
pub type Version = unsafe extern "C" fn() -> *const c_char;
pub type IsSupportedCpu = unsafe extern "C" fn() -> bool;
pub type SendCreateFn = unsafe extern "C" fn(*const SendCreate) -> SendInstance;
pub type SendDestroy = unsafe extern "C" fn(SendInstance);
pub type SendVideoV2 = unsafe extern "C" fn(SendInstance, *const VideoFrameV2);
pub type SendGetTally = unsafe extern "C" fn(SendInstance, *mut Tally, u32) -> bool;
pub type SendGetNoConnections = unsafe extern "C" fn(SendInstance, u32) -> c_int;
pub type FindCreateV2 = unsafe extern "C" fn(*const FindCreate) -> FindInstance;
pub type FindDestroy = unsafe extern "C" fn(FindInstance);
pub type FindWaitForSources = unsafe extern "C" fn(FindInstance, u32) -> bool;
pub type FindGetCurrentSources = unsafe extern "C" fn(FindInstance, *mut u32) -> *const Source;
pub type RecvCreateV3Fn = unsafe extern "C" fn(*const RecvCreateV3) -> RecvInstance;
pub type RecvDestroy = unsafe extern "C" fn(RecvInstance);
pub type RecvCaptureV2 =
    unsafe extern "C" fn(RecvInstance, *mut VideoFrameV2, *mut c_void, *mut c_void, u32) -> c_int;
pub type RecvFreeVideoV2 = unsafe extern "C" fn(RecvInstance, *const VideoFrameV2);
pub type RecvGetPerformance =
    unsafe extern "C" fn(RecvInstance, *mut RecvPerformance, *mut RecvPerformance);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_the_headers_on_64_bit() {
        assert_eq!(std::mem::size_of::<VideoFrameV2>(), 72);
        assert_eq!(std::mem::offset_of!(VideoFrameV2, timecode), 32);
        assert_eq!(std::mem::offset_of!(VideoFrameV2, p_data), 40);
        assert_eq!(std::mem::offset_of!(VideoFrameV2, p_metadata), 56);
        assert_eq!(std::mem::size_of::<SendCreate>(), 24);
        assert_eq!(std::mem::size_of::<RecvCreateV3>(), 40);
        assert_eq!(std::mem::size_of::<Tally>(), 2);
        assert_eq!(FOURCC_UYVA, 0x4156_5955);
    }
}
