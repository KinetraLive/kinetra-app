//! Web images: http(s) image links in field values and data sources, shown on air from a copy in
//! the project (owner's decision 2026-10-31; it replaces B8, "web images don't show on air").
//!
//! - The renderer never loads them from the internet (its CSP allows only this server): it asks
//!   for `/web-image?u=<link>` instead (`renderer/src/layer.ts`). The server answers from
//!   `<project>/.web-images/<key>.<ext>` ([`key`]: the first 32 hex digits of the link's
//!   SHA-256), downloading the link first when there is no copy yet. The copy is used from then
//!   on, offline and after a restart.
//! - **Only links in use are downloaded:** the link must be in a live layer's values (Program,
//!   staged Preview content, the editors' previews; list values too: `LiveHub::mentions`), so
//!   neither a page nor another computer on the network can make Kinetra fetch an address of its
//!   choosing. A copy that is already there is served without that check (a local file).
//! - **Safe:** http(s) links without credentials only ([`is_web_url`]); at most 5 redirects; at
//!   most `ASSET_IMAGE_MAX_BYTES`, counted while reading; 15 s; the bytes must be a PNG, JPEG,
//!   GIF, WebP or SVG (sniffed like imports; the server's content type is ignored). SVG is served
//!   with scripts blocked, like assets. Logs name the host only (links may carry tokens).
//! - **Public addresses only** (owner's decision 2026-10-31: a collaborator or a data source must
//!   not make Kinetra probe the venue's network): every address Kinetra connects to must be
//!   public ([`is_public`]), whoever wrote the link. Host names go through [`PublicOnly`], the
//!   client's DNS resolver, which drops every non-public address it resolves (so a name that
//!   later resolves elsewhere still can't reach one); IP literals are checked before the request
//!   and on every redirect; no proxy (it would resolve for us). Tests and the e2e fixture server
//!   allow loopback ([`WebImages::with_address_check`]).
//! - One download per link at a time (later requests wait for it); a failed link is tried again
//!   after 30 s at the earliest.
//! - Copies are written atomically; the first one adds `.web-images/` to the project's
//!   `.gitignore`. Cloud backup includes them (owner's decision). [`sweep`] (on project open)
//!   removes the copies that no layer, template default or source uses any more.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;

use crate::core::assets::sniff;
use crate::core::fsx;
use crate::core::model::ASSET_IMAGE_MAX_BYTES;
use crate::core::project;

/// The folder of the copies inside a project.
pub const DIR: &str = ".web-images";
/// Longest link, in bytes.
pub const URL_MAX: usize = 2048;
/// Image types a copy may be (sniffed), as file extensions.
const EXTS: [&str; 5] = ["png", "jpg", "gif", "webp", "svg"];
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);
/// A failed link isn't downloaded again sooner than this.
const RETRY_AFTER: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 5;

/// Whether an address is on the public internet: not loopback, private (10/8, 172.16/12,
/// 192.168/16, fc00::/7), shared (100.64/10: carrier NAT, Tailscale), link-local, unspecified,
/// broadcast, multicast, documentation, benchmarking or reserved. IPv4-mapped and NAT64
/// (64:ff9b::/96) IPv6 addresses are judged by their IPv4 address.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (18..=19).contains(&b))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [a, b] = s[6].to_be_bytes();
                let [c, d] = s[7].to_be_bytes();
                return is_public(IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d)));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8))
        }
    }
}

/// The refusal of a non-public address (found in a request error's sources).
#[derive(Debug)]
struct NotPublic;

impl std::fmt::Display for NotPublic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not a public address")
    }
}

impl std::error::Error for NotPublic {}

/// The addresses of a resolved host that pass `allowed`; an error when none does.
fn keep_allowed(
    addrs: impl Iterator<Item = SocketAddr>,
    allowed: fn(IpAddr) -> bool,
) -> Result<Vec<SocketAddr>, NotPublic> {
    let kept: Vec<SocketAddr> = addrs.filter(|a| allowed(a.ip())).collect();
    if kept.is_empty() {
        Err(NotPublic)
    } else {
        Ok(kept)
    }
}

