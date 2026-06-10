# ARCHITECTURE

WizSearch is a discovery tool. The main screen is search; the app queries several
sources concurrently and renders one unified previewable grid. Collecting locally is
the end of the funnel.

```
React grid  ── invoke/events ──  Rust host
                                  ├─ search/     orchestration, merge, rate limiters
                                  ├─ sources/    PURE SearchSource impls (data in, data + FetchPlans out)
                                  ├─ preview/    wzstream:// protocol (Range streaming, remote proxy), ffmpeg thumbs
                                  ├─ collection/ SQLite + FTS5, jobs, sha256 dedupe
                                  ├─ settings/   typed registry -> generated settings UI
                                  ├─ sidecars/   pinned + SHA-256-verified ffmpeg/ffprobe/yt-dlp
                                  └─ security/   keychain, allowlist validation, startup assertions
```

## The flow
1. User types; frontend calls `search_start` (debounce is a setting).
2. `SearchHost` spawns one task per enabled source with a per-source timeout. A
   source failure/timeout never affects the others.
3. Each completion re-merges everything received so far (round_robin or grouped, a
   setting) and emits `search://update`; the grid renders progressively.
4. Tiles preview via `wzstream://localhost/remote?src=..&url=..` — the host proxies
   the bytes after checking the URL against the source's allowlist. Audio plays
   inline; video/GIF loops on hover; green screens show poster + loop.
5. Only when the user clicks Collect does the host validate and execute the item's
   declarative `FetchPlan`, hash it, dedupe it, store it under the collection dir,
   and index it in FTS5.

## Module boundaries (enforced)
- `sources/` must not import shell/fs/db/reqwest/keyring/tauri. It receives a
  read-only `SourceContext` (rate-limited http + read-only credential). Enforced by
  `tests/source_purity.rs` + `scripts/check_source_purity.sh` in CI.
- Swappable seams: `SearchSource` (sources), `SourceHttp`/`SourceContext` (transport),
  `SettingsBackend` (settings persistence), merge strategy (function keyed by a
  setting), preview protocol handlers.
- Future layers (AI tagging, external plugins, browse-and-capture WebView) attach to
  these seams without changing `SearchSource`.

## Linux media reality (why wzstream exists)
WebKitGTK will not reliably play media from naive paths or cross-origin URLs, and
seeking needs Range support. So: a custom URI scheme protocol in Rust serves local
files with HTTP Range and proxies remote previews (status/Content-Range passthrough),
and `bundle.linux.appimage.bundleMediaFramework = true` ships the GStreamer bits in
the AppImage. Previews prefer MP4/animated WebP over raw GIF.
