//! KLIPY — GIFs, stickers, clips, memes. Free per-user API key, passed in the
//! URL path (their design). Tenor is dead and GIPHY is paid, so this is the
//! primary GIF/clip source. https://klipy.com/developers

use async_trait::async_trait;
use serde_json::Value;

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "klipy",
    name: "KLIPY",
    homepage: "https://klipy.com",
    asset_types: &[AssetType::Gif, AssetType::Sticker, AssetType::Video],
    requires_key: true,
    key_help_url: "https://klipy.com/developers",
    allowed_hosts: &["klipy.com", "klipy.co", "klipy.media"],
    default_rate_limit_per_min: 60,
    default_enabled: true,
    default_timeout_ms: 8000,
    embedded_credential: "",
};

pub struct Klipy;

fn endpoint_for(t: AssetType) -> Option<&'static str> {
    match t {
        AssetType::Gif => Some("gifs"),
        AssetType::Sticker => Some("stickers"),
        AssetType::Video => Some("clips"),
        _ => None,
    }
}

#[async_trait]
impl SearchSource for Klipy {
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

        let page: u32 = req
            .cursor
            .as_deref()
            .map(|c| {
                c.parse()
                    .map_err(|_| SourceError::Parse("bad cursor".into()))
            })
            .transpose()?
            .unwrap_or(1);
        // split the page budget across the asset-type endpoints we hit
        let per = (req.page_size / wanted.len() as u32).max(4);

        let mut items = Vec::new();
        let mut any_next = false;
        for t in wanted {
            let Some(ep) = endpoint_for(t) else { continue };
            let url = format!(
                "https://api.klipy.com/api/v1/{key}/{ep}/search?q={}&page={page}&per_page={per}&customer_id=wizsearch",
                urlencode(&req.query)
            );
            let resp = ctx.http().get(&url, &[]).await?.ok()?;
            let json = resp.json()?;
            let (mut page_items, has_next) = parse_klipy_page(&json, t);
            items.append(&mut page_items);
            any_next = any_next || has_next;
        }

        Ok(SearchPage {
            items,
            next_cursor: any_next.then(|| (page + 1).to_string()),
        })
    }
}

/// defensive parse of KLIPY's { data: { data: [...], has_next } } shape;
/// tolerates missing fields rather than failing the whole page
fn parse_klipy_page(json: &Value, asset_type: AssetType) -> (Vec<ResultItem>, bool) {
    let data = &json["data"];
    let has_next = data["has_next"].as_bool().unwrap_or(false);
    let Some(rows) = data["data"].as_array() else {
        return (Vec::new(), false);
    };

    let mut out = Vec::new();
    for row in rows {
        let id = row["id"]
            .as_i64()
            .map(|n| n.to_string())
            .or_else(|| row["id"].as_str().map(String::from))
            .or_else(|| row["slug"].as_str().map(String::from))
            .unwrap_or_default();
        let title = row["title"]
            .as_str()
            .or_else(|| row["slug"].as_str())
            .unwrap_or("untitled")
            .to_string();
        let files = &row["file"];

        // preview: mp4/webp beat raw gif (Linux playback + bandwidth)
        let preview = pick(files, &["md", "sm", "hd"], &["mp4", "webp", "gif"]);
        let thumb = pick(files, &["xs", "sm"], &["webp", "gif", "jpg", "png"]);
        // collect target: best quality, gif for gif/sticker, mp4 for clips
        let full_formats: &[&str] = match asset_type {
            AssetType::Video => &["mp4", "gif", "webp"],
            _ => &["gif", "mp4", "webp"],
        };
        let full = pick(files, &["hd", "md", "sm"], full_formats);

        let Some((full_url, _, _)) = full else {
            continue;
        };
        let (preview_url, width, height) = match preview {
            Some((u, w, h)) => (Some(u), w, h),
            None => (None, None, None),
        };
        let preview_kind = match preview_url.as_deref() {
            Some(u) if u.ends_with(".mp4") => PreviewKind::VideoLoop,
            _ => PreviewKind::AnimatedImage,
        };

        let ext = full_url.rsplit('.').next().unwrap_or("gif").to_string();
        out.push(ResultItem {
            id: format!("klipy:{}:{id}", asset_type.as_str()),
            source: DESCRIPTOR.id.to_string(),
            asset_type,
            title: title.clone(),
            thumbnail_url: thumb.map(|(u, _, _)| u),
            preview_stream_url: preview_url,
            preview_kind,
            duration_ms: None,
            width,
            height,
            license: Some("KLIPY content (check originating rights)".to_string()),
            attribution: row["url"].as_str().map(String::from),
            origin_url: row["url"].as_str().map(String::from),
            fetch_plan: FetchPlan::HttpGet {
                url: full_url,
                headers: vec![],
                filename_hint: format!("{}.{ext}", safe_slug(&title)),
            },
        });
    }
    (out, has_next)
}

/// walk file.{size}.{format}.url in preference order
fn pick(
    files: &Value,
    sizes: &[&str],
    formats: &[&str],
) -> Option<(String, Option<u32>, Option<u32>)> {
    for size in sizes {
        for fmt in formats {
            let f = &files[size][fmt];
            if let Some(url) = f["url"].as_str() {
                let w = f["width"].as_u64().map(|v| v as u32);
                let h = f["height"].as_u64().map(|v| v as u32);
                return Some((url.to_string(), w, h));
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
    let trimmed = slug.trim_matches('-');
    let mut out = String::new();
    let mut prev_dash = false;
    for c in trimmed.chars().take(48) {
        if c == '-' {
            if !prev_dash {
                out.push(c);
            }
            prev_dash = true;
        } else {
            out.push(c);
            prev_dash = false;
        }
    }
    if out.is_empty() {
        "asset".to_string()
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
    fn parses_klipy_shape() {
        let payload = json!({
            "result": true,
            "data": {
                "data": [{
                    "id": 42,
                    "title": "deal with it",
                    "url": "https://klipy.com/gifs/deal-with-it",
                    "file": {
                        "hd": { "gif": { "url": "https://static.klipy.media/hd.gif", "width": 480, "height": 270 } },
                        "md": { "mp4": { "url": "https://static.klipy.media/md.mp4", "width": 320, "height": 180 } },
                        "xs": { "webp": { "url": "https://static.klipy.media/xs.webp", "width": 100, "height": 56 } }
                    }
                }],
                "has_next": true
            }
        });
        let (items, has_next) = parse_klipy_page(&payload, AssetType::Gif);
        assert!(has_next);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(
            it.preview_stream_url.as_deref(),
            Some("https://static.klipy.media/md.mp4")
        );
        assert!(matches!(it.preview_kind, PreviewKind::VideoLoop));
        match &it.fetch_plan {
            FetchPlan::HttpGet {
                url, filename_hint, ..
            } => {
                assert_eq!(url, "https://static.klipy.media/hd.gif");
                assert_eq!(filename_hint, "deal-with-it.gif");
            }
            _ => panic!("expected HttpGet"),
        }
    }

    #[test]
    fn slug_is_filesystem_safe() {
        assert_eq!(safe_slug("Deal With It!! 😎"), "deal-with-it");
    }
}
