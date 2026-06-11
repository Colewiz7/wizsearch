//! Imgflip — classic meme templates, no key. `api.imgflip.com/get_memes`
//! returns the 100 trending templates with no auth; we filter them by the
//! query client-side. Great for the "I need the Drake / Distracted Boyfriend
//! template" case.

use async_trait::async_trait;
use serde_json::Value;

use super::{
    AssetType, FetchPlan, PreviewKind, ResultItem, SearchPage, SearchRequest, SearchSource,
    SourceContext, SourceDescriptor, SourceError,
};

static DESCRIPTOR: SourceDescriptor = SourceDescriptor {
    id: "imgflip",
    name: "Imgflip",
    homepage: "https://imgflip.com",
    asset_types: &[AssetType::Image],
    requires_key: false,
    key_help_url: "",
    key_hint: "",
    allowed_hosts: &["imgflip.com"],
    default_rate_limit_per_min: 30,
    default_enabled: true,
    default_timeout_ms: 8000,
    embedded_credential: "",
};

pub struct Imgflip;

#[async_trait]
impl SearchSource for Imgflip {
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
        // the template list is fixed (100 items), so there's nothing to page
        if req.cursor.is_some() {
            return Ok(SearchPage::empty());
        }

        let resp = ctx
            .http()
            .get("https://api.imgflip.com/get_memes", &[])
            .await?
            .ok()?;
        let json = resp.json()?;
        Ok(SearchPage {
            items: parse_and_filter(&json, &req.query),
            next_cursor: None,
        })
    }
}

fn parse_and_filter(json: &Value, query: &str) -> Vec<ResultItem> {
    let Some(memes) = json["data"]["memes"].as_array() else {
        return Vec::new();
    };
    let q = query.trim().to_lowercase();
    let terms: Vec<&str> = q.split_whitespace().collect();

    let mut out = Vec::new();
    for m in memes {
        let Some(id) = m["id"].as_str() else { continue };
        let name = m["name"].as_str().unwrap_or("meme template");
        // empty query shows all trending; otherwise keep templates whose name
        // contains every term
        let name_lc = name.to_lowercase();
        if !terms.is_empty() && !terms.iter().all(|t| name_lc.contains(t)) {
            continue;
        }
        let Some(url) = m["url"].as_str() else {
            continue;
        };
        let width = m["width"].as_u64().map(|v| v as u32);
        let height = m["height"].as_u64().map(|v| v as u32);
        let ext = url
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 4)
            .unwrap_or("jpg")
            .to_string();

        out.push(ResultItem {
            id: format!("imgflip:{id}"),
            source: DESCRIPTOR.id.to_string(),
            asset_type: AssetType::Image,
            title: name.to_string(),
            thumbnail_url: Some(url.to_string()),
            preview_stream_url: None,
            preview_kind: PreviewKind::PosterLoop,
            duration_ms: None,
            width,
            height,
            license: Some("Imgflip meme template".to_string()),
            attribution: Some(format!("https://imgflip.com/memetemplate/{id}")),
            origin_url: Some(format!("https://imgflip.com/memetemplate/{id}")),
            fetch_plan: FetchPlan::HttpGet {
                url: url.to_string(),
                headers: vec![],
                filename_hint: format!("{}.{ext}", safe_slug(name)),
            },
        });
    }
    out
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
        "template".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({ "success": true, "data": { "memes": [
            { "id": "181913649", "name": "Drake Hotline Bling", "url": "https://i.imgflip.com/30b1gx.jpg", "width": 1200, "height": 1200 },
            { "id": "112126428", "name": "Distracted Boyfriend", "url": "https://i.imgflip.com/1ur9b0.jpg", "width": 1200, "height": 800 }
        ]}})
    }

    #[test]
    fn filters_by_query() {
        let hits = parse_and_filter(&sample(), "drake");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Drake Hotline Bling");
        match &hits[0].fetch_plan {
            FetchPlan::HttpGet {
                url, filename_hint, ..
            } => {
                assert_eq!(url, "https://i.imgflip.com/30b1gx.jpg");
                assert_eq!(filename_hint, "drake-hotline-bling.jpg");
            }
            _ => panic!("expected HttpGet"),
        }
    }

    #[test]
    fn empty_query_returns_all() {
        assert_eq!(parse_and_filter(&sample(), "").len(), 2);
        assert_eq!(parse_and_filter(&sample(), "nonexistent").len(), 0);
    }
}
