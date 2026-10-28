//! Records Rocket League's Stats API (plan: kinetra-private docs/ROCKET_LEAGUE_PLAN.md, slice 0).
//! Read-only: connects as a client, writes nothing to the game.
//!
//! ```text
//! cargo run --example rl_capture -- [ws://localhost:49123] [seconds]
//! ```
//! Waits up to 2 minutes for the game, then records for `seconds` (default 300) into
//! `review/rl-capture/<time>.jsonl` (gitignored): one line per message, `{t, frame, form, msg}`
//! (`t` ms since the first message, `form` = whether `Data` came as an object or a JSON string).
//! **Player names and ids are replaced** before anything is written or printed: every object with a
//! `Shortcut` or `PrimaryId` (players, scorer, target, …) gets "Player N" / `Anon|N|0`, the same N
//! for the same name all through the file. Team names, arena and numbers are kept.
//! At the end it prints what slice 0 must settle: the transport, frames and messages, events and
//! their counts, the `Data` form, the keys of each event, statfeed types, the highest rate.
//! If the WebSocket handshake fails on an open port, it reports whether the port speaks raw JSON.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::{json, Map, Value};
use tokio_tungstenite::tungstenite::Message;

const WAIT_FOR_GAME: Duration = Duration::from_secs(120);

/// Replaces player names and ids, consistently (see the module docs).
#[derive(Default)]
struct Anon {
    names: HashMap<String, usize>,
}

impl Anon {
    fn n(&mut self, name: &str) -> usize {
        let next = self.names.len() + 1;
        *self.names.entry(name.to_string()).or_insert(next)
    }

    fn walk(&mut self, v: &mut Value) {
        match v {
            Value::Object(o) => {
                if o.contains_key("Shortcut") || o.contains_key("PrimaryId") {
                    let name = o
                        .get("Name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !name.is_empty() {
                        let n = self.n(&name);
                        o.insert("Name".into(), json!(format!("Player {n}")));
                        if o.contains_key("PrimaryId") {
                            o.insert("PrimaryId".into(), json!(format!("Anon|{n}|0")));
                        }
                    } else if o.contains_key("PrimaryId") {
                        o.insert("PrimaryId".into(), json!("Anon|0|0"));
                    }
                }
                for x in o.values_mut() {
                    self.walk(x);
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| self.walk(x)),
            _ => {}
        }
    }
}

#[derive(Default)]
struct Summary {
    frames: usize,
    multi_object_frames: usize,
    binary_frames: usize,
    bad_json: usize,
    events: BTreeMap<String, usize>,
    forms: BTreeMap<&'static str, usize>,
    keys: BTreeMap<String, BTreeSet<String>>,
    statfeed: BTreeSet<String>,
    per_second: BTreeMap<u64, usize>,
}

impl Summary {
    fn keys_of(&mut self, at: &str, v: &Value) {
        if let Some(o) = v.as_object() {
            self.keys
                .entry(at.to_string())
                .or_default()
                .extend(o.keys().cloned());
        }
    }

    fn note(&mut self, event: &str, data: &Value, t: u64) {
        *self.events.entry(event.to_string()).or_default() += 1;
        *self.per_second.entry(t / 1000).or_default() += 1;
        self.keys_of(event, data);
        if event == "UpdateState" {
            for p in data["Players"].as_array().into_iter().flatten() {
                self.keys_of("UpdateState.Players[]", p);
            }
            self.keys_of("UpdateState.Game", &data["Game"]);
            for t in data["Game"]["Teams"].as_array().into_iter().flatten() {
                self.keys_of("UpdateState.Game.Teams[]", t);
            }
            self.keys_of("UpdateState.Game.Ball", &data["Game"]["Ball"]);
            self.keys_of("UpdateState.Game.Target", &data["Game"]["Target"]);
        }
        if event == "GoalScored" {
            self.keys_of("GoalScored.Scorer", &data["Scorer"]);
        }
        if event == "StatfeedEvent" {
            self.statfeed.insert(format!(
                "{} = \"{}\"",
                data["EventName"].as_str().unwrap_or("?"),
                data["Type"].as_str().unwrap_or("?")
            ));
        }
    }

