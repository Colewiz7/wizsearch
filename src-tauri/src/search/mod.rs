//! Host-side search orchestration: runs all enabled sources concurrently with
//! per-source timeout + failure isolation, merges into one ranked list, and
//! emits progressive updates to the frontend.

pub mod rate_limit;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::security;
use crate::settings::SettingsStore;
use crate::sources::{
    HttpResponse, ResultItem, SearchPage, SearchRequest, SearchSource, SourceContext,
    SourceDescriptor, SourceError, SourceHttp,
};
use rate_limit::RateLimiter;

pub const EVENT_SEARCH_UPDATE: &str = "search://update";

// A browser-like UA so the scrape sources (tenor, kym, myinstants) and reddit's
// public json answer us instead of serving a bot wall. Requests go out from the
// user's own machine, so this is honest about being a normal desktop client.
pub const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0";

// ---------- the context handed to sources ----------

struct HostHttp {
    client: reqwest::Client,
    limiter: Arc<RateLimiter>,
    descriptor: &'static SourceDescriptor,
    cookies: Option<String>,
    timeout: Duration,
}

#[async_trait]
impl SourceHttp for HostHttp {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, SourceError> {
        // even source API calls must stay on the source's allowlist
        security::validate_source_url(url, self.descriptor)
            .map_err(|e| SourceError::Network(e.to_string()))?;
        // rate limit is enforced here, inside the only client sources have
        self.limiter.acquire().await;

        let mut req = self
            .client
            .get(url)
            .timeout(self.timeout)
            .header("User-Agent", USER_AGENT);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        if let Some(c) = &self.cookies {
            req = req.header("Cookie", c.clone());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?
            .to_vec();
        Ok(HttpResponse { status, body })
    }

