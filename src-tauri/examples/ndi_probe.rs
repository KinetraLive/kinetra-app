//! NDI® output probe (Windows, developer machines with the NDI Runtime installed; never in CI):
//! the real frame pipeline (page → GPU capture → UYVA → NDI) into a receiver in a second process,
//! measured. Nothing appears on screen: the capture's host window is never shown.
//!
//! ```text
//! cargo build --example fixture_server            (only for --page renderer)
//! cargo run --example ndi_probe -- [options]
//!   --page test|heavy|renderer   what to capture (default test)
//!       test:     transparent page with known patches, a moving bar and a frame counter
//!       heavy:    full-frame noise with varying alpha, moving every frame (worst case for SpeedHQ)
//!       renderer: Kinetra's renderer with 4 presets on air (lower third, scoreboard + clock,
//!                 ticker, bug) from the fixture server, like the output-engine bench
//!   --rate 50|59.94|60|…         (default 50)      --size 1920x1080
//!   --format uyva|uyvy|bgra      (default uyva)    --alpha straight|premultiplied
//!   --mode async|sync            (default async)   --adapter gpu|warp
//!   --secs N (default 30)        --warmup N (default 5)
//!   --png PATH                   write one received frame (straight RGBA)
//! ```
//!
//! Prints one JSON report: send-call time (the SpeedHQ encode in sync mode), tick → sent, CPU of
//! this process and of the capture's WebView2 processes, memory, counters, what the receiver got
//! (frames, NDI drops, counter repeats/skips, alpha and color at known pixels, latency) and the
//! thread count before, during and after (zero cost when off).

#[cfg(not(windows))]
fn main() {
    eprintln!("ndi_probe runs on Windows only");
}

#[cfg(windows)]
fn main() {
    probe::main();
}

#[cfg(windows)]
mod probe {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use kinetra_lib::core::frame_out::{percentile, AlphaMode, FrameRate, PixelFormat};
    use kinetra_lib::frames::{Adapter, FrameSource, Page, SourceConfig};
    use kinetra_lib::ndi::recv::Receiver;
    use kinetra_lib::ndi::{NdiSender, Runtime, SendMode};
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::*;

    fn arg(args: &[String], k: &str) -> Option<String> {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    }

