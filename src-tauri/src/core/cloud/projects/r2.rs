//! Requests to storage (Cloudflare R2) with presigned URLs (CLOUD_PROJECTS.md §3.1, §3.4,
//! §3.5): to the URL exactly as given, with exactly the headers given, never an
//! `Authorization` header (a client of its own, without the session), no compression (the
//! bytes must arrive as stored). URLs are secrets until they expire, and their paths name
//! blobs by hash: logs get [`redact`] (the host only), never the URL.

use std::time::Duration;

use super::wire::Presigned;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// No total timeout (a 16 MiB part on a slow link takes minutes); a stalled transfer fails.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// What storage said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum R2Error {
    /// `403`: the URL expired or the signature doesn't match → ask the server for a new one.
    Expired,
    /// `400 BadDigest`: the bytes don't match their checksum → the file changed: re-hash it and
    /// start it again.
    BadDigest,
    /// No answer, a timeout or a 5xx: retry later.
    Offline,
    /// Anything else (the status).
    Status(u16),
}

/// The URL as it may appear in a log: scheme and host, nothing of the path or the query.
pub fn redact(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if scheme.is_empty() {
        format!("{host}/…")
    } else {
        format!("{scheme}://{host}/…")
    }
}

/// The storage client (one per app; cheap to clone).
#[derive(Clone)]
pub struct R2 {
    http: reqwest::Client,
}

impl R2 {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .no_gzip()
            .build()
            .map_err(|_| "The HTTP client could not start".to_string())?;
        Ok(Self { http })
    }

    fn request(&self, method: reqwest::Method, p: &Presigned) -> reqwest::RequestBuilder {
        let mut req = self.http.request(method, &p.url);
        for (k, v) in &p.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req
    }

    /// `PUT` with `body`; the `ETag` of the stored object (multipart parts need it).
    pub async fn put(&self, p: &Presigned, body: reqwest::Body) -> Result<Option<String>, R2Error> {
        let resp = self
            .request(reqwest::Method::PUT, p)
            .body(body)
            .send()
            .await
            .map_err(|e| {
                tracing::debug!("storage PUT {}: {}", redact(&p.url), kind(&e));
                R2Error::Offline
            })?;
        let status = resp.status().as_u16();
        tracing::debug!("storage PUT {} → {status}", redact(&p.url));
        if (200..300).contains(&status) {
            return Ok(resp
                .headers()
                .get(reqwest::header::ETAG)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string));
        }
        let body = resp.bytes().await.unwrap_or_default();
        Err(error(status, &body))
    }

    /// `GET`: the response to stream (status checked).
    pub async fn get(&self, p: &Presigned) -> Result<reqwest::Response, R2Error> {
        let resp = self
            .request(reqwest::Method::GET, p)
            .send()
            .await
            .map_err(|e| {
                tracing::debug!("storage GET {}: {}", redact(&p.url), kind(&e));
                R2Error::Offline
            })?;
        let status = resp.status().as_u16();
        tracing::debug!("storage GET {} → {status}", redact(&p.url));
        if (200..300).contains(&status) {
            return Ok(resp);
        }
        let body = resp.bytes().await.unwrap_or_default();
        Err(error(status, &body))
    }
}

fn kind(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "timeout"
    } else {
        "no connection"
    }
}

/// An error status (and S3's XML body) as an [`R2Error`].
fn error(status: u16, body: &[u8]) -> R2Error {
    match status {
        403 => R2Error::Expired,
        400 if xml_code(body).as_deref() == Some("BadDigest") => R2Error::BadDigest,
        s if s >= 500 => R2Error::Offline,
        s => R2Error::Status(s),
    }
}

/// `<Code>…</Code>` of an S3 error body.
fn xml_code(body: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(body).ok()?;
    let start = s.find("<Code>")? + "<Code>".len();
    let end = s[start..].find("</Code>")? + start;
    Some(s[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_keeps_only_the_host() {
        let url = "https://acc.eu.r2.cloudflarestorage.com/kinetra-projects/u/0123abcd?X-Amz-Signature=deadbeef&X-Amz-Credential=k";
        assert_eq!(redact(url), "https://acc.eu.r2.cloudflarestorage.com/…");
        assert_eq!(redact("http://127.0.0.1:9/b/c?x=1"), "http://127.0.0.1:9/…");
        assert_eq!(redact("host?x"), "host/…");
    }

    #[test]
    fn storage_errors() {
        assert_eq!(error(403, b""), R2Error::Expired);
        let bad =
            b"<?xml version=\"1.0\"?><Error><Code>BadDigest</Code><Message>x</Message></Error>";
        assert_eq!(error(400, bad), R2Error::BadDigest);
        assert_eq!(
            error(400, b"<Error><Code>InvalidArgument</Code></Error>"),
            R2Error::Status(400)
        );
        assert_eq!(error(503, b""), R2Error::Offline);
        assert_eq!(error(404, b""), R2Error::Status(404));
    }

    #[test]
    fn presigned_debug_hides_the_url() {
        let p = Presigned {
            url: "https://h.example/blob/abc?X-Amz-Signature=s3cr3t".into(),
            headers: Default::default(),
            expires_at: None,
        };
        let d = format!("{p:?}");
        assert!(!d.contains("s3cr3t") && !d.contains("abc"), "{d}");
    }
}
