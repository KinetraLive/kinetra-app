//! The D3D11 device a frame source draws and reads back on: the high-performance adapter (as the
//! output-engine bench measured it), or WARP (software) for tests on machines without a GPU.
//! Multithread-protected: capture callbacks copy on it from a WinRT thread pool thread while the
//! output clock converts on its own thread.

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::*;

use super::FrameError;

/// Which adapter to create the device on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adapter {
    HighPerformance,
    /// DXGI adapter 0: the one driving the primary display (measurements: hybrid laptops).
    Primary,
    Warp,
}

pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    /// The adapter's description, for logs and measurements.
    pub name: String,
}

// SAFETY: D3D11 devices are free-threaded; the immediate context is used from several threads,
// which the device's multithread protection (enabled in `create`) serializes.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

pub fn create(adapter: Adapter) -> Result<Gpu, FrameError> {
    let gpu = |e: windows::core::Error| FrameError::Gpu(e.to_string());
    let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
    let (mut device, mut context) = (None, None);
    let name;
    unsafe {
        match adapter {
            Adapter::HighPerformance | Adapter::Primary => {
                let factory: IDXGIFactory6 = CreateDXGIFactory1().map_err(gpu)?;
                let a: IDXGIAdapter1 = if adapter == Adapter::Primary {
                    factory.EnumAdapters1(0).map_err(gpu)?
                } else {
                    factory
                        .EnumAdapterByGpuPreference(0, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE)
                        .map_err(gpu)?
                };
                let desc = a.GetDesc1().map_err(gpu)?;
                name = String::from_utf16_lossy(&desc.Description)
                    .trim_end_matches('\0')
                    .to_string();
                D3D11CreateDevice(
                    &a,
                    D3D_DRIVER_TYPE_UNKNOWN,
                    HMODULE::default(),
                    flags,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
                .map_err(gpu)?;
            }
            Adapter::Warp => {
                name = "WARP".to_string();
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_WARP,
                    HMODULE::default(),
                    flags,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
                .map_err(gpu)?;
            }
        }
    }
    let device: ID3D11Device = device.ok_or_else(|| FrameError::Gpu("no device".into()))?;
    let context = context.ok_or_else(|| FrameError::Gpu("no context".into()))?;
    unsafe {
        let _ = device
            .cast::<ID3D11Multithread>()
            .map_err(gpu)?
            .SetMultithreadProtected(true);
    }
    Ok(Gpu {
        device,
        context,
        name,
    })
}

/// The process's device for `adapter`, shared by every frame source while any holds it (each
/// device costs the driver ~50 MB; a second output measured +53 MB with its own). Created on
/// first use, released with the last holder.
pub fn shared(adapter: Adapter) -> Result<std::sync::Arc<Gpu>, FrameError> {
    use std::sync::{Arc, Mutex, Weak};
    static DEVICES: Mutex<Vec<(Adapter, Weak<Gpu>)>> = Mutex::new(Vec::new());
    let mut devices = DEVICES
        .lock()
        .map_err(|_| FrameError::Gpu("poisoned".into()))?;
    devices.retain(|(_, w)| w.strong_count() > 0);
    if let Some(g) = devices
        .iter()
        .find(|(a, _)| *a == adapter)
        .and_then(|(_, w)| w.upgrade())
    {
        return Ok(g);
    }
    let g = Arc::new(create(adapter)?);
    devices.push((adapter, Arc::downgrade(&g)));
    Ok(g)
}

/// A 2D texture: GPU-only with `bind` flags, or a CPU-readable staging copy.
pub fn texture(
    device: &ID3D11Device,
    w: u32,
    h: u32,
    format: DXGI_FORMAT,
    bind: D3D11_BIND_FLAG,
    staging: bool,
) -> Result<ID3D11Texture2D, FrameError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: if staging {
            D3D11_USAGE_STAGING
        } else {
            D3D11_USAGE_DEFAULT
        },
        BindFlags: if staging { 0 } else { bind.0 as u32 },
        CPUAccessFlags: if staging {
            D3D11_CPU_ACCESS_READ.0 as u32
        } else {
            0
        },
        MiscFlags: 0,
    };
    let mut t = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut t)) }
        .map_err(|e| FrameError::Gpu(e.to_string()))?;
    t.ok_or_else(|| FrameError::Gpu("no texture".into()))
}
