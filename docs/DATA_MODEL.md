# DATA_MODEL

SQLite (WAL) + FTS5, schema in `src-tauri/migrations/0001_init.sql`, applied via
`PRAGMA user_version`. The DB holds the LOCAL collection only; remote search results
are live and never stored.

## Tables
- `assets` — one collected item: uid, source, source_item_id, asset_type, title,
  license, attribution, origin_url, dimensions/duration, favorite, collected_at,
  dupe_group_id.
- `file_variants` — files per asset (kind: original | thumbnail | loop), rel_path
  (relative to the collection dir), mime, bytes, sha256.
- `tags` / `asset_tags` — user tags, case-insensitive unique.
- `jobs` — collect/thumbnail job history (queued|running|done|error) for retry/debug.
- `dupe_groups` — exact-duplicate groups keyed by content sha256. Collecting bytes
  that already exist returns the existing asset (`was_duplicate: true`) instead of
  storing a copy.
- `settings` — non-secret settings as JSON. Secrets never land here.
- `assets_fts` — FTS5 (contentless, `contentless_delete=1`) over title, description,
  tags, attribution. Kept in sync by collection code on collect / tag change /
  delete. User input is term-quoted (`"term"*`) so FTS syntax can't be injected.

## Files on disk
```
<collection.dir>/                user-configurable, default ~/WizSearch
  audio/0a1b2c3d-vine-boom.mp3   uid-prefix + sanitized stem
  gif/...
  green_screen/...
  .thumbs/<uid>.webp             ffmpeg-generated posters
  .tmp/                          in-flight downloads (hash-verified, then renamed)
```
Everything under the collection dir is git-ignored; the repo never contains media.
