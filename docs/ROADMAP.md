# ROADMAP

## M1 — discovery core (DONE)
- `SearchSource` trait + concurrent isolated host orchestration with per-source
  timeout, rate limiting, progressive merged emission.
- Sources covering all four asset types: myinstants (scrape, audio), KLIPY (per-user
  key; gif/sticker/clip), Pexels (per-user key; green screen/video).
- Unified virtualized masonry grid: inline streaming audio rows, hover-to-loop
  video/GIF tiles, green-screen poster+loop.
- wzstream:// protocol (Range streaming local, allowlist-checked remote proxy),
  `bundleMediaFramework = true`.
- Collect-to-local: SQLite + FTS5 (assets, file_variants, tags, asset_tags, jobs,
  dupe_groups, settings, assets_fts), sha256 dedupe, ffmpeg poster thumbs.
- Typed settings registry + generated settings UI; keychain-backed per-source keys.
- Sidecar download + SHA-256 verification; yt-dlp self-update.
- Drag-out, copy-path, reveal-in-file-manager.

## M2 — more sources + yt-dlp discovery
- Pixabay, Freesound, Mixkit sources; opt-in GIPHY (per-user key, off by default).
- yt-dlp short-video-meme discovery source (`ytsearchN:`, metadata only) and
  yt-dlp-executed FetchPlans (explicit user action, like everything else).
- Per-source result weighting in the merge; collection export/import.

## M3 — power features
- Browse-and-capture fallback WebView for sources that resist both API and scraping.
- Waveform previews for audio; loop-segment preview generation via ffmpeg.
- Windows release packaging + CI builds.

## M4 — intelligence (kept out until the base is solid)
- Local AI tagging behind a trait (same registry pattern, swappable backends).
- External/WASM source plugins.
- Trend scoring / watchlists.

The `SearchSource` trait is the stability contract: M2-M4 must not break M1 sources.
