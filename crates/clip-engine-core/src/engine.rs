use crate::cloud::{
    self, AccessRequest, AdminUser, CloudClient, CloudClip, CloudUser, DeviceSession,
    PasswordReset, UploadIntent,
};
use crate::database::Database;
use crate::media;
use crate::models::{AppConfig, Clip, ExportConfig, PublishJob, Selection};
use crate::paths::AppPaths;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct Engine {
    pub paths: AppPaths,
    pub database: Database,
    pub cloud: CloudClient,
    pub encoder: Arc<RwLock<String>>,
    pub quality: i64,
    runtime: Arc<tokio::runtime::Handle>,
}

impl Engine {
    pub fn initialize(handle: tokio::runtime::Handle) -> anyhow::Result<Self> {
        let mut paths = AppPaths::discover()?;
        let legacy = std::env::current_dir()
            .ok()
            .map(|directory| directory.join("data").join("clip-engine.json"));
        let database = Database::initialize(paths.database.clone(), legacy.as_deref())?;
        if let Some(saved) = database.setting("source_directory")? {
            paths.source = PathBuf::from(saved);
        } else {
            database.put_setting("source_directory", &paths.source.to_string_lossy())?;
        }
        let api_base = database.setting("api_base_url")?.unwrap_or_else(|| {
            option_env!("CLIP_ENGINE_API_URL")
                .unwrap_or("https://api.clips.dab.dev")
                .to_string()
        });
        let cloud = CloudClient::new(api_base)?;
        let encoder = Arc::new(RwLock::new("libx264".to_string()));
        let engine = Self {
            paths: paths.clone(),
            database,
            cloud,
            encoder: encoder.clone(),
            quality: 20,
            runtime: Arc::new(handle),
        };
        let detect_paths = engine.paths.clone();
        engine.runtime.spawn(async move {
            *encoder.write().await = media::detect_encoder(&detect_paths).await;
        });
        Ok(engine)
    }

