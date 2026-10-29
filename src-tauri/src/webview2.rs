//! WebView2 specifics (Windows) Tauri doesn't expose, through `webview2-com` on the controller
//! wry already created. Elsewhere these are no-ops.
//!
//! - [`allow_audio_devices`]: stores the Microphone permission state Allow for an origin in the
//!   webview's profile (`ICoreWebView2Profile4::SetPermissionState`). WebView2 hides audio output
//!   devices (empty labels, no device ids for `setSinkId`) until the page has microphone
//!   permission; a stored state grants it without a prompt and without capturing anything (no
//!   page of ours calls `getUserMedia`). Persisted in the profile, so it holds for every window
//!   of that origin. Verified on WebView2 with `examples/webview2_spike.rs`: the
//!   granted origin lists every output with its label and `setSinkId` accepts a device id; an
//!   origin without it sees one unlabeled default device.
//! - [`allow_output_media`]: the same plus the Autoplay permission state Allow, so output pages
//!   play media unmuted without a user gesture. Chosen over the browser argument
//!   `--autoplay-policy=no-user-gesture-required`: Tauri doesn't expose wry's `autoplay`, and
//!   custom browser arguments replace wry's defaults and must be identical for every window
//!   sharing the user data folder (a mismatch fails webview creation). Spike: on the granted
//!   origin unmuted `play()` was accepted (`paused=false`; its promise stays pending only
//!   because the spike's window is never shown), on the other origin it rejects at once with
//!   `NotAllowedError`.
//! - [`on_process_failed`]: calls back when the page's render process exits or hangs, or the
//!   browser process exits (output window crash recovery). GPU and utility process exits are
//!   left to WebView2, which restarts them.

use tauri::WebviewWindow;

/// Why a page died (reported by WebView2 on Windows).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFailure {
    BrowserExited,
    RenderExited,
    RenderUnresponsive,
}

/// `scheme://host[:port]` of a URL, the form `SetPermissionState` takes.
pub fn origin_of(url: &tauri::Url) -> Option<String> {
    let o = url.origin();
    o.is_tuple().then(|| o.ascii_serialization())
}

/// The origin app pages load from: the dev server in development, else the custom protocol's
/// `http(s)://tauri.localhost` (WebView2's form of it).
pub fn app_origin(config: &tauri::Config, label: &str) -> Option<String> {
    if tauri::is_dev() {
        if let Some(url) = &config.build.dev_url {
            return origin_of(url);
        }
    }
    let https = config
        .app
        .windows
        .iter()
        .find(|w| w.label == label)
        .is_some_and(|w| w.use_https_scheme);
    Some(
        if https {
            "https://tauri.localhost"
        } else {
            "http://tauri.localhost"
        }
        .to_string(),
    )
}

