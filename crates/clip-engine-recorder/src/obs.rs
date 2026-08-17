use super::{RecorderBackend, ReplayFile};
use anyhow::{Context, Result};
use clip_engine_recorder_protocol::{
    AudioSourceCapability, AudioSourceKind, CaptureBackend, EncoderCapability, FrameRateCapability,
    Rational, RecorderCapabilities, RecorderConfig, RecorderState, RecorderStatus,
    ScreenCapability,
};
use display_info::DisplayInfo;
use libobs_wrapper::{
    context::ObsContext,
    data::output::{ObsOutputTrait, ObsReplayBufferOutputRef},
    data::{object::ObsObjectTrait, ObsDataSetters},
    encoders::{video::ObsVideoEncoder, ObsAudioEncoderBuilder, ObsContextEncoders},
    scenes::{ObsSceneRef, SceneItemExtSceneTrait, SceneItemTrait},
    sources::ObsSourceRef,
    sys,
    utils::{ObjectInfo, ObsPath, ObsString, StartupPaths},
};
use std::{
    collections::BTreeSet,
    env,
    ffi::{c_char, CStr},
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime},
};

pub struct ObsBackend {
    context: ObsContext,
    capabilities: RecorderCapabilities,
    config: Option<RecorderConfig>,
    status: RecorderStatus,
    scene: Option<ObsSceneRef>,
    sources: Vec<ObsSourceRef>,
    output: Option<ObsReplayBufferOutputRef>,
}

impl ObsBackend {
    pub fn new() -> Result<Self> {
        check_libobs_version()?;

        let startup = ObsContext::builder()
            .set_startup_paths(discover_startup_paths())
            .set_start_glib_loop(true);
        let context = ObsContext::new(startup).context("initialize libobs")?;
        let capabilities = discover_capabilities(&context)?;

        Ok(Self {
            context,
            capabilities,
            config: None,
            status: RecorderStatus::default(),
            scene: None,
            sources: Vec::new(),
            output: None,
        })
    }

