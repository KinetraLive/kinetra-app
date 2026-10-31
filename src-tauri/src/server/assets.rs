//! `GET /assets/{name}`: the media library (images, video, audio) from the open project's
//! `assets/` folder.
//!
//! - Only path-safe names (`core::assets::is_asset_name`: no separators, no leading dot, a
//!   library extension), only from that folder; anything else is a 404.
//! - Content type by extension. SVG also gets `Content-Security-Policy: script-src 'none'` (an
//!   SVG opened directly can't run script); every response has `X-Content-Type-Options: nosniff`
//!   (router middleware).
//! - `ETag` (size + modification time) and `Last-Modified`; `If-None-Match` / `If-Modified-Since`
//!   answer `304`. `Cache-Control: no-cache`: renderers revalidate, so a replaced file shows up
//!   on the next load while unchanged ones cost a 304.
//! - **Range requests** (video seeking and preload): `Accept-Ranges: bytes`; a single range
//!   (`bytes=a-b`, `a-`, `-n`) answers `206` with `Content-Range`; an unsatisfiable one `416`
//!   (`Content-Range: bytes */<len>`); several ranges, or an `If-Range` that doesn't match the
//!   ETag / Last-Modified, get the whole file (`200`).
//! - Bodies are streamed in [`CHUNK`] pieces read on the blocking pool through one buffer per
//!   response (reused for every chunk); a file is never held in memory whole.
//!
//! The same responses serve `GET /web-image?u=<link>` (`super::routes`): copies of web images in
//! `<project>/.web-images/` (`core::web_images`).
//!
//! The folder comes from [`AssetDir`], which the server manager sets from the open project
//! (`ProjectObserver`), so `core` never sees axum types.

use std::fs::File;
use std::future::Future;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};
use std::task::{Context, Poll};
use std::time::UNIX_EPOCH;

use axum::body::{Body, Bytes, HttpBody};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use http_body::{Frame, SizeHint};
use tokio::task::JoinHandle;

use crate::core::assets::{content_type, is_asset_name, ASSETS_DIR};
use crate::core::web_images::WebImages;

/// Bytes read per streamed chunk.
pub const CHUNK: usize = 64 * 1024;

/// The open project's folder for `/assets/*` and `/web-image` (none: every file is a 404), and
/// the downloader of web images. Cheap to clone.
#[derive(Clone, Default)]
pub struct AssetDir {
    project: Arc<RwLock<Option<PathBuf>>>,
    pub(super) web: Arc<WebImages>,
}

impl AssetDir {
    /// Web images may also come from this computer (loopback): for tests and the e2e fixture
    /// server, whose "web" servers run on 127.0.0.1. The app uses [`Default`] (public only).
    #[doc(hidden)]
    pub fn allowing_loopback_web_images() -> Self {
        Self {
            project: Arc::default(),
            web: Arc::new(WebImages::with_address_check(|ip| {
                ip.is_loopback() || crate::core::web_images::is_public(ip)
            })),
        }
    }

    /// Serves `<project>/assets/` (or nothing).
    pub fn set_project(&self, project: Option<PathBuf>) {
        *self.project.write().unwrap_or_else(PoisonError::into_inner) = project;
    }