#[cfg(windows)]
mod imp {
    use super::PageFailure;
    use tauri::WebviewWindow;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Profile4, ICoreWebView2_13, COREWEBVIEW2_PERMISSION_KIND,
        COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE,
        COREWEBVIEW2_PERMISSION_STATE, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
        COREWEBVIEW2_PERMISSION_STATE_DEFAULT, COREWEBVIEW2_PROCESS_FAILED_KIND,
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
    };
    use webview2_com::{ProcessFailedEventHandler, SetPermissionStateCompletedHandler};
    use windows::core::{Interface, HSTRING};

    pub fn set_permissions(w: &WebviewWindow, origin: String, autoplay: bool, allow: bool) {
        let label = w.label().to_string();
        let state: COREWEBVIEW2_PERMISSION_STATE = if allow {
            COREWEBVIEW2_PERMISSION_STATE_ALLOW
        } else {
            COREWEBVIEW2_PERMISSION_STATE_DEFAULT
        };
        // Runs on the main thread; the call is asynchronous (its handler only logs).
        let r = w.with_webview(move |pw| {
            let result = (|| -> windows::core::Result<()> {
                let mut kinds: Vec<COREWEBVIEW2_PERMISSION_KIND> =
                    vec![COREWEBVIEW2_PERMISSION_KIND_MICROPHONE];
                if autoplay {
                    kinds.push(COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY);
                }
                let profile = unsafe { pw.controller().CoreWebView2()? }
                    .cast::<ICoreWebView2_13>()
                    .and_then(|w| unsafe { w.Profile() })?
                    .cast::<ICoreWebView2Profile4>()?;
                for kind in kinds {
                    let done_label = label.clone();
                    let done_origin = origin.clone();
                    unsafe {
                        profile.SetPermissionState(
                            kind,
                            &HSTRING::from(origin.as_str()),
                            state,
                            &SetPermissionStateCompletedHandler::create(Box::new(move |r| {
                                if let Err(e) = r {
                                    tracing::warn!(
                                        "{done_label}: media permission {} for {done_origin}: {e}",
                                        kind.0
                                    );
                                }
                                Ok(())
                            })),
                        )?;
                    }
                }
                Ok(())
            })();
            if let Err(e) = result {
                // An old WebView2 runtime (before 1.0.1661) has no Profile4: the default
                // device still plays.
                tracing::warn!("{label}: audio devices for {origin}: {e}");
            }
        });
        if let Err(e) = r {
            tracing::warn!("audio device permission: {e}");
        }
    }

    pub fn on_process_failed(w: &WebviewWindow, f: Box<dyn Fn(PageFailure) + Send + 'static>) {
        let label = w.label().to_string();
        let r = w.with_webview(move |pw| {
            let result = (|| -> windows::core::Result<()> {
                let webview = unsafe { pw.controller().CoreWebView2()? };
                let mut token = 0i64;
                let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                    if let Some(args) = args {
                        unsafe { args.ProcessFailedKind(&mut kind)? };
                    }
                    let failure = match kind {
                        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => {
                            PageFailure::BrowserExited
                        }
                        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => {
                            PageFailure::RenderExited
                        }
                        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => {
                            PageFailure::RenderUnresponsive
                        }
                        _ => return Ok(()),
                    };
                    f(failure);
                    Ok(())
                }));
                // Removed with the webview (the window is destroyed, never reused).
                unsafe { webview.add_ProcessFailed(&handler, &mut token) }
            })();
            if let Err(e) = result {
                tracing::warn!("{label}: crash recovery unavailable: {e}");
            }
        });
        if let Err(e) = r {
            tracing::warn!("crash recovery: {e}");
        }
    }
}

/// Lets pages of `origin` see audio output devices (labels and ids) in this window's profile.
/// `allow: false` puts the origin back to the default (asks; never asked in practice).
pub fn allow_audio_devices(w: &WebviewWindow, origin: &str, allow: bool) {
    #[cfg(windows)]
    imp::set_permissions(w, origin.to_string(), false, allow);
    #[cfg(not(windows))]
    let _ = (w, origin, allow);
}

/// [`allow_audio_devices`] plus unmuted autoplay without a user gesture (output windows: media
/// layers play with sound from Take, nobody clicks an output).
pub fn allow_output_media(w: &WebviewWindow, origin: &str, allow: bool) {
    #[cfg(windows)]
    imp::set_permissions(w, origin.to_string(), true, allow);
    #[cfg(not(windows))]
    let _ = (w, origin, allow);
}

/// Calls `f` (on the main thread: keep it short, never touch the window from it) when the
/// window's page fails.
pub fn on_process_failed(w: &WebviewWindow, f: impl Fn(PageFailure) + Send + 'static) {
    #[cfg(windows)]
    imp::on_process_failed(w, Box::new(f));
    #[cfg(not(windows))]
    let _ = (w, f);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins() {
        let o = |s: &str| origin_of(&tauri::Url::parse(s).unwrap());
        assert_eq!(
            o("http://127.0.0.1:7770/screen/a?output=b").as_deref(),
            Some("http://127.0.0.1:7770")
        );
        assert_eq!(
            o("http://tauri.localhost/settings").as_deref(),
            Some("http://tauri.localhost")
        );
        assert_eq!(
            o("http://localhost:1420/").as_deref(),
            Some("http://localhost:1420")
        );
        assert_eq!(o("data:text/html,x"), None);
    }
}
