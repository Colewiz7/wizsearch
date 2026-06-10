//! The wzstream:// protocol. ALL media playback goes through here; the frontend
//! never touches raw paths or remote URLs directly.
//!
//! wzstream://localhost/local?path=<abs path>          local collected files
//! wzstream://localhost/remote?src=<source>&url=<url>  remote previews (proxied)
//!
//! Local serving supports HTTP Range (webkit needs it for seeking and for audio
//! on Linux). Remote serving forwards Range to the origin and streams back.

pub mod ffmpeg;
pub mod server;

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use http::{header, Response, StatusCode};
use percent_encoding::percent_decode_str;

use crate::security;
use crate::sources::SourceDescriptor;

// serve local files in chunks; webkit re-requests with ranges
const LOCAL_CHUNK: u64 = 4 * 1024 * 1024;
// hard cap for proxied remote bodies (previews are small; this stops abuse)
const REMOTE_CAP: usize = 64 * 1024 * 1024;

pub struct StreamState {
    pub client: reqwest::Client,
    pub descriptors: Vec<&'static SourceDescriptor>,
    pub settings: std::sync::Arc<crate::settings::SettingsStore>,
    pub default_collection_dir: PathBuf,
}

impl StreamState {
    /// local serving is confined to the (user-configurable) collection dir
    pub fn collection_dir(&self) -> PathBuf {
        let configured = self.settings.string_or(
            "collection.dir",
            &self.default_collection_dir.to_string_lossy(),
        );
        PathBuf::from(configured)
    }
}

fn err(status: StatusCode, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(msg.as_bytes().to_vec())
        .unwrap_or_default()
}

fn query_param(query: &str, name: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        if it.next() == Some(name) {
            let raw = it.next().unwrap_or("");
            return percent_decode_str(raw)
                .decode_utf8()
                .ok()
                .map(|s| s.into_owned());
        }
    }
    None
}

/// parse "bytes=a-b" (either end optional)
fn parse_range(value: &str, len: u64) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (start_s, end_s) = spec.split_once('-')?;
    if start_s.is_empty() {
        // suffix range: last N bytes
        let n: u64 = end_s.parse().ok()?;
        if n == 0 || len == 0 {
            return None;
        }
        let start = len.saturating_sub(n);
        return Some((start, len - 1));
    }
    let start: u64 = start_s.parse().ok()?;
    if start >= len {
        return None;
    }
    let end = if end_s.is_empty() {
        (start + LOCAL_CHUNK - 1).min(len - 1)
    } else {
        end_s.parse::<u64>().ok()?.min(len - 1)
    };
    (start <= end).then_some((start, end))
}

pub async fn handle(state: &StreamState, request: http::Request<Vec<u8>>) -> Response<Vec<u8>> {
    let uri = request.uri().clone();
    let path = uri.path();
    let query = uri.query().unwrap_or("");
    let range = request
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    match path {
        "/local" => {
            let Some(p) = query_param(query, "path") else {
                return err(StatusCode::BAD_REQUEST, "missing path");
            };
            serve_local(state, PathBuf::from(p), range).await
        }
        "/remote" => {
            let (Some(src), Some(url)) = (query_param(query, "src"), query_param(query, "url"))
            else {
                return err(StatusCode::BAD_REQUEST, "missing src or url");
            };
            serve_remote(state, &src, &url, range).await
        }
        _ => err(StatusCode::NOT_FOUND, "unknown stream path"),
    }
}

async fn serve_local(
    state: &StreamState,
    path: PathBuf,
    range: Option<String>,
) -> Response<Vec<u8>> {
    let confined_root = state.collection_dir();
    let result = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let canon = path.canonicalize().map_err(|e| e.to_string())?;
        let root = confined_root.canonicalize().map_err(|e| e.to_string())?;
        if !canon.starts_with(&root) {
            return Err("outside collection dir".to_string());
        }
        let mut file = std::fs::File::open(&canon).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        let mime = mime_guess::from_path(&canon)
            .first_or_octet_stream()
            .to_string();

        let (start, end, partial) = match range.as_deref().and_then(|r| parse_range(r, len)) {
            Some((s, e)) => (s, e, true),
            None => (
                0,
                len.saturating_sub(1).min(LOCAL_CHUNK - 1),
                len > LOCAL_CHUNK,
            ),
        };
        let count = end - start + 1;
        file.seek(SeekFrom::Start(start))
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; count as usize];
        file.read_exact(&mut buf).map_err(|e| e.to_string())?;
        Ok((buf, start, end, len, mime, partial))
    })
    .await;

    match result {
        Ok(Ok((buf, start, end, len, mime, partial))) => {
            let mut builder = Response::builder()
                .status(if partial {
                    StatusCode::PARTIAL_CONTENT
                } else {
                    StatusCode::OK
                })
                .header(header::CONTENT_TYPE, mime)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CONTENT_LENGTH, buf.len());
            if partial {
                builder =
                    builder.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"));
            }
            builder.body(buf).unwrap_or_default()
        }
        Ok(Err(e)) => err(StatusCode::FORBIDDEN, &e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn serve_remote(
    state: &StreamState,
    source_id: &str,
    url: &str,
    range: Option<String>,
) -> Response<Vec<u8>> {
    let Some(desc) = state.descriptors.iter().find(|d| d.id == source_id) else {
        return err(StatusCode::FORBIDDEN, "unknown source");
    };
    // previews obey the same allowlist as fetch plans
    if let Err(e) = security::validate_source_url(url, desc) {
        return err(StatusCode::FORBIDDEN, &e.to_string());
    }

    let mut req = state.client.get(url).header(
        "User-Agent",
        concat!("wizsearch/", env!("CARGO_PKG_VERSION")),
    );
    if let Some(r) = &range {
        req = req.header(header::RANGE, r.clone());
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_GATEWAY, &e.to_string()),
    };

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for h in [
        header::CONTENT_TYPE,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::CONTENT_LENGTH,
    ] {
        if let Some(v) = resp.headers().get(&h) {
            builder = builder.header(h, v);
        }
    }
    let body = match resp.bytes().await {
        Ok(b) if b.len() <= REMOTE_CAP => b.to_vec(),
        Ok(_) => return err(StatusCode::PAYLOAD_TOO_LARGE, "preview too large"),
        Err(e) => return err(StatusCode::BAD_GATEWAY, &e.to_string()),
    };
    builder.body(body).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_parsing() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=999-", 1000), Some((999, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("nonsense", 1000), None);
    }

    #[test]
    fn query_params_decode() {
        assert_eq!(
            query_param("src=pexels&url=https%3A%2F%2Fa.com%2Fb.mp4", "url"),
            Some("https://a.com/b.mp4".to_string())
        );
        assert_eq!(query_param("a=1", "b"), None);
    }
}
