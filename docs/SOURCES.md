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

## Shipped in M1
| Source | Mode | Assets | Auth | Notes |
|---|---|---|---|---|
| myinstants | scrape | audio | none | No API. Parse search pages for `play('/media/sounds/..')`. License unclear (mostly third-party clips); labeled as such. |
| KLIPY | API | gif, sticker, video (clips) | per-user free key in URL path | Tenor is dead, GIPHY is paid; KLIPY is the GIF/clip source. Defensive JSON parsing of `data.data[].file.{size}.{format}`. MP4/WebP previews preferred over raw GIF. |
| Pexels | API | video, green_screen | per-user free key (Authorization header) | `/videos/search`; green-screen-only filter augments the query with "green screen". Pexels License, attribution captured. |

## Planned next (same trait, no core changes)
- Pixabay (per-user key; video/green screens), Freesound (per-user token; SFX with
  `preview-(hq|lq)-(mp3|ogg)` preview URLs), Mixkit (scrape; green screens),
  yt-dlp short-video discovery (no key; `ytsearchN:` with
  `--skip-download --flat-playlist --dump-single-json`, metadata only, downloads stay
  user-explicit), opt-in GIPHY (per-user key, disabled by default — registry already
  supports it; add the source module when wired).

## Adding a source (checklist)
1. New module in `src-tauri/src/sources/`, implement `SearchSource`, static
   `SourceDescriptor` with honest `allowed_hosts` and an empty `embedded_credential`.
2. Register it in `lib.rs` `source_list`. Its enabled/rate-limit/key settings are
   generated automatically from the descriptor.
3. Unit-test the response parser with a canned payload (see klipy/pexels tests).
4. `cargo test` must pass, including source_purity.
