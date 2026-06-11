//! The SearchSource trait and everything a source is allowed to touch.
//!
//! Sources are PURE: they get a read-only context (rate-limited http, read-only
//! credential) and return data + declarative FetchPlans. They must never import
//! shell, filesystem, database, network-client, or tauri modules. That boundary
//! is enforced by tests/source_purity.rs and scripts/check_source_purity.sh.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod giphy;
pub mod imgflip;
pub mod imgur;
pub mod klipy;
pub mod kym;
pub mod myinstants;
pub mod pexels;
pub mod tenor;
pub mod ytsearch;

// ---------- core enums ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Audio,
    Gif,
    Sticker,
    Image,
    Video,
    GreenScreen,
}

impl AssetType {
    pub const ALL: [AssetType; 6] = [
        AssetType::Audio,
        AssetType::Gif,
        AssetType::Sticker,
        AssetType::Image,
        AssetType::Video,
        AssetType::GreenScreen,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            AssetType::Audio => "audio",
            AssetType::Gif => "gif",
            AssetType::Sticker => "sticker",
            AssetType::Image => "image",
            AssetType::Video => "video",
            AssetType::GreenScreen => "green_screen",
        }
    }
}

/// How the frontend should preview this item. Playback always goes through the
/// wzstream protocol, never raw paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewKind {
    /// inline audio row with play/pause + progress
    AudioStream,
    /// muted looping <video> on hover
    VideoLoop,
    /// animated webp/gif swapped in on hover
    AnimatedImage,
    /// static poster, looping video on hover (green screens)
    PosterLoop,
}

// ---------- fetch plans ----------

/// Declarative download plan. Sources only DESCRIBE the fetch; the host executes
/// it after explicit user selection and validates the URL against the source's
/// allowlist (security::validate_fetch_plan).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FetchPlan {
    HttpGet {
        url: String,
        #[serde(default)]
        headers: Vec<(String, String)>,
        filename_hint: String,
    },
    /// future: yt-dlp execution by the host. No source returns this yet.
    YtDlp { url: String, filename_hint: String },
}

// ---------- result items ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultItem {
    /// stable-ish id within the source, used for dedupe in the grid
    pub id: String,
    /// source descriptor id
    pub source: String,
    pub asset_type: AssetType,
    pub title: String,
    pub thumbnail_url: Option<String>,
    /// remote preview media URL; the frontend wraps it in wzstream://…/remote
    pub preview_stream_url: Option<String>,
    pub preview_kind: PreviewKind,
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub license: Option<String>,
    pub attribution: Option<String>,
    pub origin_url: Option<String>,
    pub fetch_plan: FetchPlan,
}

// ---------- search request / response ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    /// empty = all types the source supports
    pub asset_types: Vec<AssetType>,
    /// opaque cursor returned by a previous page from this source
    pub cursor: Option<String>,
    pub page_size: u32,
}

impl SearchRequest {
    /// asset types this request wants, intersected with what the source offers
    pub fn wanted(&self, supported: &[AssetType]) -> Vec<AssetType> {
        if self.asset_types.is_empty() {
            supported.to_vec()
        } else {
            supported
                .iter()
                .copied()
                .filter(|t| self.asset_types.contains(t))
                .collect()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPage {
    pub items: Vec<ResultItem>,
    pub next_cursor: Option<String>,
}

impl SearchPage {
    pub fn empty() -> Self {
        SearchPage {
            items: Vec::new(),
            next_cursor: None,
        }
    }
}

// ---------- errors ----------

#[derive(Debug, thiserror::Error, Serialize, Deserialize, Clone)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum SourceError {
    #[error("this source needs an API key; add one in Settings")]
    MissingCredential,
    #[error("auth rejected: {0}")]
    AuthRejected(String),
    #[error("http status {0}")]
    HttpStatus(u16),
    #[error("network: {0}")]
    Network(String),
    #[error("parse: {0}")]
    Parse(String),
    #[error("unsupported request")]
    Unsupported,
}

// ---------- the read-only context sources receive ----------

pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn text(&self) -> Result<String, SourceError> {
        String::from_utf8(self.body.clone()).map_err(|e| SourceError::Parse(e.to_string()))
    }

    pub fn json(&self) -> Result<serde_json::Value, SourceError> {
        serde_json::from_slice(&self.body).map_err(|e| SourceError::Parse(e.to_string()))
    }

    pub fn ok(self) -> Result<Self, SourceError> {
        match self.status {
            200..=299 => Ok(self),
            401 | 403 => Err(SourceError::AuthRejected(format!("status {}", self.status))),
            s => Err(SourceError::HttpStatus(s)),
        }
    }
}

/// Injected http client. The host implementation acquires the source's rate
/// limiter before every request, so sources cannot bypass rate limits.
#[async_trait]
pub trait SourceHttp: Send + Sync {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, SourceError>;
    /// form-encoded POST, for oauth token exchanges. Same rate limiting and
    /// allowlist checks as get.
    async fn post_form(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        form: &[(&str, &str)],
    ) -> Result<HttpResponse, SourceError>;
}

#[async_trait]
pub trait SourceContext: Send + Sync {
    fn http(&self) -> &dyn SourceHttp;
    /// per-user credential from the OS keychain, read-only. None if unset.
    fn credential(&self) -> Option<String>;
    /// read-only access to this source's own settings: config("foo") reads
    /// sources.<id>.foo from the registry
    fn config(&self, key: &str) -> Option<String>;
    /// metadata-only yt-dlp search (`ytsearchN:`), run by the host. Returns the
    /// --dump-single-json output. Sources never touch the binary themselves.
    async fn ytdlp_search_json(&self, query: &str, count: u32) -> Result<String, SourceError>;
}

// ---------- descriptor + trait ----------

#[derive(Debug, Clone, Serialize)]
pub struct SourceDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub homepage: &'static str,
    pub asset_types: &'static [AssetType],
    pub requires_key: bool,
    /// where the user gets a free key, shown in Settings
    pub key_help_url: &'static str,
    /// optional Settings description for the key field (format hints etc.)
    pub key_hint: &'static str,
    /// hosts (suffix match) that previews and fetch plans may touch
    pub allowed_hosts: &'static [&'static str],
    pub default_rate_limit_per_min: u32,
    /// default for the generated sources.<id>.enabled setting (opt-in sources
    /// like GIPHY ship disabled)
    pub default_enabled: bool,
    /// default for the generated sources.<id>.timeout_ms setting (yt-dlp is slow)
    pub default_timeout_ms: u32,
    /// MUST be empty. Asserted at startup: WizSearch never ships a developer key.
    pub embedded_credential: &'static str,
}

#[async_trait]
pub trait SearchSource: Send + Sync {
    fn descriptor(&self) -> &'static SourceDescriptor;

    /// One page of results. Pure: read inputs, call ctx.http(), return data.
    async fn search(
        &self,
        ctx: &dyn SourceContext,
        req: &SearchRequest,
    ) -> Result<SearchPage, SourceError>;
}
