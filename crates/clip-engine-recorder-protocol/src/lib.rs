use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashSet,
    fmt,
    io::{self, Read, Write},
};

pub const PROTOCOL_VERSION: u16 = 1;
pub const DEFAULT_SOCKET_NAME: &str = "clip-engine-recorder";
pub const MAX_FRAME_BYTES: u32 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Rational {
    pub numerator: u32,
    pub denominator: u32,
}

impl Rational {
    pub const fn new(numerator: u32, denominator: u32) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    pub fn as_f64(self) -> f64 {
        if self.denominator == 0 {
            return 0.0;
        }
        self.numerator as f64 / self.denominator as f64
    }

    pub fn validate(self) -> Result<(), String> {
        if self.numerator == 0 || self.denominator == 0 {
            return Err("Frame rate must be greater than zero.".into());
        }
        if self.as_f64() > 1_000.0 {
            return Err("Frame rate is above the supported numeric range.".into());
        }
        Ok(())
    }
}

impl Default for Rational {
    fn default() -> Self {
        Self::new(120, 1)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Hotkey {
    pub key: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub meta: bool,
}

impl Hotkey {
    pub fn normalized(mut self) -> Self {
        self.key = self.key.trim().to_ascii_uppercase();
        self
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.key.trim().is_empty() {
            return Err("A replay hotkey must include a key.".into());
        }
        if self.key.chars().any(char::is_whitespace) {
            return Err("A replay hotkey key cannot contain whitespace.".into());
        }
        Ok(())
    }
}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            key: "F8".into(),
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
        }
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            write!(formatter, "Ctrl+")?;
        }
        if self.alt {
            write!(formatter, "Alt+")?;
        }
        if self.shift {
            write!(formatter, "Shift+")?;
        }
        if self.meta {
            write!(formatter, "Meta+")?;
        }
        formatter.write_str(&self.key)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum CaptureBackend {
    WindowsGraphicsCapture,
    X11,
    PipeWire,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum AudioSourceKind {
    System,
    Application,
    Microphone,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScreenCapability {
    pub id: String,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: Option<f64>,
    pub backend: CaptureBackend,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSourceCapability {
    pub id: String,
    pub label: String,
    pub kind: AudioSourceKind,
    pub process_id: Option<u32>,
    pub available: bool,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FrameRateCapability {
    pub min: Rational,
    pub max: Rational,
    #[serde(default)]
    pub native: Vec<Rational>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EncoderCapability {
    pub id: String,
    pub label: String,
    pub hardware: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioRoute {
    pub source_id: String,
    pub track: u8,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecorderConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    #[serde(default)]
    pub screen_id: String,
    #[serde(default = "default_width")]
    pub output_width: u32,
    #[serde(default = "default_height")]
    pub output_height: u32,
    #[serde(default)]
    pub fps: Rational,
    #[serde(default = "default_replay_seconds")]
    pub replay_seconds: u32,
    #[serde(default = "default_auto")]
    pub video_encoder: String,
    #[serde(default = "default_video_bitrate")]
    pub video_bitrate_kbps: u32,
    #[serde(default = "default_audio_encoder")]
    pub audio_encoder: String,
    #[serde(default = "default_audio_bitrate")]
    pub audio_bitrate_kbps: u32,
    #[serde(default)]
    pub audio_routes: Vec<AudioRoute>,
    #[serde(default)]
    pub hotkey: Option<Hotkey>,
    #[serde(default)]
    pub output_directory: String,
}

fn default_schema_version() -> u16 {
    1
}

fn default_width() -> u32 {
    1_920
}

fn default_height() -> u32 {
    1_080
}

fn default_replay_seconds() -> u32 {
    30
}

fn default_auto() -> String {
    "auto".into()
}

fn default_video_bitrate() -> u32 {
    50_000
}

fn default_audio_encoder() -> String {
    "ffmpeg_aac".into()
}

fn default_audio_bitrate() -> u32 {
    160
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            screen_id: String::new(),
            output_width: default_width(),
            output_height: default_height(),
            fps: Rational::default(),
            replay_seconds: default_replay_seconds(),
            video_encoder: default_auto(),
            video_bitrate_kbps: default_video_bitrate(),
            audio_encoder: default_audio_encoder(),
            audio_bitrate_kbps: default_audio_bitrate(),
            audio_routes: Vec::new(),
            hotkey: Some(Hotkey::default()),
            output_directory: String::new(),
        }
    }
}

impl RecorderConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "Recorder configuration version {} is not supported.",
                self.schema_version
            ));
        }
        if !(320..=16_384).contains(&self.output_width)
            || !(180..=16_384).contains(&self.output_height)
        {
            return Err("Recorder output dimensions are outside the supported range.".into());
        }
        self.fps.validate()?;
        if self.replay_seconds == 0 {
            return Err("Replay length must be greater than zero.".into());
        }
        if self.video_bitrate_kbps == 0 {
            return Err("Video bitrate must be greater than zero.".into());
        }
        if self.audio_bitrate_kbps == 0 {
            return Err("Audio bitrate must be greater than zero.".into());
        }
        let mut used_tracks = HashSet::new();
        for route in &self.audio_routes {
            if route.source_id.trim().is_empty() {
                return Err("Audio routes must identify a source.".into());
            }
            if !(1..=6).contains(&route.track) {
                return Err("OBS supports audio tracks 1 through 6.".into());
            }
            if route.enabled && !used_tracks.insert(route.track) {
                return Err(format!(
                    "Audio track {} is assigned more than once.",
                    route.track
                ));
            }
        }
        if let Some(hotkey) = &self.hotkey {
            hotkey.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecorderCapabilities {
    pub backend: CaptureBackend,
    #[serde(default)]
    pub screens: Vec<ScreenCapability>,
    #[serde(default)]
    pub audio_sources: Vec<AudioSourceCapability>,
    #[serde(default)]
    pub video_encoders: Vec<EncoderCapability>,
    #[serde(default)]
    pub audio_encoders: Vec<EncoderCapability>,
    #[serde(default)]
    pub frame_rates: Vec<FrameRateCapability>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

impl Default for RecorderCapabilities {
    fn default() -> Self {
        Self {
            backend: CaptureBackend::Unknown,
            screens: Vec::new(),
            audio_sources: Vec::new(),
            video_encoders: Vec::new(),
            audio_encoders: Vec::new(),
            frame_rates: vec![FrameRateCapability {
                min: Rational::new(1, 1),
                max: Rational::new(1_000, 1),
                native: Vec::new(),
            }],
            diagnostics: Vec::new(),
        }
    }
}

impl RecorderCapabilities {
    pub fn supports_fps(&self, fps: Rational) -> bool {
        if fps.validate().is_err() {
            return false;
        }
        self.frame_rates.iter().any(|range| {
            let value = fps.as_f64();
            value >= range.min.as_f64() && value <= range.max.as_f64()
        })
    }

    pub fn validate_config(&self, config: &RecorderConfig) -> Result<(), String> {
        config.validate()?;
        if !self.screens.is_empty()
            && !config.screen_id.trim().is_empty()
            && !self
                .screens
                .iter()
                .any(|screen| screen.id == config.screen_id)
        {
            return Err(format!(
                "The selected screen '{}' is no longer available.",
                config.screen_id
            ));
        }
        if !self.supports_fps(config.fps) {
            return Err(format!(
                "The selected frame rate {:.3} fps is outside the capture path's reported range.",
                config.fps.as_f64()
            ));
        }
        if !config.video_encoder.trim().is_empty()
            && !config.video_encoder.eq_ignore_ascii_case("auto")
            && !self
                .video_encoders
                .iter()
                .any(|encoder| encoder.id == config.video_encoder)
        {
            return Err(format!(
                "The selected video encoder '{}' is not available.",
                config.video_encoder
            ));
        }
        if !config.audio_encoder.trim().is_empty()
            && !config.audio_encoder.eq_ignore_ascii_case("auto")
            && !self
                .audio_encoders
                .iter()
                .any(|encoder| encoder.id == config.audio_encoder)
        {
            return Err(format!(
                "The selected audio encoder '{}' is not available.",
                config.audio_encoder
            ));
        }
        let available_sources = self
            .audio_sources
            .iter()
            .filter(|source| source.available)
            .map(|source| source.id.as_str())
            .collect::<HashSet<_>>();
        if !available_sources.is_empty() {
            for route in config.audio_routes.iter().filter(|route| route.enabled) {
                let is_windows_application =
                    matches!(self.backend, CaptureBackend::WindowsGraphicsCapture)
                        && route.source_id.starts_with("application:");
                if !is_windows_application && !available_sources.contains(route.source_id.as_str())
                {
                    return Err(format!(
                        "The selected audio source '{}' is not available.",
                        route.source_id
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum RecorderState {
    #[default]
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecorderStatus {
    pub state: RecorderState,
    pub replay_active: bool,
    pub configured: bool,
    pub last_replay_path: Option<String>,
    pub last_error: Option<String>,
    pub rss_bytes: Option<u64>,
    pub gpu_memory_bytes: Option<u64>,
    #[serde(default)]
    pub hotkey_registered: bool,
    #[serde(default)]
    pub hotkey_error: Option<String>,
}

impl Default for RecorderStatus {
    fn default() -> Self {
        Self {
            state: RecorderState::Stopped,
            replay_active: false,
            configured: false,
            last_replay_path: None,
            last_error: None,
            rss_bytes: None,
            gpu_memory_bytes: None,
            hotkey_registered: false,
            hotkey_error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecorderError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
}

impl RecorderError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RecorderRequest {
    Hello {
        protocol_version: u16,
        auth_token: String,
    },
    GetStatus,
    GetCapabilities,
    ApplyConfig {
        config: RecorderConfig,
    },
    Start,
    Stop,
    SaveReplay,
    Ping,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RecorderResponse {
    Hello {
        protocol_version: u16,
        service_version: String,
    },
    Status(RecorderStatus),
    Capabilities(RecorderCapabilities),
    Accepted,
    Pong,
    Error(RecorderError),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RecorderEvent {
    StatusChanged(RecorderStatus),
    CapabilitiesChanged(RecorderCapabilities),
    ReplaySaved { path: String, duration_seconds: u32 },
    Log { level: String, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ServiceMessage {
    Response {
        request_id: u64,
        response: RecorderResponse,
    },
    Event(RecorderEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClientMessage {
    pub request_id: u64,
    pub request: RecorderRequest,
}

pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let length = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "IPC frame is too large"))?;
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC frame is above the configured limit",
        ));
    }
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()
}

pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> io::Result<T> {
    let mut length = [0; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length);
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC frame is above the configured limit",
        ));
    }
    let mut payload = vec![0; length as usize];
    reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frame_round_trip() {
        let message = ClientMessage {
            request_id: 42,
            request: RecorderRequest::Ping,
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &message).unwrap();
        let decoded: ClientMessage = read_frame(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn frame_reader_rejects_oversized_payloads() {
        let bytes = (MAX_FRAME_BYTES + 1).to_le_bytes();
        assert!(read_frame::<_, ClientMessage>(&mut Cursor::new(bytes)).is_err());
    }

    #[test]
    fn config_accepts_high_frame_rates() {
        let config = RecorderConfig {
            fps: Rational::new(240, 1),
            ..RecorderConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn capabilities_reject_unsupported_frame_rates() {
        let capabilities = RecorderCapabilities {
            frame_rates: vec![FrameRateCapability {
                min: Rational::new(30, 1),
                max: Rational::new(120, 1),
                native: vec![Rational::new(60, 1)],
            }],
            ..RecorderCapabilities::default()
        };
        let config = RecorderConfig {
            fps: Rational::new(240, 1),
            ..RecorderConfig::default()
        };
        assert!(capabilities.validate_config(&config).is_err());
    }

    #[test]
    fn config_rejects_duplicate_enabled_audio_tracks() {
        let config = RecorderConfig {
            audio_routes: vec![
                AudioRoute {
                    source_id: "system:default".into(),
                    track: 1,
                    enabled: true,
                },
                AudioRoute {
                    source_id: "microphone:default".into(),
                    track: 1,
                    enabled: true,
                },
            ],
            ..RecorderConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn hotkey_formats_modifiers() {
        let hotkey = Hotkey {
            key: "f8".into(),
            ctrl: true,
            alt: false,
            shift: true,
            meta: false,
        }
        .normalized();
        assert_eq!(hotkey.to_string(), "Ctrl+Shift+F8");
    }
}
