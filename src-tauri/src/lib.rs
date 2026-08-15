mod cloud;
mod commands;
mod database;
mod media;
mod media_server;
mod models;
mod paths;

use commands::AppState;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data = app.path().app_local_data_dir()?;
            let source = app
                .path()
                .video_dir()
                .unwrap_or_else(|_| data.clone())
                .join("Clip Engine")
                .join("Inbox");
            let resource = app.path().resource_dir()?;
            let paths = paths::AppPaths::new(data, source, &resource)?;
            let legacy = std::env::current_dir()
                .ok()
                .map(|directory| directory.join("data").join("clip-engine.json"));
            let database =
                database::Database::initialize(paths.database.clone(), legacy.as_deref())?;
            let asset_scope = app.asset_protocol_scope();
            for clip in database.clips()? {
                asset_scope.allow_file(&clip.source_path)?;
            }
            let media_base_url = Some(
                media_server::MediaServer::start(
                    paths.previews.clone(),
                    database.clone(),
                    paths.ffmpeg.clone(),
                )?
                .base_url,
            );
            let api_base = database.setting("api_base_url")?.unwrap_or_else(|| {
                option_env!("CLIP_ENGINE_API_URL")
                    .unwrap_or("https://api.clips.dab.dev")
                    .to_string()
            });
            let cloud = cloud::CloudClient::new(api_base)?;
            let encoder = Arc::new(RwLock::new("libx264".to_string()));
            app.manage(AppState {
                paths: paths.clone(),
                database,
                cloud,
                encoder: encoder.clone(),
                quality: 20,
                media_base_url,
                asset_scope,
            });
            tauri::async_runtime::spawn(async move {
                *encoder.write().await = media::detect_encoder(&paths).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::list_clips,
            commands::list_jobs,
            commands::prepare_preview,
            commands::import_clips,
            commands::scan_clips,
            commands::delete_clip,
            commands::delete_job,
            commands::publish_clip,
            commands::redeem_invite,
            commands::login,
            commands::validate_password_reset,
            commands::logout,
            commands::current_user,
            commands::cloud_clips,
            commands::extend_cloud_clip,
            commands::request_access,
            commands::access_request_status,
            commands::clear_access_request,
            commands::admin_access_requests,
            commands::review_access_request,
            commands::create_password_reset,
            commands::admin_users,
            commands::set_user_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Clip Engine");
}
