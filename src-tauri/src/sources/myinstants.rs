//! myinstants.com — scraped, no API. Audio (sound buttons) only.
//! License is unclear (mostly clips of third-party media); we mark it as such.

use async_trait::async_trait;
use scraper::{Html, Selector};

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

const BASE: &str = "https://www.myinstants.com";

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "myinstants",
    name: "Myinstants",
    homepage: "https://www.myinstants.com",
    asset_types: &[AssetType::Audio],
    requires_key: false,
    key_help_url: "",
    allowed_hosts: &["myinstants.com"],
    default_rate_limit_per_min: 20,
    embedded_credential: "",
};

pub struct MyInstants;

#[async_trait]
impl SearchSource for MyInstants {
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

        let page: u32 = req
            .cursor
            .as_deref()
            .map(|c| {
                c.parse()
                    .map_err(|_| SourceError::Parse("bad cursor".into()))
            })
            .transpose()?
            .unwrap_or(1);

        let url = format!("{BASE}/search/?name={}&page={page}", urlencode(&req.query));
        let resp = ctx
            .http()
            .get(&url, &[("Accept", "text/html")])
            .await?
            .ok()?;
        let html = resp.text()?;

        // parse synchronously after the last await; scraper's DOM is !Send so it
        // must never live across an await point
        let items = parse_search_page(&html);
        let next_cursor = if items.is_empty() {
            None
        } else {
            Some((page + 1).to_string())
        };

        Ok(SearchPage { items, next_cursor })
    }
}

fn parse_search_page(html: &str) -> Vec<ResultItem> {
    let doc = Html::parse_document(html);
    // each result: <div class="instant"><button class="small-button"
    //   onclick="play('/media/sounds/x.mp3', ...)"></button>
    //   <a class="instant-link" href="/en/instant/...">Title</a></div>
    let instant_sel = Selector::parse("div.instant").expect("static selector");
    let link_sel = Selector::parse("a.instant-link").expect("static selector");
    let btn_sel = Selector::parse("button").expect("static selector");

    let mut out = Vec::new();
    for inst in doc.select(&instant_sel) {
        let Some(link) = inst.select(&link_sel).next() else {
            continue;
        };
        let title = link.text().collect::<String>().trim().to_string();
        let origin = link.value().attr("href").map(|h| format!("{BASE}{h}"));

        let mut mp3: Option<String> = None;
        for btn in inst.select(&btn_sel) {
            let onclick = btn
                .value()
                .attr("onclick")
                .or_else(|| btn.value().attr("onmousedown"))
                .unwrap_or("");
            if let Some(path) = extract_play_path(onclick) {
                mp3 = Some(if path.starts_with("http") {
                    path
                } else {
                    format!("{BASE}{path}")
                });
                break;
            }
        }
        let Some(mp3) = mp3 else { continue };
        if title.is_empty() {
            continue;
        }

        let filename_hint = mp3.rsplit('/').next().unwrap_or("sound.mp3").to_string();

        out.push(ResultItem {
            id: format!("myinstants:{filename_hint}"),
            source: DESCRIPTOR.id.to_string(),
            asset_type: AssetType::Audio,
            title,
            thumbnail_url: None,
            preview_stream_url: Some(mp3.clone()),
            preview_kind: PreviewKind::AudioStream,
            duration_ms: None,
            width: None,
            height: None,
            license: Some("unknown (user-uploaded, often third-party)".to_string()),
            attribution: origin.clone(),
            origin_url: origin,
            fetch_plan: FetchPlan::HttpGet {
                url: mp3,
                headers: vec![],
                filename_hint,
            },
        });
    }
    out
}

/// pull the first quoted path out of `play('/media/sounds/x.mp3', ...)`
fn extract_play_path(onclick: &str) -> Option<String> {
    let start = onclick.find("play(")?;
    let rest = &onclick[start + 5..];
    let quote = rest.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let rest = &rest[1..];
    let end = rest.find(quote)?;
    let path = &rest[..end];
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// minimal query-string escape; sources can't import host url crates, and this
/// covers everything a search query needs
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
    fn extracts_play_path() {
        assert_eq!(
            extract_play_path("play('/media/sounds/vine-boom.mp3', this, false)"),
            Some("/media/sounds/vine-boom.mp3".to_string())
        );
        assert_eq!(extract_play_path("nothing here"), None);
    }

    #[test]
    fn parses_instant_markup() {
        let html = r#"
        <html><body>
          <div class="instant">
            <button class="small-button" onclick="play('/media/sounds/boom.mp3', this)"></button>
            <a class="instant-link" href="/en/instant/vine-boom/">Vine Boom</a>
          </div>
        </body></html>"#;
        let items = parse_search_page(html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Vine Boom");
        assert_eq!(
            items[0].preview_stream_url.as_deref(),
            Some("https://www.myinstants.com/media/sounds/boom.mp3")
        );
        assert!(matches!(items[0].preview_kind, PreviewKind::AudioStream));
    }
}
