//! Tenor — GIFs and stickers via the v2 API. The old v1 API is dead; v2 runs
//! behind a free Google Cloud key (enable "Tenor API" in a Google Cloud
//! project). https://developers.google.com/tenor/guides/quickstart

use async_trait::async_trait;
use serde_json::Value;

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "tenor",
    name: "Tenor",
    homepage: "https://tenor.com",
    asset_types: &[AssetType::Gif, AssetType::Sticker],
    requires_key: true,
    key_help_url: "https://developers.google.com/tenor/guides/quickstart",
    key_hint: "Free Google Cloud API key with the Tenor API enabled (the old standalone Tenor keys are dead).",
    allowed_hosts: &["tenor.com", "tenor.googleapis.com"],
    default_rate_limit_per_min: 60,
    default_enabled: true,
    default_timeout_ms: 8000,
    embedded_credential: "",
};

pub struct Tenor;

#[async_trait]
impl SearchSource for Tenor {
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

        // stickers are a search filter, not a separate endpoint
        let sticker_only = wanted == [AssetType::Sticker];
        let asset_type = if sticker_only {
            AssetType::Sticker
        } else {
            AssetType::Gif
        };

        let mut url = format!(
            "https://tenor.googleapis.com/v2/search?key={key}&q={}&limit={}&media_filter=gif,tinygif,mp4,tinymp4,webp,tinywebp",
            urlencode(&req.query),
            req.page_size.clamp(1, 50),
        );
        if sticker_only {
            url.push_str("&searchfilter=sticker");
        }
        // tenor pages with an opaque pos string, exactly our cursor model
        if let Some(pos) = &req.cursor {
            url.push_str(&format!("&pos={}", urlencode(pos)));
        }

        let resp = ctx.http().get(&url, &[]).await?.ok()?;
        let json = resp.json()?;
        let items = parse_tenor_page(&json, asset_type);
        let next_cursor = json["next"]
            .as_str()
            .filter(|s| !s.is_empty() && !items.is_empty())
            .map(String::from);

        Ok(SearchPage { items, next_cursor })
    }
}

fn parse_tenor_page(json: &Value, asset_type: AssetType) -> Vec<ResultItem> {
    let Some(results) = json["results"].as_array() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for r in results {
        let Some(id) = r["id"].as_str() else { continue };
        let formats = &r["media_formats"];
        let title = r["title"]
            .as_str()
            .filter(|t| !t.is_empty())
            .or_else(|| r["content_description"].as_str())
            .unwrap_or("untitled")
            .to_string();

        // preview: small mp4/webp loop; thumb: tiny static-ish variant
        let preview = pick(formats, &["tinymp4", "mp4", "tinywebp", "webp", "tinygif"]);
        let thumb = pick(formats, &["tinygif", "tinywebp", "gif"]);
        // collect: full-size gif (editors expect gif from tenor), mp4 fallback
        let full = pick(formats, &["gif", "mp4", "webp"]);
        let Some((full_url, _, _, _)) = full else {
            continue;
        };

        let (preview_url, width, height, duration_ms) = match preview {
            Some((u, w, h, d)) => (Some(u), w, h, d),
            None => (None, None, None, None),
        };
        let preview_kind = match preview_url.as_deref() {
            Some(u) if u.contains(".mp4") => PreviewKind::VideoLoop,
            _ => PreviewKind::AnimatedImage,
        };
        let ext = full_url
            .rsplit('.')
            .next()
            .unwrap_or("gif")
            .split('?')
            .next()
            .unwrap_or("gif")
            .to_string();

        out.push(ResultItem {
            id: format!("tenor:{id}"),
            source: DESCRIPTOR.id.to_string(),
            asset_type,
            title: title.clone(),
            thumbnail_url: thumb.map(|(u, _, _, _)| u),
            preview_stream_url: preview_url,
            preview_kind,
            duration_ms,
            width,
            height,
            license: Some("Tenor content (check originating rights)".to_string()),
            attribution: r["itemurl"].as_str().map(String::from),
            origin_url: r["itemurl"].as_str().map(String::from),
            fetch_plan: FetchPlan::HttpGet {
                url: full_url,
                headers: vec![],
                filename_hint: format!("{}.{ext}", safe_slug(&title)),
            },
        });
    }
    out
}

/// url, width, height, duration_ms of a media_formats entry
type PickedFormat = (String, Option<u32>, Option<u32>, Option<u64>);

fn pick(formats: &Value, names: &[&str]) -> Option<PickedFormat> {
    for name in names {
        let f = &formats[name];
        if let Some(url) = f["url"].as_str() {
            let dims = f["dims"].as_array();
            let w = dims
                .and_then(|d| d.first())
                .and_then(Value::as_u64)
                .map(|v| v as u32);
            let h = dims
                .and_then(|d| d.get(1))
                .and_then(Value::as_u64)
                .map(|v| v as u32);
            let dur = f["duration"]
                .as_f64()
                .filter(|d| *d > 0.0)
                .map(|d| (d * 1000.0) as u64);
            return Some((url.to_string(), w, h, dur));
        }
    }
    None
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
        "tenor".to_string()
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
    fn parses_tenor_shape() {
        let payload = json!({
            "results": [{
                "id": "16989471141791455574",
                "title": "",
                "content_description": "Confused Math GIF",
                "itemurl": "https://tenor.com/view/confused-math-gif-16989471141791455574",
                "media_formats": {
                    "gif": { "url": "https://media.tenor.com/full.gif", "dims": [498, 280] },
                    "tinymp4": { "url": "https://media.tenor.com/tiny.mp4", "dims": [220, 124], "duration": 2.5 },
                    "tinygif": { "url": "https://media.tenor.com/tiny.gif", "dims": [220, 124] }
                }
            }],
            "next": "CAgQ0u4"
        });
        let items = parse_tenor_page(&payload, AssetType::Gif);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.title, "Confused Math GIF");
        assert_eq!(
            it.preview_stream_url.as_deref(),
            Some("https://media.tenor.com/tiny.mp4")
        );
        assert!(matches!(it.preview_kind, PreviewKind::VideoLoop));
        assert_eq!(it.duration_ms, Some(2500));
        match &it.fetch_plan {
            FetchPlan::HttpGet { url, .. } => assert_eq!(url, "https://media.tenor.com/full.gif"),
            _ => panic!("expected HttpGet"),
        }
    }
}
