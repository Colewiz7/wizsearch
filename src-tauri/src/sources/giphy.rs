//! GIPHY — scraped, no key. The API needs a per-user key, but giphy.com's
//! search page embeds `gifs/<slug>-<id>` links, and GIPHY's media URLs are
//! derivable from the id (`media.giphy.com/media/<id>/giphy.gif` etc.). So we
//! scrape the ids and build the media urls. Biggest keyless GIF library.

use async_trait::async_trait;

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "giphy",
    name: "GIPHY",
    homepage: "https://giphy.com",
    asset_types: &[AssetType::Gif],
    requires_key: false,
    key_help_url: "",
    key_hint: "",
    allowed_hosts: &["giphy.com"],
    default_rate_limit_per_min: 30,
    default_enabled: true,
    default_timeout_ms: 10000,
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
        if req.wanted(DESCRIPTOR.asset_types).is_empty() {
            return Ok(SearchPage::empty());
        }
        // single page; the search page is infinite-scroll with no cheap cursor
        if req.cursor.is_some() {
            return Ok(SearchPage::empty());
        }

        let url = format!("https://giphy.com/search/{}", slugify_query(&req.query));
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

/// "deal with it" -> "deal-with-it" for the /search/<slug> path
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

fn parse_search(html: &str) -> Vec<ResultItem> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    // scan for every `gifs/<slug>-<id>` occurrence (href or embedded json)
    for (slug, id) in scan_gif_links(html) {
        if !seen.insert(id.clone()) {
            continue;
        }
        let title = if slug.is_empty() {
            "giphy gif".to_string()
        } else {
            slug.replace('-', " ")
        };
        out.push(ResultItem {
            id: format!("giphy:{id}"),
            source: DESCRIPTOR.id.to_string(),
            asset_type: AssetType::Gif,
            title: title.clone(),
            thumbnail_url: Some(format!("https://media.giphy.com/media/{id}/200w.webp")),
            // hover loops the mp4 (smoother on Linux than a raw gif)
            preview_stream_url: Some(format!("https://media.giphy.com/media/{id}/giphy.mp4")),
            preview_kind: PreviewKind::VideoLoop,
            duration_ms: None,
            width: None,
            height: None,
            license: Some("GIPHY content (check originating rights)".to_string()),
            attribution: Some(format!("https://giphy.com/gifs/{id}")),
            origin_url: Some(format!("https://giphy.com/gifs/{id}")),
            fetch_plan: FetchPlan::HttpGet {
                url: format!("https://media.giphy.com/media/{id}/giphy.gif"),
                headers: vec![],
                filename_hint: format!("{}.gif", safe_slug(&title)),
            },
        });
    }
    out
}

/// pull (slug, id) pairs out of every `gifs/<slug>-<id>` path in the page. The
/// id is the trailing hyphen segment (>=13 mixed alphanumerics); the slug is the
/// words before it.
fn scan_gif_links(html: &str) -> Vec<(String, String)> {
    let needle = "gifs/";
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(pos) = html[i..].find(needle) {
        let start = i + pos + needle.len();
        let mut end = start;
        while end < bytes.len() {
            let c = bytes[end];
            if c.is_ascii_alphanumeric() || c == b'-' {
                end += 1;
            } else {
                break;
            }
        }
        i = end.max(start + 1);
        let path = &html[start..end];
        if let Some((slug, id)) = split_slug_id(path) {
            out.push((slug, id));
        }
    }
    out
}

/// "ai-cat-funny-CbM0J4DEIqKHsKxYHA" -> ("ai-cat-funny", "CbM0J4DEIqKHsKxYHA")
fn split_slug_id(path: &str) -> Option<(String, String)> {
    let id = path.rsplit('-').next()?;
    // giphy ids are long mixed-case/alnum; require length + at least one
    // uppercase-or-digit so a lowercase slug word is never mistaken for an id
    let looks_like_id = id.len() >= 13
        && id
            .chars()
            .any(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && id.chars().all(|c| c.is_ascii_alphanumeric());
    if !looks_like_id {
        return None;
    }
    let slug = path[..path.len() - id.len()]
        .trim_end_matches('-')
        .to_string();
    Some((slug, id.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_slug_and_id() {
        assert_eq!(
            split_slug_id("ai-cat-funny-meme-boxing-cbm0J4DEIqKHsKxYHA"),
            Some((
                "ai-cat-funny-meme-boxing".to_string(),
                "cbm0J4DEIqKHsKxYHA".to_string()
            ))
        );
        // a plain lowercase word is not an id
        assert_eq!(split_slug_id("justaword"), None);
    }

    #[test]
    fn scrapes_ids_and_builds_media_urls() {
        let html = r#"
        <a href="/gifs/ai-cat-funny-meme-boxing-cbm0J4DEIqKHsKxYHA"></a>
        {"url":"https://giphy.com/gifs/cat-cats-meowtakeover-yWku98eNsMSZOEEWnC"}
        <a href="/gifs/ai-cat-funny-meme-boxing-cbm0J4DEIqKHsKxYHA"></a>
        "#;
        let items = parse_search(html);
        assert_eq!(items.len(), 2); // deduped the repeat
        assert_eq!(items[0].title, "ai cat funny meme boxing");
        match &items[0].fetch_plan {
            FetchPlan::HttpGet { url, .. } => assert_eq!(
                url,
                "https://media.giphy.com/media/cbm0J4DEIqKHsKxYHA/giphy.gif"
            ),
            _ => panic!("expected HttpGet"),
        }
        assert_eq!(
            items[0].preview_stream_url.as_deref(),
            Some("https://media.giphy.com/media/cbm0J4DEIqKHsKxYHA/giphy.mp4")
        );
    }
}