/// The client's DNS resolver: the system's answers, without the addresses `allowed` refuses.
pub struct PublicOnly {
    allowed: fn(IpAddr) -> bool,
}

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let allowed = self.allowed;
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let kept = keep_allowed(addrs, allowed)?;
            Ok(Box::new(kept.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Whether a link's host is an IP literal that `allowed` refuses (names are the resolver's job).
fn literal_refused(url: &reqwest::Url, allowed: fn(IpAddr) -> bool) -> bool {
    url.host_str()
        .map(|h| h.trim_start_matches('[').trim_end_matches(']'))
        .and_then(|h| h.parse::<IpAddr>().ok())
        .is_some_and(|ip| !allowed(ip))
}

/// An http(s) link with a host and no user name or password, at most [`URL_MAX`] bytes, without
/// spaces or control characters.
pub fn is_web_url(s: &str) -> bool {
    if s.len() > URL_MAX || s.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return false;
    }
    let Ok(u) = reqwest::Url::parse(s) else {
        return false;
    };
    matches!(u.scheme(), "http" | "https")
        && u.host_str().is_some_and(|h| !h.is_empty())
        && u.username().is_empty()
        && u.password().is_none()
}

/// The copy's file stem for a link.
pub fn key(url: &str) -> String {
    Sha256::digest(url.as_bytes())[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether a field value holds `url`: the value itself, or a cell of a list value (JSON rows).
pub fn mentions(value: &str, url: &str) -> bool {
    value == url
        || (value.starts_with('[')
            && serde_json::to_string(url).is_ok_and(|quoted| value.contains(&quoted)))
}

/// Adds the web links a value holds (itself, or the cells of a list value) to `out`.
pub fn urls_in(value: &str, out: &mut HashSet<String>) {
    if is_web_url(value) {
        out.insert(value.to_string());
    } else if value.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<Value>(value) {
            urls_in_json(&v, out);
        }
    }
}

/// Adds every web link among the strings of a JSON document (list values inside it too).
pub fn urls_in_json(v: &Value, out: &mut HashSet<String>) {
    match v {
        Value::String(s) => urls_in(s, out),
        Value::Array(a) => a.iter().for_each(|x| urls_in_json(x, out)),
        Value::Object(o) => o.values().for_each(|x| urls_in_json(x, out)),
        _ => {}
    }
}

/// The image type of downloaded bytes, as an extension.
fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    let head = &bytes[..bytes.len().min(512)];
    EXTS.into_iter().find(|e| sniff(e, head))
}

/// The copy of `key` in `dir`, if there is one.
pub fn copy_name(dir: &Path, key: &str) -> Option<String> {
    EXTS.iter()
        .map(|e| format!("{key}.{e}"))
        .find(|n| dir.join(n).is_file())
}

/// Removes every file in `<project>/.web-images/` that isn't the copy of a link in `used`.
/// Returns how many it removed.
pub fn sweep(project: &Path, used: &HashSet<String>) -> usize {
    let dir = project.join(DIR);
    let keep: HashSet<String> = used.iter().map(|u| key(u)).collect();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut removed = 0;
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let stem = name.split('.').next().unwrap_or_default();
        if keep.contains(stem) {
            continue;
        }
        match fsx::remove_file_own(&path) {
            Ok(()) => removed += 1,
            Err(e) => tracing::warn!("web images: couldn't remove {name}: {e}"),
        }
    }
    removed
}

/// The links a project uses on disk: every web link in `screens/*.json` and in
/// `templates/*/template.json` (field defaults).
pub fn urls_on_disk(project: &Path, out: &mut HashSet<String>) {
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(project.join("screens")) {
        files.extend(rd.flatten().map(|e| e.path()));
    }
    if let Ok(rd) = std::fs::read_dir(project.join("templates")) {
        files.extend(rd.flatten().map(|e| e.path().join("template.json")));
    }
    for f in files
        .iter()
        .filter(|f| f.extension().is_some_and(|x| x == "json"))
    {
        if let Some(v) = std::fs::read(f)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            urls_in_json(&v, out);
        }
    }
}

/// The host of a link, for messages and logs (never the whole link).
pub fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

type Running = Arc<OnceCell<Result<String, String>>>;

/// The downloader (one per output server; see the module docs).
pub struct WebImages {
    client: OnceLock<Result<reqwest::Client, String>>,
    running: Mutex<HashMap<String, Running>>,
    failed: Mutex<HashMap<String, Instant>>,
    /// Which addresses may be connected to: [`is_public`] outside tests.
    allowed: fn(IpAddr) -> bool,
}

