//! Reddit — meme subs via the public search JSON, no key. Which subreddits to
//! search is a setting (sources.reddit.subreddits). GIFs/images fetch from
//! reddit's own mirrors; videos collect through yt-dlp so they keep audio.

use async_trait::async_trait;
use serde_json::Value;

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

pub const DEFAULT_SUBREDDITS: &str = "memes,MemeVideos,gifs,reactiongifs,greenscreenvideos";

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "reddit",
    name: "Reddit",
    homepage: "https://www.reddit.com",
    asset_types: &[
        AssetType::Gif,
        AssetType::Image,
        AssetType::Video,
        AssetType::GreenScreen,
    ],
    requires_key: false,
    key_help_url: "",
    allowed_hosts: &["reddit.com", "redd.it", "redditmedia.com"],
    default_rate_limit_per_min: 30,
    default_enabled: true,
    default_timeout_ms: 10000,
    embedded_credential: "",
};

pub struct Reddit;

#[async_trait]
impl SearchSource for Reddit {
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

        let subs = ctx
            .config("subreddits")
            .unwrap_or_else(|| DEFAULT_SUBREDDITS.to_string());
        let multi: String = subs
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("+");
        if multi.is_empty() {
            return Err(SourceError::Parse("no subreddits configured".into()));
        }

        let mut url = format!(
            "https://www.reddit.com/r/{multi}/search.json?q={}&restrict_sr=1&limit={}&sort=relevance&raw_json=1",
            urlencode(&req.query),
            req.page_size.clamp(1, 50),
        );
        if let Some(after) = &req.cursor {
            url.push_str(&format!("&after={}", urlencode(after)));
        }

        let resp = ctx.http().get(&url, &[]).await?.ok()?;
        let json = resp.json()?;
        let items = parse_listing(&json, &req.wanted(DESCRIPTOR.asset_types));
        let next_cursor = json["data"]["after"]
            .as_str()
            .filter(|a| !a.is_empty() && !items.is_empty())
            .map(String::from);

        Ok(SearchPage { items, next_cursor })
    }
}

fn parse_listing(json: &Value, wanted: &[AssetType]) -> Vec<ResultItem> {
    let Some(children) = json["data"]["children"].as_array() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for child in children {
        let post = &child["data"];
        if post["over_18"].as_bool().unwrap_or(false) {
            continue;
        }
        let Some(id) = post["id"].as_str() else {
            continue;
        };
        let title = post["title"].as_str().unwrap_or("untitled").to_string();
        let permalink = post["permalink"].as_str().unwrap_or_default();
        let origin = format!("https://www.reddit.com{permalink}");
        let sub = post["subreddit"].as_str().unwrap_or("reddit");

        let preview_img = &post["preview"]["images"][0];
        let thumb = preview_img["resolutions"]
            .as_array()
            .and_then(|r| r.iter().find(|v| v["width"].as_u64().unwrap_or(0) >= 320))
            .or_else(|| preview_img["resolutions"].as_array().and_then(|r| r.last()))
            .and_then(|v| v["url"].as_str())
            .or_else(|| preview_img["source"]["url"].as_str())
            .map(String::from);

        let gif_mp4 = preview_img["variants"]["mp4"]["source"]["url"]
            .as_str()
            .map(String::from);
        let gif_full = preview_img["variants"]["gif"]["source"]["url"]
            .as_str()
            .map(String::from);
        let reddit_video = post["media"]["reddit_video"]["fallback_url"]
            .as_str()
            .or_else(|| post["preview"]["reddit_video_preview"]["fallback_url"].as_str())
            .map(String::from);
        let source_img = preview_img["source"]["url"].as_str().map(String::from);
        let width = preview_img["source"]["width"].as_u64().map(|v| v as u32);
        let height = preview_img["source"]["height"].as_u64().map(|v| v as u32);
        let is_green = sub.to_ascii_lowercase().contains("greenscreen");

        // classify: animated gif > hosted video > plain image
        let (asset_type, preview_url, preview_kind, plan) = if let Some(gif) = gif_full {
            (
                AssetType::Gif,
                gif_mp4.clone(),
                PreviewKind::VideoLoop,
                FetchPlan::HttpGet {
                    url: gif,
                    headers: vec![],
                    filename_hint: format!("{}.gif", safe_slug(&title)),
                },
            )
        } else if let Some(video) = reddit_video {
            (
                if is_green {
                    AssetType::GreenScreen
                } else {
                    AssetType::Video
                },
                Some(video),
                PreviewKind::PosterLoop,
                // yt-dlp merges the separate v.redd.it audio track back in
                FetchPlan::YtDlp {
                    url: origin.clone(),
                    filename_hint: format!("{}.mp4", safe_slug(&title)),
                },
            )
        } else if let Some(img) = source_img {
            (
                AssetType::Image,
                None,
                PreviewKind::PosterLoop,
                FetchPlan::HttpGet {
                    url: img.clone(),
                    headers: vec![],
                    filename_hint: format!("{}.{}", safe_slug(&title), ext_of(&img, "jpg")),
                },
            )
        } else {
            continue; // text/link post, nothing collectable
        };

        if !wanted.contains(&asset_type) {
            continue;
        }

        out.push(ResultItem {
            id: format!("reddit:{id}"),
            source: DESCRIPTOR.id.to_string(),
            asset_type,
            title: title.clone(),
            thumbnail_url: thumb,
            preview_stream_url: preview_url,
            preview_kind,
            duration_ms: post["media"]["reddit_video"]["duration"]
                .as_f64()
                .map(|d| (d * 1000.0) as u64),
            width,
            height,
            license: Some("Reddit user content (third-party rights)".to_string()),
            attribution: Some(format!("r/{sub}")),
            origin_url: Some(origin),
            fetch_plan: plan,
        });
    }
    out
}

