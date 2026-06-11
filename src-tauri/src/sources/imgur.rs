//! Imgur — scraped, no key. The v3 API needs a Client-ID, but imgur.com's own
//! search page renders gallery tiles server-side, so we scrape those instead.
//! Full image = the thumbnail with its size suffix dropped (imgur's documented
//! thumb scheme: `<id>b.jpg` -> `<id>.jpg`).

use async_trait::async_trait;
use scraper::{Html, Selector};

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "imgur",
    name: "Imgur",
    homepage: "https://imgur.com",
    asset_types: &[AssetType::Image],
    requires_key: false,
    key_help_url: "",
    key_hint: "",
    allowed_hosts: &["imgur.com"],
    default_rate_limit_per_min: 30,
    default_enabled: true,
    default_timeout_ms: 10000,
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
        if req.wanted(DESCRIPTOR.asset_types).is_empty() {
            return Ok(SearchPage::empty());
        }
        // single page; the search page is infinite-scroll with no cheap cursor
        if req.cursor.is_some() {
            return Ok(SearchPage::empty());
        }

        let url = format!("https://imgur.com/search?q={}", urlencode(&req.query));
        let resp = ctx
            .http()
            .get(&url, &[("Accept", "text/html")])
            .await?
            .ok()?;
        let html = resp.text()?;
        Ok(SearchPage {
            items: parse_search(&html),
            next_cursor: None,
        })
    }
}

fn parse_search(html: &str) -> Vec<ResultItem> {
    let doc = Html::parse_document(html);
    // <a class="image-list-link" href="/gallery/HASH"><img alt="title"
    //   src="//i.imgur.com/HASHb.jpg"></a>
    let link_sel = Selector::parse("a.image-list-link").expect("static selector");
    let img_sel = Selector::parse("img").expect("static selector");

    let mut out = Vec::new();
    for link in doc.select(&link_sel) {
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let hash = href.trim_start_matches("/gallery/").trim_matches('/');
        if hash.is_empty() {
            continue;
        }
        let Some(img) = link.select(&img_sel).next() else {
            continue;
        };
        let Some(raw_src) = img
            .value()
            .attr("src")
            .or_else(|| img.value().attr("data-src"))
        else {
            continue;
        };
        if !raw_src.contains("i.imgur.com") {
            continue;
        }
        // imgur serves protocol-relative urls
        let thumb = if raw_src.starts_with("//") {
            format!("https:{raw_src}")
        } else {
            raw_src.to_string()
        };
        let full = full_from_thumb(&thumb);
        let title = img
            .value()
            .attr("alt")
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or("imgur image")
            .to_string();
        let ext = full
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 4)
            .unwrap_or("jpg")
            .to_string();

        out.push(ResultItem {
            id: format!("imgur:{hash}"),
            source: DESCRIPTOR.id.to_string(),
            asset_type: AssetType::Image,
            title: title.clone(),
            thumbnail_url: Some(thumb),
            preview_stream_url: None,
            preview_kind: PreviewKind::PosterLoop,
            duration_ms: None,
            width: None,
            height: None,
            license: Some("Imgur user content (third-party rights)".to_string()),
            attribution: Some(format!("https://imgur.com/gallery/{hash}")),
            origin_url: Some(format!("https://imgur.com/gallery/{hash}")),
            fetch_plan: FetchPlan::HttpGet {
                url: full,
                headers: vec![],
                filename_hint: format!("{}.{ext}", safe_slug(&title)),
            },
        });
    }
    out
}

/// `https://i.imgur.com/UXCBWlWb.jpg` -> `https://i.imgur.com/UXCBWlW.jpg`.
/// Imgur thumbnails are the base id + one size-suffix letter; base ids are 5 or
/// 7 chars, so a 6- or 8-char stem has a suffix to drop.
fn full_from_thumb(thumb: &str) -> String {
    let Some((base, ext)) = thumb.rsplit_once('.') else {
        return thumb.to_string();
    };
    let Some((prefix, stem)) = base.rsplit_once('/') else {
        return thumb.to_string();
    };
    if stem.len() == 6 || stem.len() == 8 {
        format!("{prefix}/{}.{ext}", &stem[..stem.len() - 1])
    } else {
        thumb.to_string()
    }
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

    #[test]
    fn full_image_drops_thumb_suffix() {
        assert_eq!(
            full_from_thumb("https://i.imgur.com/UXCBWlWb.jpg"),
            "https://i.imgur.com/UXCBWlW.jpg"
        );
        assert_eq!(
            full_from_thumb("https://i.imgur.com/CJ3iTb.jpg"),
            "https://i.imgur.com/CJ3iT.jpg"
        );
    }

    #[test]
    fn parses_search_tiles() {
        let html = r#"
        <html><body>
          <a class="image-list-link" href="/gallery/UXCBWlW" data-page="0">
            <img alt="Introducing the carpet shark" src="//i.imgur.com/UXCBWlWb.jpg" />
          </a>
          <a class="image-list-link" href="/gallery/CJ3iT">
            <img alt="" src="//i.imgur.com/CJ3iTb.jpg" />
          </a>
          <a class="image-list-link" href="/gallery/skip"><img src="//example.com/x.jpg" /></a>
        </body></html>"#;
        let items = parse_search(html);
        assert_eq!(items.len(), 2); // non-imgur img skipped
        assert_eq!(items[0].title, "Introducing the carpet shark");
        assert_eq!(
            items[0].thumbnail_url.as_deref(),
            Some("https://i.imgur.com/UXCBWlWb.jpg")
        );
        match &items[0].fetch_plan {
            FetchPlan::HttpGet { url, .. } => {
                assert_eq!(url, "https://i.imgur.com/UXCBWlW.jpg")
            }
            _ => panic!("expected HttpGet"),
        }
    }
}
