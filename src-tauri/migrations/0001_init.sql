-- WizSearch schema v1. Local collection only: remote search results are live
-- and never stored here.

CREATE TABLE assets (
  id            INTEGER PRIMARY KEY,
  uid           TEXT NOT NULL UNIQUE,        -- uuid v4
  source        TEXT NOT NULL,               -- source descriptor id
  source_item_id TEXT,                       -- the source's id for this item
  asset_type    TEXT NOT NULL,               -- audio|gif|sticker|video|green_screen
  title         TEXT NOT NULL,
  description   TEXT,
  license       TEXT,
  attribution   TEXT,
  origin_url    TEXT,
  duration_ms   INTEGER,
  width         INTEGER,
  height        INTEGER,
  favorite      INTEGER NOT NULL DEFAULT 0,
  collected_at  INTEGER NOT NULL,            -- unix ms
  dupe_group_id INTEGER REFERENCES dupe_groups(id)
);

CREATE INDEX idx_assets_type ON assets(asset_type);
CREATE INDEX idx_assets_source ON assets(source);

CREATE TABLE file_variants (
  id          INTEGER PRIMARY KEY,
  asset_id    INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL,                 -- original|thumbnail|loop
  rel_path    TEXT NOT NULL,                 -- relative to collection.dir
  mime        TEXT,
  bytes       INTEGER,
  sha256      TEXT,
  width       INTEGER,
  height      INTEGER,
  duration_ms INTEGER
);

CREATE INDEX idx_variants_asset ON file_variants(asset_id);
CREATE INDEX idx_variants_sha ON file_variants(sha256);

CREATE TABLE tags (
  id   INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE
);

CREATE TABLE asset_tags (
  asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  tag_id   INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY (asset_id, tag_id)
);

-- collect/download jobs, kept for history + retry
CREATE TABLE jobs (
  id         INTEGER PRIMARY KEY,
  kind       TEXT NOT NULL,                  -- collect|thumbnail
  status     TEXT NOT NULL,                  -- queued|running|done|error
  payload    TEXT NOT NULL,                  -- json
  error      TEXT,
  asset_id   INTEGER REFERENCES assets(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

-- exact-duplicate groups by content hash
CREATE TABLE dupe_groups (
  id     INTEGER PRIMARY KEY,
  sha256 TEXT NOT NULL UNIQUE
);

-- non-secret settings as json values; secrets live in the OS keychain
CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- FTS over the local collection only; kept in sync by collection code
CREATE VIRTUAL TABLE assets_fts USING fts5(
  title,
  description,
  tags,
  attribution,
  content='',
  contentless_delete=1
);