    pub fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.runtime.spawn(future);
    }

    pub fn config(&self) -> anyhow::Result<AppConfig> {
        let encoder = self
            .encoder
            .try_read()
            .map(|value| value.clone())
            .unwrap_or_else(|_| "libx264".into());
        Ok(AppConfig {
            source_directory: self.source_directory().to_string_lossy().to_string(),
            audio_track_labels: self
                .database
                .setting("audio_track_labels")?
                .unwrap_or_else(|| "Game / System,Discord,Microphone".into())
                .split(',')
                .map(|value| value.trim().to_string())
                .collect(),
            authenticated: self.cloud.authenticated(),
            pending_access_request: self.cloud.pending_access_request(),
            r2_configured: self.cloud.authenticated(),
            public_base_url: Some("https://clips.dab.dev".into()),
            api_base_url: self.database.setting("api_base_url")?.unwrap_or_else(|| {
                option_env!("CLIP_ENGINE_API_URL")
                    .unwrap_or("https://api.clips.dab.dev")
                    .to_string()
            }),
            media_base_url: None,
            platform: std::env::consts::OS.into(),
            export: ExportConfig {
                width: 1920,
                height: 1080,
                fps: 120,
                codec: encoder,
                crf: self.quality,
            },
        })
    }

    pub fn clips(&self) -> anyhow::Result<Vec<Clip>> {
        self.database.clips()
    }

    pub fn jobs(&self) -> anyhow::Result<Vec<PublishJob>> {
        self.database.jobs()
    }

    pub fn thumbnail_path(&self, clip: &Clip) -> PathBuf {
        clip.preview_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.paths.previews.join(format!("{}.jpg", clip.id)))
    }

    pub fn prepare_preview(&self, id: &str, force: bool) -> anyhow::Result<Clip> {
        let clip = self
            .database
            .clip(id)?
            .ok_or_else(|| anyhow::anyhow!("Clip not found"))?;
        self.schedule_preview(clip, force)
    }

    fn schedule_preview(&self, mut clip: Clip, force: bool) -> anyhow::Result<Clip> {
        let thumbnail_path = self.paths.previews.join(format!("{}.jpg", clip.id));
        if !force && clip.preview_status == "ready" && thumbnail_path.is_file() {
            return Ok(clip);
        }
        if clip.preview_status == "processing" {
            return Ok(clip);
        }
        clip.preview_status = "processing".into();
        clip.preview_path = None;
        clip.preview_error = None;
        self.database.put_clip(&clip)?;
        let engine = self.clone();
        let mut task_clip = clip.clone();
        let source = PathBuf::from(&clip.source_path);
        self.spawn(async move {
            match media::make_preview(
                &engine.paths,
                &source,
                &thumbnail_path,
                task_clip.duration,
            )
                .await
            {
                Ok(()) => {
                    task_clip.preview_status = "ready".into();
                    task_clip.preview_path = Some(thumbnail_path.to_string_lossy().to_string());
                    task_clip.preview_error = None;
                }
                Err(error) => {
                    task_clip.preview_status = "failed".into();
                    task_clip.preview_error = Some(error.to_string());
                }
            }
            let _ = engine.database.put_clip(&task_clip);
        });
        Ok(clip)
    }

    async fn register_path(&self, path: PathBuf) -> anyhow::Result<Clip> {
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
        if let Some(clip) = self.database.clip_by_fingerprint(&fingerprint)? {
            self.schedule_preview(clip.clone(), false)?;
            return Ok(clip);
        }
        let clip = media::probe(&self.paths, &path, None).await?;
        self.database.put_clip(&clip)?;
        self.schedule_preview(clip.clone(), false)?;
        Ok(clip)
    }

    pub async fn import_clips(&self, paths: Vec<PathBuf>) -> anyhow::Result<Vec<Clip>> {
        let mut imported = Vec::new();
        for path in paths {
            imported.push(self.register_path(path).await?);
        }
        Ok(imported)
    }

    pub fn source_directory(&self) -> PathBuf {
        self.paths.source.clone()
    }

    pub fn set_source_directory(&mut self, path: PathBuf) -> anyhow::Result<PathBuf> {
        let path = path.canonicalize().unwrap_or(path);
        if !path.is_dir() {
            anyhow::bail!("Choose an existing folder to use as the inbox.");
        }
        self.database
            .put_setting("source_directory", &path.to_string_lossy())?;
        self.paths.source = path.clone();
        Ok(path)
    }

    pub async fn scan_clips(&self) -> anyhow::Result<Vec<Clip>> {
        let source = self.source_directory();
        if !source.is_dir() {
            anyhow::bail!("Inbox folder does not exist: {}", source.display());
        }
        let mut directory = tokio::fs::read_dir(&source).await?;
        while let Some(entry) = directory.next_entry().await? {
            let path = entry.path();
            if path.is_file() && supported(&path) {
                self.register_path(path).await?;
            }
        }
        self.database.clips()
    }

    pub async fn delete_clip(&self, id: &str) -> anyhow::Result<u64> {
        let clip = self
            .database
            .clip(id)?
            .ok_or_else(|| anyhow::anyhow!("Clip not found"))?;
        let jobs = self.database.jobs_for_clip(id)?;
        if jobs
            .iter()
            .any(|job| matches!(job.status.as_str(), "queued" | "transcoding" | "uploading"))
        {
            anyhow::bail!("Wait for active work to finish before deleting this clip.");
        }
        let mut removed = 0;
        if let Some(preview) = clip.preview_path {
            if Path::new(&preview).starts_with(&self.paths.previews)
                && tokio::fs::remove_file(preview).await.is_ok()
            {
                removed += 1;
            }
        }
        let thumbnail = self.paths.previews.join(format!("{}.jpg", clip.id));
        if thumbnail.starts_with(&self.paths.previews)
            && tokio::fs::remove_file(thumbnail).await.is_ok()
        {
            removed += 1;
        }
        for job in jobs {
            let path = self.paths.exports.join(&job.output_name);
            if path.starts_with(&self.paths.exports) && tokio::fs::remove_file(path).await.is_ok() {
                removed += 1;
            }
        }
        self.database.delete_clip(id)?;
        Ok(removed)
    }

    pub async fn delete_job(&self, id: &str) -> anyhow::Result<u64> {
        let job = self
            .database
            .job(id)?
            .ok_or_else(|| anyhow::anyhow!("Published version not found"))?;
        if matches!(job.status.as_str(), "queued" | "transcoding" | "uploading") {
            anyhow::bail!("Wait for active work to finish.");
        }
        if let Some(remote_id) = &job.remote_clip_id {
            self.cloud.delete_clip(remote_id).await?;
        }
        let output = self.paths.exports.join(&job.output_name);
        let removed = if output.starts_with(&self.paths.exports)
            && tokio::fs::remove_file(output).await.is_ok()
        {
            1
        } else {
            0
        };
        self.database.delete_job(id)?;
        Ok(removed)
    }

    pub async fn delete_published(&self, clip_id: &str) -> anyhow::Result<()> {
        let jobs = self.database.jobs_for_clip(clip_id)?;
        for job in jobs {
            if job.status == "complete" {
                self.delete_job(&job.id).await?;
            } else if job.status == "failed" {
                self.database.delete_job(&job.id)?;
            }
        }
        Ok(())
    }

    pub fn publish_clip(
        &self,
        clip_id: String,
        title: String,
        selection: Selection,
    ) -> anyhow::Result<PublishJob> {
        if !self.cloud.authenticated() {
            anyhow::bail!("Sign in with an invitation before publishing.");
        }
        let clip = self
            .database
            .clip(&clip_id)?
            .ok_or_else(|| anyhow::anyhow!("Clip not found"))?;
        if selection.start < 0.0
            || selection.end <= selection.start
            || selection.end > clip.duration + 0.05
        {
            anyhow::bail!("The trim selection is invalid.");
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
            anyhow::bail!("An audio selection is invalid.");
        }
        let mut selection = selection;
        selection.export = Some(media::resolve_export_profile(
            clip.width,
            clip.height,
            clip.fps,
            &selection,
        )?);
        let title = {
            let trimmed = title.trim();
            if trimmed.is_empty() {
                "Untitled clip".to_string()
            } else {
                trimmed.chars().take(160).collect::<String>()
            }
        };
        let id = uuid::Uuid::new_v4().to_string();
        let output_name = format!("{}-{}.mp4", media::safe_base_name(&title), &id[..8]);
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
        self.database.put_job(&job)?;
        let engine = self.clone();
        let task_job = job.clone();
        self.spawn(async move {
            run_publish(engine, clip, task_job, title).await;
        });
        Ok(job)
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
        device_name: &str,
    ) -> anyhow::Result<DeviceSession> {
        let secret = crate::auth::credential_secret(username, password);
        let owner_token = username
            .trim()
            .eq_ignore_ascii_case("admin")
            .then_some(password);
        self.cloud
            .login(username, &secret, owner_token, device_name)
            .await
    }

    pub async fn request_access(
        &self,
        username: &str,
        display_name: &str,
        password: &str,
    ) -> anyhow::Result<AccessRequest> {
        let secret = crate::auth::credential_secret(username, password);
        self.cloud
            .request_access(username, display_name, &secret)
            .await
    }

    pub async fn redeem_invite(
        &self,
        invite: &str,
        username: &str,
        password: &str,
        display_name: &str,
        device_name: &str,
    ) -> anyhow::Result<DeviceSession> {
        let secret = crate::auth::credential_secret(username, password);
        self.cloud
            .redeem(
                &crate::auth::invite_token(invite),
                username,
                &secret,
                display_name,
                device_name,
            )
            .await
    }

    pub async fn validate_password_reset(&self, token: &str, username: &str) -> anyhow::Result<()> {
        self.cloud
            .validate_password_reset(&crate::auth::invite_token(token), username)
            .await
    }

    pub async fn logout(&self) -> anyhow::Result<()> {
        self.cloud.logout().await
    }

    pub async fn me(&self) -> anyhow::Result<CloudUser> {
        self.cloud.me().await
    }

    pub async fn cloud_clips(&self) -> anyhow::Result<Vec<CloudClip>> {
        self.cloud.clips().await
    }

    pub async fn extend_cloud_clip(&self, id: &str) -> anyhow::Result<String> {
        Ok(self.cloud.extend_clip(id).await?.expires_at)
    }

    pub async fn access_request_status(&self) -> anyhow::Result<AccessRequest> {
        self.cloud.access_request_status().await
    }

    pub fn clear_access_request(&self) -> anyhow::Result<()> {
        self.cloud.clear_access_request()
    }

    pub async fn admin_users(&self) -> anyhow::Result<Vec<AdminUser>> {
        self.cloud.users().await
    }

    pub async fn admin_access_requests(&self) -> anyhow::Result<Vec<AccessRequest>> {
        self.cloud.access_requests().await
    }

    pub async fn review_access_request(&self, id: &str, decision: &str) -> anyhow::Result<()> {
        self.cloud.review_access_request(id, decision).await
    }

    pub async fn create_password_reset(&self, id: &str) -> anyhow::Result<PasswordReset> {
        self.cloud.create_password_reset(id).await
    }

    pub async fn set_user_status(&self, id: &str, status: &str) -> anyhow::Result<()> {
        self.cloud.set_user_status(id, status).await
    }
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

