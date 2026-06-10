//! Tauri commands: the only surface the frontend can call.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::collection::{CollectOutcome, CollectedAsset, Collection, Db};
use crate::search::{SearchHost, SourceInfo};
use crate::security;
use crate::settings::{SettingDef, SettingsStore};
use crate::sidecars;
use crate::sources::{AssetType, ResultItem, SearchPage};

pub struct AppState {
    pub db: Arc<Db>,
    pub settings: Arc<SettingsStore>,
    pub search: Arc<SearchHost>,
    pub client: reqwest::Client,
    pub app_data: PathBuf,
    pub default_collection_dir: PathBuf,
}

impl AppState {
    pub fn collection_dir(&self) -> PathBuf {
        PathBuf::from(self.settings.string_or(
            "collection.dir",
            &self.default_collection_dir.to_string_lossy(),
        ))
    }
}

type CmdResult<T> = Result<T, String>;

// ---------- search ----------

#[tauri::command]
pub async fn search_start(
    app: AppHandle,
    state: State<'_, AppState>,
    query: String,
    asset_types: Vec<AssetType>,
) -> CmdResult<u64> {
    Ok(state.search.start_search(app, query, asset_types).await)
}

#[tauri::command]
pub async fn search_more(
    state: State<'_, AppState>,
    source: String,
    query: String,
    asset_types: Vec<AssetType>,
    cursor: Option<String>,
) -> CmdResult<SearchPage> {
    state
        .search
        .search_more(&source, query, asset_types, cursor)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn sources_list(state: State<'_, AppState>) -> Vec<SourceInfo> {
    state.search.source_infos()
}

// ---------- collect ----------

#[tauri::command]
pub async fn collect_item(
    app: AppHandle,
    state: State<'_, AppState>,
    item: ResultItem,
) -> CmdResult<CollectOutcome> {
    // the fetch only ever happens here, after explicit user selection
    let desc = state
        .search
        .descriptors()
        .into_iter()
        .find(|d| d.id == item.source)
        .ok_or_else(|| format!("unknown source '{}'", item.source))?;
    security::validate_fetch_plan(&item.fetch_plan, desc).map_err(|e| e.to_string())?;

    let dir = state.collection_dir();
    let job_id = Collection::job_create(
        &state.db,
        "collect",
        &serde_json::to_string(&item).unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    let _ = app.emit("collect://started", &item.id);

    let downloaded = Collection::download_plan(&state.client, &item.fetch_plan, &dir).await;
    let (tmp, sha256, bytes, mime) = match downloaded {
        Ok(v) => v,
        Err(e) => {
            let _ = Collection::job_finish(&state.db, job_id, None, Some(&e.to_string()));
            let _ = app.emit("collect://failed", (&item.id, e.to_string()));
            return Err(e.to_string());
        }
    };

    let stored = Collection::store_collected(&state.db, &dir, &item, &tmp, &sha256, bytes, mime);
    match stored {
        Ok(outcome) => {
            let _ = Collection::job_finish(&state.db, job_id, Some(outcome.asset.id), None);
            // best-effort thumbnail for video-ish assets
            if !outcome.was_duplicate
                && state.settings.bool_or("collection.make_thumbnails", true)
                && matches!(item.asset_type, AssetType::Video | AssetType::GreenScreen)
            {
                let ffmpeg = sidecars::tool_path(&state.app_data, "ffmpeg");
                if ffmpeg.exists() {
                    let input = PathBuf::from(&outcome.asset.abs_path);
                    if let Ok(rel) = crate::preview::ffmpeg::make_thumbnail(
                        &ffmpeg,
                        &input,
                        &dir,
                        &outcome.asset.uid,
                    )
                    .await
                    {
                        let _ = Collection::record_thumbnail(&state.db, outcome.asset.id, &rel);
                    }
                }
            }
            let _ = app.emit("collect://done", &outcome);
            Ok(outcome)
        }
        Err(e) => {
            let _ = Collection::job_finish(&state.db, job_id, None, Some(&e.to_string()));
            let _ = app.emit("collect://failed", (&item.id, e.to_string()));
            Err(e.to_string())
        }
    }
}

// ---------- collection ----------

#[tauri::command]
pub fn collection_search(
    state: State<'_, AppState>,
    query: String,
    asset_type: Option<String>,
) -> CmdResult<Vec<CollectedAsset>> {
    Collection::search(
        &state.db,
        &state.collection_dir(),
        &query,
        asset_type.as_deref(),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn collection_delete(state: State<'_, AppState>, asset_id: i64) -> CmdResult<()> {
    Collection::delete(&state.db, &state.collection_dir(), asset_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn asset_set_tags(
    state: State<'_, AppState>,
    asset_id: i64,
    tags: Vec<String>,
) -> CmdResult<()> {
    Collection::set_tags(&state.db, asset_id, &tags).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn asset_set_favorite(
    state: State<'_, AppState>,
    asset_id: i64,
    favorite: bool,
) -> CmdResult<()> {
    Collection::set_favorite(&state.db, asset_id, favorite).map_err(|e| e.to_string())
}

// ---------- settings ----------

#[tauri::command]
pub fn settings_defs(state: State<'_, AppState>) -> Vec<SettingDef> {
    state.settings.defs().to_vec()
}

#[tauri::command]
pub fn settings_values(state: State<'_, AppState>) -> std::collections::HashMap<String, Value> {
    state.settings.all_values()
}

#[tauri::command]
pub fn settings_set(state: State<'_, AppState>, key: String, value: Value) -> CmdResult<()> {
    state.settings.set(&key, value).map_err(|e| e.to_string())
}

// ---------- secrets (keychain only; values are write-only from the UI) ----------

#[tauri::command]
pub async fn secret_set(state: State<'_, AppState>, key: String, value: String) -> CmdResult<()> {
    // only keys registered as Secret settings may be written
    let is_secret = state
        .settings
        .def(&key)
        .map(|d| matches!(d.kind, crate::settings::SettingKind::Secret { .. }))
        .unwrap_or(false);
    if !is_secret {
        return Err(format!("'{key}' is not a registered secret"));
    }
    tokio::task::spawn_blocking(move || security::secret_set(&key, &value))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn secret_exists(key: String) -> CmdResult<bool> {
    tokio::task::spawn_blocking(move || security::secret_get(&key))
        .await
        .map_err(|e| e.to_string())?
        .map(|v| v.is_some())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn secret_clear(key: String) -> CmdResult<()> {
    tokio::task::spawn_blocking(move || security::secret_clear(&key))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

// ---------- sidecars ----------

#[tauri::command]
pub fn sidecars_status(state: State<'_, AppState>) -> CmdResult<Vec<sidecars::SidecarStatus>> {
    sidecars::status(&state.app_data).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sidecars_ensure(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Vec<sidecars::SidecarStatus>> {
    let result = sidecars::ensure_all(&state.client, &state.app_data).await;
    match &result {
        Ok(statuses) => {
            let _ = app.emit("sidecar://updated", statuses);
        }
        Err(e) => {
            let _ = app.emit("sidecar://error", e.to_string());
        }
    }
    result.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn ytdlp_update(state: State<'_, AppState>) -> CmdResult<String> {
    if !state.settings.bool_or("sidecars.ytdlp_self_update", true) {
        return Err("yt-dlp self-update is disabled in Settings".into());
    }
    sidecars::ytdlp_self_update(&state.app_data)
        .await
        .map_err(|e| e.to_string())
}

// ---------- misc ----------

/// absolute path of a collected asset, for drag-out / copy-path / reveal
#[tauri::command]
pub fn asset_path(state: State<'_, AppState>, asset_id: i64) -> CmdResult<String> {
    let asset = Collection::get_asset(&state.db, &state.collection_dir(), asset_id)
        .map_err(|e| e.to_string())?
        .ok_or("asset not found")?;
    Ok(asset.abs_path)
}

pub fn handle_stream_request(
    app: &AppHandle,
    request: http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let stream_state = crate::preview::StreamState {
            client: state.client.clone(),
            descriptors: state.search.descriptors(),
            settings: state.settings.clone(),
            default_collection_dir: state.default_collection_dir.clone(),
        };
        let response = crate::preview::handle(&stream_state, request).await;
        responder.respond(response);
    });
}