impl Default for WebImages {
    fn default() -> Self {
        Self::with_address_check(is_public)
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl WebImages {
    /// A downloader that connects only to addresses `allowed` accepts. Tests and the e2e fixture
    /// server allow loopback; everything else uses [`Default`] ([`is_public`]).
    #[doc(hidden)]
    pub fn with_address_check(allowed: fn(IpAddr) -> bool) -> Self {
        Self {
            client: OnceLock::new(),
            running: Mutex::default(),
            failed: Mutex::default(),
            allowed,
        }
    }

    fn client(&self) -> Result<reqwest::Client, String> {
        let allowed = self.allowed;
        self.client
            .get_or_init(|| {
                let redirects = reqwest::redirect::Policy::custom(move |a| {
                    if a.previous().len() >= MAX_REDIRECTS {
                        a.error("too many redirects")
                    } else if literal_refused(a.url(), allowed) {
                        a.error(NotPublic)
                    } else {
                        a.follow()
                    }
                });
                reqwest::Client::builder()
                    .user_agent(concat!("Kinetra/", env!("CARGO_PKG_VERSION")))
                    .connect_timeout(CONNECT_TIMEOUT)
                    .timeout(TOTAL_TIMEOUT)
                    .redirect(redirects)
                    .dns_resolver(Arc::new(PublicOnly { allowed }))
                    .no_proxy()
                    .build()
                    .map_err(|_| "The HTTP client could not start".to_string())
            })
            .clone()
    }

    /// The file name of `url`'s copy in `<project>/.web-images/`, downloading it first when there
    /// is none. The caller has checked that the link is in use.
    pub async fn fetch(&self, project: &Path, url: &str) -> Result<String, String> {
        let key = key(url);
        if lock(&self.failed)
            .get(&key)
            .is_some_and(|t| t.elapsed() < RETRY_AFTER)
        {
            return Err("The image couldn't be downloaded a moment ago".into());
        }
        let cell = lock(&self.running).entry(key.clone()).or_default().clone();
        let result = cell
            .get_or_init(|| self.download(project, url, &key))
            .await
            .clone();
        {
            let mut running = lock(&self.running);
            if running.get(&key).is_some_and(|c| Arc::ptr_eq(c, &cell)) {
                running.remove(&key);
            }
        }
        match &result {
            Ok(_) => {
                lock(&self.failed).remove(&key);
            }
            Err(e) => {
                tracing::warn!("web images: {} failed: {e}", host_of(url));
                lock(&self.failed).insert(key, Instant::now());
            }
        }
        result
    }

    async fn download(&self, project: &Path, url: &str, key: &str) -> Result<String, String> {
        let dir = project.join(DIR);
        // Another request may have finished the copy meanwhile.
        let (d, k) = (dir.clone(), key.to_string());
        if let Ok(Some(name)) = tokio::task::spawn_blocking(move || copy_name(&d, &k)).await {
            return Ok(name);
        }
        let parsed = reqwest::Url::parse(url).map_err(|_| "The link isn't valid".to_string())?;
        if literal_refused(&parsed, self.allowed) {
            return Err(NOT_PUBLIC.into());
        }
        let mut resp = self
            .client()?
            .get(parsed)
            .send()
            .await
            .map_err(|e| request_error(&e))?;
        if !resp.status().is_success() {
            return Err(format!(
                "The server answered HTTP {}",
                resp.status().as_u16()
            ));
        }
        let too_big = || {
            format!(
                "The image is larger than {} MB",
                ASSET_IMAGE_MAX_BYTES >> 20
            )
        };
        if resp
            .content_length()
            .is_some_and(|n| n > ASSET_IMAGE_MAX_BYTES)
        {
            return Err(too_big());
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| request_error(&e))? {
            if (body.len() + chunk.len()) as u64 > ASSET_IMAGE_MAX_BYTES {
                return Err(too_big());
            }
            body.extend_from_slice(&chunk);
        }
        let ext = image_ext(&body)
            .ok_or_else(|| "The link isn't a PNG, JPEG, GIF, WebP or SVG image".to_string())?;
        let name = format!("{key}.{ext}");
        let (project, file) = (project.to_path_buf(), dir.join(&name));
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't save the copy: {e}"))?;
            if let Err(e) = project::ensure_ignored(&project, &format!("{DIR}/")) {
                tracing::warn!("web images: couldn't update .gitignore: {e}");
            }
            fsx::write_bytes_atomic(&file, &body)
                .map_err(|e| format!("Couldn't save the copy: {e}"))
        })
        .await
        .map_err(|_| "Couldn't save the copy".to_string())??;
        Ok(name)
    }
}

const NOT_PUBLIC: &str = "The link points to a private or local network address";

fn request_error(e: &reqwest::Error) -> String {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(s) = source {
        if s.is::<NotPublic>() {
            return NOT_PUBLIC.into();
        }
        source = s.source();
    }
    if e.is_timeout() {
        "The download timed out".into()
    } else if e.is_connect() {
        "Couldn't connect to the image's server".into()
    } else if e.is_redirect() {
        "The link redirects too many times".into()
    } else {
        "The download failed".into()
    }
}

#[cfg(test)]
mod tests;