    fn rebuild_graph(&mut self, config: &RecorderConfig) -> Result<()> {
        self.capabilities
            .validate_config(config)
            .map_err(anyhow::Error::msg)?;
        self.stop_if_active()?;

        let video_info = libobs_wrapper::data::video::ObsVideoInfoBuilder::new()
            .fps_num(config.fps.numerator)
            .fps_den(config.fps.denominator)
            .base_width(config.output_width)
            .base_height(config.output_height)
            .output_width(config.output_width)
            .output_height(config.output_height)
            .build();
        self.context
            .reset_video(video_info)
            .context("reset libobs video context")?;

        let mut scene = self
            .context
            .scene("Clip Engine Recorder", Some(0))
            .context("create recorder scene")?;
        let screen_id = if config.screen_id.trim().is_empty() {
            self.capabilities
                .screens
                .first()
                .map(|screen| screen.id.clone())
                .unwrap_or_else(|| "0".into())
        } else {
            config.screen_id.clone()
        };
        let screen_source = self
            .create_screen_source(&screen_id)
            .context("create display capture source")?;
        let screen_item = scene
            .add_source(screen_source.clone())
            .context("add display capture source")?;
        let _ = screen_item.fit_source_to_screen();

        let mut sources = vec![screen_source];
        let mut tracks = BTreeSet::new();
        for route in config.audio_routes.iter().filter(|route| route.enabled) {
            let (source, track) = self
                .create_audio_source(&route.source_id, route.track)
                .with_context(|| format!("create audio source {}", route.source_id))?;
            set_audio_mixers(&source, track)?;
            scene
                .add_source(source.clone())
                .with_context(|| format!("add audio source {}", route.source_id))?;
            sources.push(source);
            tracks.insert(track);
        }

        let staging_directory = staging_directory(config);
        fs::create_dir_all(&staging_directory)
            .with_context(|| format!("create replay directory {}", staging_directory.display()))?;
        let mut output_settings = self.context.data().context("create replay settings")?;
        output_settings.set_string("directory", path_string(&staging_directory))?;
        output_settings.set_string("format", "clip-engine-replay-%CCYY-%MM-%DD-%hh-%mm-%ss-%r")?;
        output_settings.set_string("extension", "mkv")?;
        output_settings.set_bool("allow_spaces", false)?;
        output_settings.set_int("max_time_sec", i64::from(config.replay_seconds))?;
        // A zero max size means duration/bitrate control remains authoritative. The recorder
        // intentionally does not impose a memory ceiling; encoded replay storage is sized by
        // the requested duration and encoder settings.
        output_settings.set_int("max_size_mb", 0)?;
        let output_info = ObjectInfo::new(
            "replay_buffer",
            "Clip Engine Replay Buffer",
            Some(output_settings),
            None,
        );
        let mut output = self
            .context
            .replay_buffer(output_info)
            .context("create replay buffer output")?;

        let video_encoder_id = self
            .select_video_encoder(&config.video_encoder)
            .context("select video encoder")?;
        let mut video_settings = self
            .context
            .data()
            .context("create video encoder settings")?;
        video_settings.set_string("rate_control", "CBR")?;
        video_settings.set_int("bitrate", i64::from(config.video_bitrate_kbps))?;
        let video_encoder = ObsVideoEncoder::new_from_info(
            ObjectInfo::new(
                video_encoder_id,
                "Clip Engine Video Encoder",
                Some(video_settings),
                None,
            ),
            self.context.runtime().clone(),
        )
        .context("create video encoder")?;
        output
            .set_video_encoder(video_encoder)
            .context("attach video encoder")?;

        let audio_encoder_id = self.select_audio_encoder(&config.audio_encoder)?;
        for track in tracks {
            let mut audio_encoder = self
                .select_audio_encoder_builder(&audio_encoder_id)
                .context("select audio encoder")?;
            let mut audio_settings = self
                .context
                .data()
                .context("create audio encoder settings")?;
            audio_settings.set_int("bitrate", i64::from(config.audio_bitrate_kbps))?;
            audio_encoder.set_settings(audio_settings);
            audio_encoder
                .apply_to_context(
                    &mut output,
                    &format!("Clip Engine Audio Track {track}"),
                    None,
                    None,
                    usize::from(track - 1),
                )
                .with_context(|| format!("attach audio encoder track {track}"))?;
        }

        self.scene = Some(scene);
        self.sources = sources;
        self.output = Some(output);
        self.config = Some(config.clone());
        self.status.configured = true;
        self.status.last_error = None;
        Ok(())
    }

    fn stop_if_active(&mut self) -> Result<()> {
        if let Some(output) = self.output.as_mut() {
            if output.is_active().context("query replay output state")? {
                output.stop().context("stop replay buffer")?;
            }
        }
        self.status.replay_active = false;
        if !matches!(self.status.state, RecorderState::Error) {
            self.status.state = RecorderState::Stopped;
        }
        Ok(())
    }

    fn create_screen_source(&self, screen_id: &str) -> Result<ObsSourceRef> {
        let (source_id, settings) = match self.capabilities.backend {
            CaptureBackend::WindowsGraphicsCapture => {
                let mut settings = self
                    .context
                    .data()
                    .context("create Windows capture settings")?;
                let monitor = screen_id.parse::<i64>().unwrap_or(0);
                settings.set_int("monitor", monitor)?;
                settings.set_string("monitor_id", screen_id)?;
                settings.set_bool("capture_cursor", true)?;
                ("monitor_capture", settings)
            }
            CaptureBackend::X11 => {
                let mut settings = self.context.data().context("create X11 capture settings")?;
                settings.set_int("screen", screen_id.parse::<i64>().unwrap_or(0))?;
                settings.set_bool("show_cursor", true)?;
                ("xshm_input", settings)
            }
            CaptureBackend::PipeWire => {
                let mut settings = self
                    .context
                    .data()
                    .context("create PipeWire capture settings")?;
                // OBS opens the portal session on first use. The portal owns the final
                // monitor selection and returns a restore token for later launches.
                settings.set_bool("ShowCursor", true)?;
                settings.set_bool("show_cursor", true)?;
                settings.set_string("RestoreToken", "")?;
                ("pipewire-screen-capture-source", settings)
            }
            CaptureBackend::Unknown => {
                anyhow::bail!("no usable screen capture backend was reported")
            }
        };

        ObsSourceRef::new(
            source_id,
            "Clip Engine Display",
            Some(settings.into_immutable()),
            None,
            self.context.runtime().clone(),
        )
        .map_err(Into::into)
    }

