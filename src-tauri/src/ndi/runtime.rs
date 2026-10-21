//! The user's installed NDI® Runtime, loaded at run time. Kinetra ships no NDI binary (plan §1.2):
//! the runtime comes from NDI Tools or the NDI Runtime installer (ndi.link/NDIRedistV6), which set
//! `NDI_RUNTIME_DIR_V6` (and `_V5`) to its folder.
//!
//! Every function is looked up by its exported name (the runtime exports them all, alongside
//! `NDIlib_v5_load`), so a newer runtime's longer function table can't shift anything. The library
//! stays loaded once loaded (NDI keeps threads; unloading under them would crash), but
//! `NDIlib_initialize` / `NDIlib_destroy` follow use: [`Runtime::acquire`] initializes on the first
//! holder and the last [`Runtime`] dropped destroys.

use std::ffi::{CStr, OsString};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

use super::ffi::*;
use super::NdiError;

/// The runtime's file name on this platform.
#[cfg(windows)]
pub const LIBRARY: &str = "Processing.NDI.Lib.x64.dll";
#[cfg(not(windows))]
pub const LIBRARY: &str = "libndi.dylib";

/// NDI's own download page for the runtime (from the SDK's `NDILIB_REDIST_URL`).
pub const DOWNLOAD_URL: &str = "https://ndi.link/NDIRedistV6";

/// The functions Kinetra calls.
pub struct Api {
    pub initialize: Initialize,
    pub destroy: Destroy,
    pub version: Version,
    pub is_supported_cpu: IsSupportedCpu,
    pub send_create: SendCreateFn,
    pub send_destroy: SendDestroy,
    pub send_video_v2: SendVideoV2,
    pub send_video_async_v2: SendVideoV2,
    pub send_get_tally: SendGetTally,
    pub send_get_no_connections: SendGetNoConnections,
    pub find_create_v2: FindCreateV2,
    pub find_destroy: FindDestroy,
    pub find_wait_for_sources: FindWaitForSources,
    pub find_get_current_sources: FindGetCurrentSources,
    pub recv_create_v3: RecvCreateV3Fn,
    pub recv_destroy: RecvDestroy,
    pub recv_capture_v2: RecvCaptureV2,
    pub recv_free_video_v2: RecvFreeVideoV2,
    pub recv_get_performance: RecvGetPerformance,
}

/// Where to look for the runtime, best first: the folders NDI's installers put in
/// `NDI_RUNTIME_DIR_V6` / `_V5`.
pub fn candidates(env: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for var in ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"] {
        if let Some(dir) = env(var).filter(|d| !d.is_empty()) {
            let p = PathBuf::from(dir).join(LIBRARY);
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    #[cfg(not(windows))]
    out.push(PathBuf::from("/usr/local/lib").join(LIBRARY));
    out
}

/// The first candidate that exists (no library is loaded): how Preferences tells whether the
/// runtime is installed.
pub fn find_installed() -> Option<PathBuf> {
    candidates(|v| std::env::var_os(v))
        .into_iter()
        .find(|p| p.is_file())
}

/// The loaded function table (loaded once per process; a failure is retried next time, so
/// installing the runtime while Kinetra runs works).
static API: Mutex<Option<&'static Api>> = Mutex::new(None);

#[cfg(windows)]
fn load(path: &std::path::Path) -> Result<&'static Api, NdiError> {
    use windows::core::{HSTRING, PCSTR};
    use windows::Win32::System::LibraryLoader::{
        GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
    };
    unsafe {
        // Its own folder first for its dependencies, then the system's; never the current or the
        // app's folder.
        let module = LoadLibraryExW(
            &HSTRING::from(path.as_os_str()),
            None,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
        .map_err(|e| NdiError::Load(e.to_string()))?;
        macro_rules! sym {
            ($name:literal, $ty:ty) => {{
                let p = GetProcAddress(module, PCSTR(concat!($name, "\0").as_ptr()))
                    .ok_or(NdiError::Symbol($name))?;
                // SAFETY: the export has the signature of the header's declaration.
                std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(p)
            }};
        }
        let api = Api {
            initialize: sym!("NDIlib_initialize", Initialize),
            destroy: sym!("NDIlib_destroy", Destroy),
            version: sym!("NDIlib_version", Version),
            is_supported_cpu: sym!("NDIlib_is_supported_CPU", IsSupportedCpu),
            send_create: sym!("NDIlib_send_create", SendCreateFn),
            send_destroy: sym!("NDIlib_send_destroy", SendDestroy),
            send_video_v2: sym!("NDIlib_send_send_video_v2", SendVideoV2),
            send_video_async_v2: sym!("NDIlib_send_send_video_async_v2", SendVideoV2),
            send_get_tally: sym!("NDIlib_send_get_tally", SendGetTally),
            send_get_no_connections: sym!("NDIlib_send_get_no_connections", SendGetNoConnections),
            find_create_v2: sym!("NDIlib_find_create_v2", FindCreateV2),
            find_destroy: sym!("NDIlib_find_destroy", FindDestroy),
            find_wait_for_sources: sym!("NDIlib_find_wait_for_sources", FindWaitForSources),
            find_get_current_sources: sym!(
                "NDIlib_find_get_current_sources",
                FindGetCurrentSources
            ),
            recv_create_v3: sym!("NDIlib_recv_create_v3", RecvCreateV3Fn),
            recv_destroy: sym!("NDIlib_recv_destroy", RecvDestroy),
            recv_capture_v2: sym!("NDIlib_recv_capture_v2", RecvCaptureV2),
            recv_free_video_v2: sym!("NDIlib_recv_free_video_v2", RecvFreeVideoV2),
            recv_get_performance: sym!("NDIlib_recv_get_performance", RecvGetPerformance),
        };
        Ok(Box::leak(Box::new(api)))
    }
}

#[cfg(not(windows))]
fn load(_path: &std::path::Path) -> Result<&'static Api, NdiError> {
    Err(NdiError::Load(
        "NDI output isn't available on this system yet".into(),
    ))
}

