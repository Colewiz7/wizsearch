# SOURCES

Strategy: a few sources done well, shipping NO keys of any kind. Per-user free keys
(stored in the OS keychain) or scraping; never a shared developer secret.

## The SearchSource contract
- Async trait in `src-tauri/src/sources/mod.rs`. One module per source.
- Input: `SearchRequest { query, asset_types, cursor, page_size }` (opaque per-source
  cursor paging).
- Output: `SearchPage { items: Vec<ResultItem>, next_cursor }`. Each `ResultItem`
  carries title/source/asset_type, thumbnail_url, preview_stream_url, preview_kind,
  duration/width/height, license, attribution, origin_url, and a declarative
  `FetchPlan` the host executes only on explicit user selection.
- Sources are PURE: no shell/fs/db/credential/network-client imports. They use
  `ctx.http()` (rate-limited, allowlist-checked, injected) and `ctx.credential()`
  (read-only). Enforced by `tests/source_purity.rs` + CI grep.
- `SourceDescriptor` declares `allowed_hosts` (suffix match, https only) used to
  validate every preview URL and fetch plan, `default_rate_limit_per_min`, and
  `embedded_credential` which MUST stay `""` (asserted at startup).

## Shipped
| Source | Mode | Assets | Auth | Notes |
|---|---|---|---|---|
| myinstants | scrape | audio | none | No API. Parse search pages for `play('/media/sounds/..')`. License unclear (mostly third-party clips); labeled as such. |
| KLIPY | API | gif, sticker, video (clips) | per-user free key in URL path | Defensive JSON parsing of `data.data[].file.{size}.{format}`. MP4/WebP previews over raw GIF. |
| Tenor | scrape | gif | none | Google killed the free Tenor key signup, so we pull the `"results":[...]` JSON the public search page embeds (same shape the API returned). Single page, no cursor. |
| GIPHY | API | gif, sticker | per-user key, **disabled by default** | Opt-in (`default_enabled: false`); flip it on in Settings after adding a key. |
| Reddit | public JSON | gif, image, video, green_screen | none | `www.reddit.com/r/<subs>/search.json` over a comma-separated subreddit list (a setting). Reddit 403s datacenter IPs but answers normal desktop clients on residential connections (where the app runs); a browser UA is sent. GIFs fetch reddit's own mirrors; videos collect via yt-dlp so v.redd.it audio is merged back. NSFW filtered. |
| Imgur | API v3 | gif, image, video | per-user free Client-ID | `gallery/search`; albums unwrap to their first image; `<id>m.jpg` thumbnail trick. |
| Know Your Meme | scrape | image, gif | none | Image search grid; masonry rendition swapped to `/original/`. Cloudflare may block; that's an error chip. |
| YouTube | yt-dlp | video | none | Metadata-only discovery (`ytsearchN:` + `--flat-playlist --dump-single-json`) through `ctx.ytdlp_search_json` (host runs the binary, source stays pure). Collect = YtDlp fetch plan. 30s default timeout. |
| Pexels | API | video, green_screen | per-user free key (Authorization header) | `/videos/search`; green-screen-only filter augments the query. Pexels License, attribution captured. |

## Planned next (same trait, no core changes)
- Pixabay (per-user key; video/green screens), Freesound (per-user token; SFX with
  `preview-(hq|lq)-(mp3|ogg)` preview URLs), Mixkit (scrape; green screens).

## Adding a source (checklist)
1. New module in `src-tauri/src/sources/`, implement `SearchSource`, static
   `SourceDescriptor` with honest `allowed_hosts` and an empty `embedded_credential`.
2. Register it in `lib.rs` `source_list`. Its enabled/rate-limit/key settings are
   generated automatically from the descriptor.
3. Unit-test the response parser with a canned payload (see klipy/pexels tests).
4. `cargo test` must pass, including source_purity.