    fn create_audio_source(&self, source_id: &str, track: u8) -> Result<(ObsSourceRef, u8)> {
        let mut settings = self
            .context
            .data()
            .context("create audio source settings")?;
        let (source_type, name) = if cfg!(windows) {
            if let Some(window) = source_id.strip_prefix("application:") {
                settings.set_string("window", window)?;
                settings.set_int("priority", 2)?;
                settings.set_bool("use_device_timing", false)?;
                (
                    "wasapi_process_output_capture",
                    format!("Clip Engine Application Audio Track {track}"),
                )
            } else if source_id.starts_with("microphone:") {
                settings.set_string("device_id", source_id.trim_start_matches("microphone:"))?;
                settings.set_bool("use_device_timing", true)?;
                (
                    "wasapi_input_capture",
                    format!("Clip Engine Microphone Track {track}"),
                )
            } else {
                settings.set_string("device_id", source_id.trim_start_matches("system:"))?;
                settings.set_bool("use_device_timing", true)?;
                (
                    "wasapi_output_capture",
                    format!("Clip Engine System Audio Track {track}"),
                )
            }
        } else if source_id.starts_with("microphone:") {
            settings.set_string("device_id", source_id.trim_start_matches("microphone:"))?;
            (
                "pulse_input_capture",
                format!("Clip Engine Microphone Track {track}"),
            )
        } else if source_id.starts_with("system:") {
            settings.set_string("device_id", source_id.trim_start_matches("system:"))?;
            (
                "pulse_output_capture",
                format!("Clip Engine System Audio Track {track}"),
            )
        } else {
            anyhow::bail!(
                "per-application audio source {source_id} is not available on this Linux backend"
            );
        };

        let source = ObsSourceRef::new(
            source_type,
            name,
            Some(settings.into_immutable()),
            None,
            self.context.runtime().clone(),
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok((source, track))
    }

    fn select_video_encoder(&self, requested: &str) -> Result<String> {
        let mut encoders = self.context.available_video_encoders()?;
        let requested = requested.trim();
        let selected = if requested.is_empty() || requested.eq_ignore_ascii_case("auto") {
            encoders
                .iter()
                .position(|encoder| {
                    let id: ObsString = encoder.get_encoder_id().clone().into();
                    is_hardware_encoder(&id.to_string())
                })
                .unwrap_or(0)
        } else {
            encoders
                .iter()
                .position(|encoder| {
                    let id: ObsString = encoder.get_encoder_id().clone().into();
                    id == requested
                })
                .ok_or_else(|| anyhow::anyhow!("video encoder {requested} is not available"))?
        };
        if encoders.is_empty() {
            anyhow::bail!("OBS reported no video encoders");
        }
        let encoder = encoders.swap_remove(selected);
        let id: ObsString = encoder.get_encoder_id().clone().into();
        Ok(id.to_string())
    }

    fn select_audio_encoder(&self, requested: &str) -> Result<String> {
        let encoders = self.context.available_audio_encoders()?;
        let requested = requested.trim();
        let selected = if requested.is_empty() || requested.eq_ignore_ascii_case("auto") {
            encoders
                .iter()
                .find_map(|encoder| {
                    let id: ObsString = encoder.get_encoder_id().clone().into();
                    (id == "ffmpeg_aac").then(|| id.to_string())
                })
                .or_else(|| {
                    encoders.first().map(|encoder| {
                        let id: ObsString = encoder.get_encoder_id().clone().into();
                        id.to_string()
                    })
                })
        } else {
            encoders.iter().find_map(|encoder| {
                let id: ObsString = encoder.get_encoder_id().clone().into();
                (id == requested).then(|| id.to_string())
            })
        };
        selected.ok_or_else(|| anyhow::anyhow!("audio encoder {requested} is not available"))
    }

    fn select_audio_encoder_builder(&self, requested: &str) -> Result<ObsAudioEncoderBuilder> {
        self.context
            .available_audio_encoders()?
            .into_iter()
            .find(|encoder| {
                let id: ObsString = encoder.get_encoder_id().clone().into();
                id == requested
            })
            .ok_or_else(|| anyhow::anyhow!("audio encoder {requested} is not available"))
    }
}

fn check_libobs_version() -> Result<()> {
    if !ObsContext::check_version_compatibility() {
        anyhow::bail!(
            "the loaded libobs major version does not match the pinned bindings (expected {})",
            sys::LIBOBS_API_MAJOR_VER
        );
    }
    let actual = ObsContext::get_version_global().context("read loaded libobs version")?;
    let expected = format!(
        "{}.{}.{}",
        sys::LIBOBS_API_MAJOR_VER,
        sys::LIBOBS_API_MINOR_VER,
        sys::LIBOBS_API_PATCH_VER
    );
    let actual_version = actual.split('.').take(3).collect::<Vec<_>>().join(".");
    if actual_version != expected {
        anyhow::bail!(
            "loaded libobs {actual_version} does not match the pinned runtime {expected}; install the bundled OBS runtime or set CLIP_ENGINE_OBS_ROOT to a matching build"
        );
    }
    Ok(())
}

impl RecorderBackend for ObsBackend {
    fn capabilities(&self) -> RecorderCapabilities {
        self.capabilities.clone()
    }

