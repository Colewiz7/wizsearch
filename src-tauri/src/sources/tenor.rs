//! Tenor — GIFs and stickers, scraped, no key. Google killed the free Tenor
//! key signup, so instead of the API we pull the `"results":[...]` JSON that
//! tenor.com's search page embeds (same shape the API returns). Single page;
//! the public site is infinite-scroll with no cheap cursor.

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
    asset_types: &[AssetType::Gif],
    requires_key: false,
    key_help_url: "",
    key_hint: "",
    allowed_hosts: &["tenor.com"],
    default_rate_limit_per_min: 30,
    default_enabled: true,
    default_timeout_ms: 10000,
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
        if req.wanted(DESCRIPTOR.asset_types).is_empty() {
            return Ok(SearchPage::empty());
        }
        // single page only; no cursor to follow on the scraped page
        if req.cursor.is_some() {
            return Ok(SearchPage::empty());
        }

        let slug = slugify_query(&req.query);
        let url = format!("https://tenor.com/search/{slug}-gifs");
        let resp = ctx
            .http()
            .get(&url, &[("Accept", "text/html")])
            .await?
            .ok()?;
        let html = resp.text()?;

        // pull the embedded results array and parse it like an API response
        let results = extract_results(&html)
            .ok_or_else(|| SourceError::Parse("no results json on tenor page".into()))?;
        Ok(SearchPage {
            items: parse_tenor_page(&results, AssetType::Gif),
            next_cursor: None,
        })
    }
}

/// "deal with it" -> "deal-with-it" for the /search/<slug>-gifs path
fn slugify_query(q: &str) -> String {
    let slug: String = q
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let out: String = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if out.is_empty() {
        "funny".to_string()
    } else {
        out
    }
}

/// find `"results":[ ... ]` in the page and return it as a parsed JSON array,
/// matching brackets while respecting strings and escapes
fn extract_results(html: &str) -> Option<Value> {
    let needle = "\"results\":[";
    let start = html.find(needle)? + needle.len() - 1; // point at the '['
    let bytes = html.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'[' | b'{' => depth += 1,
            b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    let arr = &html[start..=i];
                    return serde_json::from_str::<Value>(arr).ok();
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_tenor_page(results: &Value, asset_type: AssetType) -> Vec<ResultItem> {
    let Some(results) = results.as_array() else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn slugifies_queries() {
        assert_eq!(slugify_query("deal with it"), "deal-with-it");
        assert_eq!(slugify_query("  spongebob!! "), "spongebob");
        assert_eq!(slugify_query(""), "funny");
    }

    #[test]
    fn extracts_embedded_results_array() {
        // mimics tenor's page: results array buried in a larger script blob,
        // with trailing json after it and a nested object inside
        let html = r#"<script>window.__X = {"foo":1,"results":[{"id":"7","title":"hi","media_formats":{"gif":{"url":"https://media.tenor.com/x.gif","dims":[1,2]}}}],"next":"abc"};</script>"#;
        let results = extract_results(html).expect("should find results array");
        let items = parse_tenor_page(&results, AssetType::Gif);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "tenor:7");
    }

    #[test]
    fn parses_tenor_shape() {
        // the extracted value is the results array itself
        let payload = json!([{
            "id": "16989471141791455574",
            "title": "",
            "content_description": "Confused Math GIF",
            "itemurl": "https://tenor.com/view/confused-math-gif-16989471141791455574",
            "media_formats": {
                "gif": { "url": "https://media.tenor.com/full.gif", "dims": [498, 280] },
                "tinymp4": { "url": "https://media.tenor.com/tiny.mp4", "dims": [220, 124], "duration": 2.5 },
                "tinygif": { "url": "https://media.tenor.com/tiny.gif", "dims": [220, 124] }
            }
        }]);
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