async fn run_publish(engine: Engine, clip: Clip, mut job: PublishJob, title: String) {
    let result: anyhow::Result<()> = async {
        let selection = job
            .selection
            .clone()
            .expect("publish jobs always contain a selection");
        let profile = media::resolve_export_profile(clip.width, clip.height, clip.fps, &selection)?;
        let output = engine.paths.exports.join(&job.output_name);
        let thumbnail = engine.paths.exports.join(format!(".{}.jpg", job.id));
        job.status = "transcoding".into();
        engine.database.put_job(&job)?;
        let encoder = engine.encoder.read().await.clone();
        media::export_clip(
            &engine.paths,
            Path::new(&clip.source_path),
            &output,
            &selection,
            &profile,
            &encoder,
            engine.quality,
            |progress| {
                job.progress = progress * 0.72;
                let _ = engine.database.put_job(&job);
            },
        )
        .await?;
        media::make_thumbnail(
            &engine.paths,
            &output,
            &thumbnail,
            selection.end - selection.start,
        )
        .await?;
        let video_size = tokio::fs::metadata(&output).await?.len();
        if video_size > media::MAX_PUBLISH_BYTES {
            anyhow::bail!("The transcoded clip is over 200 MB. Choose a lower quality or shorten the trim.");
        }
        let thumbnail_size = tokio::fs::metadata(&thumbnail).await?.len();
        let created = engine
            .cloud
            .create_upload(&UploadIntent {
                title: &title,
                video_size,
                thumbnail_size,
                duration: selection.end - selection.start,
                width: profile.width,
                height: profile.height,
                fps: profile.fps as f64,
            })
            .await?;
        job.status = "uploading".into();
        job.remote_clip_id = Some(created.clip_id.clone());
        job.progress = 0.74;
        engine.database.put_job(&job)?;
        cloud::upload_file(
            &created.credentials,
            &created.video_key,
            &output,
            "video/mp4",
            |progress| {
                job.progress = 0.74 + progress * 0.23;
                let _ = engine.database.put_job(&job);
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
        let completion = engine.cloud.complete_upload(&created.upload_id).await?;
        let remote = engine
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
        engine.database.put_job(&job)?;
        let _ = engine.database.delete_failed_jobs_for_clip(&job.clip_id);
        let _ = tokio::fs::remove_file(thumbnail).await;
        Ok(())
    }
    .await;
    if let Err(error) = result {
        if let Some(remote_clip_id) = job.remote_clip_id.take() {
            let _ = engine.cloud.delete_clip(&remote_clip_id).await;
        }
        job.status = "failed".into();
        job.error = Some(format!("{error:#}"));
        let _ = engine.database.put_job(&job);
    }
}