    fn status(&self) -> RecorderStatus {
        let mut status = self.status.clone();
        status.rss_bytes = current_rss_bytes();
        status
    }

    fn apply_config(&mut self, config: RecorderConfig) -> Result<()> {
        if let Err(error) = self.rebuild_graph(&config) {
            self.status.state = RecorderState::Error;
            self.status.last_error = Some(format!("{error:#}"));
            return Err(error);
        }
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        if self.output.is_none() {
            anyhow::bail!("recorder is not configured");
        }
        self.status.state = RecorderState::Starting;
        let result = self.output.as_ref().expect("output checked above").start();
        if let Err(error) = result {
            self.status.state = RecorderState::Error;
            self.status.replay_active = false;
            self.status.last_error = Some(error.to_string());
            return Err(error.into());
        }
        self.status.state = RecorderState::Running;
        self.status.replay_active = true;
        self.status.last_error = None;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.stop_if_active()?;
        self.status.state = RecorderState::Stopped;
        Ok(())
    }

    fn save_replay(&mut self) -> Result<ReplayFile> {
        if !self.status.replay_active {
            anyhow::bail!("replay buffer is not running");
        }
        let config = self
            .config
            .as_ref()
            .context("recorder configuration is missing")?
            .clone();
        let source_path = self
            .output
            .as_ref()
            .context("replay output is missing")?
            .save_buffer()
            .context("save replay buffer")?;
        let source_path = wait_for_stable_file(&source_path)?;
        let destination_directory = output_directory(&config);
        let destination = handoff_replay(&source_path, &destination_directory)?;
        self.status.last_replay_path = Some(path_string(&destination));
        Ok(ReplayFile {
            path: destination,
            duration_seconds: config.replay_seconds,
        })
    }
}

fn discover_capabilities(context: &ObsContext) -> Result<RecorderCapabilities> {
    let input_types = enumerate_input_types(context)?;
    let screens = DisplayInfo::all()
        .unwrap_or_default()
        .into_iter()
        .map(|display| ScreenCapability {
            id: display.id.to_string(),
            label: if display.friendly_name.is_empty() {
                display.name
            } else {
                display.friendly_name
            },
            width: display.width,
            height: display.height,
            refresh_hz: (display.frequency > 0.0).then_some(f64::from(display.frequency)),
            backend: detect_backend(&input_types).0,
        })
        .collect::<Vec<_>>();
    let (backend, mut diagnostics) = detect_backend(&input_types);

    let mut video_encoders = Vec::new();
    for encoder in context.available_video_encoders()? {
        let id: ObsString = encoder.get_encoder_id().clone().into();
        let id = id.to_string();
        video_encoders.push(EncoderCapability {
            label: id.clone(),
            hardware: is_hardware_encoder(&id),
            id,
        });
    }
    let mut audio_encoders = Vec::new();
    for encoder in context.available_audio_encoders()? {
        let id: ObsString = encoder.get_encoder_id().clone().into();
        let id = id.to_string();
        audio_encoders.push(EncoderCapability {
            label: id.clone(),
            hardware: false,
            id,
        });
    }

    let mut audio_sources = Vec::new();
    if input_types.iter().any(|id| id == "pulse_output_capture") {
        audio_sources.push(AudioSourceCapability {
            id: "system:default".into(),
            label: "System audio".into(),
            kind: AudioSourceKind::System,
            process_id: None,
            available: true,
            detail: Some("PulseAudio/PipeWire default output monitor".into()),
        });
    }
    if input_types.iter().any(|id| id == "pulse_input_capture") {
        audio_sources.push(AudioSourceCapability {
            id: "microphone:default".into(),
            label: "Default microphone".into(),
            kind: AudioSourceKind::Microphone,
            process_id: None,
            available: true,
            detail: Some("PulseAudio/PipeWire default input".into()),
        });
    }
    if input_types.iter().any(|id| id == "wasapi_output_capture") {
        audio_sources.push(AudioSourceCapability {
            id: "system:default".into(),
            label: "System audio".into(),
            kind: AudioSourceKind::System,
            process_id: None,
            available: true,
            detail: Some("Default WASAPI output".into()),
        });
    }
    if input_types.iter().any(|id| id == "wasapi_input_capture") {
        audio_sources.push(AudioSourceCapability {
            id: "microphone:default".into(),
            label: "Default microphone".into(),
            kind: AudioSourceKind::Microphone,
            process_id: None,
            available: true,
            detail: Some("Default WASAPI input".into()),
        });
    }
    if input_types
        .iter()
        .any(|id| id == "wasapi_process_output_capture")
    {
        #[cfg(windows)]
        {
            let applications = enumerate_windows();
            if applications.is_empty() {
                diagnostics.push(
                    "Application audio capture is available, but no visible titled windows were enumerated.".into(),
                );
            } else {
                audio_sources.extend(applications.into_iter().map(|(title, process_id)| {
                    AudioSourceCapability {
                        id: format!("application:{title}"),
                        label: title.clone(),
                        kind: AudioSourceKind::Application,
                        process_id: Some(process_id),
                        available: true,
                        detail: Some(format!("WASAPI process loopback, PID {process_id}")),
                    }
                }));
            }
        }
        #[cfg(not(windows))]
        diagnostics.push(
            "Application audio capture is available through window selectors; application entries are refreshed by the desktop client.".into(),
        );
    } else if matches!(backend, CaptureBackend::PipeWire) {
        diagnostics.push(
            "Per-application audio enumeration is compositor/session-manager dependent on Wayland."
                .into(),
        );
    }

    let max_fps = screens
        .iter()
        .filter_map(|screen| screen.refresh_hz)
        .fold(240.0_f64, f64::max)
        .ceil() as u32;
    let native = [30, 60, 120, 144, 165, 240]
        .into_iter()
        .filter(|fps| *fps <= max_fps)
        .map(|fps| Rational::new(fps, 1))
        .collect();

    Ok(RecorderCapabilities {
        backend,
        screens,
        audio_sources,
        video_encoders,
        audio_encoders,
        frame_rates: vec![FrameRateCapability {
            min: Rational::new(1, 1),
            max: Rational::new(max_fps, 1),
            native,
        }],
        diagnostics,
    })
}

#[cfg(windows)]
fn enumerate_windows() -> Vec<(String, u32)> {
    use std::ffi::c_void;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible, HWND,
    };

