use crate::cloud::{
    self, AccessRequest, CloudClient, CloudClip, CloudUser, DeviceSession, UploadIntent,
};
use crate::database::Database;
use crate::media;
use crate::models::{AppConfig, Clip, ExportConfig, PublishJob, Selection};
use crate::paths::AppPaths;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub paths: AppPaths,
    pub database: Database,
    pub cloud: CloudClient,
    pub encoder: Arc<RwLock<String>>,
    pub quality: i64,
    pub media_base_url: Option<String>,
    pub asset_scope: tauri::scope::fs::Scope,
}

type CommandResult<T> = Result<T, String>;
fn command_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> CommandResult<AppConfig> {
    let encoder = state.encoder.read().await.clone();
    Ok(AppConfig {
        source_directory: state.paths.source.to_string_lossy().to_string(),
        audio_track_labels: state
            .database
            .setting("audio_track_labels")
            .map_err(command_error)?
            .unwrap_or_else(|| "Game / System,Discord,Microphone".into())
            .split(',')
            .map(|value| value.trim().to_string())
            .collect(),
        authenticated: state.cloud.authenticated(),
        pending_access_request: state.cloud.pending_access_request(),
        r2_configured: state.cloud.authenticated(),
        public_base_url: Some("https://clips.dab.dev".into()),
        api_base_url: state
            .database
            .setting("api_base_url")
            .map_err(command_error)?
            .unwrap_or_else(|| "https://api.clips.dab.dev".into()),
        media_base_url: state.media_base_url.clone(),
        platform: std::env::consts::OS.into(),
        export: ExportConfig {
            width: 1920,
            height: 1080,
            fps: 120,
            codec: encoder,
            crf: state.quality,
        },
    })
}

#[tauri::command]
pub fn list_clips(state: State<'_, AppState>) -> CommandResult<Vec<Clip>> {
    state.database.clips().map_err(command_error)
}

#[tauri::command]
pub fn list_jobs(state: State<'_, AppState>) -> CommandResult<Vec<PublishJob>> {
    state.database.jobs().map_err(command_error)
}

fn schedule_preview(state: &AppState, mut clip: Clip, force: bool) -> anyhow::Result<Clip> {
    let preview_path = state.paths.previews.join(format!("{}.mp4", clip.id));
    let thumbnail_path = state.paths.previews.join(format!("{}.jpg", clip.id));
    if !force
        && clip.preview_status == "ready"
        && preview_path.is_file()
        && thumbnail_path.is_file()
    {
        return Ok(clip);
    }
    if clip.preview_status == "processing" {
        return Ok(clip);
    }
    clip.preview_status = "processing".into();
    clip.preview_path = None;
    clip.preview_error = None;
    state.database.put_clip(&clip)?;
    let task_state = state.clone();
    let mut task_clip = clip.clone();
    let source = PathBuf::from(&clip.source_path);
    tauri::async_runtime::spawn(async move {
        match media::make_preview(&task_state.paths, &source, &preview_path, &thumbnail_path).await
        {
            Ok(()) => {
                task_clip.preview_status = "ready".into();
                task_clip.preview_path = Some(preview_path.to_string_lossy().to_string());
                task_clip.preview_error = None;
            }
            Err(error) => {
                task_clip.preview_status = "failed".into();
                task_clip.preview_error = Some(error.to_string());
            }
        }
        let _ = task_state.database.put_clip(&task_clip);
    });
    Ok(clip)
}

#[tauri::command]
pub fn prepare_preview(
    id: String,
    force: Option<bool>,
    state: State<'_, AppState>,
) -> CommandResult<Clip> {
    let clip = state
        .database
        .clip(&id)
        .map_err(command_error)?
        .ok_or_else(|| "Clip not found".to_string())?;
    schedule_preview(&state, clip, force.unwrap_or(false)).map_err(command_error)
}

async fn register_path(state: &AppState, path: PathBuf) -> anyhow::Result<Clip> {
    if !path.is_file() {
        anyhow::bail!("{} is not a file", path.display());
    }
    let metadata = tokio::fs::metadata(&path).await?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_millis())
        .unwrap_or(0);
    let fingerprint = format!("{}:{}:{}", path.to_string_lossy(), metadata.len(), modified);
    if let Some(clip) = state.database.clip_by_fingerprint(&fingerprint)? {
        state.asset_scope.allow_file(&path)?;
        return Ok(clip);
    }
    let clip = media::probe(&state.paths, &path, None).await?;
    state.asset_scope.allow_file(&path)?;
    state.database.put_clip(&clip)?;
    Ok(clip)
}

#[tauri::command]
pub async fn import_clips(
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> CommandResult<Vec<Clip>> {
    let mut imported = Vec::new();
    for path in paths {
        imported.push(
            register_path(&state, PathBuf::from(path))
                .await
                .map_err(command_error)?,
        );
    }
    Ok(imported)
}

fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mkv" | "mp4" | "mov" | "webm" | "avi" | "m4v"
            )
        })
}

