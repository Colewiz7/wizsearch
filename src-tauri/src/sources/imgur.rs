//! Imgur — gallery search via the v3 API. Free per-user Client-ID (register an
//! app, pick "anonymous usage"). https://api.imgur.com/oauth2/addclient

use async_trait::async_trait;
use serde_json::Value;

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "imgur",
    name: "Imgur",
    homepage: "https://imgur.com",
    asset_types: &[AssetType::Gif, AssetType::Image, AssetType::Video],
    requires_key: true,
    key_help_url: "https://api.imgur.com/oauth2/addclient",
    key_hint: "",
    allowed_hosts: &["imgur.com"],
    default_rate_limit_per_min: 30,
    default_enabled: true,
    default_timeout_ms: 8000,
    embedded_credential: "",
};

pub struct Imgur;

#[async_trait]
impl SearchSource for Imgur {
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
            .unwrap_or(0);

        let url = format!(
            "https://api.imgur.com/3/gallery/search/viral/all/{page}?q={}",
            urlencode(&req.query)
        );
        let auth = format!("Client-ID {key}");
        let resp = ctx
            .http()
            .get(&url, &[("Authorization", &auth)])
            .await?
            .ok()?;
        let json = resp.json()?;
        let items = parse_gallery(&json, &wanted, req.page_size as usize);
        let next_cursor = (!items.is_empty()).then(|| (page + 1).to_string());

        Ok(SearchPage { items, next_cursor })
    }
}

fn parse_gallery(json: &Value, wanted: &[AssetType], cap: usize) -> Vec<ResultItem> {
    let Some(rows) = json["data"].as_array() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for row in rows {
        if out.len() >= cap {
            break;
        }
        if row["nsfw"].as_bool().unwrap_or(false) {
            continue;
        }
        // albums carry their media in images[0]; plain posts are the image
        let media = if row["is_album"].as_bool().unwrap_or(false) {
            &row["images"][0]
        } else {
            row
        };
        let Some(link) = media["link"].as_str() else {
            continue;
        };
        let Some(id) = media["id"].as_str().or(row["id"].as_str()) else {
            continue;
        };
        let title = row["title"]
            .as_str()
            .or_else(|| media["description"].as_str())
            .unwrap_or("untitled")
            .to_string();
        let mime = media["type"].as_str().unwrap_or("");
        let animated = media["animated"].as_bool().unwrap_or(false);
        let mp4 = media["mp4"].as_str().map(String::from);
        let width = media["width"].as_u64().map(|v| v as u32);
        let height = media["height"].as_u64().map(|v| v as u32);

        let (asset_type, preview_url, preview_kind, full_url, ext) = if mime == "video/mp4" {
            let Some(mp4) = mp4.clone() else { continue };
            (
                AssetType::Video,
                Some(mp4.clone()),
                PreviewKind::VideoLoop,
                mp4,
                "mp4",
            )
        } else if animated {
            // animated gif: hover the mp4, collect the gif
            (
                AssetType::Gif,
                mp4.clone(),
                PreviewKind::VideoLoop,
                link.to_string(),
                "gif",
            )
        } else if mime.starts_with("image/") {
            (
                AssetType::Image,
                None,
                PreviewKind::PosterLoop,
                link.to_string(),
                ext_of(link, "jpg"),
            )
        } else {
            continue;
        };
        if !wanted.contains(&asset_type) {
            continue;
        }

        // imgur thumb trick: <id>m.jpg is a 320px thumbnail
        let thumb = Some(format!("https://i.imgur.com/{id}m.jpg"));
        let origin = format!("https://imgur.com/{id}");

        out.push(ResultItem {
            id: format!("imgur:{id}"),
            source: DESCRIPTOR.id.to_string(),
            asset_type,
            title: title.clone(),
            thumbnail_url: thumb,
            preview_stream_url: preview_url,
            preview_kind,
            duration_ms: None,
            width,
            height,
            license: Some("Imgur user content (third-party rights)".to_string()),
            attribution: Some(origin.clone()),
            origin_url: Some(origin),
            fetch_plan: FetchPlan::HttpGet {
                url: full_url,
                headers: vec![],
                filename_hint: format!("{}.{ext}", safe_slug(&title)),
            },
        });
    }
    out
}

fn ext_of<'a>(url: &'a str, fallback: &'a str) -> &'a str {
    url.split('?')
        .next()
        .and_then(|u| u.rsplit('.').next())
        .filter(|e| e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or(fallback)
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
        "imgur".to_string()
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
    fn parses_gallery_albums_and_images() {
        let payload = json!({ "data": [
            {
                "id": "alb1", "title": "perfect reaction", "is_album": true, "nsfw": false,
                "images": [{
                    "id": "img1", "type": "image/gif", "animated": true,
                    "link": "https://i.imgur.com/img1.gif", "mp4": "https://i.imgur.com/img1.mp4",
                    "width": 400, "height": 300
                }]
            },
            {
                "id": "vid1", "title": "meme clip", "is_album": false, "nsfw": false,
                "type": "video/mp4", "animated": true,
                "link": "https://i.imgur.com/vid1.mp4", "mp4": "https://i.imgur.com/vid1.mp4",
                "width": 720, "height": 720
            },
            { "id": "bad1", "title": "nope", "nsfw": true, "is_album": false,
              "type": "image/gif", "animated": true, "link": "https://i.imgur.com/bad1.gif" }
        ]});
        let items = parse_gallery(&payload, &AssetType::ALL, 10);
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].asset_type, AssetType::Gif));
        match &items[0].fetch_plan {
            FetchPlan::HttpGet { url, .. } => assert_eq!(url, "https://i.imgur.com/img1.gif"),
            _ => panic!("expected HttpGet"),
        }
        assert_eq!(
            items[0].preview_stream_url.as_deref(),
            Some("https://i.imgur.com/img1.mp4")
        );
        assert!(matches!(items[1].asset_type, AssetType::Video));
    }
}