    unsafe extern "system" fn collect_window(hwnd: HWND, lparam: isize) -> i32 {
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if length <= 0 {
            return 1;
        }
        let mut title = vec![0u16; length as usize + 1];
        let written = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
        if written <= 0 {
            return 1;
        }
        let title = String::from_utf16_lossy(&title[..written as usize])
            .trim()
            .to_string();
        if title.is_empty() {
            return 1;
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) };
        if process_id == 0 {
            return 1;
        }
        let windows = unsafe { &mut *(lparam as *mut Vec<(String, u32)>) };
        if !windows.iter().any(|(existing, _)| existing == &title) {
            windows.push((title, process_id));
        }
        1
    }

    let mut windows = Vec::new();
    unsafe {
        EnumWindows(
            Some(collect_window),
            (&mut windows as *mut Vec<(String, u32)>).cast::<c_void>() as isize,
        );
    }
    windows
}

fn enumerate_input_types(context: &ObsContext) -> Result<Vec<String>> {
    let runtime = context.runtime().clone();
    runtime
        .run_with_obs_result(move || {
            let mut input_types = Vec::new();
            let mut index = 0;
            loop {
                let mut id: *const c_char = std::ptr::null();
                let has_next = unsafe { sys::obs_enum_input_types(index, &mut id) };
                if !has_next {
                    break;
                }
                if !id.is_null() {
                    let id = unsafe { CStr::from_ptr(id) }.to_string_lossy().into_owned();
                    input_types.push(id);
                }
                index += 1;
            }
            input_types
        })
        .map_err(Into::into)
}

