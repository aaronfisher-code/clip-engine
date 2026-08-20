use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashSet,
    fmt,
    io::{self, Read, Write},
    thread,
    time::{Duration, Instant},
};

pub const PROTOCOL_VERSION: u16 = 4;
pub const RECORDER_CONFIG_SCHEMA_VERSION: u16 = 2;
pub const DEFAULT_SOCKET_NAME: &str = "clip-engine-recorder";
pub const MAX_FRAME_BYTES: u32 = 8 * 1024 * 1024;
pub const IPC_TIMEOUT: Duration = Duration::from_secs(90);

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
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
    PlaybackDevice,
    Microphone,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum SystemAudioMode {
    #[default]
    Mixed,
    ExcludeApplications,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScreenCapability {
    pub id: String,
    /// Previous platform-specific identifier retained so saved configurations
    /// can migrate when the stable display identifier changes.
    #[serde(default)]
    pub legacy_id: Option<String>,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum RecorderMode {
    #[default]
    Automatic,
    Advanced,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum RateControl {
    Cbr,
    #[default]
    Cqp,
    Vbr,
    Cqvbr,
}

impl RateControl {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cbr => "CBR",
            Self::Cqp => "CQP / Constant QP",
            Self::Vbr => "VBR",
            Self::Cqvbr => "CQVBR",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Multipass {
    Disabled,
    #[default]
    QuarterResolution,
    FullResolution,
}

impl Multipass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::QuarterResolution => "Quarter resolution",
            Self::FullResolution => "Full resolution",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EncoderSettingKind {
    Boolean,
    Integer,
    Float,
    Text,
    List,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EncoderSettingCapability {
    pub key: String,
    pub kind: EncoderSettingKind,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub option_values: Vec<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EncoderCapability {
    pub id: String,
    pub label: String,
    pub hardware: bool,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub settings: Vec<EncoderSettingCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioRoute {
    pub source_id: String,
    pub track: u8,
    #[serde(default)]
    pub track_name: String,
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
    pub mode: RecorderMode,
    #[serde(default)]
    pub screen_id: String,
    #[serde(default = "default_width")]
    pub output_width: u32,
    #[serde(default = "default_height")]
    pub output_height: u32,
    #[serde(default = "default_true")]
    pub match_display: bool,
    #[serde(default = "default_true")]
    pub match_display_fps: bool,
    #[serde(default)]
    pub fps: Rational,
    #[serde(default = "default_replay_seconds")]
    pub replay_seconds: u32,
    #[serde(default = "default_auto")]
    pub video_encoder: String,
    #[serde(default)]
    pub rate_control: RateControl,
    #[serde(default = "default_quality_level")]
    pub quality_level: u32,
    #[serde(default = "default_video_bitrate")]
    pub video_bitrate_kbps: u32,
    #[serde(default)]
    pub max_bitrate_kbps: u32,
    #[serde(default = "default_keyframe_interval")]
    pub keyframe_interval_seconds: u32,
    #[serde(default = "default_preset")]
    pub preset: String,
    #[serde(default = "default_tuning")]
    pub tuning: String,
    #[serde(default)]
    pub multipass: Multipass,
    #[serde(default = "default_profile")]
    pub profile: String,
    #[serde(default)]
    pub lookahead: bool,
    #[serde(default = "default_true")]
    pub adaptive_quantization: bool,
    #[serde(default = "default_b_frames")]
    pub b_frames: u8,
    #[serde(default = "default_b_frame_ref_mode")]
    pub b_frame_ref_mode: String,
    #[serde(default = "default_split_encode")]
    pub split_encode: String,
    #[serde(default)]
    pub gpu: i32,
    #[serde(default)]
    pub rescale_output: bool,
    #[serde(default = "default_container_format")]
    pub container_format: String,
    #[serde(default)]
    pub custom_encoder_options: String,
    #[serde(default = "default_audio_encoder")]
    pub audio_encoder: String,
    #[serde(default = "default_audio_bitrate")]
    pub audio_bitrate_kbps: u32,
    #[serde(default)]
    pub system_audio_mode: SystemAudioMode,
    #[serde(default)]
    pub audio_routes: Vec<AudioRoute>,
    #[serde(default)]
    pub hotkey: Option<Hotkey>,
    #[serde(default = "default_true")]
    pub notify_on_save: bool,
    #[serde(default)]
    pub output_directory: String,
}

fn default_schema_version() -> u16 {
    // A missing version identifies the pre-versioned configuration format.
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

fn default_quality_level() -> u32 {
    18
}

fn default_keyframe_interval() -> u32 {
    2
}

fn default_preset() -> String {
    "p5".into()
}

fn default_tuning() -> String {
    "hq".into()
}

fn default_profile() -> String {
    "main".into()
}

fn default_b_frames() -> u8 {
    2
}

fn default_b_frame_ref_mode() -> String {
    "middle".into()
}

fn default_split_encode() -> String {
    "auto".into()
}

fn default_container_format() -> String {
    "mkv".into()
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
            schema_version: RECORDER_CONFIG_SCHEMA_VERSION,
            mode: RecorderMode::Automatic,
            screen_id: String::new(),
            output_width: default_width(),
            output_height: default_height(),
            match_display: true,
            match_display_fps: true,
            fps: Rational::default(),
            replay_seconds: default_replay_seconds(),
            video_encoder: default_auto(),
            rate_control: RateControl::default(),
            quality_level: default_quality_level(),
            video_bitrate_kbps: default_video_bitrate(),
            max_bitrate_kbps: 0,
            keyframe_interval_seconds: default_keyframe_interval(),
            preset: default_preset(),
            tuning: default_tuning(),
            multipass: Multipass::default(),
            profile: default_profile(),
            lookahead: false,
            adaptive_quantization: true,
            b_frames: default_b_frames(),
            b_frame_ref_mode: default_b_frame_ref_mode(),
            split_encode: default_split_encode(),
            gpu: 0,
            rescale_output: false,
            container_format: default_container_format(),
            custom_encoder_options: String::new(),
            audio_encoder: default_audio_encoder(),
            audio_bitrate_kbps: default_audio_bitrate(),
            system_audio_mode: SystemAudioMode::default(),
            audio_routes: Vec::new(),
            hotkey: Some(Hotkey::default()),
            notify_on_save: true,
            output_directory: String::new(),
        }
    }
}

impl RecorderConfig {
    pub fn migrate(mut self) -> Self {
        if self.schema_version < RECORDER_CONFIG_SCHEMA_VERSION {
            self.schema_version = RECORDER_CONFIG_SCHEMA_VERSION;
            self.mode = RecorderMode::Advanced;
            self.match_display = false;
            self.match_display_fps = false;
            self.rate_control = RateControl::Cbr;
            self.max_bitrate_kbps = 0;
        }
        self
    }

    pub fn normalize(self) -> Self {
        self.migrate()
    }

    pub fn automatic_capture_config(&self) -> Self {
        let mut config = self.clone();
        let defaults = Self::default();
        config.video_encoder = defaults.video_encoder;
        config.rate_control = defaults.rate_control;
        config.quality_level = defaults.quality_level;
        config.video_bitrate_kbps = defaults.video_bitrate_kbps;
        config.max_bitrate_kbps = defaults.max_bitrate_kbps;
        config.keyframe_interval_seconds = defaults.keyframe_interval_seconds;
        config.preset = defaults.preset;
        config.tuning = defaults.tuning;
        config.multipass = defaults.multipass;
        config.profile = defaults.profile;
        config.lookahead = defaults.lookahead;
        config.adaptive_quantization = defaults.adaptive_quantization;
        config.b_frames = defaults.b_frames;
        config.b_frame_ref_mode = defaults.b_frame_ref_mode;
        config.split_encode = defaults.split_encode;
        config.gpu = defaults.gpu;
        config.rescale_output = defaults.rescale_output;
        config.container_format = defaults.container_format;
        config.custom_encoder_options = defaults.custom_encoder_options;
        config
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(1..=RECORDER_CONFIG_SCHEMA_VERSION).contains(&self.schema_version) {
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
        if self.quality_level == 0 || self.quality_level > 63 {
            return Err("Video quality must be between 1 and 63.".into());
        }
        if self.max_bitrate_kbps > 0 && self.max_bitrate_kbps < self.video_bitrate_kbps {
            return Err("Maximum bitrate cannot be below the target bitrate.".into());
        }
        if self.keyframe_interval_seconds > 60 {
            return Err("Keyframe interval must be between 0 and 60 seconds.".into());
        }
        if self.b_frames > 8 {
            return Err("B-frame count must be between 0 and 8.".into());
        }
        if !matches!(self.container_format.as_str(), "mkv" | "mp4") {
            return Err("Container format must be MKV or MP4.".into());
        }
        if self.audio_bitrate_kbps == 0 {
            return Err("Audio bitrate must be greater than zero.".into());
        }
        let mut used_tracks = HashSet::new();
        for route in &self.audio_routes {
            if route.source_id.trim().is_empty() {
                return Err("Audio routes must identify a source.".into());
            }
            if route.enabled
                && route
                    .source_id
                    .strip_prefix("application:")
                    .is_some_and(|target| target.trim().is_empty())
            {
                return Err("Application audio routes must identify an application.".into());
            }
            if route.enabled
                && route
                    .source_id
                    .strip_prefix("playback:")
                    .is_some_and(|target| target.trim().is_empty())
            {
                return Err("Playback-device audio routes must identify a device.".into());
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
    pub audio_isolation_available: bool,
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
            audio_isolation_available: false,
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
        let has_enabled_system_audio = config
            .audio_routes
            .iter()
            .any(|route| route.enabled && route.source_id.starts_with("system:"));
        let has_enabled_application_audio = config
            .audio_routes
            .iter()
            .any(|route| route.enabled && route.source_id.starts_with("application:"));
        if config.system_audio_mode == SystemAudioMode::ExcludeApplications
            && has_enabled_system_audio
            && has_enabled_application_audio
            && !self.audio_isolation_available
        {
            return Err(
                "Excluding application audio from the system track is not available on this capture backend."
                    .into(),
            );
        }
        if !self.screens.is_empty()
            && !config.screen_id.trim().is_empty()
            && !self.screens.iter().any(|screen| {
                screen.id == config.screen_id
                    || screen.legacy_id.as_deref() == Some(config.screen_id.as_str())
            })
        {
            return Err(format!(
                "The selected screen '{}' is no longer available.",
                config.screen_id
            ));
        }
        if !config.match_display_fps && !self.supports_fps(config.fps) {
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
                let is_native_application_audio = route.source_id.starts_with("application:")
                    && matches!(
                        self.backend,
                        CaptureBackend::WindowsGraphicsCapture
                            | CaptureBackend::X11
                            | CaptureBackend::PipeWire
                    );
                if !is_native_application_audio
                    && !available_sources.contains(route.source_id.as_str())
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

    pub fn normalize_config(&self, input: &RecorderConfig) -> RecorderConfig {
        let mut config = input.clone();
        if let Some(screen) = self.screens.iter().find(|screen| {
            screen.id == config.screen_id
                || screen.legacy_id.as_deref() == Some(config.screen_id.as_str())
        }) {
            config.screen_id = screen.id.clone();
        }
        if config.mode != RecorderMode::Advanced {
            return config;
        }

        let requested_encoder = config.video_encoder.trim();
        let Some(encoder) = self
            .video_encoders
            .iter()
            .find(|encoder| {
                !requested_encoder.is_empty()
                    && !requested_encoder.eq_ignore_ascii_case("auto")
                    && encoder.id == requested_encoder
            })
            .or_else(|| self.video_encoders.first())
        else {
            return config;
        };

        // Advanced mode is tied to one encoder, just like OBS's encoder
        // properties dialog. Automatic selection cannot provide a stable set
        // of dependent values, so bind it to the currently advertised choice.
        config.video_encoder = encoder.id.clone();
        if encoder.settings.is_empty() {
            return config;
        }

        let defaults = RecorderConfig::default();
        let rate_control = encoder_setting(encoder, &["rate_control", "rc"]);
        if let Some(setting) = rate_control {
            if setting.kind == EncoderSettingKind::List && !setting.options.is_empty() {
                let candidates = [
                    config.rate_control,
                    RateControl::Cqp,
                    RateControl::Cqvbr,
                    RateControl::Vbr,
                    RateControl::Cbr,
                ];
                config.rate_control = candidates
                    .into_iter()
                    .find(|candidate| rate_control_option_supported(setting, *candidate))
                    .unwrap_or(RateControl::Cbr);
            }
        } else {
            config.rate_control = RateControl::Cbr;
        }

        config.quality_level =
            encoder_setting(encoder, &["target_quality", "cqp", "cq", "qp", "crf"])
                .map(|setting| normalize_encoder_integer(config.quality_level, setting))
                .unwrap_or(defaults.quality_level);
        config.video_bitrate_kbps = encoder_setting(encoder, &["bitrate", "bitrate_kbps"])
            .map(|setting| normalize_encoder_integer(config.video_bitrate_kbps, setting))
            .unwrap_or(defaults.video_bitrate_kbps);
        config.max_bitrate_kbps = encoder_setting(encoder, &["max_bitrate", "max_bitrate_kbps"])
            .map(|setting| normalize_encoder_integer(config.max_bitrate_kbps, setting))
            .unwrap_or_default();
        config.keyframe_interval_seconds = encoder_setting(
            encoder,
            &["keyint_sec", "keyframe_interval", "keyframe_interval_sec"],
        )
        .map(|setting| normalize_encoder_integer(config.keyframe_interval_seconds, setting))
        .unwrap_or(defaults.keyframe_interval_seconds);
        normalize_encoder_string(
            &mut config.preset,
            encoder_setting(encoder, &["preset", "preset2"]),
            "",
        );
        normalize_encoder_string(
            &mut config.tuning,
            encoder_setting(encoder, &["tune", "tuning"]),
            "",
        );
        config.multipass = normalize_multipass(
            config.multipass,
            encoder_setting(encoder, &["multipass", "multi_pass"]),
        );
        normalize_encoder_string(
            &mut config.profile,
            encoder_setting(encoder, &["profile"]),
            "",
        );
        if encoder_setting(encoder, &["lookahead", "rc-lookahead", "look_ahead"]).is_none() {
            config.lookahead = defaults.lookahead;
        }
        if encoder_setting(
            encoder,
            &["adaptive_quantization", "spatial-aq", "spatial_aq"],
        )
        .is_none()
        {
            config.adaptive_quantization = defaults.adaptive_quantization;
        }
        config.b_frames = encoder_setting(encoder, &["bframes", "bf", "b_frames"])
            .map(|setting| normalize_encoder_integer(config.b_frames.into(), setting).min(8) as u8)
            .unwrap_or(defaults.b_frames);
        normalize_encoder_string(
            &mut config.b_frame_ref_mode,
            encoder_setting(encoder, &["bf_ref_mode", "b_ref_mode", "bframe_ref_mode"]),
            "",
        );
        normalize_encoder_string(
            &mut config.split_encode,
            encoder_setting(encoder, &["split_encode", "split-encode"]),
            "",
        );
        config.gpu = encoder_setting(encoder, &["gpu", "device"])
            .map(|setting| normalize_encoder_integer(config.gpu.max(0) as u32, setting) as i32)
            .unwrap_or(defaults.gpu);
        if encoder_setting(encoder, &["rescale"]).is_none() {
            config.rescale_output = defaults.rescale_output;
        }
        config
    }
}

fn encoder_setting<'a>(
    encoder: &'a EncoderCapability,
    keys: &[&str],
) -> Option<&'a EncoderSettingCapability> {
    keys.iter()
        .find_map(|key| encoder.settings.iter().find(|setting| setting.key == *key))
}

fn encoder_option_index(setting: &EncoderSettingCapability, requested: &str) -> Option<usize> {
    setting
        .options
        .iter()
        .position(|option| option.eq_ignore_ascii_case(requested))
        .or_else(|| {
            setting
                .option_values
                .iter()
                .position(|option| option.eq_ignore_ascii_case(requested))
        })
}

fn rate_control_option_supported(
    setting: &EncoderSettingCapability,
    requested: RateControl,
) -> bool {
    setting.options.iter().enumerate().any(|(index, option)| {
        rate_control_option_matches(option, requested)
            || setting
                .option_values
                .get(index)
                .is_some_and(|value| rate_control_option_matches(value, requested))
    })
}

fn rate_control_option_matches(option: &str, requested: RateControl) -> bool {
    let option = option.to_ascii_lowercase().replace([' ', '_', '-'], "");
    match requested {
        RateControl::Cbr => option == "cbr" || option.contains("constantbitrate"),
        RateControl::Cqp => {
            option == "cqp" || option == "cq" || option == "crf" || option.contains("constantqp")
        }
        RateControl::Vbr => option == "vbr" || option.contains("variablebitrate"),
        RateControl::Cqvbr => option.contains("cqvbr") || option.contains("qualityvbr"),
    }
}

fn normalize_encoder_integer(value: u32, setting: &EncoderSettingCapability) -> u32 {
    let mut value = f64::from(value);
    let minimum = setting.min.unwrap_or(0.0).ceil().max(0.0);
    let maximum = setting
        .max
        .unwrap_or(f64::from(u32::MAX))
        .floor()
        .max(minimum);
    value = value.clamp(minimum, maximum);
    if let Some(step) = setting.step.filter(|step| step.is_finite() && *step > 0.0) {
        value = minimum + ((value - minimum) / step).round() * step;
        value = value.clamp(minimum, maximum);
    }
    value.round().clamp(0.0, f64::from(u32::MAX)) as u32
}

fn normalize_encoder_string(
    value: &mut String,
    setting: Option<&EncoderSettingCapability>,
    fallback: &str,
) {
    let Some(setting) = setting else {
        *value = fallback.into();
        return;
    };
    if setting.kind != EncoderSettingKind::List || setting.options.is_empty() {
        return;
    }
    let index = encoder_option_index(setting, value)
        .unwrap_or(0)
        .min(setting.options.len().saturating_sub(1));
    *value = setting.options[index].clone();
}

fn normalize_multipass(
    requested: Multipass,
    setting: Option<&EncoderSettingCapability>,
) -> Multipass {
    let Some(setting) = setting else {
        return Multipass::Disabled;
    };
    if setting.kind != EncoderSettingKind::List || setting.options.is_empty() {
        return requested;
    }
    let candidates = [
        requested,
        Multipass::Disabled,
        Multipass::QuarterResolution,
        Multipass::FullResolution,
    ];
    candidates
        .into_iter()
        .find(|candidate| multipass_option_supported(setting, *candidate))
        .unwrap_or(Multipass::Disabled)
}

fn multipass_option_supported(setting: &EncoderSettingCapability, requested: Multipass) -> bool {
    setting.options.iter().enumerate().any(|(index, option)| {
        multipass_option_matches(option, requested)
            || setting
                .option_values
                .get(index)
                .is_some_and(|value| multipass_option_matches(value, requested))
    })
}

fn multipass_option_matches(option: &str, requested: Multipass) -> bool {
    let option = option.to_ascii_lowercase().replace([' ', '_', '-'], "");
    match requested {
        Multipass::Disabled => {
            option == "disabled" || option == "off" || option == "none" || option == "0"
        }
        Multipass::QuarterResolution => option.contains("quarter") || option == "qres",
        Multipass::FullResolution => option.contains("full") || option == "fullres",
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
    #[serde(default)]
    pub effective_settings: Option<EffectiveRecorderSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveRecorderSettings {
    pub mode: RecorderMode,
    pub video_encoder: String,
    #[serde(default)]
    pub video_codec: String,
    pub output_width: u32,
    pub output_height: u32,
    pub fps: Rational,
    pub rate_control: String,
    #[serde(default)]
    pub quality_level: Option<u32>,
    #[serde(default)]
    pub video_bitrate_kbps: Option<u32>,
    #[serde(default)]
    pub max_bitrate_kbps: Option<u32>,
    pub container_format: String,
    #[serde(default)]
    pub diagnostics: Vec<String>,
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
            effective_settings: None,
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
        config: Box<RecorderConfig>,
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
    write_all_resilient(writer, &length.to_le_bytes())?;
    write_all_resilient(writer, &payload)?;
    writer.flush()
}

pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> io::Result<T> {
    let mut length = [0; 4];
    read_exact_resilient(reader, &mut length)?;
    let length = u32::from_le_bytes(length);
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC frame is above the configured limit",
        ));
    }
    let mut payload = vec![0; length as usize];
    read_exact_resilient(reader, &mut payload)?;
    serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn is_ipc_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn read_exact_resilient<R: Read>(reader: &mut R, buf: &mut [u8]) -> io::Result<()> {
    let deadline = Instant::now() + IPC_TIMEOUT;
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "recorder IPC connection closed",
                ));
            }
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if is_ipc_timeout(&error) => {
                if filled == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "timed out waiting for the recorder helper",
                    ));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "timed out waiting for the rest of a recorder IPC frame",
                    ));
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn write_all_resilient<W: Write>(writer: &mut W, mut buf: &[u8]) -> io::Result<()> {
    let deadline = Instant::now() + IPC_TIMEOUT;
    while !buf.is_empty() {
        match writer.write(buf) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write recorder IPC frame",
                ));
            }
            Ok(count) => buf = &buf[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if is_ipc_timeout(&error) => {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "timed out writing to the recorder helper",
                    ));
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
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
    fn read_frame_retries_would_block_after_a_partial_read() {
        let message = ClientMessage {
            request_id: 7,
            request: RecorderRequest::Ping,
        };
        let mut encoded = Vec::new();
        write_frame(&mut encoded, &message).unwrap();

        let mut reader = PartialThenBlock {
            stage: 0,
            data: encoded,
        };
        let decoded: ClientMessage = read_frame(&mut reader).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn would_block_without_data_is_a_timeout() {
        let error = read_frame::<_, ClientMessage>(&mut WouldBlockOnce).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn would_block_and_timed_out_are_ipc_timeouts() {
        assert!(is_ipc_timeout(&io::Error::from(io::ErrorKind::WouldBlock)));
        assert!(is_ipc_timeout(&io::Error::from(io::ErrorKind::TimedOut)));
    }

    struct WouldBlockOnce;

    impl Read for WouldBlockOnce {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::WouldBlock))
        }
    }

    struct PartialThenBlock {
        stage: u8,
        data: Vec<u8>,
    }

    impl Read for PartialThenBlock {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.stage {
                0 => {
                    self.stage = 1;
                    let count = 2.min(self.data.len()).min(buf.len());
                    buf[..count].copy_from_slice(&self.data[..count]);
                    self.data.drain(..count);
                    Ok(count)
                }
                1 => {
                    self.stage = 2;
                    Err(io::Error::from(io::ErrorKind::WouldBlock))
                }
                _ => {
                    let count = self.data.len().min(buf.len());
                    buf[..count].copy_from_slice(&self.data[..count]);
                    self.data.drain(..count);
                    Ok(count)
                }
            }
        }
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
                    track_name: String::new(),
                    enabled: true,
                },
                AudioRoute {
                    source_id: "microphone:default".into(),
                    track: 1,
                    track_name: String::new(),
                    enabled: true,
                },
            ],
            ..RecorderConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn native_backends_accept_custom_application_audio_selectors() {
        let config = RecorderConfig {
            audio_encoder: "auto".into(),
            audio_routes: vec![AudioRoute {
                source_id: "application:spotify".into(),
                track: 3,
                track_name: String::new(),
                enabled: true,
            }],
            ..RecorderConfig::default()
        };
        let system_source = AudioSourceCapability {
            id: "system:default".into(),
            label: "System".into(),
            kind: AudioSourceKind::System,
            process_id: None,
            available: true,
            detail: None,
        };
        for backend in [
            CaptureBackend::WindowsGraphicsCapture,
            CaptureBackend::X11,
            CaptureBackend::PipeWire,
        ] {
            let capabilities = RecorderCapabilities {
                backend,
                audio_sources: vec![system_source.clone()],
                ..RecorderCapabilities::default()
            };
            assert!(capabilities.validate_config(&config).is_ok());
        }
    }

    #[test]
    fn enabled_application_audio_routes_require_a_target() {
        let config = RecorderConfig {
            audio_routes: vec![AudioRoute {
                source_id: "application:".into(),
                track: 3,
                track_name: String::new(),
                enabled: true,
            }],
            ..RecorderConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn playback_device_routes_preserve_opaque_windows_ids() {
        let device_id =
            r#"playback:\\?\SWD#MMDEVAPI#{0.0.0.00000000}.{01234567-89ab-cdef-0123-456789abcdef}"#;
        let config = RecorderConfig {
            audio_encoder: "auto".into(),
            audio_routes: vec![AudioRoute {
                source_id: device_id.into(),
                track: 2,
                track_name: "Voicemeeter Input".into(),
                enabled: true,
            }],
            ..RecorderConfig::default()
        };
        let capabilities = RecorderCapabilities {
            backend: CaptureBackend::WindowsGraphicsCapture,
            audio_sources: vec![AudioSourceCapability {
                id: device_id.into(),
                label: "Voicemeeter Input".into(),
                kind: AudioSourceKind::PlaybackDevice,
                process_id: None,
                available: true,
                detail: Some("WASAPI render endpoint".into()),
            }],
            ..RecorderCapabilities::default()
        };

        let json = serde_json::to_string(&capabilities).unwrap();
        assert!(json.contains(r#""kind":"playbackDevice""#));
        let decoded: RecorderCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, capabilities);
        assert!(capabilities.validate_config(&config).is_ok());
    }

    #[test]
    fn stale_playback_device_routes_are_rejected_when_capabilities_are_known() {
        let config = RecorderConfig {
            audio_encoder: "auto".into(),
            audio_routes: vec![AudioRoute {
                source_id: "playback:missing-device".into(),
                track: 2,
                track_name: String::new(),
                enabled: true,
            }],
            ..RecorderConfig::default()
        };
        let capabilities = RecorderCapabilities {
            backend: CaptureBackend::WindowsGraphicsCapture,
            audio_sources: vec![AudioSourceCapability {
                id: "playback:present-device".into(),
                label: "Present device".into(),
                kind: AudioSourceKind::PlaybackDevice,
                process_id: None,
                available: true,
                detail: Some("WASAPI render endpoint".into()),
            }],
            ..RecorderCapabilities::default()
        };
        assert!(capabilities.validate_config(&config).is_err());
    }

    #[test]
    fn system_audio_mode_defaults_to_mixed_for_legacy_json() {
        let config: RecorderConfig = serde_json::from_str(r#"{"replaySeconds":30}"#).unwrap();
        assert_eq!(config.system_audio_mode, SystemAudioMode::Mixed);
    }

    #[test]
    fn system_audio_mode_round_trips_through_json() {
        let config = RecorderConfig {
            system_audio_mode: SystemAudioMode::ExcludeApplications,
            ..RecorderConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let decoded: RecorderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            decoded.system_audio_mode,
            SystemAudioMode::ExcludeApplications
        );
    }

    #[test]
    fn audio_track_names_round_trip_and_default_for_legacy_routes() {
        let legacy: RecorderConfig = serde_json::from_str(
            r#"{"audioRoutes":[{"sourceId":"system:default","track":1,"enabled":true}]}"#,
        )
        .unwrap();
        assert_eq!(legacy.audio_routes[0].track_name, "");

        let config = RecorderConfig {
            audio_routes: vec![AudioRoute {
                source_id: "system:default".into(),
                track: 1,
                track_name: "Game mix".into(),
                enabled: true,
            }],
            ..RecorderConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let decoded: RecorderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.audio_routes[0].track_name, "Game mix");
    }

    #[test]
    fn application_exclusion_requires_backend_support_when_system_audio_is_enabled() {
        let config = RecorderConfig {
            system_audio_mode: SystemAudioMode::ExcludeApplications,
            audio_encoder: "auto".into(),
            audio_routes: vec![
                AudioRoute {
                    source_id: "system:default".into(),
                    track: 1,
                    track_name: String::new(),
                    enabled: true,
                },
                AudioRoute {
                    source_id: "application:discord".into(),
                    track: 2,
                    track_name: String::new(),
                    enabled: true,
                },
            ],
            ..RecorderConfig::default()
        };
        let capabilities = RecorderCapabilities {
            backend: CaptureBackend::PipeWire,
            audio_sources: vec![AudioSourceCapability {
                id: "system:default".into(),
                label: "System".into(),
                kind: AudioSourceKind::System,
                process_id: None,
                available: true,
                detail: None,
            }],
            ..RecorderCapabilities::default()
        };
        assert!(capabilities.validate_config(&config).is_err());

        let supported = RecorderCapabilities {
            audio_isolation_available: true,
            ..capabilities
        };
        assert!(supported.validate_config(&config).is_ok());
    }

    #[test]
    fn default_config_uses_automatic_replay_safe_settings() {
        let config = RecorderConfig::default();
        assert_eq!(config.schema_version, RECORDER_CONFIG_SCHEMA_VERSION);
        assert_eq!(config.mode, RecorderMode::Automatic);
        assert_eq!(config.rate_control, RateControl::Cqp);
        assert_eq!(config.container_format, "mkv");
        assert!(config.adaptive_quantization);
        assert!(!config.lookahead);
        assert!(config.notify_on_save);
        assert_eq!(config.system_audio_mode, SystemAudioMode::Mixed);
    }

    #[test]
    fn missing_notify_on_save_defaults_to_enabled() {
        let config: RecorderConfig = serde_json::from_str(r#"{"replaySeconds":30}"#).unwrap();
        assert!(config.notify_on_save);
    }

    #[test]
    fn capability_normalization_replaces_incompatible_encoder_values() {
        let capabilities = RecorderCapabilities {
            video_encoders: vec![EncoderCapability {
                id: "ffmpeg_svt_av1".into(),
                label: "SVT-AV1".into(),
                hardware: false,
                codec: "av1".into(),
                family: "AV1".into(),
                settings: vec![
                    EncoderSettingCapability {
                        key: "rate_control".into(),
                        kind: EncoderSettingKind::List,
                        options: vec!["CBR".into()],
                        option_values: vec!["CBR".into()],
                        min: None,
                        max: None,
                        step: None,
                        description: None,
                    },
                    EncoderSettingCapability {
                        key: "preset".into(),
                        kind: EncoderSettingKind::List,
                        options: vec!["8".into(), "9".into()],
                        option_values: vec!["8".into(), "9".into()],
                        min: None,
                        max: None,
                        step: None,
                        description: None,
                    },
                    EncoderSettingCapability {
                        key: "cqp".into(),
                        kind: EncoderSettingKind::Integer,
                        options: Vec::new(),
                        option_values: Vec::new(),
                        min: Some(0.0),
                        max: Some(51.0),
                        step: Some(1.0),
                        description: None,
                    },
                ],
            }],
            ..RecorderCapabilities::default()
        };
        let config = RecorderConfig {
            mode: RecorderMode::Advanced,
            video_encoder: "ffmpeg_svt_av1".into(),
            rate_control: RateControl::Cqp,
            quality_level: 63,
            preset: "p5".into(),
            tuning: "hq".into(),
            profile: "main".into(),
            b_frames: 4,
            ..RecorderConfig::default()
        };

        let normalized = capabilities.normalize_config(&config);

        assert_eq!(normalized.rate_control, RateControl::Cbr);
        assert_eq!(normalized.quality_level, 51);
        assert_eq!(normalized.preset, "8");
        assert_eq!(normalized.tuning, "");
        assert_eq!(normalized.profile, "");
        assert_eq!(normalized.b_frames, RecorderConfig::default().b_frames);
        assert_eq!(normalized.b_frame_ref_mode, "");
        assert_eq!(normalized.split_encode, "");
    }

    #[test]
    fn legacy_config_migrates_to_preserved_advanced_settings() {
        let config = RecorderConfig {
            schema_version: 1,
            mode: RecorderMode::Automatic,
            match_display: true,
            match_display_fps: true,
            rate_control: RateControl::Cqp,
            video_bitrate_kbps: 24_000,
            ..RecorderConfig::default()
        }
        .migrate();

        assert_eq!(config.schema_version, RECORDER_CONFIG_SCHEMA_VERSION);
        assert_eq!(config.mode, RecorderMode::Advanced);
        assert_eq!(config.rate_control, RateControl::Cbr);
        assert_eq!(config.video_bitrate_kbps, 24_000);
        assert!(!config.match_display);
        assert!(!config.match_display_fps);
    }

    #[test]
    fn unversioned_legacy_json_migrates_without_losing_bitrate() {
        let config: RecorderConfig = serde_json::from_str::<RecorderConfig>(
            r#"{
                "videoBitrateKbps": 32000,
                "audioBitrateKbps": 192,
                "containerFormat": "mkv"
            }"#,
        )
        .unwrap()
        .normalize();

        assert_eq!(config.schema_version, RECORDER_CONFIG_SCHEMA_VERSION);
        assert_eq!(config.mode, RecorderMode::Advanced);
        assert_eq!(config.rate_control, RateControl::Cbr);
        assert_eq!(config.video_bitrate_kbps, 32_000);
        assert_eq!(config.audio_bitrate_kbps, 192);
    }

    #[test]
    fn automatic_mode_preserves_video_overrides_for_mode_switching() {
        let config = RecorderConfig {
            mode: RecorderMode::Automatic,
            match_display: false,
            output_width: 2_560,
            rate_control: RateControl::Cbr,
            video_bitrate_kbps: 90_000,
            container_format: "mp4".into(),
            ..RecorderConfig::default()
        }
        .normalize();
        assert_eq!(config.rate_control, RateControl::Cbr);
        assert_eq!(config.video_bitrate_kbps, 90_000);
        assert_eq!(config.container_format, "mp4");
        assert!(!config.match_display);
        assert_eq!(config.output_width, 2_560);

        let capture_config = config.automatic_capture_config();
        assert_eq!(capture_config.rate_control, RateControl::Cqp);
        assert_eq!(capture_config.video_bitrate_kbps, default_video_bitrate());
        assert_eq!(capture_config.container_format, "mkv");
    }

    #[test]
    fn advanced_quality_fields_round_trip_through_json() {
        let config = RecorderConfig {
            mode: RecorderMode::Advanced,
            rate_control: RateControl::Cqvbr,
            quality_level: 22,
            max_bitrate_kbps: 60_000,
            multipass: Multipass::FullResolution,
            container_format: "mkv".into(),
            custom_encoder_options: "temporal-aq=1".into(),
            ..RecorderConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let decoded: RecorderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, config);
    }

    #[test]
    fn apply_config_round_trip_through_ipc_frame() {
        let message = ClientMessage {
            request_id: 7,
            request: RecorderRequest::ApplyConfig {
                config: Box::new(RecorderConfig {
                    mode: RecorderMode::Advanced,
                    video_encoder: "obs_nvenc".into(),
                    rate_control: RateControl::Cbr,
                    video_bitrate_kbps: 80_000,
                    ..RecorderConfig::default()
                }),
            },
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &message).unwrap();
        let decoded: ClientMessage = read_frame(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn effective_settings_round_trip_through_ipc_frames() {
        let message = ServiceMessage::Event(RecorderEvent::StatusChanged(RecorderStatus {
            effective_settings: Some(EffectiveRecorderSettings {
                mode: RecorderMode::Automatic,
                video_encoder: "obs_nvenc".into(),
                video_codec: "h264".into(),
                output_width: 2_560,
                output_height: 1_440,
                fps: Rational::new(144, 1),
                rate_control: "CQP".into(),
                quality_level: Some(18),
                video_bitrate_kbps: None,
                max_bitrate_kbps: None,
                container_format: "mkv".into(),
                diagnostics: vec!["look-ahead unavailable".into()],
            }),
            ..RecorderStatus::default()
        }));
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &message).unwrap();
        let decoded: ServiceMessage = read_frame(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn validation_rejects_unsafe_container_and_bitrate_values() {
        let config = RecorderConfig {
            container_format: "avi".into(),
            ..RecorderConfig::default()
        };
        assert!(config.validate().is_err());

        let config = RecorderConfig {
            video_bitrate_kbps: 50_000,
            max_bitrate_kbps: 10_000,
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
