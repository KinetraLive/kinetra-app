//! NDI® output: sends frames from `crate::frames` as NDI sources, through the user's installed
//! NDI Runtime (never bundled). Nothing is loaded or running until an NDI output is on.
//! Plan and licensing: kinetra-private `docs/NDI_OUTPUT_PLAN.md`.
//!
//! NDI® is a registered trademark of Vizrt NDI AB.

pub mod ffi;
#[cfg(windows)]
pub mod host;
pub mod recv;
pub mod runtime;
pub mod sender;

pub use runtime::Runtime;
pub use sender::{NdiSender, SendMode, SenderStats};

/// The system's region as an ISO country code ("NO"), from Windows' region setting (Settings ›
/// Time & language › Region; `HKCU\Control Panel\International\Geo` `Name`).
#[cfg(windows)]
pub fn system_region() -> Option<String> {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let mut buf = [0u16; 16];
    let mut len = std::mem::size_of_val(&buf) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\International\\Geo"),
            w!("Name"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if r.is_err() {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    let s = s.trim_end_matches('\0').trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// The system's region as an ISO country code ("NO"), from macOS' region setting (System
/// Settings › General › Language & Region). `countryCode` rather than `regionCode` (macOS 14+):
/// Kinetra runs on macOS 11.
#[cfg(target_os = "macos")]
pub fn system_region() -> Option<String> {
    #[allow(deprecated)]
    let code = objc2_foundation::NSLocale::currentLocale().countryCode()?;
    let s = code.to_string();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn system_region() -> Option<String> {
    None
}

/// What Preferences needs to offer NDI outputs (`ndi_info`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NdiInfo {
    /// NDI outputs can run on this system (Windows).
    pub available: bool,
    /// The NDI Runtime is installed (its file exists; nothing is loaded).
    pub installed: bool,
    pub download_url: &'static str,
    /// A new output's frame rate for this region.
    pub default_rate: crate::core::model::NdiRate,
}

pub fn info() -> NdiInfo {
    NdiInfo {
        available: cfg!(windows),
        installed: runtime::find_installed().is_some(),
        download_url: runtime::DOWNLOAD_URL,
        default_rate: crate::core::outputs::default_ndi_rate(system_region().as_deref()),
    }
}

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