    pub(super) fn project(&self) -> Option<PathBuf> {
        self.project
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn assets(&self) -> Option<PathBuf> {
        self.project().map(|p| p.join(ASSETS_DIR))
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found").into_response()
}

/// `Sun, 04 Oct 2026 10:00:00 GMT`.
fn http_date(secs: i64) -> Option<String> {
    let t = time::OffsetDateTime::from_unix_timestamp(secs).ok()?;
    let local = crate::core::localtime::day_label(time::PrimitiveDateTime::new(t.date(), t.time()));
    // "Mon 5 Oct" → weekday and month names.
    let mut parts = local.split(' ');
    let (wd, _, month) = (parts.next()?, parts.next()?, parts.next()?);
    Some(format!(
        "{wd}, {:02} {month} {} {:02}:{:02}:{:02} GMT",
        t.day(),
        t.year(),
        t.hour(),
        t.minute(),
        t.second()
    ))
}

/// A single byte range of a `len`-byte file: `Some(Ok((start, end_inclusive)))`,
/// `Some(Err(()))` when unsatisfiable, `None` to ignore the header (not bytes, several ranges,
/// malformed).
pub fn parse_range(h: &str, len: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = h.trim().strip_prefix("bytes=")?.trim();
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        // Suffix: the last n bytes.
        let n: u64 = b.parse().ok()?;
        if n == 0 || len == 0 {
            return Some(Err(()));
        }
        return Some(Ok((len.saturating_sub(n), len - 1)));
    }
    let start: u64 = a.parse().ok()?;
    let end: u64 = if b.is_empty() {
        u64::MAX
    } else {
        b.parse().ok()?
    };
    if end < start {
        return None;
    }
    if start >= len {
        return Some(Err(()));
    }
    Some(Ok((start, end.min(len - 1))))
}

/// A file streamed from `start` for `remaining` bytes (see the module docs).
struct FileBody {
    remaining: u64,
    state: Stream,
}

enum Stream {
    Idle(Option<(File, Vec<u8>)>),
    Reading(JoinHandle<(File, Vec<u8>, io::Result<usize>)>),
    Done,
}

impl FileBody {
    fn new(file: File, remaining: u64) -> Self {
        let buf = vec![
            0u8;
            CHUNK
                .min(usize::try_from(remaining).unwrap_or(CHUNK))
                .max(1)
        ];
        Self {
            remaining,
            state: Stream::Idle(Some((file, buf))),
        }
    }
}

impl HttpBody for FileBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = &mut *self;
        loop {
            match &mut this.state {
                Stream::Done => return Poll::Ready(None),
                Stream::Idle(slot) => {
                    let Some((mut f, mut buf)) = slot.take().filter(|_| this.remaining > 0) else {
                        this.state = Stream::Done;
                        return Poll::Ready(None);
                    };
                    let want = buf
                        .len()
                        .min(usize::try_from(this.remaining).unwrap_or(usize::MAX));
                    this.state = Stream::Reading(tokio::task::spawn_blocking(move || {
                        let r = f.read(&mut buf[..want]);
                        (f, buf, r)
                    }));
                }
                Stream::Reading(h) => {
                    let (f, buf, r) = match Pin::new(h).poll(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Ok(v)) => v,
                        Poll::Ready(Err(e)) => {
                            this.state = Stream::Done;
                            return Poll::Ready(Some(Err(io::Error::other(e))));
                        }
                    };
                    match r {
                        Ok(0) => {
                            this.state = Stream::Done;
                            return Poll::Ready(Some(Err(io::ErrorKind::UnexpectedEof.into())));
                        }
                        Ok(n) => {
                            this.remaining -= n as u64;
                            let data = Bytes::copy_from_slice(&buf[..n]);
                            this.state = Stream::Idle(Some((f, buf)));
                            return Poll::Ready(Some(Ok(Frame::data(data))));
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                            this.state = Stream::Idle(Some((f, buf)));
                        }
                        Err(e) => {
                            this.state = Stream::Done;
                            return Poll::Ready(Some(Err(e)));
                        }
                    }
                }
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        matches!(self.state, Stream::Done) || self.remaining == 0
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.remaining)
    }
}

enum Answer {
    NotModified,
    Full(File),
    Partial(File, u64, u64),
    Unsatisfiable,
}

struct Opened {
    answer: Answer,
    len: u64,
    etag: String,
    modified: Option<String>,
}

pub(super) async fn asset_response(dir: &AssetDir, name: &str, req: &HeaderMap) -> Response {
    if !is_asset_name(name) {
        return not_found();
    }
    let Some(root) = dir.assets() else {
        return not_found();
    };
    file_response(root.join(name), name, req).await
}

