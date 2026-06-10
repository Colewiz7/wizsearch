//! Local collection: SQLite + FTS5, collect jobs, exact dedupe by sha256.
//! Collecting is the END of the funnel: it only ever happens on explicit user
//! action, by executing a validated FetchPlan.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::settings::SettingsBackend;
use crate::sources::{FetchPlan, ResultItem};

const MIGRATIONS: &[&str] = &[include_str!("../../migrations/0001_init.sql")];

#[derive(Debug, thiserror::Error)]
pub enum CollectionError {
    #[error("db: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("download: {0}")]
    Download(String),
    #[error("{0}")]
    Other(String),
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------- DTOs ----------

#[derive(Debug, Clone, Serialize)]
pub struct CollectedAsset {
    pub id: i64,
    pub uid: String,
    pub source: String,
    pub asset_type: String,
    pub title: String,
    pub license: Option<String>,
    pub attribution: Option<String>,
    pub origin_url: Option<String>,
    pub duration_ms: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub favorite: bool,
    pub collected_at: i64,
    pub tags: Vec<String>,
    /// absolute path of the original file (for drag-out / copy / reveal)
    pub abs_path: String,
    /// absolute path of the thumbnail if one exists
    pub thumb_path: Option<String>,
    pub mime: Option<String>,
    pub bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectOutcome {
    pub asset: CollectedAsset,
    /// true when the exact same file (sha256) was already in the library
    pub was_duplicate: bool,
}

// ---------- DB ----------

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, CollectionError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Db {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, CollectionError> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Db {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<(), CollectionError> {
        let conn = self.conn.lock().expect("db lock");
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let target = (i + 1) as i64;
            if version < target {
                conn.execute_batch(sql)?;
                conn.pragma_update(None, "user_version", target)?;
            }
        }
        Ok(())
    }

    fn with<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, rusqlite::Error>,
    ) -> Result<T, CollectionError> {
        let conn = self.conn.lock().expect("db lock");
        Ok(f(&conn)?)
    }
}

// settings persistence rides on the same db
impl SettingsBackend for std::sync::Arc<Db> {
    fn load_all(&self) -> Result<HashMap<String, String>, String> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT key, value FROM settings")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.collect()
        })
        .map(|v: Vec<(String, String)>| v.into_iter().collect())
        .map_err(|e| e.to_string())
    }

    fn save(&self, key: &str, json_value: &str) -> Result<(), String> {
        self.with(|c| {
            c.execute(
                "INSERT INTO settings(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, json_value],
            )
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
}

// ---------- collection ops ----------

pub struct Collection;

impl Collection {
    /// Download a fetch plan target into the collection dir, hashing as we go.
    /// The caller has already validated the plan against the source allowlist.
    pub async fn download_plan(
        client: &reqwest::Client,
        plan: &FetchPlan,
        collection_dir: &Path,
    ) -> Result<(PathBuf, String, u64, Option<String>), CollectionError> {
        let FetchPlan::HttpGet {
            url,
            headers,
            filename_hint,
        } = plan
        else {
            return Err(CollectionError::Download(
                "yt-dlp plans are not supported yet".into(),
            ));
        };

        let tmp_dir = collection_dir.join(".tmp");
        tokio::fs::create_dir_all(&tmp_dir).await?;
        let tmp_path = tmp_dir.join(format!("{}.part", uuid::Uuid::new_v4()));

        let mut req = client.get(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| CollectionError::Download(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(CollectionError::Download(format!(
                "http {} for {url}",
                resp.status()
            )));
        }
        let mime = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.split(';').next().unwrap_or(s).trim().to_string());

        let mut hasher = Sha256::new();
        let mut bytes: u64 = 0;
        {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            let mut stream = resp;
            while let Some(chunk) = stream
                .chunk()
                .await
                .map_err(|e| CollectionError::Download(e.to_string()))?
            {
                hasher.update(&chunk);
                bytes += chunk.len() as u64;
                file.write_all(&chunk).await?;
            }
            file.flush().await?;
        }
        let sha256 = hex::encode(hasher.finalize());
        let _ = filename_hint; // final naming happens in store_collected
        Ok((tmp_path, sha256, bytes, mime))
    }

    /// Move the downloaded temp file into place and record everything.
    pub fn store_collected(
        db: &Db,
        collection_dir: &Path,
        item: &ResultItem,
        tmp_path: &Path,
        sha256: &str,
        bytes: u64,
        mime: Option<String>,
    ) -> Result<CollectOutcome, CollectionError> {
        // exact dedupe: same content hash anywhere in the library?
        let existing: Option<i64> = db.with(|c| {
            c.query_row(
                "SELECT asset_id FROM file_variants WHERE sha256 = ?1 AND kind = 'original' LIMIT 1",
                params![sha256],
                |r| r.get(0),
            )
            .optional()
        })?;
        if let Some(asset_id) = existing {
            let _ = std::fs::remove_file(tmp_path);
            let asset = Self::get_asset(db, collection_dir, asset_id)?
                .ok_or_else(|| CollectionError::Other("dupe row vanished".into()))?;
            return Ok(CollectOutcome {
                asset,
                was_duplicate: true,
            });
        }

        let uid = uuid::Uuid::new_v4().to_string();
        let filename_hint = match &item.fetch_plan {
            FetchPlan::HttpGet { filename_hint, .. } => filename_hint.clone(),
            FetchPlan::YtDlp { filename_hint, .. } => filename_hint.clone(),
        };
        let ext = Path::new(&filename_hint)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin")
            .to_ascii_lowercase();
        let stem = Path::new(&filename_hint)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("asset");
        // confine to the collection dir: uid prefix + sanitized stem, no user paths
        let safe_stem: String = stem
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .take(48)
            .collect();
        let rel_path = format!(
            "{}/{}-{}.{ext}",
            item.asset_type.as_str(),
            &uid[..8],
            safe_stem
        );
        let final_path = collection_dir.join(&rel_path);
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(tmp_path, &final_path).or_else(|_| {
            // cross-device fallback
            std::fs::copy(tmp_path, &final_path).map(|_| ())?;
            std::fs::remove_file(tmp_path)
        })?;

        let asset_id = db.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute(
                "INSERT INTO dupe_groups(sha256) VALUES (?1) ON CONFLICT(sha256) DO NOTHING",
                params![sha256],
            )?;
            let group_id: i64 = tx.query_row(
                "SELECT id FROM dupe_groups WHERE sha256 = ?1",
                params![sha256],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO assets(uid, source, source_item_id, asset_type, title, license,
                                    attribution, origin_url, duration_ms, width, height,
                                    collected_at, dupe_group_id)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    uid,
                    item.source,
                    item.id,
                    item.asset_type.as_str(),
                    item.title,
                    item.license,
                    item.attribution,
                    item.origin_url,
                    item.duration_ms.map(|d| d as i64),
                    item.width.map(|w| w as i64),
                    item.height.map(|h| h as i64),
                    now_ms(),
                    group_id,
                ],
            )?;
            let asset_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO file_variants(asset_id, kind, rel_path, mime, bytes, sha256, width, height, duration_ms)
                 VALUES (?1, 'original', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    asset_id,
                    rel_path,
                    mime,
                    bytes as i64,
                    sha256,
                    item.width.map(|w| w as i64),
                    item.height.map(|h| h as i64),
                    item.duration_ms.map(|d| d as i64),
                ],
            )?;
            tx.execute(
                "INSERT INTO assets_fts(rowid, title, description, tags, attribution)
                 VALUES (?1, ?2, '', '', ?3)",
                params![asset_id, item.title, item.attribution.clone().unwrap_or_default()],
            )?;
            tx.commit()?;
            Ok(asset_id)
        })?;

        let asset = Self::get_asset(db, collection_dir, asset_id)?
            .ok_or_else(|| CollectionError::Other("asset row vanished".into()))?;
        Ok(CollectOutcome {
            asset,
            was_duplicate: false,
        })
    }

    pub fn record_thumbnail(db: &Db, asset_id: i64, rel_path: &str) -> Result<(), CollectionError> {
        db.with(|c| {
            c.execute(
                "INSERT INTO file_variants(asset_id, kind, rel_path) VALUES (?1, 'thumbnail', ?2)",
                params![asset_id, rel_path],
            )
        })?;
        Ok(())
    }

    pub fn get_asset(
        db: &Db,
        collection_dir: &Path,
        asset_id: i64,
    ) -> Result<Option<CollectedAsset>, CollectionError> {
        let mut assets = Self::query_assets(
            db,
            collection_dir,
            "WHERE a.id = ?1",
            &[&asset_id as &dyn rusqlite::ToSql],
        )?;
        Ok(assets.pop())
    }

    pub fn list(
        db: &Db,
        collection_dir: &Path,
        asset_type: Option<&str>,
    ) -> Result<Vec<CollectedAsset>, CollectionError> {
        match asset_type {
            Some(t) => Self::query_assets(
                db,
                collection_dir,
                "WHERE a.asset_type = ?1 ORDER BY a.collected_at DESC",
                &[&t as &dyn rusqlite::ToSql],
            ),
            None => Self::query_assets(db, collection_dir, "ORDER BY a.collected_at DESC", &[]),
        }
    }

    /// FTS5 search over the local collection only
    pub fn search(
        db: &Db,
        collection_dir: &Path,
        query: &str,
        asset_type: Option<&str>,
    ) -> Result<Vec<CollectedAsset>, CollectionError> {
        let q = query.trim();
        if q.is_empty() {
            return Self::list(db, collection_dir, asset_type);
        }
        // quote each term so user input can't break FTS syntax
        let fts_query: String = q
            .split_whitespace()
            .map(|t| format!("\"{}\"*", t.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" ");
        match asset_type {
            Some(t) => Self::query_assets(
                db,
                collection_dir,
                "JOIN assets_fts f ON f.rowid = a.id
                 WHERE assets_fts MATCH ?1 AND a.asset_type = ?2
                 ORDER BY rank",
                &[
                    &fts_query as &dyn rusqlite::ToSql,
                    &t as &dyn rusqlite::ToSql,
                ],
            ),
            None => Self::query_assets(
                db,
                collection_dir,
                "JOIN assets_fts f ON f.rowid = a.id WHERE assets_fts MATCH ?1 ORDER BY rank",
                &[&fts_query as &dyn rusqlite::ToSql],
            ),
        }
    }

    fn query_assets(
        db: &Db,
        collection_dir: &Path,
        tail: &str,
        params_slice: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<CollectedAsset>, CollectionError> {
        let sql = format!(
            "SELECT a.id, a.uid, a.source, a.asset_type, a.title, a.license, a.attribution,
                    a.origin_url, a.duration_ms, a.width, a.height, a.favorite, a.collected_at,
                    (SELECT rel_path FROM file_variants v WHERE v.asset_id = a.id AND v.kind='original' LIMIT 1),
                    (SELECT rel_path FROM file_variants v WHERE v.asset_id = a.id AND v.kind='thumbnail' LIMIT 1),
                    (SELECT mime FROM file_variants v WHERE v.asset_id = a.id AND v.kind='original' LIMIT 1),
                    (SELECT bytes FROM file_variants v WHERE v.asset_id = a.id AND v.kind='original' LIMIT 1)
             FROM assets a {tail}"
        );
        let rows: Vec<CollectedAsset> = db.with(|c| {
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt.query_map(params_slice, |r| {
                let rel: Option<String> = r.get(13)?;
                let thumb_rel: Option<String> = r.get(14)?;
                Ok(CollectedAsset {
                    id: r.get(0)?,
                    uid: r.get(1)?,
                    source: r.get(2)?,
                    asset_type: r.get(3)?,
                    title: r.get(4)?,
                    license: r.get(5)?,
                    attribution: r.get(6)?,
                    origin_url: r.get(7)?,
                    duration_ms: r.get(8)?,
                    width: r.get(9)?,
                    height: r.get(10)?,
                    favorite: r.get::<_, i64>(11)? != 0,
                    collected_at: r.get(12)?,
                    tags: Vec::new(),
                    abs_path: rel
                        .map(|p| collection_dir.join(p).to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    thumb_path: thumb_rel
                        .map(|p| collection_dir.join(p).to_string_lossy().into_owned()),
                    mime: r.get(15)?,
                    bytes: r.get(16)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })?;

        // attach tags
        let mut out = rows;
        for asset in &mut out {
            asset.tags = db.with(|c| {
                let mut stmt = c.prepare(
                    "SELECT t.name FROM tags t JOIN asset_tags at ON at.tag_id = t.id
                     WHERE at.asset_id = ?1 ORDER BY t.name",
                )?;
                let names = stmt.query_map(params![asset.id], |r| r.get::<_, String>(0))?;
                names.collect::<Result<Vec<_>, _>>()
            })?;
        }
        Ok(out)
    }

    pub fn set_tags(db: &Db, asset_id: i64, tags: &[String]) -> Result<(), CollectionError> {
        db.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM asset_tags WHERE asset_id = ?1",
                params![asset_id],
            )?;
            for tag in tags {
                let tag = tag.trim();
                if tag.is_empty() {
                    continue;
                }
                tx.execute(
                    "INSERT INTO tags(name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
                    params![tag],
                )?;
                let tag_id: i64 =
                    tx.query_row("SELECT id FROM tags WHERE name = ?1", params![tag], |r| {
                        r.get(0)
                    })?;
                tx.execute(
                    "INSERT INTO asset_tags(asset_id, tag_id) VALUES (?1, ?2)
                     ON CONFLICT DO NOTHING",
                    params![asset_id, tag_id],
                )?;
            }
            // refresh FTS row
            let (title, attribution): (String, Option<String>) = tx.query_row(
                "SELECT title, attribution FROM assets WHERE id = ?1",
                params![asset_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            tx.execute("DELETE FROM assets_fts WHERE rowid = ?1", params![asset_id])?;
            tx.execute(
                "INSERT INTO assets_fts(rowid, title, description, tags, attribution)
                 VALUES (?1, ?2, '', ?3, ?4)",
                params![
                    asset_id,
                    title,
                    tags.join(" "),
                    attribution.unwrap_or_default()
                ],
            )?;
            tx.commit()?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn set_favorite(db: &Db, asset_id: i64, favorite: bool) -> Result<(), CollectionError> {
        db.with(|c| {
            c.execute(
                "UPDATE assets SET favorite = ?2 WHERE id = ?1",
                params![asset_id, favorite as i64],
            )
        })?;
        Ok(())
    }

    /// delete the rows and the files
    pub fn delete(db: &Db, collection_dir: &Path, asset_id: i64) -> Result<(), CollectionError> {
        let rel_paths: Vec<String> = db.with(|c| {
            let mut stmt = c.prepare("SELECT rel_path FROM file_variants WHERE asset_id = ?1")?;
            let rows = stmt.query_map(params![asset_id], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()
        })?;
        db.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute("DELETE FROM assets_fts WHERE rowid = ?1", params![asset_id])?;
            tx.execute("DELETE FROM assets WHERE id = ?1", params![asset_id])?;
            tx.commit()?;
            Ok(())
        })?;
        for rel in rel_paths {
            let p = collection_dir.join(rel);
            // only ever delete inside the collection dir
            if p.starts_with(collection_dir) {
                let _ = std::fs::remove_file(p);
            }
        }
        Ok(())
    }

    // ---------- jobs ----------

    pub fn job_create(db: &Db, kind: &str, payload: &str) -> Result<i64, CollectionError> {
        db.with(|c| {
            c.execute(
                "INSERT INTO jobs(kind, status, payload, created_at, updated_at)
                 VALUES (?1, 'running', ?2, ?3, ?3)",
                params![kind, payload, now_ms()],
            )?;
            Ok(c.last_insert_rowid())
        })
    }

    pub fn job_finish(
        db: &Db,
        job_id: i64,
        asset_id: Option<i64>,
        error: Option<&str>,
    ) -> Result<(), CollectionError> {
        db.with(|c| {
            c.execute(
                "UPDATE jobs SET status = ?2, error = ?3, asset_id = ?4, updated_at = ?5 WHERE id = ?1",
                params![
                    job_id,
                    if error.is_some() { "error" } else { "done" },
                    error,
                    asset_id,
                    now_ms(),
                ],
            )
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{AssetType, PreviewKind};

    fn item() -> ResultItem {
        ResultItem {
            id: "test:1".into(),
            source: "test".into(),
            asset_type: AssetType::Audio,
            title: "Vine Boom".into(),
            thumbnail_url: None,
            preview_stream_url: None,
            preview_kind: PreviewKind::AudioStream,
            duration_ms: Some(1200),
            width: None,
            height: None,
            license: Some("unknown".into()),
            attribution: Some("myinstants".into()),
            origin_url: None,
            fetch_plan: FetchPlan::HttpGet {
                url: "https://example.com/boom.mp3".into(),
                headers: vec![],
                filename_hint: "boom.mp3".into(),
            },
        }
    }

    fn store_fake(db: &Db, dir: &Path, content: &[u8]) -> CollectOutcome {
        let tmp = dir.join(format!("{}.part", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(&tmp, content).unwrap();
        let sha = hex::encode(Sha256::digest(content));
        Collection::store_collected(db, dir, &item(), &tmp, &sha, content.len() as u64, None)
            .unwrap()
    }

    #[test]
    fn collect_then_fts_search_finds_it() {
        let db = Db::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("wiz-test-{}", uuid::Uuid::new_v4()));
        let out = store_fake(&db, &dir, b"audio-bytes");
        assert!(!out.was_duplicate);
        assert!(Path::new(&out.asset.abs_path).exists());

        let hits = Collection::search(&db, &dir, "vine", None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Vine Boom");

        let miss = Collection::search(&db, &dir, "zebra", None).unwrap();
        assert!(miss.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn same_bytes_dedupe_into_one_asset() {
        let db = Db::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("wiz-test-{}", uuid::Uuid::new_v4()));
        let first = store_fake(&db, &dir, b"same-bytes");
        let second = store_fake(&db, &dir, b"same-bytes");
        assert!(!first.was_duplicate);
        assert!(second.was_duplicate);
        assert_eq!(first.asset.id, second.asset.id);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tags_update_fts() {
        let db = Db::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("wiz-test-{}", uuid::Uuid::new_v4()));
        let out = store_fake(&db, &dir, b"tagged");
        Collection::set_tags(&db, out.asset.id, &["reaction".into(), "bass".into()]).unwrap();

        let hits = Collection::search(&db, &dir, "reaction", None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].tags,
            vec!["bass".to_string(), "reaction".to_string()]
        );

        Collection::set_tags(&db, out.asset.id, &[]).unwrap();
        let miss = Collection::search(&db, &dir, "reaction", None).unwrap();
        assert!(miss.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn delete_removes_rows_and_files() {
        let db = Db::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join(format!("wiz-test-{}", uuid::Uuid::new_v4()));
        let out = store_fake(&db, &dir, b"deleted");
        let path = out.asset.abs_path.clone();
        Collection::delete(&db, &dir, out.asset.id).unwrap();
        assert!(!Path::new(&path).exists());
        assert!(Collection::list(&db, &dir, None).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