    async fn post_form(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, SourceError> {
        security::validate_source_url(url, self.descriptor)
            .map_err(|e| SourceError::Network(e.to_string()))?;
        self.limiter.acquire().await;

        let mut req = self
            .client
            .post(url)
            .timeout(self.timeout)
            .header("User-Agent", USER_AGENT);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req
            .form(form)
            .send()
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?
            .to_vec();
        Ok(HttpResponse { status, body })
    }
}

struct HostContext {
    http: HostHttp,
    credential: Option<String>,
    source_id: &'static str,
    settings: Arc<SettingsStore>,
    app_data: std::path::PathBuf,
    timeout: Duration,
}

#[async_trait]
impl SourceContext for HostContext {
    fn http(&self) -> &dyn SourceHttp {
        &self.http
    }
    fn credential(&self) -> Option<String> {
        self.credential.clone()
    }
    fn config(&self, key: &str) -> Option<String> {
        let v = self
            .settings
            .get(&format!("sources.{}.{key}", self.source_id))
            .ok()?;
        v.as_str().map(String::from)
    }
    async fn ytdlp_search_json(&self, query: &str, count: u32) -> Result<String, SourceError> {
        let bin = crate::sidecars::tool_path(&self.app_data, "yt-dlp");
        if !bin.exists() {
            return Err(SourceError::Network(
                "yt-dlp is not installed yet (Settings > Bundled tools)".into(),
            ));
        }
        // metadata only, never a download; rate limited like everything else
        self.http.limiter.acquire().await;
        let count = count.clamp(1, 25);
        let run = tokio::process::Command::new(&bin)
            .arg(format!("ytsearch{count}:{query}"))
            .arg("--skip-download")
            .arg("--flat-playlist")
            .arg("--dump-single-json")
            .arg("--no-warnings")
            .output();
        let output = tokio::time::timeout(self.timeout, run)
            .await
            .map_err(|_| SourceError::Network("yt-dlp timed out".into()))?
            .map_err(|e| SourceError::Network(e.to_string()))?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            let tail: String = err.lines().last().unwrap_or("unknown error").to_string();
            return Err(SourceError::Network(format!("yt-dlp: {tail}")));
        }
        String::from_utf8(output.stdout).map_err(|e| SourceError::Parse(e.to_string()))
    }
}

// ---------- host ----------

struct RegisteredSource {
    source: Arc<dyn SearchSource>,
    limiter: Arc<RateLimiter>,
}

pub struct SearchHost {
    client: reqwest::Client,
    sources: Vec<RegisteredSource>,
    settings: Arc<SettingsStore>,
    app_data: std::path::PathBuf,
    generation: AtomicU64,
    accum: tokio::sync::Mutex<Accum>,
}

#[derive(Default)]
struct Accum {
    search_id: u64,
    by_source: HashMap<String, Vec<ResultItem>>,
    statuses: HashMap<String, SourceStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceStatus {
    pub id: String,
    pub state: String, // "pending" | "done" | "error" | "disabled" | "timeout"
    pub error: Option<String>,
    pub count: usize,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchUpdate {
    pub search_id: u64,
    pub items: Vec<ResultItem>,
    pub sources: Vec<SourceStatus>,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub homepage: String,
    pub key_help_url: String,
    pub asset_types: Vec<crate::sources::AssetType>,
    pub requires_key: bool,
    pub has_key: bool,
    pub enabled: bool,
}

impl SearchHost {
    pub fn new(
        settings: Arc<SettingsStore>,
        app_data: std::path::PathBuf,
        sources: Vec<Arc<dyn SearchSource>>,
    ) -> Self {
        let registered = sources
            .into_iter()
            .map(|source| {
                let d = source.descriptor();
                let per_min = settings.i64_or(
                    &format!("sources.{}.rate_limit_per_min", d.id),
                    d.default_rate_limit_per_min as i64,
                );
                RegisteredSource {
                    limiter: Arc::new(RateLimiter::per_minute(per_min as u32)),
                    source,
                }
            })
            .collect();
        SearchHost {
            client: reqwest::Client::new(),
            sources: registered,
            settings,
            app_data,
            generation: AtomicU64::new(0),
            accum: tokio::sync::Mutex::new(Accum::default()),
        }
    }

    /// per-source timeout (a setting; yt-dlp needs way longer than http APIs)
    fn timeout_for(&self, d: &SourceDescriptor) -> Duration {
        Duration::from_millis(self.settings.i64_or(
            &format!("sources.{}.timeout_ms", d.id),
            d.default_timeout_ms as i64,
        ) as u64)
    }

    fn enabled(&self, d: &SourceDescriptor) -> bool {
        self.settings
            .bool_or(&format!("sources.{}.enabled", d.id), d.default_enabled)
    }

    pub fn descriptors(&self) -> Vec<&'static SourceDescriptor> {
        self.sources.iter().map(|r| r.source.descriptor()).collect()
    }

    fn find(&self, id: &str) -> Option<&RegisteredSource> {
        self.sources.iter().find(|r| r.source.descriptor().id == id)
    }

    pub async fn source_infos(&self) -> Vec<SourceInfo> {
        let mut out = Vec::new();
        for r in &self.sources {
            let d = r.source.descriptor();
            let has_key = security::secret_get_async(security::credential_key(d.id))
                .await
                .ok()
                .flatten()
                .is_some();
            out.push(SourceInfo {
                id: d.id.to_string(),
                name: d.name.to_string(),
                homepage: d.homepage.to_string(),
                key_help_url: d.key_help_url.to_string(),
                asset_types: d.asset_types.to_vec(),
                requires_key: d.requires_key,
                has_key,
                enabled: self.enabled(d),
            });
        }
        out
    }

    async fn make_context(&self, reg: &RegisteredSource, timeout: Duration) -> HostContext {
        let d = reg.source.descriptor();
        let credential = security::secret_get_async(security::credential_key(d.id))
            .await
            .ok()
            .flatten();
        let cookies = security::secret_get_async(security::cookie_key(d.id))
            .await
            .ok()
            .flatten();
        HostContext {
            credential,
            source_id: d.id,
            settings: self.settings.clone(),
            app_data: self.app_data.clone(),
            timeout,
            http: HostHttp {
                client: self.client.clone(),
                limiter: reg.limiter.clone(),
                descriptor: d,
                cookies,
                timeout,
            },
        }
    }

    /// Kick off a new search generation. Old in-flight searches stop emitting.
    pub async fn start_search(
        self: &Arc<Self>,
        app: AppHandle,
        query: String,
        asset_types: Vec<crate::sources::AssetType>,
    ) -> u64 {
        let search_id = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let page_size = self.settings.i64_or("search.page_size", 24) as u32;

        {
            let mut acc = self.accum.lock().await;
            *acc = Accum {
                search_id,
                ..Default::default()
            };
            for r in &self.sources {
                let d = r.source.descriptor();
                let enabled = self.enabled(d);
                acc.statuses.insert(
                    d.id.to_string(),
                    SourceStatus {
                        id: d.id.to_string(),
                        state: if enabled { "pending" } else { "disabled" }.into(),
                        error: None,
                        count: 0,
                        next_cursor: None,
                    },
                );
            }
        }
        self.emit_update(&app, search_id).await;

        for reg in &self.sources {
            let d = reg.source.descriptor();
            if !self.enabled(d) {
                continue;
            }
            let timeout = self.timeout_for(d);
            let req = SearchRequest {
                query: query.clone(),
                asset_types: asset_types.clone(),
                cursor: None,
                page_size,
            };
            let ctx = self.make_context(reg, timeout).await;
            let source = reg.source.clone();
            let host = self.clone();
            let app = app.clone();
            let source_id = d.id.to_string();

            tauri::async_runtime::spawn(async move {
                let outcome = tokio::time::timeout(timeout, source.search(&ctx, &req)).await;
                // failure isolation: one source's error/timeout never touches the rest
                let (state, error, items, next_cursor) = match outcome {
                    Ok(Ok(page)) => ("done", None, page.items, page.next_cursor),
                    Ok(Err(e)) => ("error", Some(e.to_string()), vec![], None),
                    Err(_) => ("timeout", Some("timed out".to_string()), vec![], None),
                };
                if host.generation.load(Ordering::SeqCst) != search_id {
                    return; // superseded by a newer search
                }
                {
                    let mut acc = host.accum.lock().await;
                    if acc.search_id != search_id {
                        return;
                    }
                    let count = items.len();
                    acc.by_source.insert(source_id.clone(), items);
                    if let Some(s) = acc.statuses.get_mut(&source_id) {
                        s.state = state.into();
                        s.error = error;
                        s.count = count;
                        s.next_cursor = next_cursor;
                    }
                }
                host.emit_update(&app, search_id).await;
            });
        }
        search_id
    }

    async fn emit_update(&self, app: &AppHandle, search_id: u64) {
        let strategy = self
            .settings
            .string_or("search.merge_strategy", "round_robin");
        let acc = self.accum.lock().await;
        if acc.search_id != search_id {
            return;
        }
        let order: Vec<String> = self
            .sources
            .iter()
            .map(|r| r.source.descriptor().id.to_string())
            .collect();
        let items = merge_results(&acc.by_source, &order, &strategy);
        let mut sources: Vec<SourceStatus> = acc.statuses.values().cloned().collect();
        sources.sort_by_key(|s| order.iter().position(|o| *o == s.id).unwrap_or(usize::MAX));
        let done = sources.iter().all(|s| s.state != "pending");
        let update = SearchUpdate {
            search_id,
            items,
            sources,
            done,
        };
        if let Err(e) = app.emit(EVENT_SEARCH_UPDATE, &update) {
            log::warn!("emit search update failed: {e}");
        }
    }

    /// One more page from a single source (user hit "load more").
    pub async fn search_more(
        &self,
        source_id: &str,
        query: String,
        asset_types: Vec<crate::sources::AssetType>,
        cursor: Option<String>,
    ) -> Result<SearchPage, SourceError> {
        let reg = self
            .find(source_id)
            .ok_or_else(|| SourceError::Parse(format!("unknown source {source_id}")))?;
        let timeout = self.timeout_for(reg.source.descriptor());
        let page_size = self.settings.i64_or("search.page_size", 24) as u32;
        let ctx = self.make_context(reg, timeout).await;
        let req = SearchRequest {
            query,
            asset_types,
            cursor,
            page_size,
        };
        tokio::time::timeout(timeout, reg.source.search(&ctx, &req))
            .await
            .map_err(|_| SourceError::Network("timed out".into()))?
    }
}

/// Merge per-source result lists into one grid order.
/// round_robin: interleave sources (default). grouped: concatenate in source order.
pub fn merge_results(
    by_source: &HashMap<String, Vec<ResultItem>>,
    source_order: &[String],
    strategy: &str,
) -> Vec<ResultItem> {
    let lists: Vec<&Vec<ResultItem>> = source_order
        .iter()
        .filter_map(|id| by_source.get(id))
        .collect();
    match strategy {
        "grouped" => lists.into_iter().flatten().cloned().collect(),
        _ => {
            // round-robin interleave
            let mut out = Vec::new();
            let longest = lists.iter().map(|l| l.len()).max().unwrap_or(0);
            for i in 0..longest {
                for list in &lists {
                    if let Some(item) = list.get(i) {
                        out.push(item.clone());
                    }
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{AssetType, FetchPlan, PreviewKind};

    fn item(source: &str, n: usize) -> ResultItem {
        ResultItem {
            id: format!("{source}:{n}"),
            source: source.to_string(),
            asset_type: AssetType::Audio,
            title: format!("{source} {n}"),
            thumbnail_url: None,
            preview_stream_url: None,
            preview_kind: PreviewKind::AudioStream,
            duration_ms: None,
            width: None,
            height: None,
            license: None,
            attribution: None,
            origin_url: None,
            fetch_plan: FetchPlan::HttpGet {
                url: "https://example.com/x".into(),
                headers: vec![],
                filename_hint: "x".into(),
            },
        }
    }

    #[test]
    fn round_robin_interleaves() {
        let mut by_source = HashMap::new();
        by_source.insert("a".to_string(), vec![item("a", 0), item("a", 1)]);
        by_source.insert("b".to_string(), vec![item("b", 0)]);
        let order = vec!["a".to_string(), "b".to_string()];
        let merged = merge_results(&by_source, &order, "round_robin");
        let ids: Vec<&str> = merged.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["a:0", "b:0", "a:1"]);
    }

    #[test]
    fn grouped_concatenates_in_order() {
        let mut by_source = HashMap::new();
        by_source.insert("a".to_string(), vec![item("a", 0)]);
        by_source.insert("b".to_string(), vec![item("b", 0), item("b", 1)]);
        let order = vec!["b".to_string(), "a".to_string()];
        let merged = merge_results(&by_source, &order, "grouped");
        let ids: Vec<&str> = merged.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["b:0", "b:1", "a:0"]);
    }
}