fn ext_of(url: &str, fallback: &str) -> String {
    url.split('?')
        .next()
        .and_then(|u| u.rsplit('.').next())
        .filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or(fallback)
        .to_string()
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
        "reddit".to_string()
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
    fn classifies_gif_video_image() {
        let payload = json!({ "data": { "after": "t3_next", "children": [
            { "data": {
                "id": "g1", "title": "funny gif", "permalink": "/r/memes/comments/g1/funny/",
                "subreddit": "memes", "over_18": false,
                "preview": { "images": [{
                    "source": { "url": "https://preview.redd.it/x.gif", "width": 480, "height": 270 },
                    "resolutions": [{ "url": "https://preview.redd.it/x-320.gif", "width": 320 }],
                    "variants": {
                        "gif": { "source": { "url": "https://preview.redd.it/x.gif" } },
                        "mp4": { "source": { "url": "https://preview.redd.it/x.mp4" } }
                    }
                }]}
            }},
            { "data": {
                "id": "v1", "title": "funny video", "permalink": "/r/MemeVideos/comments/v1/funny/",
                "subreddit": "MemeVideos", "over_18": false,
                "media": { "reddit_video": { "fallback_url": "https://v.redd.it/abc/DASH_720.mp4", "duration": 9.0 } },
                "preview": { "images": [{ "source": { "url": "https://preview.redd.it/v.jpg", "width": 1280, "height": 720 }, "resolutions": [] }] }
            }},
            { "data": {
                "id": "n1", "title": "nsfw thing", "over_18": true, "permalink": "/r/memes/x/", "subreddit": "memes"
            }}
        ]}});
        let items = parse_listing(&payload, &AssetType::ALL);
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].asset_type, AssetType::Gif));
        assert!(matches!(items[0].fetch_plan, FetchPlan::HttpGet { .. }));
        assert!(matches!(items[1].asset_type, AssetType::Video));
        match &items[1].fetch_plan {
            FetchPlan::YtDlp { url, .. } => {
                assert_eq!(
                    url,
                    "https://www.reddit.com/r/MemeVideos/comments/v1/funny/"
                )
            }
            _ => panic!("videos must collect via yt-dlp"),
        }
        assert_eq!(items[1].duration_ms, Some(9000));
    }
}
