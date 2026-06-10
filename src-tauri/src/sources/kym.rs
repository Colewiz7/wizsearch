//! Know Your Meme — scraped image search, no API. Mostly meme templates and
//! reaction images from i.kym-cdn.com. Cloudflare sometimes blocks scrapes;
//! that surfaces as a source error chip, never a broken grid.

use async_trait::async_trait;
use scraper::{Html, Selector};

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "kym",
    name: "Know Your Meme",
    homepage: "https://knowyourmeme.com",
    asset_types: &[AssetType::Image, AssetType::Gif],
    requires_key: false,
    key_help_url: "",
    key_hint: "",
    allowed_hosts: &["knowyourmeme.com", "kym-cdn.com"],
    default_rate_limit_per_min: 15,
    default_enabled: true,
    default_timeout_ms: 10000,
    embedded_credential: "",
};

pub struct Kym;

#[async_trait]
impl SearchSource for Kym {
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
            "https://knowyourmeme.com/search?context=images&q={}&page={page}",
            urlencode(&req.query)
        );
        let resp = ctx
            .http()
            .get(&url, &[("Accept", "text/html")])
            .await?
            .ok()?;
        let html = resp.text()?;

        let items = parse_image_search(&html, &wanted);
        let next_cursor = (!items.is_empty()).then(|| (page + 1).to_string());
        Ok(SearchPage { items, next_cursor })
    }
}

fn parse_image_search(html: &str, wanted: &[AssetType]) -> Vec<ResultItem> {
    let doc = Html::parse_document(html);
    // result tiles: <a class="photo" href="/photos/123-slug"><img src=".../masonry/..." ...>
    let link_sel = Selector::parse("a.photo").expect("static selector");
    let img_sel = Selector::parse("img").expect("static selector");

    let mut out = Vec::new();
    for link in doc.select(&link_sel) {
        let href = link.value().attr("href").unwrap_or_default();
        let Some(img) = link.select(&img_sel).next() else {
            continue;
        };
        let Some(src) = img
            .value()
            .attr("data-src")
            .or_else(|| img.value().attr("src"))
        else {
            continue;
        };
        if !src.contains("kym-cdn.com") {
            continue;
        }
        let title = img
            .value()
            .attr("alt")
            .or_else(|| img.value().attr("title"))
            .unwrap_or("meme image")
            .trim()
            .to_string();

        // masonry/newsfeed renditions swap to the original full-size file
        let full = src
            .replace("/masonry/", "/original/")
            .replace("/newsfeed/", "/original/");
        let ext = ext_of(&full, "jpg");
        let is_gif = ext.eq_ignore_ascii_case("gif");
        let asset_type = if is_gif {
            AssetType::Gif
        } else {
            AssetType::Image
        };
        if !wanted.contains(&asset_type) {
            continue;
        }

        let origin = if href.starts_with("http") {
            href.to_string()
        } else {
            format!("https://knowyourmeme.com{href}")
        };

        out.push(ResultItem {
            id: format!("kym:{}", href.trim_matches('/')),
            source: DESCRIPTOR.id.to_string(),
            asset_type,
            title: title.clone(),
            thumbnail_url: Some(src.to_string()),
            preview_stream_url: is_gif.then(|| full.clone()),
            preview_kind: if is_gif {
                PreviewKind::AnimatedImage
            } else {
                PreviewKind::PosterLoop
            },
            duration_ms: None,
            width: None,
            height: None,
            license: Some("KYM-hosted content (third-party rights)".to_string()),
            attribution: Some(origin.clone()),
            origin_url: Some(origin),
            fetch_plan: FetchPlan::HttpGet {
                url: full,
                headers: vec![],
                filename_hint: format!("{}.{ext}", safe_slug(&title)),
            },
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
        "kym".to_string()
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

    #[test]
    fn parses_photo_grid() {
        let html = r#"
        <html><body>
          <a class="photo" href="/photos/123-stonks">
            <img src="https://i.kym-cdn.com/photos/images/masonry/000/123/456/stonks.jpg" alt="Stonks" />
          </a>
          <a class="photo" href="/photos/789-dancing">
            <img data-src="https://i.kym-cdn.com/photos/images/masonry/000/789/000/dance.gif" alt="Dancing" />
          </a>
        </body></html>"#;
        let items = parse_image_search(html, &AssetType::ALL);
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].asset_type, AssetType::Image));
        match &items[0].fetch_plan {
            FetchPlan::HttpGet { url, .. } => {
                assert_eq!(
                    url,
                    "https://i.kym-cdn.com/photos/images/original/000/123/456/stonks.jpg"
                )
            }
            _ => panic!("expected HttpGet"),
        }
        assert!(matches!(items[1].asset_type, AssetType::Gif));
        assert!(matches!(items[1].preview_kind, PreviewKind::AnimatedImage));
    }
}