#[tauri::command]
pub async fn scan_clips(state: State<'_, AppState>) -> CommandResult<Vec<Clip>> {
    let mut directory = tokio::fs::read_dir(&state.paths.source)
        .await
        .map_err(command_error)?;
    while let Some(entry) = directory.next_entry().await.map_err(command_error)? {
        let path = entry.path();
        if path.is_file() && supported(&path) {
            register_path(&state, path).await.map_err(command_error)?;
        }
    }
    state.database.clips().map_err(command_error)
}

#[tauri::command]
pub async fn delete_clip(id: String, state: State<'_, AppState>) -> CommandResult<u64> {
    let clip = state
        .database
        .clip(&id)
        .map_err(command_error)?
        .ok_or_else(|| "Clip not found".to_string())?;
    let jobs = state.database.jobs_for_clip(&id).map_err(command_error)?;
    if jobs
        .iter()
        .any(|job| matches!(job.status.as_str(), "queued" | "transcoding" | "uploading"))
    {
        return Err("Wait for active work to finish before deleting this clip.".into());
    }
    let mut removed = 0;
    if let Some(preview) = clip.preview_path {
        if Path::new(&preview).starts_with(&state.paths.previews)
            && tokio::fs::remove_file(preview).await.is_ok()
        {
            removed += 1;
        }
    }
    let thumbnail = state.paths.previews.join(format!("{}.jpg", clip.id));
    if thumbnail.starts_with(&state.paths.previews)
        && tokio::fs::remove_file(thumbnail).await.is_ok()
    {
        removed += 1;
    }
    for job in jobs {
        let path = state.paths.exports.join(&job.output_name);
        if path.starts_with(&state.paths.exports) && tokio::fs::remove_file(path).await.is_ok() {
            removed += 1;
        }
    }
    state.database.delete_clip(&id).map_err(command_error)?;
    Ok(removed)
}

#[tauri::command]
pub async fn delete_job(id: String, state: State<'_, AppState>) -> CommandResult<u64> {
    let job = state
        .database
        .job(&id)
        .map_err(command_error)?
        .ok_or_else(|| "Published version not found".to_string())?;
    if matches!(job.status.as_str(), "queued" | "transcoding" | "uploading") {
        return Err("Wait for active work to finish.".into());
    }
    if let Some(remote_id) = &job.remote_clip_id {
        state
            .cloud
            .delete_clip(remote_id)
            .await
            .map_err(command_error)?;
    }
    let output = state.paths.exports.join(&job.output_name);
    let removed = if output.starts_with(&state.paths.exports)
        && tokio::fs::remove_file(output).await.is_ok()
    {
        1
    } else {
        0
    };
    state.database.delete_job(&id).map_err(command_error)?;
    Ok(removed)
}

