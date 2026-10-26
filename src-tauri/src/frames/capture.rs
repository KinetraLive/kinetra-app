//! A page captured on the GPU: a real, GPU-rendering WebView2 hosted in composition mode
//! (`CreateCoreWebView2CompositionController`), its root visual captured with
//! Windows.Graphics.Capture (`GraphicsCaptureItem::CreateFromVisual`) into a D3D11 frame pool on
//! the frame source's device. Alpha survives as premultiplied BGRA; a visual has no window
//! outline, so no capture border. Proved by the output-engine bench's capture gate (alpha exact,
//! off-screen / never shown / 4 at once, 2026-10-10).
//!
//! Each capture runs a UI thread (WebView2 and the compositor need a message loop) that blocks in
//! `GetMessageW`: no polling. Its host window is off-screen and never shown, so nothing appears
//! on the user's screens and nothing takes focus. Every captured frame is copied into
//! [`Capture::texture`] (a fixed texture the converter reads), so the output clock always
//! converts the newest one.
//!
//! The environment (own user-data folder, so its browser arguments can differ from the app's)
//! turns off Chromium's occlusion detection and background throttling; without them a hidden page
//! would stop drawing.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    CoreWebView2EnvironmentOptions, CreateCoreWebView2CompositionControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, ProcessFailedEventHandler,
};
use windows::core::{Interface, Ref, HSTRING, PCWSTR};
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_STA, DQTYPE_THREAD_CURRENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Compositor;
use windows_numerics::Vector2;

use super::clock::now_us;
use super::gpu::{texture, Gpu};
use super::FrameError;

/// Chromium switches for a page that must keep drawing while nobody can see it.
pub const BROWSER_ARGS: &str = "--disable-features=CalculateNativeWinOcclusion \
     --disable-backgrounding-occluded-windows --disable-renderer-backgrounding \
     --disable-background-timer-throttling";

/// What the capture shows.
#[derive(Debug, Clone)]
pub enum Page {
    /// A URL (the renderer: `http://127.0.0.1:<port>/screen/<id>?audio=0`).
    Url(String),
    /// Inline HTML (tests and measurements).
    Html(String),
}

pub struct Capture {
    /// The newest captured frame (w×h premultiplied BGRA, shader-readable).
    pub texture: ID3D11Texture2D,
    /// Frames captured so far.
    pub captured: Arc<AtomicU64>,
    /// When the newest frame arrived (µs, QPC; 0 = none yet).
    pub latest_us: Arc<AtomicI64>,
    /// The WebView2 browser process (its renderer and GPU processes are its children).
    pub browser_pid: u32,
    thread_id: u32,
    thread: Option<std::thread::JoinHandle<()>>,
}

// SAFETY: the texture is a free-threaded D3D11 object; the rest is plain data and a join handle.
unsafe impl Send for Capture {}
unsafe impl Sync for Capture {}

struct SendBox<T>(T);
// SAFETY: moves COM pointers of free-threaded D3D11 objects to the capture thread.
unsafe impl<T> Send for SendBox<T> {}

/// Called (on the capture's UI thread) when the page's render or browser process failed: the page
/// shows nothing new until the capture is started again.
pub type OnFailed = Arc<dyn Fn() + Send + Sync>;

/// What a capture shows and how.
#[derive(Clone)]
pub struct CaptureOptions {
    pub page: Page,
    /// CSS px (= physical px).
    pub width: u32,
    pub height: u32,
    /// The WebView2 user data folder.
    pub data_dir: PathBuf,
    /// Capture every frame the page draws (`MinUpdateInterval` 1 ms, Windows 11 24H2+); off =
    /// the system's ~60 Hz default (what older Windows always does; measurements only).
    pub follow_page: bool,
    pub on_failed: Option<OnFailed>,
    /// The environment's browser arguments; `None` = [`BROWSER_ARGS`].
    pub browser_args: Option<String>,
    /// The environment's language (wry sets the system UI language; an environment can share
    /// the app's browser process only with identical options).
    pub language: Option<String>,
}

/// The system UI language as wry passes it to WebView2 ("en-US").
pub fn system_language() -> String {
    use windows::Win32::Globalization::{
        GetUserDefaultUILanguage, LCIDToLocaleName, LOCALE_ALLOW_NEUTRAL_NAMES,
    };
    // LOCALE_NAME_MAX_LENGTH.
    let mut lang = [0u16; 85];
    unsafe {
        let lcid = GetUserDefaultUILanguage();
        LCIDToLocaleName(u32::from(lcid), Some(&mut lang), LOCALE_ALLOW_NEUTRAL_NAMES);
    }
    let n = lang.iter().position(|&c| c == 0).unwrap_or(lang.len());
    String::from_utf16_lossy(&lang[..n])
}