    fn print(&self) {
        println!("\n== Summary ==");
        println!(
            "frames: {} (with more than one JSON object: {}; binary: {}; not JSON: {})",
            self.frames, self.multi_object_frames, self.binary_frames, self.bad_json
        );
        println!("Data form: {:?}", self.forms);
        println!(
            "highest rate: {} messages in one second",
            self.per_second.values().max().copied().unwrap_or(0)
        );
        println!("events:");
        for (e, n) in &self.events {
            println!("  {e}: {n}");
        }
        println!("keys:");
        for (at, k) in &self.keys {
            println!(
                "  {at}: {}",
                k.iter().cloned().collect::<Vec<_>>().join(", ")
            );
        }
        println!("statfeed types: {:?}", self.statfeed);
    }
}

/// Every JSON object in a frame (the game may put several in one, or none).
fn objects(text: &str) -> Vec<Result<Value, ()>> {
    let mut out = Vec::new();
    let mut it = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    loop {
        match it.next() {
            Some(Ok(v)) => out.push(Ok(v)),
            Some(Err(_)) => {
                out.push(Err(()));
                break;
            }
            None => break,
        }
    }
    out
}

/// A raw TCP look at the port when the WebSocket handshake failed: does it send JSON by itself?
async fn tcp_probe(url: &str) {
    let host = url
        .trim_start_matches("ws://")
        .trim_start_matches("wss://")
        .split('/')
        .next()
        .unwrap_or("localhost:49123")
        .to_string();
    println!("trying {host} as plain TCP for 5 s…");
    match tokio::net::TcpStream::connect(&host).await {
        Err(e) => println!("plain TCP: {e}"),
        Ok(mut s) => {
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; 64 * 1024];
            let mut total = 0usize;
            let mut first = None;
            let until = Instant::now() + Duration::from_secs(5);
            while Instant::now() < until {
                match tokio::time::timeout(Duration::from_millis(500), s.read(&mut buf)).await {
                    Ok(Ok(0)) => break,
                    Ok(Ok(n)) => {
                        first.get_or_insert(buf[0]);
                        total += n;
                    }
                    Ok(Err(e)) => {
                        println!("plain TCP read: {e}");
                        break;
                    }
                    Err(_) => {}
                }
            }
            println!(
                "plain TCP: {total} bytes in 5 s, first byte {:?} ({})",
                first.map(char::from),
                if first == Some(b'{') {
                    "raw JSON stream"
                } else {
                    "not raw JSON"
                }
            );
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .unwrap_or_else(|| "ws://localhost:49123".to_string());
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(300);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(run(&url, seconds));
}

async fn run(url: &str, seconds: u64) {
    println!("connecting to {url} (waiting up to 2 minutes for Rocket League)…");
    let started = Instant::now();
    let ws = loop {
        match tokio_tungstenite::connect_async(url).await {
            Ok((ws, resp)) => {
                println!("WebSocket connected (HTTP {})", resp.status());
                break ws;
            }
            Err(e) => {
                let refused = matches!(
                    &e,
                    tokio_tungstenite::tungstenite::Error::Io(io)
                        if io.kind() == std::io::ErrorKind::ConnectionRefused
                );
                if refused && started.elapsed() < WAIT_FOR_GAME {
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
                println!("WebSocket failed: {e}");
                if !refused {
                    tcp_probe(url).await;
                } else {
                    println!(
                        "nothing listens on that port: is Rocket League running with \
                         PacketSendRate > 0 in DefaultStatsAPI.ini (then restarted)?"
                    );
                }
                return;
            }
        }
    };

    let dir = std::path::Path::new("review").join("rl-capture");
    std::fs::create_dir_all(&dir).expect("review/rl-capture");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("{stamp}.jsonl"));
    let mut file = std::io::BufWriter::new(std::fs::File::create(&path).expect("capture file"));
    println!("recording {seconds} s into {}", path.display());

    let (_tx, mut rx) = ws.split();
    let mut anon = Anon::default();
    let mut sum = Summary::default();
    let mut first: Option<Instant> = None;
    let until = Instant::now() + Duration::from_secs(seconds);
    let mut last_report = Instant::now();
    while Instant::now() < until {
        let left = until.saturating_duration_since(Instant::now());
        let msg = match tokio::time::timeout(left, rx.next()).await {
            Err(_) => break,
            Ok(None) => {
                println!("the game closed the connection");
                break;
            }
            Ok(Some(Err(e))) => {
                println!("connection error: {e}");
                break;
            }
            Ok(Some(Ok(m))) => m,
        };
        let text = match msg {
            Message::Text(t) => t.as_str().to_string(),
            Message::Binary(_) => {
                sum.binary_frames += 1;
                continue;
            }
            Message::Close(_) => {
                println!("the game closed the connection");
                break;
            }
            _ => continue,
        };
        sum.frames += 1;
        let t0 = *first.get_or_insert_with(Instant::now);
        let t = t0.elapsed().as_millis() as u64;
        let parts = objects(&text);
        if parts.len() > 1 {
            sum.multi_object_frames += 1;
        }
        for part in parts {
            let Ok(mut v) = part else {
                sum.bad_json += 1;
                continue;
            };
            let event = v["Event"].as_str().unwrap_or("?").to_string();
            // Data as an object, or as a JSON string holding one.
            let (form, mut data) = match v.get("Data") {
                Some(Value::String(s)) => (
                    "string",
                    serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
                ),
                Some(d) => ("object", d.clone()),
                None => ("none", Value::Null),
            };
            *sum.forms.entry(form).or_default() += 1;
            anon.walk(&mut data);
            sum.note(&event, &data, t);
            if let Some(o) = v.as_object_mut() {
                let d = if form == "string" {
                    Value::String(data.to_string())
                } else {
                    data
                };
                o.insert("Data".into(), d);
            }
            let mut line = Map::new();
            line.insert("t".into(), json!(t));
            line.insert("frame".into(), json!(sum.frames));
            line.insert("form".into(), json!(form));
            line.insert("msg".into(), v);
            let _ = writeln!(file, "{}", Value::Object(line));
        }
        if last_report.elapsed() >= Duration::from_secs(10) {
            last_report = Instant::now();
            println!(
                "{} s: {} frames, events {:?}",
                t / 1000,
                sum.frames,
                sum.events.keys().collect::<Vec<_>>()
            );
        }
    }
    let _ = file.flush();
    sum.print();
    println!("\nsaved to {} (player names replaced)", path.display());
}
