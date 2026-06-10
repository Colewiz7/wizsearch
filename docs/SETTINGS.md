# SETTINGS

Principle 4 in practice: every user-facing behavior is a registered setting with a
key, label, description, category, schema, and sane default.

## How it works
- `settings::build_registry` (Rust) is the single source of truth. Per-source
  settings (enabled, rate limit, API key) are generated from `SourceDescriptor`s, so
  a new source gets its settings for free.
- Values persist as JSON in the SQLite `settings` table through the `SettingsBackend`
  trait; in-memory reads are lock-cheap (`bool_or`/`i64_or`/`string_or`).
- `settings_set` validates against the schema (type, min/max, select membership) and
  rejects unknown keys. Bad values cannot enter the store.
- Secret kind is special: the value goes to the OS keychain, never the DB.
  `settings_set` refuses secrets; use `secret_set`.
- The Settings UI (`src/settings/SettingsView.tsx`) is generated from
  `settings_defs`; there are no hand-built controls. Adding a registry entry IS
  adding the UI.

## Current keys (defaults in parentheses)
- search: timeout_ms (8000), page_size (24), merge_strategy (round_robin|grouped),
  search_as_you_type (true), debounce_ms (450)
- preview: hover_to_play (true), hover_delay_ms (120), audio_volume (0.8),
  mute_video_previews (true)
- grid: tile_min_width (220), show_source_badges (true)
- collection: dir (~/WizSearch), make_thumbnails (true)
- sidecars: auto_download (true), ytdlp_self_update (true)
- ui: theme (dark|light)
- per source X: sources.X.enabled (true), sources.X.rate_limit_per_min (descriptor
  default), source.X.api_key (Secret, keychain) when the source requires a key

## Rule
New user-facing behavior => new registry entry with default + description, before the
feature merges. If a user could reasonably want to change it, it is a setting.
