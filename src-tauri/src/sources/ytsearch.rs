//! YouTube via yt-dlp — short video memes, no key. Discovery is metadata-only
//! (`ytsearchN:` + --flat-playlist, run by the host); the actual download only
//! happens when the user collects, via a YtDlp fetch plan.

use async_trait::async_trait;
use serde_json::Value;

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "youtube",
    name: "YouTube (yt-dlp)",
    homepage: "https://www.youtube.com",
    asset_types: &[AssetType::Video],
    requires_key: false,
    key_help_url: "",
    allowed_hosts: &[
        "youtube.com",
        "youtu.be",
        "ytimg.com",
        "ggpht.com",
        "googlevideo.com",
    ],
    default_rate_limit_per_min: 10,
    default_enabled: true,
    default_timeout_ms: 30000, // yt-dlp searches take seconds, not millis
    embedded_credential: "",
};

pub struct YtSearch;

#[async_trait]
impl SearchSource for YtSearch {
    fn descriptor(&self) -> &'static SourceDescriptor {
        &DESCRIPTOR
    }

    async fn search(
        &self,
        ctx: &dyn SourceContext,
        req: &SearchRequest,
    ) -> Result<SearchPage, SourceError> {
        if req.wanted(DESCRIPTOR.asset_types).is_empty() {
            return Ok(SearchPage::empty());
        }
        // no paging: ytsearchN always returns from the top
        if req.cursor.is_some() {
            return Ok(SearchPage::empty());
        }

        let json_text = ctx
            .ytdlp_search_json(&req.query, req.page_size.clamp(1, 20))
            .await?;
        let json: Value =
            serde_json::from_str(&json_text).map_err(|e| SourceError::Parse(e.to_string()))?;

        Ok(SearchPage {
            items: parse_flat_playlist(&json),
            next_cursor: None,
        })
    }
}

fn parse_flat_playlist(json: &Value) -> Vec<ResultItem> {
    let Some(entries) = json["entries"].as_array() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for e in entries {
        let Some(id) = e["id"].as_str() else { continue };
        let title = e["title"].as_str().unwrap_or("untitled").to_string();
        let url = e["url"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={id}"));
        let channel = e["channel"]
            .as_str()
            .or_else(|| e["uploader"].as_str())
            .unwrap_or("unknown channel");
        let duration_ms = e["duration"]
            .as_f64()
            .filter(|d| *d > 0.0)
            .map(|d| (d * 1000.0) as u64);

        // best thumbnail yt gives us in flat mode
        let thumb = e["thumbnails"]
            .as_array()
            .and_then(|ts| ts.last())
            .and_then(|t| t["url"].as_str())
            .map(String::from);

        out.push(ResultItem {
            id: format!("youtube:{id}"),
            source: DESCRIPTOR.id.to_string(),
            asset_type: AssetType::Video,
            title: title.clone(),
            thumbnail_url: thumb,
            // no cheap stream url in flat mode; tile shows the thumbnail
            preview_stream_url: None,
            preview_kind: PreviewKind::PosterLoop,
            duration_ms,
            width: None,
            height: None,
            license: Some("YouTube content (third-party rights)".to_string()),
            attribution: Some(format!("{channel} on YouTube")),
            origin_url: Some(url.clone()),
            fetch_plan: FetchPlan::YtDlp {
                url,
                filename_hint: format!("{}.mp4", safe_slug(&title)),
            },
        });
    }
    out
}

fn safe_slug(s: &str) -> String {
    let slug: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let out: String = slug.trim_matches('-').chars().take(48).collect();
    if out.is_empty() {
        "video".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_flat_playlist() {
        let payload = json!({
            "entries": [{
                "id": "dQw4w9WgXcQ",
                "title": "vine boom compilation",
                "url": "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
                "channel": "memes weekly",
                "duration": 42.0,
                "thumbnails": [
                    { "url": "https://i.ytimg.com/vi/dQw4w9WgXcQ/default.jpg" },
                    { "url": "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg" }
                ]
            }]
        });
        let items = parse_flat_playlist(&payload);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.duration_ms, Some(42_000));
        assert_eq!(
            it.thumbnail_url.as_deref(),
            Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg")
        );
        match &it.fetch_plan {
            FetchPlan::YtDlp { url, filename_hint } => {
                assert_eq!(url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
                assert_eq!(filename_hint, "vine-boom-compilation.mp4");
            }
            _ => panic!("expected YtDlp plan"),
        }
    }
}