/// A file served like an asset (see the module docs); `name` gives the content type. The caller
/// has checked the path.
pub(super) async fn file_response(path: PathBuf, name: &str, req: &HeaderMap) -> Response {
    let get = |h: header::HeaderName| req.get(h).and_then(|v| v.to_str().ok()).map(str::to_string);
    let (inm, ims, range, if_range) = (
        get(header::IF_NONE_MATCH),
        get(header::IF_MODIFIED_SINCE),
        get(header::RANGE),
        get(header::IF_RANGE),
    );
    let opened = tokio::task::spawn_blocking(move || -> Option<Opened> {
        let mut file = File::open(&path).ok()?;
        let meta = file.metadata().ok().filter(|m| m.is_file())?;
        let len = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok());
        let mtime_ms = mtime.map_or(0, |d| d.as_millis());
        let etag = format!("\"{:x}-{:x}\"", len, mtime_ms);
        let modified = mtime.and_then(|d| http_date(i64::try_from(d.as_secs()).ok()?));
        let fresh = match (&inm, &ims) {
            (Some(tags), _) => tags.split(',').any(|t| t.trim() == etag || t.trim() == "*"),
            (None, Some(since)) => modified.as_deref() == Some(since.trim()),
            (None, None) => false,
        };
        let range_applies = if_range.as_deref().is_none_or(|v| {
            let v = v.trim();
            v == etag || modified.as_deref() == Some(v)
        });
        let answer = if fresh {
            Answer::NotModified
        } else {
            match range
                .filter(|_| range_applies)
                .and_then(|r| parse_range(&r, len))
            {
                Some(Ok((start, end))) => {
                    file.seek(SeekFrom::Start(start)).ok()?;
                    Answer::Partial(file, start, end)
                }
                Some(Err(())) => Answer::Unsatisfiable,
                None => Answer::Full(file),
            }
        };
        Some(Opened {
            answer,
            len,
            etag,
            modified,
        })
    })
    .await
    .ok()
    .flatten();
    let Some(o) = opened else {
        return not_found();
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(v) = HeaderValue::from_str(&o.etag) {
        headers.insert(header::ETAG, v);
    }
    if let Some(v) = o.modified.and_then(|m| HeaderValue::from_str(&m).ok()) {
        headers.insert(header::LAST_MODIFIED, v);
    }
    let ty = content_type(name);
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(ty));
    if ty == "image/svg+xml" {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("script-src 'none'"),
        );
    }
    let len_header = |n: u64| HeaderValue::from(n);
    match o.answer {
        Answer::NotModified => (StatusCode::NOT_MODIFIED, headers).into_response(),
        Answer::Unsatisfiable => {
            if let Ok(v) = HeaderValue::from_str(&format!("bytes */{}", o.len)) {
                headers.insert(header::CONTENT_RANGE, v);
            }
            (StatusCode::RANGE_NOT_SATISFIABLE, headers).into_response()
        }
        Answer::Full(f) => {
            headers.insert(header::CONTENT_LENGTH, len_header(o.len));
            (StatusCode::OK, headers, Body::new(FileBody::new(f, o.len))).into_response()
        }
        Answer::Partial(f, start, end) => {
            let n = end - start + 1;
            headers.insert(header::CONTENT_LENGTH, len_header(n));
            if let Ok(v) = HeaderValue::from_str(&format!("bytes {start}-{end}/{}", o.len)) {
                headers.insert(header::CONTENT_RANGE, v);
            }
            (
                StatusCode::PARTIAL_CONTENT,
                headers,
                Body::new(FileBody::new(f, n)),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_dates() {
        assert_eq!(
            http_date(1_791_158_400).as_deref(),
            Some("Mon, 05 Oct 2026 00:00:00 GMT")
        );
    }

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some(Ok((0, 99))));
        assert_eq!(parse_range("bytes=900-", 1000), Some(Ok((900, 999))));
        assert_eq!(parse_range("bytes=900-5000", 1000), Some(Ok((900, 999))));
        assert_eq!(parse_range("bytes=-100", 1000), Some(Ok((900, 999))));
        assert_eq!(parse_range("bytes=-5000", 1000), Some(Ok((0, 999))));
        assert_eq!(parse_range("bytes=1000-", 1000), Some(Err(())));
        assert_eq!(parse_range("bytes=-0", 1000), Some(Err(())));
        assert_eq!(parse_range("bytes=0-1,5-9", 1000), None);
        assert_eq!(parse_range("bytes=9-1", 1000), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
        assert_eq!(parse_range("bytes=x-1", 1000), None);
    }
}