fn detect_backend(input_types: &[String]) -> (CaptureBackend, Vec<String>) {
    if cfg!(windows) {
        if input_types
            .iter()
            .any(|id| id == "monitor_capture" || id == "duplicator_monitor_capture")
        {
            return (CaptureBackend::WindowsGraphicsCapture, Vec::new());
        }
        return (
            CaptureBackend::Unknown,
            vec!["OBS did not load a Windows monitor capture source.".into()],
        );
    }

    let wayland = env::var_os("WAYLAND_DISPLAY").is_some()
        || env::var("XDG_SESSION_TYPE")
            .map(|session| session.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);
    if wayland {
        if input_types
            .iter()
            .any(|id| id == "pipewire-screen-capture-source")
        {
            return (CaptureBackend::PipeWire, Vec::new());
        }
        return (
            CaptureBackend::Unknown,
            vec![
                "Wayland capture requires the OBS PipeWire source and an xdg-desktop-portal ScreenCast implementation.".into(),
            ],
        );
    }
    if input_types.iter().any(|id| id == "xshm_input") {
        return (CaptureBackend::X11, Vec::new());
    }
    (
        CaptureBackend::Unknown,
        vec!["OBS did not load the X11 xshm_input capture source.".into()],
    )
}

fn set_audio_mixers(source: &ObsSourceRef, track: u8) -> Result<()> {
    let mixer_mask = audio_mixer_mask(track)?;
    let source_ptr = source.as_ptr();
    let runtime = source.runtime().clone();
    runtime
        .run_with_obs_result(move || unsafe {
            sys::obs_source_set_audio_mixers(source_ptr.get_ptr(), mixer_mask);
        })
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

fn audio_mixer_mask(track: u8) -> Result<u32> {
    if !(1..=6).contains(&track) {
        anyhow::bail!("audio track must be between 1 and 6");
    }
    Ok(1_u32 << u32::from(track - 1))
}

fn select_resource_root() -> Option<PathBuf> {
    if let Some(path) = env::var_os("CLIP_ENGINE_OBS_ROOT") {
        return Some(PathBuf::from(path));
    }
    let executable = env::current_exe().ok()?;
    let parent = executable.parent()?;
    let bundled = parent.join("obs");
    (bundled.join("data").is_dir() && bundled.join("obs-plugins").is_dir()).then_some(bundled)
}

fn discover_startup_paths() -> StartupPaths {
    #[cfg(windows)]
    {
        let root = select_resource_root().unwrap_or_else(|| PathBuf::from("."));
        StartupPaths::new(
            ObsPath::new(&path_string(&root.join("data/libobs"))),
            ObsPath::new(&path_string(&root.join("obs-plugins/64bit"))),
            ObsPath::new(&path_string(&root.join("data/obs-plugins/%module%"))),
        )
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(root) = select_resource_root() {
            return StartupPaths::new(
                ObsPath::new(&path_string(&root.join("data/libobs"))),
                ObsPath::new(&path_string(&root.join("obs-plugins"))),
                ObsPath::new(&path_string(&root.join("data/obs-plugins/%module%"))),
            );
        }
        StartupPaths::new(
            ObsPath::new("/usr/share/obs/libobs"),
            ObsPath::new("/usr/lib/obs-plugins"),
            ObsPath::new("/usr/share/obs/obs-plugins/%module%"),
        )
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        StartupPaths::default()
    }
}

fn output_directory(config: &RecorderConfig) -> PathBuf {
    if config.output_directory.trim().is_empty() {
        return env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("replays");
    }
    PathBuf::from(&config.output_directory)
}

fn staging_directory(config: &RecorderConfig) -> PathBuf {
    output_directory(config).join(".staging")
}

fn handoff_replay(source_path: &Path, destination_directory: &Path) -> Result<PathBuf> {
    fs::create_dir_all(destination_directory).with_context(|| {
        format!(
            "create finalized replay directory {}",
            destination_directory.display()
        )
    })?;
    let file_name = source_path
        .file_name()
        .context("OBS returned a replay path without a file name")?;
    let destination = destination_directory.join(file_name);
    let destination = if destination.exists() {
        let suffix = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let stem = source_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("replay");
        let extension = source_path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| format!(".{extension}"))
            .unwrap_or_default();
        destination_directory.join(format!("{stem}-{suffix}{extension}"))
    } else {
        destination
    };
    fs::rename(source_path, &destination).with_context(|| {
        format!(
            "atomically hand off replay {} to {}",
            source_path.display(),
            destination.display()
        )
    })?;
    Ok(destination)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn wait_for_stable_file(path: &Path) -> Result<PathBuf> {
    let deadline = SystemTime::now() + Duration::from_secs(10);
    let mut previous = None;
    loop {
        let metadata = fs::metadata(path)
            .with_context(|| format!("inspect replay file {}", path.display()))?;
        let current = (metadata.len(), metadata.modified().ok());
        if current.0 > 0 && previous == Some(current) {
            return Ok(path.to_path_buf());
        }
        previous = Some(current);
        if SystemTime::now() >= deadline {
            anyhow::bail!("replay file did not become stable: {}", path.display());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn is_hardware_encoder(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id.contains("nvenc")
        || id.contains("qsv")
        || id.contains("vaapi")
        || id.contains("amf")
        || id.contains("videotoolbox")
}

fn current_rss_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let statm = fs::read_to_string("/proc/self/statm").ok()?;
        let pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page_size <= 0 {
            return None;
        }
        pages.checked_mul(page_size as u64)
    }
    #[cfg(windows)]
    {
        use std::mem::size_of;
        use windows_sys::Win32::System::{
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
            Threading::GetCurrentProcess,
        };

        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        counters.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let success = unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut counters,
                size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            )
        };
        (success != 0).then_some(counters.WorkingSetSize as u64)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_replay_files_are_ready_for_handoff() {
        let directory = env::temp_dir().join(format!(
            "clip-engine-replay-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("replay.mkv");
        fs::write(&path, b"encoded replay").unwrap();
        assert_eq!(wait_for_stable_file(&path).unwrap(), path);
        let destination_directory = directory.join("inbox");
        let destination = handoff_replay(&path, &destination_directory).unwrap();
        assert!(destination.is_file());
        assert!(!path.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_tracks_map_to_independent_obs_mixers() {
        let masks = (1..=6)
            .map(|track| audio_mixer_mask(track).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(masks, vec![1, 2, 4, 8, 16, 32]);
        assert!(audio_mixer_mask(0).is_err());
        assert!(audio_mixer_mask(7).is_err());
    }
}
