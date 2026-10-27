//! Measures the remote preview's capture path (Windows): a renderer page captured at the picture
//! size, read back off the output clocks, JPEG-encoded only when it changed (`src/preview.rs`).
//!
//! ```text
//! cargo run --example fixture_server            # prints PORT=<n>; then `show` on its stdin
//! cargo run --example preview_probe -- http://127.0.0.1:<n>/screen/test [seconds] [out.jpg] [fps]
//! ```
//! The page is off-screen and never shown. Prints the pictures made, their sizes, the time to
//! the first one and this process's CPU time per second; writes the last picture to `out.jpg`.
//! Its own WebView2 folder in the temp folder (the app shares its browser process instead).

use std::sync::Arc;
use std::time::{Duration, Instant};

use kinetra_lib::core::remote::preview::{CapturePlan, PreviewHost, PreviewPlan, PreviewSink};
use kinetra_lib::preview::PreviewCaptures;

fn cpu_ms() -> u64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }.unwrap();
    let t = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
    (t(k) + t(u)) / 10_000
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(page) = args.next() else {
        eprintln!("usage: preview_probe <renderer page URL> [seconds] [out.jpg] [fps]");
        std::process::exit(2);
    };
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);
    let out = args.next();
    let fps: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    let sep = if page.contains('?') { '&' } else { '?' };
    let url = format!("{page}{sep}output=preview&bg=checker&mute=1&audio=0");
    let dir = std::env::temp_dir().join(format!("kinetra-preview-probe-{}", std::process::id()));
    let host = PreviewCaptures::new(dir.clone(), None);
    let sink = Arc::new(PreviewSink::default());
    sink.set_interval(Duration::from_millis(1000 / fps.max(1)));
    let plan = PreviewPlan {
        captures: vec![CapturePlan {
            stream: 1,
            url,
            width: 640,
            height: 360,
        }],
        quality: 70,
        max_bytes: 256 * 1024,
    };
    let t0 = Instant::now();
    let failed = host.apply(&plan, &sink);
    if !failed.is_empty() {
        eprintln!("capture failed: {failed:?}");
        std::process::exit(1);
    }
    println!("capture started in {} ms", t0.elapsed().as_millis());
    let started = Instant::now();
    let cpu0 = cpu_ms();
    let (mut n, mut bytes, mut first, mut last) = (0u64, 0usize, None, None);
    while started.elapsed() < Duration::from_secs(secs) {
        while let Some((_, _, f)) = sink.take_next(0) {
            n += 1;
            bytes += f.jpeg.len();
            first.get_or_insert(started.elapsed());
            last = Some(f);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let cpu = cpu_ms() - cpu0;
    println!(
        "pictures: {n} in {secs} s (changed only); avg {} B; first after {} ms",
        bytes.checked_div(n as usize).unwrap_or(0),
        first.map_or(0, |d| d.as_millis())
    );
    println!(
        "process CPU: {cpu} ms over {secs} s = {:.3} cores (this probe process; the page's \
         WebView2 processes are separate)",
        cpu as f64 / 1000.0 / secs as f64
    );
    if let (Some(path), Some(f)) = (out, last) {
        std::fs::write(&path, &f.jpeg).unwrap();
        println!("last picture {}×{} → {path}", f.width, f.height);
    }
    host.apply(&PreviewPlan::default(), &sink);
    let _ = std::fs::remove_dir_all(dir);
}