fn api() -> Result<&'static Api, NdiError> {
    let mut slot = API.lock().map_err(|_| NdiError::Load("poisoned".into()))?;
    if let Some(api) = *slot {
        return Ok(api);
    }
    let path = find_installed().ok_or(NdiError::RuntimeMissing)?;
    let api = load(&path)?;
    tracing::info!("NDI runtime loaded from {}", path.display());
    *slot = Some(api);
    Ok(api)
}

/// An initialized runtime. Cloned through `Arc`; the last one dropped calls `NDIlib_destroy`.
pub struct Runtime {
    pub api: &'static Api,
    /// `NDIlib_version()`, e.g. "NDI SDK WIN64 … 6.3.2.0".
    pub version: String,
}

static CURRENT: Mutex<Weak<Runtime>> = Mutex::new(Weak::new());

impl Runtime {
    /// The process's runtime, loading and initializing it if nothing holds it.
    pub fn acquire() -> Result<Arc<Self>, NdiError> {
        let mut cur = CURRENT
            .lock()
            .map_err(|_| NdiError::Load("poisoned".into()))?;
        if let Some(rt) = cur.upgrade() {
            return Ok(rt);
        }
        let rt = Arc::new(Self::start(api()?)?);
        *cur = Arc::downgrade(&rt);
        Ok(rt)
    }

    /// Initializes `api` (tests pass a fake table).
    pub fn start(api: &'static Api) -> Result<Self, NdiError> {
        unsafe {
            if !(api.is_supported_cpu)() {
                return Err(NdiError::CpuUnsupported);
            }
            if !(api.initialize)() {
                return Err(NdiError::CpuUnsupported);
            }
            let v = (api.version)();
            let version = if v.is_null() {
                String::new()
            } else {
                CStr::from_ptr(v).to_string_lossy().into_owned()
            };
            Ok(Self { api, version })
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe { (self.api.destroy)() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_prefer_v6_and_skip_empty_or_duplicate_dirs() {
        let env = |v: &str| match v {
            "NDI_RUNTIME_DIR_V6" => Some(OsString::from(r"C:\NDI\6")),
            "NDI_RUNTIME_DIR_V5" => Some(OsString::from(r"C:\NDI\6")),
            _ => None,
        };
        let c = candidates(env);
        assert_eq!(c[0], PathBuf::from(r"C:\NDI\6").join(LIBRARY));
        assert_eq!(c.iter().filter(|p| p.starts_with(r"C:\NDI\6")).count(), 1);
        let none = candidates(|_| Some(OsString::new()));
        assert!(none.iter().all(|p| !p.starts_with(r"C:\NDI")));
    }
}
