//! Pexels — stock video + green screens. Free per-user API key sent in the
//! Authorization header. https://www.pexels.com/api/

use async_trait::async_trait;
use serde_json::Value;

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "pexels",
    name: "Pexels",
    homepage: "https://www.pexels.com",
    asset_types: &[AssetType::Video, AssetType::GreenScreen],
    requires_key: true,
    key_help_url: "https://www.pexels.com/api/",
    key_hint: "",
    allowed_hosts: &["pexels.com"],
    default_rate_limit_per_min: 50,
    default_enabled: true,
    default_timeout_ms: 8000,
    embedded_credential: "",
};

pub struct Pexels;

#[async_trait]
impl SearchSource for Pexels {
    fn descriptor(&self) -> &'static SourceDescriptor {
        &DESCRIPTOR
    }

    async fn search(
        &self,
        ctx: &dyn SourceContext,
        req: &SearchRequest,
    ) -> Result<SearchPage, SourceError> {
        let wanted = req.wanted(DESCRIPTOR.asset_types);
        if wanted.is_empty() {
            return Ok(SearchPage::empty());
        }
        let key = ctx.credential().ok_or(SourceError::MissingCredential)?;

        // green-screen-only searches get the query augmented; mixed/all searches
        // run one plain video query
        let green_only = wanted == [AssetType::GreenScreen];
        let asset_type = if green_only {
            AssetType::GreenScreen
        } else {
            AssetType::Video
        };
        let query = if green_only {
            format!("green screen {}", req.query)
        } else {
            req.query.clone()
        };

        let page: u32 = req
            .cursor
            .as_deref()
            .map(|c| {
                c.parse()
                    .map_err(|_| SourceError::Parse("bad cursor".into()))
            })
            .transpose()?
            .unwrap_or(1);

        let url = format!(
            "https://api.pexels.com/videos/search?query={}&page={page}&per_page={}",
            urlencode(&query),
            req.page_size.clamp(1, 80)
        );
        let resp = ctx
            .http()
            .get(&url, &[("Authorization", &key)])
            .await?
            .ok()?;
        let json = resp.json()?;

        let items = parse_pexels_page(&json, asset_type);
        let next_cursor = json["next_page"].as_str().map(|_| (page + 1).to_string());

        Ok(SearchPage { items, next_cursor })
    }
}

fn parse_pexels_page(json: &Value, asset_type: AssetType) -> Vec<ResultItem> {
    let Some(videos) = json["videos"].as_array() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for v in videos {
        let id = match v["id"].as_i64() {
            Some(n) => n.to_string(),
            None => continue,
        };
        let origin = v["url"].as_str().unwrap_or_default().to_string();
        let user = v["user"]["name"].as_str().unwrap_or("Pexels creator");
        let title = title_from_url(&origin).unwrap_or_else(|| format!("Video by {user}"));
        let poster = v["image"].as_str().map(String::from);
        let duration_ms = v["duration"].as_u64().map(|s| s * 1000);
        let width = v["width"].as_u64().map(|n| n as u32);
        let height = v["height"].as_u64().map(|n| n as u32);

        let files = v["video_files"].as_array().cloned().unwrap_or_default();
        // preview: smallest mp4 that's still watchable; full: biggest mp4
        let mut mp4s: Vec<&Value> = files
            .iter()
            .filter(|f| f["file_type"].as_str() == Some("video/mp4"))
            .collect();
        if mp4s.is_empty() {
            continue;
        }
        mp4s.sort_by_key(|f| f["width"].as_u64().unwrap_or(0));
        let preview = mp4s
            .iter()
            .find(|f| f["width"].as_u64().unwrap_or(0) >= 480)
            .or(mp4s.first())
            .and_then(|f| f["link"].as_str())
            .map(String::from);
        let Some(full) = mp4s.last().and_then(|f| f["link"].as_str()) else {
            continue;
        };

        out.push(ResultItem {
            id: format!("pexels:{id}"),
            source: DESCRIPTOR.id.to_string(),
            asset_type,
            title: title.clone(),
            thumbnail_url: poster,
            preview_stream_url: preview,
            preview_kind: PreviewKind::PosterLoop,
            duration_ms,
            width,
            height,
            license: Some("Pexels License (free, no attribution required)".to_string()),
            attribution: Some(format!("Video by {user} on Pexels")),
            origin_url: Some(origin),
            fetch_plan: FetchPlan::HttpGet {
                url: full.to_string(),
                headers: vec![],
                filename_hint: format!("{}.mp4", safe_slug(&title)),
            },
        });
    }
    out
}

/// "https://www.pexels.com/video/clouds-over-mountain-12345/" -> "clouds over mountain"
fn title_from_url(url: &str) -> Option<String> {
    let seg = url.trim_end_matches('/').rsplit('/').next()?;
    let words: Vec<&str> = seg
        .split('-')
        .filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit()))
        .collect();
    if words.is_empty() {
        return None;
    }
    Some(words.join(" "))
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

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_pexels_shape() {
        let payload = json!({
            "videos": [{
                "id": 857195,
                "url": "https://www.pexels.com/video/dog-running-on-grass-857195/",
                "image": "https://images.pexels.com/videos/857195/poster.jpg",
                "duration": 12,
                "width": 1920,
                "height": 1080,
                "user": { "name": "Jane Doe" },
                "video_files": [
                    { "file_type": "video/mp4", "width": 640, "link": "https://videos.pexels.com/sd.mp4" },
                    { "file_type": "video/mp4", "width": 1920, "link": "https://videos.pexels.com/hd.mp4" }
                ]
            }],
            "next_page": "https://api.pexels.com/videos/search?page=2"
        });
        let items = parse_pexels_page(&payload, AssetType::GreenScreen);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.title, "dog running on grass");
        assert_eq!(
            it.preview_stream_url.as_deref(),
            Some("https://videos.pexels.com/sd.mp4")
        );
        assert_eq!(it.duration_ms, Some(12_000));
        match &it.fetch_plan {
            FetchPlan::HttpGet { url, .. } => assert_eq!(url, "https://videos.pexels.com/hd.mp4"),
            _ => panic!("expected HttpGet"),
        }
    }
}
