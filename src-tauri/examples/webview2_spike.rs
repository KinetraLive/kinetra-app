//! WebView2 spike for media audio routing and output crash recovery (`src/webview2.rs`), run
//! without Kinetra: `cargo run --example webview2_spike`.
//!
//! It creates its own WebView2 environment in a temp folder (never the app's profile) inside a
//! window that is never shown, serves two pages from two local ports and checks:
//! 1. with the stored Microphone permission (`ICoreWebView2Profile4::SetPermissionState` Allow)
//!    for origin A, `enumerateDevices()` on A lists audio outputs with labels and `setSinkId`
//!    accepts a non-default device id, without any capture (`getUserMedia` is never called);
//! 2. origin B (no stored permission) gets no labels (the control);
//! 3. navigating to `edge://kill` / `chrome://crash` raises `ProcessFailed` (render process
//!    exited), the event output windows recover from.
//!
//! Prints one `RESULT` line per check. Windows only.

#[cfg(not(windows))]
fn main() {
    println!("webview2_spike: Windows only");
}

#[cfg(windows)]
fn main() {
    if let Err(e) = spike::run() {
        println!("SPIKE ERROR: {e}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod spike {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::{
        CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
        CreateCoreWebView2EnvironmentCompletedHandler, ProcessFailedEventHandler,
        SetPermissionStateCompletedHandler,
    };
    use windows::core::{Interface, HSTRING, PCWSTR};
    use windows::Win32::Foundation::RECT;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, PeekMessageW, TranslateMessage, MSG,
        PM_REMOVE, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
    };

    type R<T> = Result<T, String>;

    const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><body><script>
(async () => {
  const r = {};
  try { r.perm = (await navigator.permissions.query({ name: "microphone" })).state; }
  catch (e) { r.perm = "error " + e.name; }
  const d = await navigator.mediaDevices.enumerateDevices();
  const outs = d.filter((x) => x.kind === "audiooutput");
  r.outputs = outs.length;
  r.labeled = outs.filter((x) => x.label).length;
  r.labels = outs.map((x) => x.label);
  const dev = outs.find((x) => x.deviceId && x.deviceId !== "default" && x.deviceId !== "communications");
  if (dev) {
    const a = new Audio();
    try { await a.setSinkId(dev.deviceId); r.sink = "ok " + dev.label; }
    catch (e) { r.sink = "error " + e.name + ": " + e.message; }
  } else r.sink = "no non-default device id";
  // Unmuted play() without a user gesture (1 s of digital silence: nothing is heard).
  const n = 8000, buf = new ArrayBuffer(44 + n), v = new DataView(buf);
  const str = (o, s) => [...s].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
  str(0, "RIFF"); v.setUint32(4, 36 + n, true); str(8, "WAVEfmt "); v.setUint32(16, 16, true);
  v.setUint16(20, 1, true); v.setUint16(22, 1, true); v.setUint32(24, 8000, true);
  v.setUint32(28, 8000, true); v.setUint16(32, 1, true); v.setUint16(34, 8, true);
  str(36, "data"); v.setUint32(40, n, true); for (let i = 0; i < n; i++) v.setUint8(44 + i, 128);
  const au = new Audio(URL.createObjectURL(new Blob([buf], { type: "audio/wav" })));
  try {
    r.autoplay = await Promise.race([au.play().then(() => "played unmuted"),
      new Promise((ok) => setTimeout(() => ok("play() pending after 3 s, paused=" + au.paused), 3000))]);
    au.pause();
  }
  catch (e) { r.autoplay = "blocked " + e.name; }
  fetch("/r?" + encodeURIComponent(JSON.stringify(r)));
})();
</script>"#;

    /// Serves `PAGE` at `/` and forwards `/r?<result>` to `tx`.
    fn serve(listener: TcpListener, tx: mpsc::Sender<String>) {
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).map_or(true, |n| n == 0) || h == "\r\n" {
                        break;
                    }
                }
                let path = line.split(' ').nth(1).unwrap_or("/").to_string();
                let mut s = &stream;
                if let Some(q) = path.strip_prefix("/r?") {
                    let _ = tx.send(decode(q));
                    let _ = s.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
                } else {
                    let _ = write!(
                        s,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PAGE}",
                        PAGE.len()
                    );
                }
            }
        });
    }

    fn decode(q: &str) -> String {
        let b = q.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' {
                if let Some(Ok(v)) = q.get(i + 1..i + 3).map(|h| u8::from_str_radix(h, 16)) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Pumps window messages until `rx` has a value or `timeout` passes.
    fn pump<T>(rx: &mpsc::Receiver<T>, timeout: Duration) -> Option<T> {
        let until = Instant::now() + timeout;
        let mut msg = MSG::default();
        while Instant::now() < until {
            if let Ok(v) = rx.try_recv() {
                return Some(v);
            }
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        rx.try_recv().ok()
    }

    pub fn run() -> R<()> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|e| e.to_string())?;
        let a = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let b = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let origin_a = format!(
            "http://127.0.0.1:{}",
            a.local_addr().map_err(|e| e.to_string())?.port()
        );
        let origin_b = format!(
            "http://127.0.0.1:{}",
            b.local_addr().map_err(|e| e.to_string())?.port()
        );
        let (rtx, rrx) = mpsc::channel::<String>();
        serve(a, rtx.clone());
        serve(b, rtx);

        // A window that is never shown (no ShowWindow): nothing appears on a display.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                &HSTRING::from("STATIC"),
                &HSTRING::from("webview2 spike"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                640,
                360,
                None,
                None,
                None,
                None,
            )
        }
        .map_err(|e| format!("window: {e}"))?;

        let dir =
            std::env::temp_dir().join(format!("kinetra-webview2-spike-{}", std::process::id()));
        let started = Instant::now();
        let env = {
            let (tx, rx) = mpsc::channel();
            let opts =
                ICoreWebView2EnvironmentOptions::from(CoreWebView2EnvironmentOptions::default());
            unsafe {
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(),
                    &HSTRING::from(dir.as_os_str()),
                    &opts,
                    &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                        move |err, env| {
                            let _ = tx.send(err.map(|_| env));
                            Ok(())
                        },
                    )),
                )
            }
            .map_err(|e| format!("environment: {e}"))?;
            pump(&rx, Duration::from_secs(20))
                .ok_or("environment: timed out")?
                .map_err(|e| format!("environment: {e}"))?
                .ok_or("environment: none")?
        };
        let controller = {
            let (tx, rx) = mpsc::channel();
            unsafe {
                env.CreateCoreWebView2Controller(
                    hwnd,
                    &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                        move |err, c| {
                            let _ = tx.send(err.map(|_| c));
                            Ok(())
                        },
                    )),
                )
            }
            .map_err(|e| format!("controller: {e}"))?;
            pump(&rx, Duration::from_secs(20))
                .ok_or("controller: timed out")?
                .map_err(|e| format!("controller: {e}"))?
                .ok_or("controller: none")?
        };
        let webview = unsafe {
            let _ = controller.SetBounds(RECT {
                left: 0,
                top: 0,
                right: 640,
                bottom: 360,
            });
            controller.CoreWebView2()
        }
        .map_err(|e| e.to_string())?;
        println!("webview ready in {} ms", started.elapsed().as_millis());

        // The stored permission (what src/webview2.rs does), origin A only.
        let t = Instant::now();
        let profile = webview
            .cast::<ICoreWebView2_13>()
            .and_then(|w| unsafe { w.Profile() })
            .and_then(|p| p.cast::<ICoreWebView2Profile4>())
            .map_err(|e| format!("ICoreWebView2Profile4: {e}"))?;
        for (name, kind) in [
            ("Microphone", COREWEBVIEW2_PERMISSION_KIND_MICROPHONE),
            ("Autoplay", COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY),
        ] {
            let (ptx, prx) = mpsc::channel();
            unsafe {
                profile.SetPermissionState(
                    kind,
                    &HSTRING::from(origin_a.as_str()),
                    COREWEBVIEW2_PERMISSION_STATE_ALLOW,
                    &SetPermissionStateCompletedHandler::create(Box::new(move |r| {
                        let _ = ptx.send(r);
                        Ok(())
                    })),
                )
            }
            .map_err(|e| format!("SetPermissionState: {e}"))?;
            let set = pump(&prx, Duration::from_secs(10)).ok_or("SetPermissionState: timed out")?;
            println!(
                "RESULT SetPermissionState({name}, {origin_a}) -> {set:?} in {} ms",
                t.elapsed().as_millis()
            );
        }

        // ProcessFailed.
        let (ftx, frx) = mpsc::channel::<i32>();
        let mut token = 0i64;
        unsafe {
            webview.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                    if let Some(args) = args {
                        let _ = args.ProcessFailedKind(&mut kind);
                    }
                    let _ = ftx.send(kind.0);
                    Ok(())
                })),
                &mut token,
            )
        }
        .map_err(|e| format!("add_ProcessFailed: {e}"))?;

        for (name, origin) in [("A (permission)", &origin_a), ("B (control)", &origin_b)] {
            unsafe { webview.Navigate(&HSTRING::from(format!("{origin}/").as_str())) }
                .map_err(|e| e.to_string())?;
            match pump(&rrx, Duration::from_secs(15)) {
                Some(r) => println!("RESULT origin {name} {origin}: {r}"),
                None => println!("RESULT origin {name} {origin}: no answer in 15 s"),
            }
        }

        for url in ["edge://kill", "chrome://kill", "chrome://crash"] {
            let _ = unsafe { webview.Navigate(&HSTRING::from(url)) };
            match pump(&frx, Duration::from_secs(5)) {
                Some(k) => {
                    println!(
                        "RESULT ProcessFailed after {url}: kind {k} (render process exited = {})",
                        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED.0
                    );
                    break;
                }
                None => println!("RESULT ProcessFailed after {url}: none in 5 s"),
            }
        }

        unsafe {
            let _ = webview.remove_ProcessFailed(token);
            let _ = controller.Close();
            let _ = DestroyWindow(hwnd);
        }
        drop(env);
        // The browser process lets go of the folder a moment after the controller closes.
        for _ in 0..20 {
            if std::fs::remove_dir_all(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Ok(())
    }
}
