use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    pub stream_index: i64,
    pub ordinal: i64,
    pub codec: String,
    pub channels: i64,
    pub channel_layout: Option<String>,
    pub title: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub name: String,
    pub source_path: String,
    pub fingerprint: String,
    pub created_at: String,
    pub imported_at: String,
    pub size: i64,
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    pub fps: f64,
    pub video_codec: String,
    pub audio_tracks: Vec<AudioTrack>,
    pub preview_status: String,
    pub preview_path: Option<String>,
    pub preview_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub start: f64,
    pub end: f64,
    pub audio_stream_indexes: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishJob {
    pub id: String,
    pub clip_id: String,
    pub status: String,
    pub progress: f64,
    pub created_at: String,
    pub output_name: String,
    pub selection: Option<Selection>,
    pub published_at: Option<String>,
    pub expires_at: Option<String>,
    pub url: Option<String>,
    pub media_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub remote_clip_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportConfig {
    pub width: i64,
    pub height: i64,
    pub fps: i64,
    pub codec: String,
    pub crf: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub source_directory: String,
    pub audio_track_labels: Vec<String>,
    pub authenticated: bool,
    pub pending_access_request: bool,
    pub r2_configured: bool,
    pub public_base_url: Option<String>,
    pub api_base_url: String,
    pub media_base_url: Option<String>,
    pub platform: String,
    pub export: ExportConfig,
}

#[derive(Debug, Deserialize)]
pub struct ProbeResult {
    pub streams: Vec<ProbeStream>,
    pub format: ProbeFormat,
}

#[derive(Debug, Deserialize)]
pub struct ProbeStream {
    pub index: i64,
    pub codec_type: String,
    pub codec_name: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub avg_frame_rate: Option<String>,
    pub r_frame_rate: Option<String>,
    pub channels: Option<i64>,
    pub channel_layout: Option<String>,
    pub tags: Option<ProbeTags>,
}

#[derive(Debug, Deserialize)]
pub struct ProbeFormat {
    pub duration: Option<String>,
    pub tags: Option<ProbeTags>,
}

#[derive(Debug, Deserialize)]
pub struct ProbeTags {
    pub title: Option<String>,
    pub language: Option<String>,
    pub creation_time: Option<String>,
}
