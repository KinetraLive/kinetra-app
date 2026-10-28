//! Measures the remote preview's capture path (Windows): a renderer page captured at the picture
//! size, read back off the output clocks, JPEG-encoded only when it changed (`src/preview.rs`).
//!
//! ```text
//! cargo run --example fixture_server            # prints PORT=<n>; then `show` on its stdin
//! cargo run --example preview_probe -- http://127.0.0.1:<n>/screen/test [seconds] [out.jpg] [fps]
//! cargo run --example preview_probe -- http://127.0.0.1:<n>/screen/test --memory [--streams N]
//! ```
//! The pages are off-screen and never shown. Prints the pictures made, their sizes, the time to
//! the first one and this process's CPU time per second; writes the last picture to `out.jpg`.
//! `--memory`: 1, 2, 3, 4 streams of the page (or only `--streams N` for a long run), then none;
//! after each step the private memory and CPU of this process and its WebView2 processes.
//! Its own WebView2 folder in the temp folder (the app shares its browser process instead, so
//! the first stream here also pays for a whole browser process; compare the steps after it).

#[cfg(not(windows))]
fn main() {
    eprintln!("preview_probe runs on Windows only");
}

#[cfg(windows)]
fn main() {
    probe::main();
}

#[cfg(windows)]
mod probe {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use kinetra_lib::core::remote::preview::{CapturePlan, PreviewHost, PreviewPlan, PreviewSink};
    use kinetra_lib::preview::PreviewCaptures;
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_VM_READ,
    };

    fn ft(f: FILETIME) -> u64 {
        (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
    }

    fn cpu_ms() -> u64 {
        let (mut c, mut e, mut k, mut u) = Default::default();
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) }.unwrap();
        (ft(k) + ft(u)) / 10_000
    }

    /// This process and all its descendants.
    fn tree() -> Vec<u32> {
        let mut pairs = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return vec![std::process::id()];
            };
            let mut e = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snap, &mut e).is_ok() {
                loop {
                    pairs.push((e.th32ProcessID, e.th32ParentProcessID));
                    if Process32NextW(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
        }
        let mut out = vec![std::process::id()];
        let mut i = 0;
        while i < out.len() {
            let p = out[i];
            out.extend(pairs.iter().filter(|(_, q)| *q == p).map(|(c, _)| *c));
            i += 1;
        }
        out
    }

    /// Private memory (MB) and CPU time (ms) of the tree: (sum MB, processes, cpu ms).
    fn tree_usage() -> (f64, usize, u64) {
        let pids = tree();
        let (mut mb, mut cpu) = (0.0, 0u64);
        for &pid in &pids {
            unsafe {
                let Ok(h) = OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
                    false,
                    pid,
                ) else {
                    continue;
                };
                let mut m = PROCESS_MEMORY_COUNTERS_EX {
                    cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
                    ..Default::default()
                };
                if GetProcessMemoryInfo(h, (&mut m as *mut PROCESS_MEMORY_COUNTERS_EX).cast(), m.cb)
                    .is_ok()
                {
                    mb += m.PrivateUsage as f64 / 1_048_576.0;
                }
                let (mut c, mut e, mut k, mut u) = Default::default();
                if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u).is_ok() {
                    cpu += (ft(k) + ft(u)) / 10_000;
                }
                let _ = CloseHandle(h);
            }
        }
        (mb, pids.len(), cpu)
    }

    fn plan(url: &str, n: u16) -> PreviewPlan {
        PreviewPlan {
            captures: (1..=n)
                .map(|stream| CapturePlan {
                    stream,
                    url: url.to_string(),
                    width: 640,
                    height: 360,
                })
                .collect(),
            quality: 70,
            max_bytes: 256 * 1024,
        }
    }

    /// Takes pictures off the sink for `d` (as the preview socket would); returns how many.
    fn drain(sink: &PreviewSink, d: Duration) -> u64 {
        let until = Instant::now() + d;
        let mut n = 0;
        while Instant::now() < until {
            while sink.take_next(0).is_some() {
                n += 1;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        n
    }

    fn memory(url: &str, only: Option<u16>, secs: u64) {
        let dir =
            std::env::temp_dir().join(format!("kinetra-preview-probe-{}", std::process::id()));
        let host = PreviewCaptures::new(dir.clone(), None);
        let sink = Arc::new(PreviewSink::default());
        sink.set_interval(Duration::from_millis(200));
        let (mb0, n0, _) = tree_usage();
        println!("start: {mb0:.0} MB private in {n0} processes");
        let steps: Vec<u16> = match only {
            Some(n) => vec![n, 0],
            None => vec![1, 2, 3, 4, 0],
        };
        let mut last = mb0;
        for n in steps {
            let failed = host.apply(&plan(url, n), &sink);
            assert!(failed.is_empty(), "{failed:?}");
            drain(&sink, Duration::from_secs(5)); // settle
            let (_, _, cpu_a) = tree_usage();
            let t = Instant::now();
            let pictures = drain(&sink, Duration::from_secs(secs));
            let (mb, procs, cpu_b) = tree_usage();
            let cores = (cpu_b.saturating_sub(cpu_a)) as f64 / t.elapsed().as_millis() as f64;
            println!(
            "{n} stream(s): {mb:.0} MB private ({:+.0} MB on the step before, {:+.0} on start) in \
             {procs} processes; {:.1} pictures/s; CPU {cores:.3} cores (all processes)",
            mb - last,
            mb - mb0,
            pictures as f64 / secs as f64,
        );
            last = mb;
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    pub fn main() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let Some(page) = args.first().cloned() else {
            eprintln!(
                "usage: preview_probe <renderer page URL> [seconds] [out.jpg] [fps] | --memory"
            );
            std::process::exit(2);
        };
        let sep = if page.contains('?') { '&' } else { '?' };
        let url = format!("{page}{sep}output=preview&bg=checker&mute=1&audio=0");
        if args.iter().any(|a| a == "--memory") {
            let only = args
                .iter()
                .position(|a| a == "--streams")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok());
            let secs = args
                .iter()
                .position(|a| a == "--secs")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(10);
            memory(&url, only, secs);
            return;
        }
        let mut rest = args.into_iter().skip(1);
        let secs: u64 = rest.next().and_then(|s| s.parse().ok()).unwrap_or(10);
        let out = rest.next();
        let fps: u64 = rest.next().and_then(|s| s.parse().ok()).unwrap_or(5);
        let dir =
            std::env::temp_dir().join(format!("kinetra-preview-probe-{}", std::process::id()));
        let host = PreviewCaptures::new(dir.clone(), None);
        let sink = Arc::new(PreviewSink::default());
        sink.set_interval(Duration::from_millis(1000 / fps.max(1)));
        let t0 = Instant::now();
        let failed = host.apply(&plan(&url, 1), &sink);
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
}