impl Capture {
    /// Starts a capture on `gpu`'s device. Returns once the page is navigating and capture runs.
    pub fn start(gpu: &Gpu, opts: CaptureOptions) -> Result<Self, FrameError> {
        let (w, h) = (opts.width, opts.height);
        let texture = texture(
            &gpu.device,
            w,
            h,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            D3D11_BIND_SHADER_RESOURCE,
            false,
        )?;
        let captured = Arc::new(AtomicU64::new(0));
        let latest_us = Arc::new(AtomicI64::new(0));
        let (tx, rx) = std::sync::mpsc::channel::<Result<(u32, u32), FrameError>>();
        let shared = SendBox((gpu.device.clone(), gpu.context.clone(), texture.clone()));
        let counters = (captured.clone(), latest_us.clone());
        let thread = std::thread::Builder::new()
            .name("frames-capture".into())
            .spawn(move || {
                let shared = shared;
                let (device, context, target) = shared.0;
                let r = ui_thread(&device, &context, &target, opts, counters, &tx);
                if let Err(e) = r {
                    tx.send(Err(e)).ok();
                }
            })
            .map_err(|e| FrameError::WebView(e.to_string()))?;
        match rx.recv() {
            Ok(Ok((browser_pid, thread_id))) => Ok(Self {
                texture,
                captured,
                latest_us,
                browser_pid,
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                thread.join().ok();
                Err(e)
            }
            Err(_) => {
                thread.join().ok();
                Err(FrameError::WebView("capture thread ended".into()))
            }
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(t) = self.thread.take() {
            t.join().ok();
        }
    }
}

fn wv(e: webview2_com::Error) -> FrameError {
    FrameError::WebView(e.to_string())
}

fn win(e: windows::core::Error) -> FrameError {
    FrameError::WebView(e.to_string())
}

fn ui_thread(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    target: &ID3D11Texture2D,
    opts: CaptureOptions,
    (captured, latest_us): (Arc<AtomicU64>, Arc<AtomicI64>),
    ready: &std::sync::mpsc::Sender<Result<(u32, u32), FrameError>>,
) -> Result<(), FrameError> {
    let CaptureOptions {
        page,
        width: w,
        height: h,
        data_dir: data,
        follow_page,
        on_failed,
        browser_args,
        language,
    } = opts;
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(win)?
    };
    let result = (|| {
        let _queue = unsafe {
            CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_STA,
            })
            .map_err(win)?
        };
        let compositor = Compositor::new().map_err(win)?;
        let winrt: IDirect3DDevice = unsafe {
            CreateDirect3D11DeviceFromDXGIDevice(&device.cast::<IDXGIDevice>().map_err(win)?)
                .map_err(win)?
                .cast()
                .map_err(win)?
        };

        let options = CoreWebView2EnvironmentOptions::default();
        let args = browser_args.unwrap_or_else(|| BROWSER_ARGS.to_string());
        unsafe { options.set_additional_browser_arguments(args) };
        if let Some(lang) = language {
            unsafe { options.set_language(lang) };
        }
        let options: ICoreWebView2EnvironmentOptions = options.into();
        let data = HSTRING::from(data.as_os_str());
        let (etx, erx) = std::sync::mpsc::channel();
        CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &data, &options, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |hr, env| {
                hr?;
                etx.send(env).ok();
                Ok(())
            }),
        )
        .map_err(wv)?;
        let env: ICoreWebView2Environment3 = erx
            .recv()
            .ok()
            .flatten()
            .ok_or_else(|| FrameError::WebView("no WebView2 environment".into()))?
            .cast()
            .map_err(win)?;

        let hwnd = host_window(w, h)?;
        let (ctx_tx, ctx_rx) = std::sync::mpsc::channel();
        CreateCoreWebView2CompositionControllerCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                env.CreateCoreWebView2CompositionController(hwnd, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |hr, c| {
                hr?;
                ctx_tx.send(c).ok();
                Ok(())
            }),
        )
        .map_err(wv)?;
        let comp: ICoreWebView2CompositionController = ctx_rx
            .recv()
            .ok()
            .flatten()
            .ok_or_else(|| FrameError::WebView("no WebView2 controller".into()))?;
        let controller: ICoreWebView2Controller = comp.cast().map_err(win)?;
        let visual = compositor.CreateContainerVisual().map_err(win)?;
        visual
            .SetSize(Vector2 {
                X: w as f32,
                Y: h as f32,
            })
            .map_err(win)?;
        let mut browser_pid = 0u32;
        unsafe {
            comp.SetRootVisualTarget(&visual).map_err(win)?;
            let c3: ICoreWebView2Controller3 = controller.cast().map_err(win)?;
            c3.SetBoundsMode(COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS)
                .map_err(win)?;
            c3.SetShouldDetectMonitorScaleChanges(false).map_err(win)?;
            c3.SetRasterizationScale(1.0).map_err(win)?;
            controller
                .cast::<ICoreWebView2Controller2>()
                .map_err(win)?
                .SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
                    A: 0,
                    R: 0,
                    G: 0,
                    B: 0,
                })
                .map_err(win)?;
            controller
                .SetBounds(RECT {
                    left: 0,
                    top: 0,
                    right: w as i32,
                    bottom: h as i32,
                })
                .map_err(win)?;
            controller.SetIsVisible(true).map_err(win)?;
            let core = controller.CoreWebView2().map_err(win)?;
            core.BrowserProcessId(&mut browser_pid).map_err(win)?;
            if let Some(f) = on_failed {
                // A dead or hung page draws nothing more: the owner starts the capture again.
                let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                    if let Some(args) = args {
                        args.ProcessFailedKind(&mut kind)?;
                    }
                    if matches!(
                        kind,
                        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED
                            | COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
                            | COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE
                    ) {
                        tracing::warn!("NDI capture page failed ({kind:?})");
                        f();
                    }
                    Ok(())
                }));
                let mut token = 0i64;
                core.add_ProcessFailed(&handler, &mut token).map_err(win)?;
            }
            match &page {
                Page::Url(u) => core.Navigate(&HSTRING::from(u.as_str())).map_err(win)?,
                Page::Html(html) => core
                    .NavigateToString(&HSTRING::from(html.as_str()))
                    .map_err(win)?,
            }
        }

        let item = GraphicsCaptureItem::CreateFromVisual(&visual).map_err(win)?;
        let size = SizeInt32 {
            Width: w as i32,
            Height: h as i32,
        };
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            3,
            size,
        )
        .map_err(win)?;
        let session = pool.CreateCaptureSession(&item).map_err(win)?;
        // Windows 11: no border, no cursor; 24H2+: follow the page's own frame rate instead of the
        // capture's ~60 Hz default (which beats against a 60 Hz output). Older systems refuse
        // these; capture works without them.
        let _ = session.SetIsBorderRequired(false);
        let _ = session.SetIsCursorCaptureEnabled(false);
        if follow_page {
            let _ = session.SetMinUpdateInterval(TimeSpan { Duration: 10_000 });
        }
        let (ctx, tgt) = (context.clone(), target.clone());
        pool.FrameArrived(&TypedEventHandler::new(
            move |p: Ref<Direct3D11CaptureFramePool>, _| -> windows::core::Result<()> {
                let Some(p) = p.as_ref() else { return Ok(()) };
                let frame = p.TryGetNextFrame()?;
                let got = frame.ContentSize()?;
                if got.Width != size.Width || got.Height != size.Height {
                    return Ok(());
                }
                let tex: ID3D11Texture2D = unsafe {
                    frame
                        .Surface()?
                        .cast::<IDirect3DDxgiInterfaceAccess>()?
                        .GetInterface()?
                };
                unsafe { ctx.CopyResource(&tgt, &tex) };
                latest_us.store(now_us(), Ordering::Relaxed);
                captured.fetch_add(1, Ordering::Relaxed);
                Ok(())
            },
        ))
        .map_err(win)?;
        session.StartCapture().map_err(win)?;
        ready
            .send(Ok((browser_pid, unsafe { GetCurrentThreadId() })))
            .ok();

        // Blocks until `Capture` posts WM_QUIT.
        unsafe {
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let _ = session.Close();
        let _ = pool.Close();
        unsafe {
            let _ = controller.Close();
            let _ = DestroyWindow(hwnd);
        }
        Ok(())
    })();
    unsafe { CoUninitialize() };
    result
}

/// An off-screen popup that is never shown and never activates: WebView2's parent.
fn host_window(w: u32, h: u32) -> Result<HWND, FrameError> {
    let class = HSTRING::from("KinetraFrameHost");
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None).map_err(win)?.into();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        // A second capture finds the class already registered: fine.
        RegisterClassW(&wc);
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            &class,
            &HSTRING::from("Kinetra frames"),
            WS_POPUP,
            -32000,
            -32000,
            w as i32,
            h as i32,
            None,
            None,
            Some(instance),
            None,
        )
        .map_err(win)
    }
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, m, w, l) }
}
