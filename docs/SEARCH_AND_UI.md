# SEARCH_AND_UI

## The grid is the app
One search box, one unified grid across all enabled sources. Asset-type filter chips
(Everything / Sounds / GIFs / Stickers / Videos / Green screens) map to
`AssetType` filters passed to every source.

## Progressive, isolated, merged
- `search_start` returns a search id; the host emits `search://update` with the
  full merged snapshot every time a source finishes. Stale searches stop emitting
  (generation counter), so fast typing never interleaves old results.
- Per-source status chips show pending/count/error/timeout; a failed source is a
  chip, never a broken grid.
- Merge strategy is a setting: `round_robin` (default) interleaves sources;
  `grouped` concatenates. Load-more pages each source by its own opaque cursor and
  appends deduped.

## Tiles by preview kind
- `audio_stream` — inline play/pause row with progress bar; one audio plays at a
  time (`preview/audioBus.ts`); volume is a setting.
- `video_loop` / `poster_loop` — thumbnail/poster, hover (delay is a setting) mounts
  a muted looping `<video>`; green screens get a GS badge.
- `animated_image` — static/small thumb, hover swaps in the animated WebP/GIF.
- Every tile: title, source badge (setting), duration, license line, Collect button
  with idle/working/done/duplicate/error states.

## Virtualization
`VirtuosoMasonry` (`@virtuoso.dev/masonry`, MIT) renders the masonry grid; column
count derives from container width / `grid.tile_min_width` (a setting).
react-virtuoso is available for list views.

## Playback rule
Media elements only ever get `wzstream://` URLs (`src/api/stream.ts`). Thumbnails may
load from https directly (CSP img-src), media may not (CSP media-src restricts to
wzstream).

## Library view
FTS search over collected assets, type filter, favorite, tag editing, drag-out
(tauri-plugin-drag with embedded icon), copy path (clipboard plugin), reveal in file
manager (opener plugin), delete.