    pub fn main() {
        let args: Vec<String> = std::env::args().collect();
        let r = if let Some(name) = arg(&args, "--receive") {
            receive(&name, &args)
        } else if args.iter().any(|a| a == "--host") {
            host(&args)
        } else if args.iter().any(|a| a == "--memory") {
            memory()
        } else if args.iter().any(|a| a == "--share-test") {
            share_test()
        } else {
            probe(&args)
        };
        if let Err(e) = r {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Pages

    /// The test page: transparent, an opaque green block (100..300, 100..300), a 50 % red block
    /// (400..600, 100..300), a bar moving with time, and the drawn frame's time (ms, 24 bits of
    /// `timeOrigin + rAF time`) as binary squares on row 40 (white = 1), so the receiver can measure
    /// each output frame's content-time step. The outer 8 px stay clear (capture border check).
    fn test_page(rate: FrameRate) -> String {
        let period = 1000.0 / rate.fps();
        format!(
            r#"<!doctype html><html><head><style>
    html,body{{margin:0;background:transparent;overflow:hidden}}
    #g{{position:absolute;left:100px;top:100px;width:200px;height:200px;background:rgb(0,255,0)}}
    #r{{position:absolute;left:400px;top:100px;width:200px;height:200px;background:rgba(255,0,0,0.5)}}
    #m{{position:absolute;left:0;top:900px;width:120px;height:60px;background:#1e90ff;will-change:transform}}
    .b{{position:absolute;top:40px;width:16px;height:16px;background:#000}}
    </style></head><body><div id="g"></div><div id="r"></div><div id="m"></div><div id="bits"></div><script>
    const bits=document.getElementById('bits'),m=document.getElementById('m'),el=[];
    for(let i=0;i<24;i++){{const d=document.createElement('div');d.className='b';d.style.left=(16+i*16)+'px';bits.appendChild(d);el.push(d)}}
    function f(t){{const n=Math.floor(performance.timeOrigin+t)%16777216;
    for(let i=0;i<24;i++)el[i].style.background=(n>>i)&1?'#fff':'#000';
    m.style.transform=`translateX(${{Math.floor(n/{period})*8%1700+8}}px)`;requestAnimationFrame(f)}}requestAnimationFrame(f);
    </script></body></html>"#
        )
    }

    /// Worst case for the encoder: full-frame noise with varying alpha, moved every frame.
    fn heavy_page(w: u32, h: u32) -> String {
        format!(
            r#"<!doctype html><html><head><style>html,body{{margin:0;background:transparent;overflow:hidden}}
    canvas{{position:absolute;left:-64px;top:0;will-change:transform}}</style></head><body><canvas id="c" width="{cw}" height="{h}"></canvas><script>
    const c=document.getElementById('c'),x=c.getContext('2d'),d=x.createImageData({cw},{h}),a=d.data;
    let s=12345;for(let i=0;i<a.length;i++){{s=(s*1103515245+12345)&0x7fffffff;a[i]=s>>16&255}}
    x.putImageData(d,0,0);let k=0;function f(){{k=(k+3)%64;c.style.transform=`translateX(${{k}}px)`;requestAnimationFrame(f)}}requestAnimationFrame(f);
    </script></body></html>"#,
            cw = w + 64,
        )
    }

    /// The fixture server (Kinetra's real output server, as the e2e tests use it) with the bench's
    /// scene: four presets on air.
    struct Fixture {
        child: std::process::Child,
        stdin: std::process::ChildStdin,
        stdout: BufReader<std::process::ChildStdout>,
        port: u16,
    }

    impl Fixture {
        fn start() -> Result<Self, String> {
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            let exe = dir
                .join("target")
                .join("debug")
                .join("examples")
                .join("fixture_server.exe");
            if !exe.exists() {
                return Err(format!(
                    "{} missing: cargo build --example fixture_server",
                    exe.display()
                ));
            }
            let mut child = Command::new(&exe)
                .current_dir(dir)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| e.to_string())?;
            let stdin = child.stdin.take().ok_or("stdin")?;
            let mut stdout = BufReader::new(child.stdout.take().ok_or("stdout")?);
            let mut line = String::new();
            stdout.read_line(&mut line).map_err(|e| e.to_string())?;
            let port = line
                .trim()
                .strip_prefix("PORT=")
                .and_then(|p| p.parse().ok())
                .ok_or(format!("fixture said {line:?}"))?;
            Ok(Self {
                child,
                stdin,
                stdout,
                port,
            })
        }

        fn cmd(&mut self, c: &str) -> Result<String, String> {
            writeln!(self.stdin, "{c}").map_err(|e| e.to_string())?;
            self.stdin.flush().map_err(|e| e.to_string())?;
            let mut line = String::new();
            self.stdout
                .read_line(&mut line)
                .map_err(|e| e.to_string())?;
            let line = line.trim_end();
            line.strip_prefix("OK")
                .map(|v| v.trim_start().to_string())
                .ok_or_else(|| format!("{c:.60} → {line}"))
        }

        fn scene(&mut self) -> Result<String, String> {
            let lt = self.cmd("addtemplate lower-third")?;
            let sb = self.cmd("addtemplate scoreboard")?;
            let tk = self.cmd("addtemplate ticker")?;
            let bug = self.cmd("addtemplate corner-bug")?;
            let items = r#"[{\"text\":\"Bergen 2 - 1 Oslo, 2nd period\"},{\"text\":\"Tromsø leads the table after round 14\"},{\"text\":\"Next: Stavanger v Trondheim at 19:30\"}]"#;
            let layer = |id: &str,
                         name: &str,
                         tpl: &str,
                         r: (i32, i32, i32, i32),
                         sizing: &str,
                         fields: &str| {
                format!(
                    r#"{{"id":"{id}","name":"{name}","kind":"template","templateId":"{tpl}","rect":{{"x":{},"y":{},"w":{},"h":{}}},"sizing":"{sizing}","fields":{fields},"vars":{{}}}}"#,
                    r.0, r.1, r.2, r.3
                )
            };
            let layers = [
                layer(
                    "sb",
                    "Scoreboard",
                    &sb,
                    (96, 65, 700, 68),
                    "fit",
                    r#"{"home":"BERGEN","home_score":"2","away":"OSLO","away_score":"1","period":"2ND"}"#,
                ),
                layer(
                    "tk",
                    "Ticker",
                    &tk,
                    (0, 932, 1920, 72),
                    "reflow",
                    &format!(r#"{{"label":"LATEST","items":"{items}"}}"#),
                ),
                layer(
                    "bug",
                    "Bug",
                    &bug,
                    (1584, 65, 240, 75),
                    "fit",
                    r#"{"text":"LIVE"}"#,
                ),
                layer(
                    "lt",
                    "Lower third",
                    &lt,
                    (115, 783, 960, 200),
                    "fit",
                    r#"{"label":"GUEST","name":"Ingrid Vale","title":"Head coach, Nordlys"}"#,
                ),
            ];
            let sid = "probe";
            self.cmd(&format!(
                r#"savescreen {{"id":"{sid}","name":"Probe","width":1920,"height":1080,"background":"transparent","layers":[{}],"updatedAt":""}}"#,
                layers.join(",")
            ))?;
            for l in ["sb", "tk", "bug", "lt"] {
                self.cmd(&format!("layershow {sid} {l}"))?;
            }
            self.cmd(&format!(r#"settimer {sid} sb clock {{"action":"start"}}"#))?;
            Ok(format!(
                "http://127.0.0.1:{}/screen/{sid}?audio=0",
                self.port
            ))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = writeln!(self.stdin, "quit");
            let _ = self.child.wait();
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Process measurements

    fn ft(t: FILETIME) -> u64 {
        (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
    }

    /// CPU time (kernel + user) of a process in 100 ns units; None if it's gone.
    fn cpu_100ns(pid: u32) -> Option<u64> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let (mut c, mut e, mut k, mut u) = Default::default();
            let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u).is_ok();
            let _ = CloseHandle(h);
            ok.then(|| ft(k) + ft(u))
        }
    }

    /// (idle, kernel + user) time of the whole system, 100 ns units.
    fn system_times() -> (u64, u64) {
        let (mut i, mut k, mut u) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        unsafe {
            let _ = GetSystemTimes(Some(&mut i), Some(&mut k), Some(&mut u));
        }
        (ft(i), ft(k) + ft(u))
    }

    /// `root` and all its descendants.
    fn process_tree(root: u32) -> Vec<u32> {
        let mut pairs = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return vec![root];
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
        let mut out = vec![root];
        let mut i = 0;
        while i < out.len() {
            let p = out[i];
            out.extend(
                pairs
                    .iter()
                    .filter(|(_, parent)| *parent == p)
                    .map(|(c, _)| *c),
            );
            i += 1;
        }
        out
    }

    /// Kills the msedgewebview2 processes whose parent is another msedgewebview2 under this
    /// process (the capture's renderer, GPU and utility processes; not its browser process).
    fn kill_webview_children() -> usize {
        let me = std::process::id();
        let mut procs = Vec::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return 0;
            };
            let mut e = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snap, &mut e).is_ok() {
                loop {
                    let n = e
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(e.szExeFile.len());
                    let name = String::from_utf16_lossy(&e.szExeFile[..n]).to_lowercase();
                    procs.push((e.th32ProcessID, e.th32ParentProcessID, name));
                    if Process32NextW(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
        }
        let browsers: Vec<u32> = procs
            .iter()
            .filter(|(_, parent, n)| *parent == me && n == "msedgewebview2.exe")
            .map(|(p, _, _)| *p)
            .collect();
        let mut killed = 0;
        for (pid, parent, n) in &procs {
            if n == "msedgewebview2.exe" && browsers.contains(parent) {
                unsafe {
                    if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, *pid) {
                        if TerminateProcess(h, 1).is_ok() {
                            killed += 1;
                        }
                        let _ = CloseHandle(h);
                    }
                }
            }
        }
        killed
    }

    fn thread_count() -> usize {
        let me = std::process::id();
        let mut n = 0;
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else {
                return 0;
            };
            let mut e = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            if Thread32First(snap, &mut e).is_ok() {
                loop {
                    if e.th32OwnerProcessID == me {
                        n += 1;
                    }
                    if Thread32Next(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
        }
        n
    }

    /// Private memory of another process (MB; 0 if it's gone).
    fn private_mb_of(pid: u32) -> f64 {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return 0.0;
            };
            let mut m = PROCESS_MEMORY_COUNTERS_EX {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
                ..Default::default()
            };
            let _ =
                GetProcessMemoryInfo(h, (&mut m as *mut PROCESS_MEMORY_COUNTERS_EX).cast(), m.cb);
            let _ = CloseHandle(h);
            m.PrivateUsage as f64 / 1_048_576.0
        }
    }

    fn private_mb() -> f64 {
        unsafe {
            let mut m = PROCESS_MEMORY_COUNTERS_EX {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
                ..Default::default()
            };
            let _ = GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut m as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                m.cb,
            );
            m.PrivateUsage as f64 / 1_048_576.0
        }
    }

    fn stats_json(sorted_us: &[u32]) -> String {
        let ms = |p: f64| {
            percentile(sorted_us, p)
                .map_or("null".into(), |v| format!("{:.3}", f64::from(v) / 1000.0))
        };
        format!(
            r#"{{"n":{},"p50":{},"p99":{},"max":{}}}"#,
            sorted_us.len(),
            ms(0.5),
            ms(0.99),
            ms(1.0)
        )
    }

    fn cores_json(per_second: &[f64]) -> String {
        let mut v = per_second.to_vec();
        v.sort_by(|a, b| a.total_cmp(b));
        let avg = if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        };
        let p = |q: f64| {
            v.get(((v.len().max(1) - 1) as f64 * q).round() as usize)
                .copied()
                .unwrap_or(0.0)
        };
        format!(
            r#"{{"avg":{avg:.3},"p99":{:.3},"max":{:.3}}}"#,
            p(0.99),
            p(1.0)
        )
    }

    // ---------------------------------------------------------------------------------------------
    // Sender side

    fn probe(args: &[String]) -> Result<(), String> {
        let rate = FrameRate::parse(&arg(args, "--rate").unwrap_or_else(|| "50".into()))
            .ok_or("unknown --rate")?;
        let (w, h) = arg(args, "--size")
            .and_then(|s| {
                s.split_once('x')
                    .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
            })
            .unwrap_or((1920u32, 1080u32));
        let format = match arg(args, "--format").as_deref().unwrap_or("uyva") {
            "uyva" => PixelFormat::Uyva,
            "uyvy" => PixelFormat::Uyvy,
            "bgra" => PixelFormat::Bgra,
            f => return Err(format!("unknown --format {f}")),
        };
        let alpha = match arg(args, "--alpha").as_deref().unwrap_or("straight") {
            "premultiplied" => AlphaMode::Premultiplied,
            _ => AlphaMode::Straight,
        };
        let mode = if arg(args, "--mode").as_deref() == Some("sync") {
            SendMode::Sync
        } else {
            SendMode::Async
        };
        let adapter = match arg(args, "--adapter").as_deref() {
            Some("warp") => Adapter::Warp,
            Some("primary") => Adapter::Primary,
            _ => Adapter::HighPerformance,
        };
        // --no-follow: the capture's ~60 Hz default (Windows 10 / Windows 11 before 24H2).
        let follow_page = !args.iter().any(|a| a == "--no-follow");
        // --wry-args: only wry's default browser arguments (what the app's own environment has; no
        // occlusion / background-throttling switches): does a hidden page still draw?
        let browser_args = args
            .iter()
            .any(|a| a == "--wry-args")
            .then(|| "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection".to_string());
        let secs: u64 = arg(args, "--secs")
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);
        let warmup: u64 = arg(args, "--warmup")
            .and_then(|s| s.parse().ok())
            .unwrap_or(5);
        let page_kind = arg(args, "--page").unwrap_or_else(|| "test".into());

        let threads_before = thread_count();
        let mut fixture = None;
        let page = match page_kind.as_str() {
            "test" => Page::Html(test_page(rate)),
            "heavy" => Page::Html(heavy_page(w, h)),
            "renderer" => {
                let mut f = Fixture::start()?;
                let url = f.scene()?;
                fixture = Some(f);
                Page::Url(url)
            }
            p => return Err(format!("unknown --page {p}")),
        };
        let data_dir = std::env::temp_dir().join("kinetra-ndi-probe-webview");
        // --cycles N: turn a whole output (source + NDI sender) on and off N times first; threads
        // and memory must not grow per cycle.
        let cycles: u32 = arg(args, "--cycles")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let mut per_cycle = Vec::new();
        for i in 0..cycles {
            let s = FrameSource::start(
                SourceConfig {
                    page: Page::Html(test_page(rate)),
                    width: w,
                    height: h,
                    rate,
                    format,
                    alpha,
                    background: [0, 0, 0],
                    data_dir: data_dir.clone(),
                    adapter,
                    follow_page,
                    browser_args: browser_args.clone(),
                    language: None,
                },
                None,
            )
            .map_err(|e| e.to_string())?;
            let rt = Runtime::acquire().map_err(|e| e.to_string())?;
            let o = s.subscribe();
            let snd = NdiSender::start(
                rt.clone(),
                &format!("Kinetra cycle {i}"),
                None,
                o.clone(),
                mode,
                None,
            )
            .map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_secs(2));
            drop(snd);
            drop(rt);
            s.unsubscribe(&o);
            drop(s);
            std::thread::sleep(Duration::from_millis(1500));
            per_cycle.push(format!(
                r#"{{"threads":{},"privateMb":{:.1}}}"#,
                thread_count(),
                private_mb()
            ));
        }
        let cycles_json = format!("[{}]", per_cycle.join(","));
        let source = FrameSource::start(
            SourceConfig {
                page,
                width: w,
                height: h,
                rate,
                format,
                alpha,
                background: [0, 0, 0],
                data_dir,
                adapter,
                follow_page,
                browser_args: browser_args.clone(),
                language: None,
            },
            None,
        )
        .map_err(|e| e.to_string())?;
        let runtime = Runtime::acquire().map_err(|e| e.to_string())?;
        let name = format!("Kinetra probe {}", std::process::id());
        let outlet = source.subscribe();
        let sender = NdiSender::start(runtime.clone(), &name, None, outlet.clone(), mode, None)
            .map_err(|e| e.to_string())?;

        let total = warmup + secs;
        let no_receiver = args.iter().any(|a| a == "--no-receiver");
        let mut child = (!no_receiver)
            .then(|| {
                Command::new(std::env::current_exe().unwrap_or_default())
                    .args([
                        "--receive",
                        &name,
                        if page_kind == "test" {
                            "--test-page"
                        } else {
                            "--no-checks"
                        },
                        "--secs",
                        &secs.to_string(),
                        "--warmup",
                        &warmup.to_string(),
                    ])
                    .args(
                        arg(args, "--png")
                            .map(|p| vec!["--png".to_string(), p])
                            .unwrap_or_default(),
                    )
                    .args([
                        "--format",
                        match format {
                            PixelFormat::Uyva => "uyva",
                            PixelFormat::Uyvy => "uyvy",
                            PixelFormat::Bgra => "bgra",
                        },
                    ])
                    .stdout(Stdio::piped())
                    .spawn()
                    .ok()
            })
            .flatten();

        // --stress N: N threads spinning at normal priority while measuring (overload).
        let stress: usize = arg(args, "--stress")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let spin = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let spinners: Vec<_> = (0..stress)
            .map(|_| {
                let spin = spin.clone();
                std::thread::spawn(move || {
                    let mut x = 0u64;
                    while spin.load(Ordering::Relaxed) {
                        x = std::hint::black_box(
                            x.wrapping_mul(6364136223846793005).wrapping_add(1),
                        );
                    }
                })
            })
            .collect();
        std::thread::sleep(Duration::from_secs(warmup));
        let c0 = source.counters();
        let dropped0 = outlet.dropped.load(Ordering::Relaxed);
        let sent0 = sender.stats.sent.load(Ordering::Relaxed);
        sender.stats.send_time.drain_sorted();
        sender.stats.send_time.enable(true);
        sender.stats.tick_to_sent.enable(true);
        let tree = process_tree(source.browser_pid);
        let me = std::process::id();
        let mem_start = private_mb();
        let threads_during = thread_count();
        let gpu_name = source.adapter_name.clone();
        let (mut own, mut web) = (Vec::new(), Vec::new());
        let mut last_own = cpu_100ns(me).unwrap_or(0);
        let mut last_web: u64 = tree.iter().filter_map(|p| cpu_100ns(*p)).sum();
        let mut last_t = Instant::now();
        let end = Instant::now() + Duration::from_secs(secs);
        // --timeline FILE: one CSV line per second (CPU of this process, of the capture's WebView2
        // and of the whole system; frames sent and dropped; late and missed ticks; memory).
        let mut timeline = match arg(args, "--timeline") {
            Some(p) => {
                let mut f = std::fs::File::create(&p).map_err(|e| format!("{p}: {e}"))?;
                writeln!(
                    f,
                    "second,probe_cores,webview2_cores,system_busy_pct,sent,dropped_before_send,late,missed,receivers,private_mb,webview2_mb"
                )
                .map_err(|e| e.to_string())?;
                Some(f)
            }
            None => None,
        };
        let (mut last_sys, mut last_c, mut last_sent, mut last_dropped) =
            (system_times(), source.counters(), sent0, dropped0);
        let mut second = 0u64;
        while Instant::now() < end {
            std::thread::sleep(Duration::from_secs(1));
            let dt = last_t.elapsed().as_secs_f64();
            last_t = Instant::now();
            let o = cpu_100ns(me).unwrap_or(last_own);
            let wv: u64 = tree.iter().filter_map(|p| cpu_100ns(*p)).sum();
            own.push((o - last_own) as f64 / 1e7 / dt);
            web.push(wv.saturating_sub(last_web) as f64 / 1e7 / dt);
            (last_own, last_web) = (o, wv);
            if let Some(f) = timeline.as_mut() {
                second += 1;
                let sys = system_times();
                let (idle, total) = (sys.0 - last_sys.0, sys.1 - last_sys.1);
                let busy = if total > 0 {
                    100.0 * (1.0 - idle as f64 / total as f64)
                } else {
                    0.0
                };
                last_sys = sys;
                let c = source.counters();
                let sent = sender.stats.sent.load(Ordering::Relaxed);
                let dropped = outlet.dropped.load(Ordering::Relaxed);
                let _ = writeln!(
                    f,
                    "{second},{:.3},{:.3},{busy:.1},{},{},{},{},{},{:.1},{:.1}",
                    own.last().copied().unwrap_or(0.0),
                    web.last().copied().unwrap_or(0.0),
                    sent - last_sent,
                    dropped - last_dropped,
                    c.late - last_c.late,
                    c.missed - last_c.missed,
                    sender.stats.receivers.load(Ordering::Relaxed),
                    private_mb(),
                    tree.iter().map(|p| private_mb_of(*p)).sum::<f64>(),
                );
                (last_c, last_sent, last_dropped) = (c, sent, dropped);
            }
        }
        spin.store(false, Ordering::Relaxed);
        for s in spinners {
            s.join().ok();
        }
        let send = sender.stats.send_time.drain_sorted();
        let tick_to_sent = sender.stats.tick_to_sent.drain_sorted();
        let c1 = source.counters();
        let receivers = sender.stats.receivers.load(Ordering::Relaxed);
        let sent = sender.stats.sent.load(Ordering::Relaxed) - sent0;
        let skipped = sender.stats.skipped.load(Ordering::Relaxed);
        let dropped = outlet.dropped.load(Ordering::Relaxed) - dropped0;
        let mem_end = private_mb();
        let webview_mb: f64 = tree.iter().map(|p| private_mb_of(*p)).sum();

        let mut received = String::new();
        if let Some(child) = child.as_mut() {
            if let Some(out) = child.stdout.take() {
                BufReader::new(out).read_line(&mut received).ok();
            }
            let _ = child.wait();
        }

        // Teardown in steps, counting this process's threads after each (zero cost when off).
        let settle = |ms| {
            std::thread::sleep(Duration::from_millis(ms));
            thread_count()
        };
        drop(sender);
        drop(runtime);
        let after_ndi = settle(500);
        source.unsubscribe(&outlet);
        drop(source);
        drop(fixture);
        let after_source = settle(500);
        let linger: u64 = arg(args, "--linger")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let threads_after = format!(
            r#"{{"ndiDestroyed":{after_ndi},"sourceStopped":{after_source},"after{linger}s":{}}}"#,
            settle(linger * 1000)
        );

        println!(
            r#"{{"page":"{page_kind}","size":"{w}x{h}","rate":"{}","format":"{format:?}","alpha":"{alpha:?}","mode":"{mode:?}","adapter":"{adapter:?}","gpu":"{gpu_name}","secs":{secs},"warmup":{warmup},"total":{total},
    "sendCallMs":{},"tickToSentMs":{},
    "cpuCores":{{"probe":{},"webview2":{}}},"privateMb":{{"start":{mem_start:.1},"end":{mem_end:.1},"webview2":{webview_mb:.1}}},
    "counters":{{"ticks":{},"captured":{},"repeated":{},"late":{},"missed":{},"sent":{sent},"skippedNoReceiver":{skipped},"droppedBeforeSend":{dropped}}},"receivers":{receivers},
    "threads":{{"before":{threads_before},"during":{threads_during},"after":{threads_after}}},"cycles":{cycles_json},
    "received":{}}}"#,
            rate.label(),
            stats_json(&send),
            stats_json(&tick_to_sent),
            cores_json(&own),
            cores_json(&web),
            c1.ticks - c0.ticks,
            c1.captured - c0.captured,
            c1.repeated - c0.repeated,
            c1.late - c0.late,
            c1.missed - c0.missed,
            if received.trim().is_empty() {
                "null"
            } else {
                received.trim()
            },
        );
        Ok(())
    }

    // ---------------------------------------------------------------------------------------------
    // Receiver side (a second process, so its decode doesn't count against the probe's CPU)

    /// Y of pixel (x, y) in a UYVY/UYVA frame.
    fn y_at(d: &[u8], stride: usize, x: usize, y: usize) -> u8 {
        d[y * stride + (x / 2) * 4 + if x.is_multiple_of(2) { 1 } else { 3 }]
    }

    /// `--share-test`: can a capture share a WebView2 browser process with the app? In a test
    /// folder (never the running app's): a first page with the app's exact environment options
    /// stands in for the app; a capture with the same options joins it; one with its own
    /// environment for comparison; one with other arguments in the shared folder must fail.
    fn share_test() -> Result<(), String> {
        use kinetra_lib::frames::capture::{system_language, BROWSER_ARGS};
        const APP_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";
        let shared_dir = std::env::temp_dir().join("kinetra-share-test-app");
        let own_dir = std::env::temp_dir().join("kinetra-share-test-own");
        let cfg = |dir: &std::path::Path, args: &str, lang: Option<String>| SourceConfig {
            page: Page::Html(test_page(FrameRate::R50)),
            width: 1920,
            height: 1080,
            rate: FrameRate::R50,
            format: PixelFormat::Uyva,
            alpha: AlphaMode::Straight,
            background: [0, 0, 0],
            data_dir: dir.to_path_buf(),
            adapter: Adapter::Primary,
            follow_page: true,
            browser_args: Some(args.to_string()),
            language: lang,
        };
        let tree_mb = |pid: u32| -> (usize, f64) {
            let t = process_tree(pid);
            (t.len(), t.iter().map(|p| private_mb_of(*p)).sum())
        };
        let lang = system_language();
        let app = FrameSource::start(cfg(&shared_dir, APP_ARGS, Some(lang.clone())), None)
            .map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_secs(4));
        let (app_procs, app_mb) = tree_mb(app.browser_pid);
        let shared = FrameSource::start(cfg(&shared_dir, APP_ARGS, Some(lang.clone())), None)
            .map_err(|e| e.to_string())?;
        let c0 = shared.counters().captured;
        std::thread::sleep(Duration::from_secs(4));
        let shared_captured = shared.counters().captured - c0;
        let (both_procs, both_mb) = tree_mb(app.browser_pid);
        let mismatch =
            FrameSource::start(cfg(&shared_dir, BROWSER_ARGS, Some(lang.clone())), None).err();
        let own = FrameSource::start(cfg(&own_dir, BROWSER_ARGS, None), None)
            .map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_secs(4));
        let (own_procs, own_mb) = tree_mb(own.browser_pid);
        println!(
            r#"{{"language":"{lang}","appStandIn":{{"processes":{app_procs},"privateMb":{app_mb:.1}}},"sharedCapture":{{"sameBrowserProcess":{},"captured4s":{shared_captured},"addedProcesses":{},"addedMb":{:.1}}},"ownEnvironmentCapture":{{"processes":{own_procs},"privateMb":{own_mb:.1}}},"mismatchedOptionsInSharedFolder":"{}"}}"#,
            shared.browser_pid == app.browser_pid,
            both_procs as i64 - app_procs as i64,
            both_mb - app_mb,
            mismatch.map_or("started (not refused!)".to_string(), |e| e
                .to_string()
                .replace('"', "'")),
        );
        Ok(())
    }

    /// `--memory`: this process's private memory after each step of one 1080p50 UYVA output (the
    /// test page), with a receiver connected for the last steps.
    fn memory() -> Result<(), String> {
        let mut steps: Vec<(&str, f64)> = vec![("start", private_mb())];
        let rt = Runtime::acquire().map_err(|e| e.to_string())?;
        steps.push(("NDI runtime loaded and initialized", private_mb()));
        let source = FrameSource::start(
            SourceConfig {
                page: Page::Html(test_page(FrameRate::R50)),
                width: 1920,
                height: 1080,
                rate: FrameRate::R50,
                format: PixelFormat::Uyva,
                alpha: AlphaMode::Straight,
                background: [0, 0, 0],
                data_dir: std::env::temp_dir().join("kinetra-ndi-probe-webview"),
                adapter: Adapter::Primary,
                follow_page: true,
                browser_args: None,
                language: None,
            },
            None,
        )
        .map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_secs(3));
        steps.push((
            "frame source running (device, WebView2 loader, capture; nothing converted)",
            private_mb(),
        ));
        let outlet = source.subscribe();
        let name = format!("Kinetra memory probe {}", std::process::id());
        let sender = NdiSender::start(
            rt.clone(),
            &name,
            None,
            outlet.clone(),
            SendMode::Async,
            None,
        )
        .map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_secs(3));
        steps.push((
            "NDI sender created, no receiver (nothing encoded)",
            private_mb(),
        ));
        let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .args([
                "--receive",
                &name,
                "--no-checks",
                "--secs",
                "10",
                "--warmup",
                "1",
            ])
            .stdout(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_secs(8));
        steps.push((
            "sending to a receiver (conversion, read-back, frame pool, NDI encoder)",
            private_mb(),
        ));
        // A second output's source (no receiver): what each further output costs.
        let second = FrameSource::start(
            SourceConfig {
                page: Page::Html(test_page(FrameRate::R50)),
                width: 1920,
                height: 1080,
                rate: FrameRate::R50,
                format: PixelFormat::Uyva,
                alpha: AlphaMode::Straight,
                background: [0, 0, 0],
                data_dir: std::env::temp_dir().join("kinetra-ndi-probe-webview"),
                adapter: Adapter::Primary,
                follow_page: true,
                browser_args: None,
                language: None,
            },
            None,
        )
        .map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_secs(3));
        steps.push(("a second frame source (idle)", private_mb()));
        drop(second);
        let _ = child.wait();
        drop(sender);
        source.unsubscribe(&outlet);
        drop(source);
        drop(rt);
        std::thread::sleep(Duration::from_secs(2));
        steps.push(("everything stopped", private_mb()));
        let frame = PixelFormat::Uyva.frame_bytes(1920, 1080) as f64 / 1_048_576.0;
        let lines: Vec<String> = steps
            .iter()
            .map(|(s, mb)| format!(r#"{{"step":"{s}","privateMb":{mb:.1}}}"#))
            .collect();
        println!(
            r#"{{"steps":[{}],"oneUyvaFrameMb":{frame:.2},"stagingRingMb":{:.1},"framePoolMaxMb":{:.1}}}"#,
            lines.join(","),
            frame * 3.0,
            frame * 6.0,
        );
        Ok(())
    }

    /// `--host`: the app's own NDI host (`ndi::host::NdiOutputs`, what a reconcile drives) with
    /// the fixture server's renderer, into a receiver process: plans apply, the receiver count and
    /// tally reach `live()` through the change callback, and turning it off stops everything.
    fn host(args: &[String]) -> Result<(), String> {
        use kinetra_lib::core::outputs::{NdiHost, NdiPlan};
        use kinetra_lib::ndi::host::NdiOutputs;
        use std::collections::HashMap;
        use std::sync::atomic::AtomicU64;
        use std::sync::Arc;
        let secs: u64 = arg(args, "--secs")
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let threads_before = thread_count();
        let mut fx = Fixture::start()?;
        let url = fx.scene()?.replace("?audio=0", "?audio=0&cap=1");
        let changes = Arc::new(AtomicU64::new(0));
        let c = changes.clone();
        // A failed capture page asks for a reconcile: here a flag the loop below acts on (the app
        // reconciles on the blocking pool; never on the capture's own thread).
        let restart_asked = Arc::new(AtomicU64::new(0));
        let r = restart_asked.clone();
        let host = NdiOutputs::new(
            std::env::temp_dir().join("kinetra-ndi-probe-host"),
            // --share: run the capture in an "app" environment (a test folder with the app's options).
            args.iter()
                .any(|a| a == "--share")
                .then(|| kinetra_lib::ndi::host::AppEnvironment {
                    data_dir: std::env::temp_dir().join("kinetra-share-test-app"),
                    browser_args: kinetra_lib::ndi::host::APP_BROWSER_ARGS.to_string(),
                    language: kinetra_lib::frames::capture::system_language(),
                }),
            Arc::new(move || {
                c.fetch_add(1, Ordering::Relaxed);
            }),
            Arc::new(move || {
                r.fetch_add(1, Ordering::Relaxed);
            }),
        );
        let crash = args.iter().any(|a| a == "--crash");
        let name = format!("Kinetra host probe {}", std::process::id());
        let plan = NdiPlan {
            id: "probe".into(),
            name: name.clone(),
            url,
            width: 1920,
            height: 1080,
            rate: FrameRate::R50,
            format: PixelFormat::Uyva,
            alpha: AlphaMode::Straight,
            background: [0, 0, 0],
            groups: None,
        };
        let failures = host.apply(std::slice::from_ref(&plan));
        let before_receiver = host.live();
        let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .args([
                "--receive",
                &name,
                "--no-checks",
                "--secs",
                &secs.to_string(),
                "--warmup",
                "2",
            ])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        // The most receivers `live()` reported while the receiver ran (sampled every 200 ms).
        let out = child.stdout.take();
        let reader = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(out) = out {
                BufReader::new(out).read_line(&mut s).ok();
            }
            s
        });
        let mut with_receiver = None;
        let started = Instant::now();
        let (mut killed, mut restarts) = (0usize, 0u64);
        let mut restart_failures = HashMap::new();
        while !reader.is_finished() {
            if let Some(l) = host.live().get("probe").copied() {
                if with_receiver
                    .is_none_or(|w: kinetra_lib::core::model::NdiLive| l.receivers > w.receivers)
                {
                    with_receiver = Some(l);
                }
            }
            // --crash: 5 s in, kill the capture page's WebView2 child processes (renderer, GPU…).
            if crash && killed == 0 && started.elapsed() > Duration::from_secs(5) {
                killed = kill_webview_children();
            }
            if restart_asked.swap(0, Ordering::Relaxed) > 0 {
                restarts += 1;
                restart_failures = host.apply(std::slice::from_ref(&plan));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let received = reader.join().unwrap_or_default();
        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(1500));
        let after_receiver = host.live();
        let unchanged = host.apply(std::slice::from_ref(&plan));
        let off = host.apply(&[]);
        drop(fx);
        std::thread::sleep(Duration::from_millis(500));
        println!(
            r#"{{"failures":{:?},"unchangedApplyFailures":{:?},"offFailures":{:?},"live":{{"beforeReceiver":"{:?}","withReceiver":"{:?}","afterReceiver":"{:?}","afterOff":"{:?}"}},"changeCallbacks":{},"crash":{{"killed":{killed},"restarts":{restarts},"failures":{:?}}},"threads":{{"before":{threads_before},"after":{}}},"received":{}}}"#,
            failures,
            unchanged,
            off,
            before_receiver.get("probe"),
            with_receiver,
            after_receiver.get("probe"),
            host.live().get("probe"),
            changes.load(Ordering::Relaxed),
            restart_failures,
            thread_count(),
            if received.trim().is_empty() {
                "null"
            } else {
                received.trim()
            },
        );
        Ok(())
    }

    fn receive(name: &str, args: &[String]) -> Result<(), String> {
        let secs: u64 = arg(args, "--secs")
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);
        let warmup: u64 = arg(args, "--warmup")
            .and_then(|s| s.parse().ok())
            .unwrap_or(5);
        let yuv = arg(args, "--format").as_deref() != Some("bgra");
        let png = arg(args, "--png");
        let test_page = args.iter().any(|a| a == "--test-page");
        let rt = Runtime::acquire().map_err(|e| e.to_string())?;
        let recv = Receiver::connect(rt.clone(), name, true, Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        let started = Instant::now();
        let measure_from = started + Duration::from_secs(warmup);
        let end = measure_from + Duration::from_secs(secs);
        let (mut frames, mut repeats, mut skips) = (0u64, 0u64, 0u64);
        let mut last_code: Option<u32> = None;
        // |content-time step − frame period| per received frame, µs (ms resolution).
        let mut jitter: Vec<u32> = Vec::new();
        // The longest wait between two frames (a crashed and restarted page shows here).
        let (mut last_at, mut max_gap): (Option<Instant>, Duration) = (None, Duration::ZERO);
        let mut latencies: Vec<u32> = Vec::new();
        let mut checks = String::from("null");
        let mut perf0 = None;
        let mut fourcc = 0u32;
        while Instant::now() < end {
            let Some(v) = recv.video(100).map_err(|e| e.to_string())? else {
                continue;
            };
            if Instant::now() < measure_from {
                continue;
            }
            if perf0.is_none() {
                perf0 = Some(recv.performance());
            }
            frames += 1;
            if let Some(t) = last_at {
                max_gap = max_gap.max(Instant::now().duration_since(t));
            }
            last_at = Some(Instant::now());
            let f = v.frame;
            fourcc = f.fourcc;
            let d = v.data();
            let stride = f.line_stride_in_bytes as usize;
            let (w, h) = (f.xres as usize, f.yres as usize);
            // NDI's timestamp: 100 ns UTC when the sender submitted the frame.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as i64 / 100)
                .unwrap_or(0);
            if f.timestamp > 0 && f.timestamp != i64::MAX {
                let lat = (now - f.timestamp) / 10;
                if (0..10_000_000).contains(&lat) {
                    latencies.push(lat as u32);
                }
            }
            if yuv && test_page && w >= 700 && h >= 400 {
                let code = (0..24).fold(0u32, |acc, i| {
                    acc | (u32::from(y_at(d, stride, 16 + i * 16 + 8, 48) > 128) << i)
                });
                if let Some(prev) = last_code {
                    // The page time this frame shows minus the previous one's (ms).
                    let step = f64::from(code.wrapping_sub(prev) & 0xff_ffff);
                    let period =
                        1000.0 * f64::from(f.frame_rate_d) / f64::from(f.frame_rate_n.max(1));
                    if step == 0.0 {
                        repeats += 1;
                    } else if step >= 1.5 * period {
                        skips += 1;
                    }
                    jitter.push(((step - period).abs() * 1000.0) as u32);
                }
                last_code = Some(code);
                if checks == "null" {
                    let has_alpha = f.fourcc == kinetra_lib::ndi::ffi::FOURCC_UYVA;
                    let a = |x: usize, y: usize| {
                        if has_alpha {
                            i32::from(d[stride * h + y * w + x])
                        } else {
                            -1
                        }
                    };
                    let mut edge = 0;
                    if has_alpha {
                        // The outer 4 px only (a full-frame loop stalls a debug build).
                        for y in (0..4).chain(h - 4..h) {
                            for x in 0..w {
                                edge = edge.max(a(x, y));
                            }
                        }
                        for y in 4..h - 4 {
                            for x in (0..4).chain(w - 4..w) {
                                edge = edge.max(a(x, y));
                            }
                        }
                    }
                    checks = format!(
                        r#"{{"clearAlpha":{},"greenAlpha":{},"greenY":{},"halfRedAlpha":{},"halfRedY":{},"edgeMaxAlpha":{edge}}}"#,
                        a(5, 500),
                        a(200, 200),
                        y_at(d, stride, 200, 200),
                        a(500, 200),
                        y_at(d, stride, 500, 200),
                    );
                    if let Some(p) = &png {
                        write_png(std::path::Path::new(p), d, stride, w, h, has_alpha);
                    }
                }
            }
        }
        let (t1, d1) = recv.performance();
        let (t0, d0) = perf0.unwrap_or((0, 0));
        latencies.sort_unstable();
        jitter.sort_unstable();
        println!(
            r#"{{"source":"{}","fourcc":"{}","frames":{frames},"ndiFrames":{},"ndiDropped":{},"contentRepeats":{repeats},"contentSkips":{skips},"contentJitterMs":{},"maxGapMs":{},"latencyMs":{},"checks":{checks}}}"#,
            recv.source,
            String::from_utf8_lossy(&fourcc.to_le_bytes()),
            t1 - t0,
            d1 - d0,
            stats_json(&jitter),
            max_gap.as_millis(),
            stats_json(&latencies),
        );
        Ok(())
    }

    /// A UYVY/UYVA frame as a straight-alpha RGBA PNG (stored deflate blocks; no crate).
    fn write_png(path: &std::path::Path, d: &[u8], stride: usize, w: usize, h: usize, alpha: bool) {
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let q = y * stride + (x / 2) * 4;
                let u = (f32::from(d[q]) - 128.0) * 255.0 / 224.0;
                let v = (f32::from(d[q + 2]) - 128.0) * 255.0 / 224.0;
                let yy = (f32::from(d[q + 1 + 2 * (x % 2)]) - 16.0) * 255.0 / 219.0;
                let r = yy + 1.5748 * v;
                let g = yy - 0.1873 * u - 0.4681 * v;
                let b = yy + 1.8556 * u;
                let o = (y * w + x) * 4;
                rgba[o] = r.round().clamp(0.0, 255.0) as u8;
                rgba[o + 1] = g.round().clamp(0.0, 255.0) as u8;
                rgba[o + 2] = b.round().clamp(0.0, 255.0) as u8;
                rgba[o + 3] = if alpha {
                    d[stride * h + y * w + x]
                } else {
                    255
                };
            }
        }
        let mut raw = Vec::with_capacity(w * h * 4 + h);
        for y in 0..h {
            raw.push(0);
            raw.extend_from_slice(&rgba[y * w * 4..(y + 1) * w * 4]);
        }
        let mut z = vec![0x78, 0x01];
        let blocks = raw.chunks(65535).count();
        for (i, chunk) in raw.chunks(65535).enumerate() {
            z.push(u8::from(i + 1 == blocks));
            let len = chunk.len() as u16;
            z.extend_from_slice(&len.to_le_bytes());
            z.extend_from_slice(&(!len).to_le_bytes());
            z.extend_from_slice(chunk);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &raw {
            a = (a + u32::from(x)) % 65521;
            b = (b + a) % 65521;
        }
        z.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let crc = |data: &[u8]| {
            let mut c = 0xffff_ffffu32;
            for &x in data {
                c ^= u32::from(x);
                for _ in 0..8 {
                    c = if c & 1 != 0 {
                        0xedb8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
            }
            !c
        };
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |kind: &[u8], data: &[u8]| {
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut c = kind.to_vec();
            c.extend_from_slice(data);
            png.extend_from_slice(&c);
            png.extend_from_slice(&crc(&c).to_be_bytes());
        };
        let mut ihdr = (w as u32).to_be_bytes().to_vec();
        ihdr.extend_from_slice(&(h as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(b"IHDR", &ihdr);
        chunk(b"IDAT", &z);
        chunk(b"IEND", &[]);
        std::fs::write(path, png).ok();
    }
}
