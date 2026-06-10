//! Loopback HTTP server for media previews. WebKitGTK hands <audio>/<video>
//! loading to GStreamer, which only speaks real http(s) and never calls custom
//! URI scheme handlers. So the same stream handler (allowlist, Range, path
//! confinement) is served on 127.0.0.1 with a per-session token.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::StreamState;

#[derive(Clone)]
pub struct StreamServer {
    pub port: u16,
    pub token: String,
}

/// bind synchronously (so setup gets the port), serve on the tauri runtime
pub fn start(
    state: Arc<StreamState>,
    port: u16,
) -> Result<StreamServer, Box<dyn std::error::Error>> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();

    let info = StreamServer {
        port,
        token: token.clone(),
    };
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => {
                log::error!("stream server died: {e}");
                return;
            }
        };
        loop {
            match listener.accept().await {
                Ok((conn, _)) => {
                    let state = state.clone();
                    let token = token.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = serve_conn(conn, &state, &token).await {
                            log::debug!("stream conn: {e}");
                        }
                    });
                }
                Err(e) => log::warn!("stream accept: {e}"),
            }
        }
    });
    Ok(info)
}

async fn serve_conn(conn: TcpStream, state: &StreamState, token: &str) -> std::io::Result<()> {
    let (read, mut write) = conn.into_split();
    let mut reader = BufReader::new(read);

    // request line: GET /remote?... HTTP/1.1
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();

    // headers; only Range matters
    let mut range: Option<String> = None;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).await? == 0 {
            break;
        }
        let h = h.trim();
        if h.is_empty() {
            break;
        }
        if h.len() > 6 && h[..6].eq_ignore_ascii_case("range:") {
            range = Some(h[6..].trim().to_string());
        }
    }

    let response = route(state, token, &method, &target, range).await;
    let (parts, body) = response.into_parts();

    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        parts.status.as_u16(),
        parts.status.canonical_reason().unwrap_or("")
    );
    for (k, v) in parts.headers.iter() {
        // body is fully buffered; we write our own length
        if k == http::header::CONTENT_LENGTH || k == http::header::TRANSFER_ENCODING {
            continue;
        }
        if let Ok(v) = v.to_str() {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    head.push_str(&format!("content-length: {}\r\n", body.len()));
    head.push_str("access-control-allow-origin: *\r\nconnection: close\r\n\r\n");

    write.write_all(head.as_bytes()).await?;
    write.write_all(&body).await?;
    write.flush().await?;
    Ok(())
}

async fn route(
    state: &StreamState,
    token: &str,
    method: &str,
    target: &str,
    range: Option<String>,
) -> http::Response<Vec<u8>> {
    let plain = |status: u16, msg: &str| {
        http::Response::builder()
            .status(status)
            .header(http::header::CONTENT_TYPE, "text/plain")
            .body(msg.as_bytes().to_vec())
            .unwrap_or_default()
    };
    if method != "GET" {
        return plain(405, "GET only");
    }
    // per-session token keeps other local processes out of the proxy
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    if super::query_param(query, "t").as_deref() != Some(token) {
        return plain(403, "bad token");
    }
    let mut req = http::Request::builder().method("GET").uri(target);
    if let Some(r) = range {
        req = req.header(http::header::RANGE, r);
    }
    match req.body(Vec::new()) {
        Ok(req) => super::handle(state, req).await,
        Err(_) => plain(400, "bad request"),
    }
}
