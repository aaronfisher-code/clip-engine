use crate::models::{AudioTrack, Clip, PublishJob, Selection};
use anyhow::Context;
use clip_engine_recorder_protocol::RecorderConfig;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Database {
    path: PathBuf,
}

#[derive(Deserialize)]
struct LegacyDatabase {
    #[serde(default)]
    clips: Vec<Clip>,
    #[serde(default)]
    jobs: Vec<PublishJob>,
}

impl Database {
    pub fn initialize(path: PathBuf, legacy_path: Option<&Path>) -> anyhow::Result<Self> {
        let database = Self { path };
        let connection = database.connect()?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS settings (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS clips (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               source_path TEXT NOT NULL,
               fingerprint TEXT NOT NULL UNIQUE,
               created_at TEXT NOT NULL,
               imported_at TEXT NOT NULL,
               size INTEGER NOT NULL,
               duration REAL NOT NULL,
               width INTEGER NOT NULL,
               height INTEGER NOT NULL,
               fps REAL NOT NULL,
               video_codec TEXT NOT NULL,
               audio_tracks TEXT NOT NULL,
               preview_status TEXT NOT NULL,
               preview_path TEXT,
               preview_error TEXT
             );
             CREATE TABLE IF NOT EXISTS jobs (
               id TEXT PRIMARY KEY,
               clip_id TEXT NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
               status TEXT NOT NULL,
               progress REAL NOT NULL,
               created_at TEXT NOT NULL,
               output_name TEXT NOT NULL,
               selection TEXT,
               published_at TEXT,
               expires_at TEXT,
               url TEXT,
               media_url TEXT,
               thumbnail_url TEXT,
               remote_clip_id TEXT,
               error TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_clips_created ON clips(created_at DESC);
             CREATE INDEX IF NOT EXISTS idx_jobs_clip ON jobs(clip_id, created_at DESC);",
        )?;
        connection.execute(
            "UPDATE clips SET preview_status = 'pending', preview_path = NULL, preview_error = NULL
             WHERE preview_status = 'processing'",
            [],
        )?;
        let preview_version = connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'preview_format_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if preview_version.as_deref() != Some("5") {
            connection.execute(
                "UPDATE clips SET preview_status = 'pending', preview_path = NULL, preview_error = NULL",
                [],
            )?;
            connection.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('preview_format_version', '5')",
                [],
            )?;
        }
        drop(connection);
        if let Some(legacy) = legacy_path {
            database.import_legacy_once(legacy)?;
        }
        database.fail_interrupted_jobs()?;
        Ok(database)
    }

    fn connect(&self) -> anyhow::Result<Connection> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(connection)
    }

    fn import_legacy_once(&self, path: &Path) -> anyhow::Result<()> {
        if !path.is_file() || self.setting("legacy_imported")?.is_some() {
            return Ok(());
        }
        let parsed: LegacyDatabase = serde_json::from_slice(&std::fs::read(path)?)
            .context("The legacy clip-engine.json file is invalid")?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        for clip in &parsed.clips {
            insert_clip(&transaction, clip)?;
        }
        for job in &parsed.jobs {
            insert_job(&transaction, job)?;
        }
        transaction.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('legacy_imported', ?1)",
            [chrono::Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
        let backup = path.with_extension(format!(
            "json.migrated-{}",
            chrono::Utc::now().format("%Y%m%d%H%M%S")
        ));
        std::fs::copy(path, backup)?;
        Ok(())
    }

    fn fail_interrupted_jobs(&self) -> anyhow::Result<()> {
        self.connect()?.execute(
            "UPDATE jobs SET status = 'failed', error = 'The app stopped before this job finished.'
             WHERE status IN ('queued', 'transcoding', 'uploading')",
            [],
        )?;
        Ok(())
    }

    pub fn setting(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .connect()?
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn put_setting(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.connect()?.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn recorder_config(&self) -> anyhow::Result<Option<RecorderConfig>> {
        let Some(value) = self.setting("recorder_config")? else {
            return Ok(None);
        };
        let config: RecorderConfig =
            serde_json::from_str(&value).context("The saved recorder configuration is invalid")?;
        let migrated = config.clone().normalize();
        if migrated != config {
            self.put_recorder_config(&migrated)?;
        }
        Ok(Some(migrated))
    }

    pub fn put_recorder_config(&self, config: &RecorderConfig) -> anyhow::Result<()> {
        let config = config.clone().normalize();
        let value = serde_json::to_string(&config)?;
        self.put_setting("recorder_config", &value)
    }

    pub fn clips(&self) -> anyhow::Result<Vec<Clip>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, name, source_path, fingerprint, created_at, imported_at, size, duration, width, height,
             fps, video_codec, audio_tracks, preview_status, preview_path, preview_error FROM clips ORDER BY created_at DESC",
        )?;
        let rows = statement.query_map([], clip_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn clip(&self, id: &str) -> anyhow::Result<Option<Clip>> {
        let connection = self.connect()?;
        Ok(connection.query_row(
            "SELECT id, name, source_path, fingerprint, created_at, imported_at, size, duration, width, height,
             fps, video_codec, audio_tracks, preview_status, preview_path, preview_error FROM clips WHERE id = ?1",
            [id], clip_from_row,
        ).optional()?)
    }

    pub fn clip_by_fingerprint(&self, fingerprint: &str) -> anyhow::Result<Option<Clip>> {
        let connection = self.connect()?;
        Ok(connection.query_row(
            "SELECT id, name, source_path, fingerprint, created_at, imported_at, size, duration, width, height,
             fps, video_codec, audio_tracks, preview_status, preview_path, preview_error FROM clips WHERE fingerprint = ?1",
            [fingerprint], clip_from_row,
        ).optional()?)
    }

    pub fn put_clip(&self, clip: &Clip) -> anyhow::Result<()> {
        insert_clip(&self.connect()?, clip)?;
        Ok(())
    }

    pub fn delete_clip(&self, id: &str) -> anyhow::Result<()> {
        self.connect()?
            .execute("DELETE FROM clips WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn jobs(&self) -> anyhow::Result<Vec<PublishJob>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, clip_id, status, progress, created_at, output_name, selection, published_at, expires_at,
             url, media_url, thumbnail_url, remote_clip_id, error FROM jobs ORDER BY created_at DESC",
        )?;
        let rows = statement.query_map([], job_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn job(&self, id: &str) -> anyhow::Result<Option<PublishJob>> {
        let connection = self.connect()?;
        Ok(connection.query_row(
            "SELECT id, clip_id, status, progress, created_at, output_name, selection, published_at, expires_at,
             url, media_url, thumbnail_url, remote_clip_id, error FROM jobs WHERE id = ?1",
            [id], job_from_row,
        ).optional()?)
    }

    pub fn jobs_for_clip(&self, clip_id: &str) -> anyhow::Result<Vec<PublishJob>> {
        Ok(self
            .jobs()?
            .into_iter()
            .filter(|job| job.clip_id == clip_id)
            .collect())
    }

    pub fn put_job(&self, job: &PublishJob) -> anyhow::Result<()> {
        insert_job(&self.connect()?, job)?;
        Ok(())
    }

    pub fn delete_job(&self, id: &str) -> anyhow::Result<()> {
        self.connect()?
            .execute("DELETE FROM jobs WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn delete_failed_jobs_for_clip(&self, clip_id: &str) -> anyhow::Result<()> {
        self.connect()?.execute(
            "DELETE FROM jobs WHERE clip_id = ?1 AND status = 'failed'",
            [clip_id],
        )?;
        Ok(())
    }
}

fn insert_clip(connection: &Connection, clip: &Clip) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO clips (id, name, source_path, fingerprint, created_at, imported_at, size, duration, width, height,
         fps, video_codec, audio_tracks, preview_status, preview_path, preview_error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, source_path=excluded.source_path, fingerprint=excluded.fingerprint,
         created_at=excluded.created_at, imported_at=excluded.imported_at, size=excluded.size, duration=excluded.duration,
         width=excluded.width, height=excluded.height, fps=excluded.fps, video_codec=excluded.video_codec,
         audio_tracks=excluded.audio_tracks, preview_status=excluded.preview_status, preview_path=excluded.preview_path,
         preview_error=excluded.preview_error",
        params![clip.id, clip.name, clip.source_path, clip.fingerprint, clip.created_at, clip.imported_at, clip.size,
            clip.duration, clip.width, clip.height, clip.fps, clip.video_codec, serde_json::to_string(&clip.audio_tracks)?,
            clip.preview_status, clip.preview_path, clip.preview_error],
    )?;
    Ok(())
}

fn insert_job(connection: &Connection, job: &PublishJob) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO jobs (id, clip_id, status, progress, created_at, output_name, selection, published_at, expires_at,
         url, media_url, thumbnail_url, remote_clip_id, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(id) DO UPDATE SET status=excluded.status, progress=excluded.progress, selection=excluded.selection,
         published_at=excluded.published_at, expires_at=excluded.expires_at, url=excluded.url,
         media_url=excluded.media_url, thumbnail_url=excluded.thumbnail_url, remote_clip_id=excluded.remote_clip_id,
         error=excluded.error",
        params![job.id, job.clip_id, job.status, job.progress, job.created_at, job.output_name,
            job.selection.as_ref().map(serde_json::to_string).transpose()?, job.published_at, job.expires_at,
            job.url, job.media_url, job.thumbnail_url, job.remote_clip_id, job.error],
    )?;
    Ok(())
}

fn clip_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Clip> {
    let audio: String = row.get(12)?;
    Ok(Clip {
        id: row.get(0)?,
        name: row.get(1)?,
        source_path: row.get(2)?,
        fingerprint: row.get(3)?,
        created_at: row.get(4)?,
        imported_at: row.get(5)?,
        size: row.get(6)?,
        duration: row.get(7)?,
        width: row.get(8)?,
        height: row.get(9)?,
        fps: row.get(10)?,
        video_codec: row.get(11)?,
        audio_tracks: serde_json::from_str::<Vec<AudioTrack>>(&audio).unwrap_or_default(),
        preview_status: row.get(13)?,
        preview_path: row.get(14)?,
        preview_error: row.get(15)?,
    })
}

fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PublishJob> {
    let selection: Option<String> = row.get(6)?;
    Ok(PublishJob {
        id: row.get(0)?,
        clip_id: row.get(1)?,
        status: row.get(2)?,
        progress: row.get(3)?,
        created_at: row.get(4)?,
        output_name: row.get(5)?,
        selection: selection.and_then(|value| serde_json::from_str::<Selection>(&value).ok()),
        published_at: row.get(7)?,
        expires_at: row.get(8)?,
        url: row.get(9)?,
        media_url: row.get(10)?,
        thumbnail_url: row.get(11)?,
        remote_clip_id: row.get(12)?,
        error: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_clips_without_losing_audio_tracks() {
        let directory =
            std::env::temp_dir().join(format!("clip-engine-db-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let database = Database::initialize(directory.join("test.sqlite3"), None).unwrap();
        let clip = Clip {
            id: "clip".into(),
            name: "clip.mkv".into(),
            source_path: "/tmp/clip.mkv".into(),
            fingerprint: "fp".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            imported_at: "2026-01-01T00:00:00Z".into(),
            size: 1,
            duration: 1.0,
            width: 1920,
            height: 1080,
            fps: 120.0,
            video_codec: "h264".into(),
            audio_tracks: vec![AudioTrack {
                stream_index: 1,
                ordinal: 0,
                codec: "aac".into(),
                channels: 2,
                channel_layout: None,
                title: Some("Game".into()),
                language: None,
            }],
            preview_status: "pending".into(),
            preview_path: None,
            preview_error: None,
        };
        database.put_clip(&clip).unwrap();
        assert_eq!(
            database.clip("clip").unwrap().unwrap().audio_tracks[0]
                .title
                .as_deref(),
            Some("Game")
        );
        database
            .put_setting("source_directory", "/videos/inbox")
            .unwrap();
        assert_eq!(
            database.setting("source_directory").unwrap().as_deref(),
            Some("/videos/inbox")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn persists_recorder_configuration_as_json() {
        let directory =
            std::env::temp_dir().join(format!("clip-engine-recorder-db-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let database = Database::initialize(directory.join("database.sqlite3"), None).unwrap();
        let config = RecorderConfig {
            fps: clip_engine_recorder_protocol::Rational::new(240, 1),
            replay_seconds: 45,
            ..RecorderConfig::default()
        };
        database.put_recorder_config(&config).unwrap();
        assert_eq!(database.recorder_config().unwrap(), Some(config));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn migrates_legacy_recorder_quality_settings_without_overwriting_bitrate() {
        let directory = std::env::temp_dir().join(format!(
            "clip-engine-recorder-migrate-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let database = Database::initialize(directory.join("database.sqlite3"), None).unwrap();
        let legacy = serde_json::json!({
            "schemaVersion": 1,
            "screenId": "display-1",
            "outputWidth": 2560,
            "outputHeight": 1440,
            "fps": { "numerator": 144, "denominator": 1 },
            "replaySeconds": 45,
            "videoEncoder": "obs_nvenc",
            "videoBitrateKbps": 75000,
            "audioEncoder": "ffmpeg_aac",
            "audioBitrateKbps": 192
        });
        database
            .put_setting("recorder_config", &legacy.to_string())
            .unwrap();

        let migrated = database.recorder_config().unwrap().unwrap();
        assert_eq!(migrated.schema_version, 2);
        assert_eq!(
            migrated.mode,
            clip_engine_recorder_protocol::RecorderMode::Advanced
        );
        assert_eq!(migrated.video_bitrate_kbps, 75_000);
        assert_eq!(migrated.audio_bitrate_kbps, 192);
        assert!(!migrated.match_display);
        assert!(!migrated.match_display_fps);

        let saved: serde_json::Value =
            serde_json::from_str(&database.setting("recorder_config").unwrap().unwrap()).unwrap();
        assert_eq!(saved["schemaVersion"], 2);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
