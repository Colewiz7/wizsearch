//! GIPHY — GIFs and stickers, opt-in (disabled by default; the API program is
//! paid-oriented, but personal keys exist). https://developers.giphy.com

use async_trait::async_trait;
use serde_json::Value;

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "giphy",
    name: "GIPHY",
    homepage: "https://giphy.com",
    asset_types: &[AssetType::Gif, AssetType::Sticker],
    requires_key: true,
    key_help_url: "https://developers.giphy.com/dashboard/",
    key_hint: "GIPHY API key from their developer dashboard. Source is off by default; enable it above once the key is in.",
    allowed_hosts: &["giphy.com"],
    default_rate_limit_per_min: 40,
    default_enabled: false, // opt-in: user flips it on after adding their key
    default_timeout_ms: 8000,
    embedded_credential: "",
};

pub struct Giphy;

#[async_trait]
impl SearchSource for Giphy {
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

        let offset: u32 = req
            .cursor
            .as_deref()
            .map(|c| {
                c.parse()
                    .map_err(|_| SourceError::Parse("bad cursor".into()))
            })
            .transpose()?
            .unwrap_or(0);
        let limit = req.page_size.clamp(1, 50);

        let mut items = Vec::new();
        let mut got_full_page = false;
        for t in wanted {
            let endpoint = match t {
                AssetType::Gif => "gifs",
                AssetType::Sticker => "stickers",
                _ => continue,
            };
            let url = format!(
                "https://api.giphy.com/v1/{endpoint}/search?api_key={key}&q={}&limit={limit}&offset={offset}",
                urlencode(&req.query)
            );
            let resp = ctx.http().get(&url, &[]).await?.ok()?;
            let json = resp.json()?;
            let page_items = parse_giphy_page(&json, t);
            got_full_page = got_full_page || page_items.len() as u32 >= limit;
            items.extend(page_items);
        }

        Ok(SearchPage {
            items,
            next_cursor: got_full_page.then(|| (offset + limit).to_string()),
        })
    }
}

fn parse_giphy_page(json: &Value, asset_type: AssetType) -> Vec<ResultItem> {
    let Some(rows) = json["data"].as_array() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for row in rows {
        let Some(id) = row["id"].as_str() else {
            continue;
        };
        let title = row["title"]
            .as_str()
            .filter(|t| !t.is_empty())
            .unwrap_or("untitled")
            .to_string();
        let images = &row["images"];

        // preview: fixed_width mp4/webp loop; thumb: small still
        let preview = pick(images, &["fixed_width"], &["mp4", "webp", "url"])
            .or_else(|| pick(images, &["preview_gif"], &["url"]));
        let thumb = pick(
            images,
            &["fixed_width_small_still", "fixed_width_still"],
            &["url"],
        );
        // collect: the original gif
        let full = pick(images, &["original"], &["url", "mp4", "webp"]);
        let Some((full_url, _, _)) = full else {
            continue;
        };

        let (preview_url, width, height) = match preview {
            Some((u, w, h)) => (Some(u), w, h),
            None => (None, None, None),
        };
        let preview_kind = match preview_url.as_deref() {
            Some(u) if u.contains(".mp4") => PreviewKind::VideoLoop,
            _ => PreviewKind::AnimatedImage,
        };
        let ext = full_url
            .rsplit('/')
            .next()
            .and_then(|f| f.split('?').next())
            .and_then(|f| f.rsplit('.').next())
            .unwrap_or("gif")
            .to_string();

        out.push(ResultItem {
            id: format!("giphy:{id}"),
            source: DESCRIPTOR.id.to_string(),
            asset_type,
            title: title.clone(),
            thumbnail_url: thumb.map(|(u, _, _)| u),
            preview_stream_url: preview_url,
            preview_kind,
            duration_ms: None,
            width,
            height,
            license: Some("GIPHY content (check originating rights)".to_string()),
            attribution: row["url"].as_str().map(String::from),
            origin_url: row["url"].as_str().map(String::from),
            fetch_plan: FetchPlan::HttpGet {
                url: full_url,
                headers: vec![],
                filename_hint: format!("{}.{ext}", safe_slug(&title)),
            },
        });
    }
    out
}

/// images.{rendition}.{field} in preference order; "url" is the gif itself
fn pick(
    images: &Value,
    renditions: &[&str],
    fields: &[&str],
) -> Option<(String, Option<u32>, Option<u32>)> {
    for rend in renditions {
        let r = &images[rend];
        for field in fields {
            if let Some(url) = r[field].as_str().filter(|u| !u.is_empty()) {
                let dim = |k: &str| r[k].as_str().and_then(|v| v.parse::<u32>().ok());
                return Some((url.to_string(), dim("width"), dim("height")));
            }
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
        "giphy".to_string()
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
    fn parses_giphy_shape() {
        let payload = json!({
            "data": [{
                "id": "xT9IgG50Fb7Mi0prBC",
                "title": "excited season 4 GIF",
                "url": "https://giphy.com/gifs/excited-xT9IgG50Fb7Mi0prBC",
                "images": {
                    "original": { "url": "https://media2.giphy.com/media/x/giphy.gif", "width": "480", "height": "270" },
                    "fixed_width": { "url": "https://media2.giphy.com/media/x/200w.gif", "mp4": "https://media2.giphy.com/media/x/200w.mp4", "width": "200", "height": "113" },
                    "fixed_width_small_still": { "url": "https://media2.giphy.com/media/x/100w_s.gif", "width": "100", "height": "57" }
                }
            }]
        });
        let items = parse_giphy_page(&payload, AssetType::Gif);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(
            it.preview_stream_url.as_deref(),
            Some("https://media2.giphy.com/media/x/200w.mp4")
        );
        assert!(matches!(it.preview_kind, PreviewKind::VideoLoop));
        match &it.fetch_plan {
            FetchPlan::HttpGet {
                url, filename_hint, ..
            } => {
                assert_eq!(url, "https://media2.giphy.com/media/x/giphy.gif");
                assert_eq!(filename_hint, "excited-season-4-gif.gif");
            }
            _ => panic!("expected HttpGet"),
        }
    }
}