#[tauri::command]
pub async fn publish_clip(
    clip_id: String,
    selection: Selection,
    state: State<'_, AppState>,
) -> CommandResult<PublishJob> {
    if !state.cloud.authenticated() {
        return Err("Sign in with an invitation before publishing.".into());
    }
    let clip = state
        .database
        .clip(&clip_id)
        .map_err(command_error)?
        .ok_or_else(|| "Clip not found".to_string())?;
    if selection.start < 0.0
        || selection.end <= selection.start
        || selection.end > clip.duration + 0.05
    {
        return Err("The trim selection is invalid.".into());
    }
    let valid = clip
        .audio_tracks
        .iter()
        .map(|track| track.stream_index)
        .collect::<std::collections::HashSet<_>>();
    if selection
        .audio_stream_indexes
        .iter()
        .any(|index| !valid.contains(index))
    {
        return Err("An audio selection is invalid.".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let output_name = format!("{}-{}.mp4", media::safe_base_name(&clip.name), &id[..8]);
    let job = PublishJob {
        id,
        clip_id,
        status: "queued".into(),
        progress: 0.0,
        created_at: chrono::Utc::now().to_rfc3339(),
        output_name,
        selection: Some(selection),
        published_at: None,
        expires_at: None,
        url: None,
        media_url: None,
        thumbnail_url: None,
        remote_clip_id: None,
        error: None,
    };
    state.database.put_job(&job).map_err(command_error)?;
    let task_state = state.inner().clone();
    let task_job = job.clone();
    tauri::async_runtime::spawn(async move {
        run_publish(task_state, clip, task_job).await;
    });
    Ok(job)
}

async fn run_publish(state: AppState, clip: Clip, mut job: PublishJob) {
    let result: anyhow::Result<()> = async {
        let selection = job
            .selection
            .clone()
            .expect("publish jobs always contain a selection");
        let output = state.paths.exports.join(&job.output_name);
        let thumbnail = state.paths.exports.join(format!(".{}.jpg", job.id));
        job.status = "transcoding".into();
        state.database.put_job(&job)?;
        let encoder = state.encoder.read().await.clone();
        media::export_clip(
            &state.paths,
            Path::new(&clip.source_path),
            &output,
            &selection,
            clip.fps,
            &encoder,
            state.quality,
            |progress| {
                job.progress = progress * 0.72;
                let _ = state.database.put_job(&job);
            },
        )
        .await?;
        media::make_thumbnail(
            &state.paths,
            &output,
            &thumbnail,
            selection.end - selection.start,
        )
        .await?;
        let video_size = tokio::fs::metadata(&output).await?.len();
        let thumbnail_size = tokio::fs::metadata(&thumbnail).await?.len();
        let created = state
            .cloud
            .create_upload(&UploadIntent {
                title: Path::new(&clip.name)
                    .file_stem()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or("Untitled clip"),
                video_size,
                thumbnail_size,
                duration: selection.end - selection.start,
                width: 1920,
                height: 1080,
                fps: clip.fps.clamp(1.0, 120.0),
            })
            .await?;
        job.status = "uploading".into();
        job.remote_clip_id = Some(created.clip_id.clone());
        job.progress = 0.74;
        state.database.put_job(&job)?;
        cloud::upload_file(
            &created.credentials,
            &created.video_key,
            &output,
            "video/mp4",
            |progress| {
                job.progress = 0.74 + progress * 0.23;
                let _ = state.database.put_job(&job);
            },
        )
        .await?;
        cloud::upload_file(
            &created.credentials,
            &created.thumbnail_key,
            &thumbnail,
            "image/jpeg",
            |_| {},
        )
        .await?;
        let completion = state.cloud.complete_upload(&created.upload_id).await?;
        let remote = state
            .cloud
            .clips()
            .await?
            .into_iter()
            .find(|remote| remote.id == created.clip_id)
            .ok_or_else(|| {
                anyhow::anyhow!("The published clip was not returned by the cloud library")
            })?;
        job.status = "complete".into();
        job.progress = 1.0;
        job.published_at = remote.published_at;
        job.expires_at = Some(completion.expires_at);
        job.url = remote.url;
        job.media_url = remote.media_url;
        job.thumbnail_url = remote.thumbnail_url;
        state.database.put_job(&job)?;
        let _ = tokio::fs::remove_file(thumbnail).await;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        if let Some(remote_clip_id) = job.remote_clip_id.take() {
            let _ = state.cloud.delete_clip(&remote_clip_id).await;
        }
        job.status = "failed".into();
        job.error = Some(format!("{error:#}"));
        let _ = state.database.put_job(&job);
    }
}

#[tauri::command]
pub async fn redeem_invite(
    invite_token: String,
    username: String,
    credential_secret: String,
    display_name: String,
    device_name: String,
    state: State<'_, AppState>,
) -> CommandResult<DeviceSession> {
    state
        .cloud
        .redeem(
            &invite_token,
            &username,
            &credential_secret,
            &display_name,
            &device_name,
        )
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn login(
    username: String,
    credential_secret: String,
    owner_token: Option<String>,
    device_name: String,
    state: State<'_, AppState>,
) -> CommandResult<DeviceSession> {
    state
        .cloud
        .login(
            &username,
            &credential_secret,
            owner_token.as_deref(),
            &device_name,
        )
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn validate_password_reset(
    invite_token: String,
    username: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .cloud
        .validate_password_reset(&invite_token, &username)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> CommandResult<()> {
    state.cloud.logout().await.map_err(command_error)
}

#[tauri::command]
pub async fn current_user(state: State<'_, AppState>) -> CommandResult<CloudUser> {
    state.cloud.me().await.map_err(command_error)
}

#[tauri::command]
pub async fn cloud_clips(state: State<'_, AppState>) -> CommandResult<Vec<CloudClip>> {
    state.cloud.clips().await.map_err(command_error)
}

#[tauri::command]
pub async fn extend_cloud_clip(id: String, state: State<'_, AppState>) -> CommandResult<String> {
    state
        .cloud
        .extend_clip(&id)
        .await
        .map(|extension| extension.expires_at)
        .map_err(command_error)
}

#[tauri::command]
pub async fn request_access(
    username: String,
    display_name: String,
    credential_secret: String,
    state: State<'_, AppState>,
) -> CommandResult<AccessRequest> {
    state
        .cloud
        .request_access(&username, &display_name, &credential_secret)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn access_request_status(state: State<'_, AppState>) -> CommandResult<AccessRequest> {
    state
        .cloud
        .access_request_status()
        .await
        .map_err(command_error)
}

#[tauri::command]
pub fn clear_access_request(state: State<'_, AppState>) -> CommandResult<()> {
    state.cloud.clear_access_request().map_err(command_error)
}

#[tauri::command]
pub async fn admin_access_requests(
    state: State<'_, AppState>,
) -> CommandResult<Vec<AccessRequest>> {
    state.cloud.access_requests().await.map_err(command_error)
}

#[tauri::command]
pub async fn review_access_request(
    id: String,
    decision: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .cloud
        .review_access_request(&id, &decision)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn create_password_reset(
    id: String,
    state: State<'_, AppState>,
) -> CommandResult<serde_json::Value> {
    state
        .cloud
        .create_password_reset(&id)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn admin_users(state: State<'_, AppState>) -> CommandResult<Vec<serde_json::Value>> {
    state.cloud.users().await.map_err(command_error)
}

#[tauri::command]
pub async fn set_user_status(
    id: String,
    status: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .cloud
        .set_user_status(&id, &status)
        .await
        .map_err(command_error)
}
