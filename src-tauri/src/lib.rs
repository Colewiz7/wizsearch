pub mod collection;
pub mod commands;
pub mod preview;
pub mod search;
pub mod security;
pub mod settings;
pub mod sidecars;
pub mod sources;

use std::sync::Arc;

use tauri::Manager;

use collection::Db;
use commands::AppState;
use search::SearchHost;
use settings::SettingsStore;
use sources::{
    giphy::Giphy, imgur::Imgur, klipy::Klipy, kym::Kym, myinstants::MyInstants, pexels::Pexels,
    reddit::Reddit, tenor::Tenor, ytsearch::YtSearch, SearchSource,
};

pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_drag::init())
        .register_asynchronous_uri_scheme_protocol("wzstream", |ctx, request, responder| {
            commands::handle_stream_request(ctx.app_handle(), request, responder);
        })
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data)?;

            // every source is one module behind the same trait
            let source_list: Vec<Arc<dyn SearchSource>> = vec![
                Arc::new(MyInstants),
                Arc::new(Klipy),
                Arc::new(Tenor),
                Arc::new(Giphy),
                Arc::new(Reddit),
                Arc::new(Imgur),
                Arc::new(Kym),
                Arc::new(YtSearch),
                Arc::new(Pexels),
            ];
            let descriptors: Vec<_> = source_list.iter().map(|s| s.descriptor()).collect();

            // hard invariant: no developer key ever ships in a build
            security::assert_no_embedded_credentials(&descriptors);

            let default_collection_dir = dirs::home_dir()
                .unwrap_or_else(|| app_data.clone())
                .join("WizSearch");

            let db = Arc::new(Db::open(&app_data.join("wizsearch.db"))?);
            let settings = Arc::new(SettingsStore::new(settings::build_registry(
                &descriptors,
                &default_collection_dir.to_string_lossy(),
            )));
            settings.attach_backend(Box::new(db.clone()))?;

            let search = Arc::new(SearchHost::new(
                settings.clone(),
                app_data.clone(),
                source_list,
            ));
            let client = reqwest::Client::new();

            // loopback media server: webkitgtk's GStreamer path can't read
            // custom URI schemes, so previews stream over 127.0.0.1 instead
            let stream_state = Arc::new(preview::StreamState {
                client: client.clone(),
                descriptors: search.descriptors(),
                settings: settings.clone(),
                default_collection_dir: default_collection_dir.clone(),
            });
            let port = settings.i64_or("preview.stream_port", 0).clamp(0, 65535) as u16;
            let stream = preview::server::start(stream_state, port)
                .map_err(|e| format!("stream server failed to start: {e}"))?;

            let state = AppState {
                db,
                settings: settings.clone(),
                search,
                client: client.clone(),
                app_data: app_data.clone(),
                default_collection_dir,
                stream,
            };
            app.manage(state);

            // first-run sidecar download in the background (a setting, on by default)
            if settings.bool_or("sidecars.auto_download", true) {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = app_handle.state::<AppState>();
                    match sidecars::ensure_all(&state.client, &state.app_data).await {
                        Ok(statuses) => {
                            use tauri::Emitter;
                            let _ = app_handle.emit("sidecar://updated", &statuses);
                        }
                        Err(e) => log::warn!("sidecar setup: {e}"),
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search_start,
            commands::search_more,
            commands::sources_list,
            commands::collect_item,
            commands::collection_search,
            commands::collection_delete,
            commands::asset_set_tags,
            commands::asset_set_favorite,
            commands::settings_defs,
            commands::settings_values,
            commands::settings_set,
            commands::secret_set,
            commands::secret_exists,
            commands::secret_clear,
            commands::sidecars_status,
            commands::sidecars_ensure,
            commands::ytdlp_update,
            commands::asset_path,
            commands::stream_base,
        ])
        .run(tauri::generate_context!())
        .expect("error while running wizsearch");
}
