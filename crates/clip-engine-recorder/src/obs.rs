use super::{RecorderBackend, ReplayFile};
use anyhow::{Context, Result};
use clip_engine_recorder_protocol::{
    AudioRoute, AudioSourceCapability, AudioSourceKind, CaptureBackend, EffectiveRecorderSettings,
    EncoderCapability, EncoderSettingCapability, EncoderSettingKind, FrameRateCapability,
    Multipass, RateControl, Rational, RecorderCapabilities, RecorderConfig, RecorderMode,
    RecorderState, RecorderStatus, ScreenCapability, SystemAudioMode,
};
use display_info::DisplayInfo;
use libobs_wrapper::{
    context::ObsContext,
    data::output::{ObsOutputTrait, ObsReplayBufferOutputRef},
    data::{
        object::ObsObjectTrait,
        properties::{ObsProperty, ObsPropertyObject},
        ObsData, ObsDataSetters,
    },
    encoders::{
        video::ObsVideoEncoder, ObsAudioEncoderBuilder, ObsContextEncoders, ObsVideoEncoderBuilder,
    },
    enums::ObsOutputStopSignal,
    scenes::{ObsSceneRef, SceneItemExtSceneTrait, SceneItemTrait},
    sources::ObsSourceRef,
    sys,
    unsafe_send::Sendable,
    utils::{ObjectInfo, ObsCalldataExt, ObsError, ObsPath, ObsString, StartupPaths},
};
#[cfg(target_os = "linux")]
use std::process::Command;
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::{c_char, CStr},
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};
#[cfg(windows)]
use windows::{
    core::HRESULT,
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Media::Audio::{eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE},
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
            StructuredStorage::{PropVariantClear, PropVariantToString, PROPVARIANT},
            CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
        },
    },
};

pub struct ObsBackend {
    context: ObsContext,
    capabilities: RecorderCapabilities,
    input_types: BTreeSet<String>,
    config: Option<RecorderConfig>,
    status: RecorderStatus,
    scene: Option<ObsSceneRef>,
    sources: Vec<ObsSourceRef>,
    output: Option<ObsReplayBufferOutputRef>,
}

impl ObsBackend {
    pub fn new() -> Result<Self> {
        prepare_obs_muxer()?;
        check_libobs_version()?;

        let startup = ObsContext::builder()
            .set_startup_paths(discover_startup_paths())
            .set_start_glib_loop(true);
        let context = ObsContext::new(startup).context("initialize libobs")?;
        let input_types = enumerate_input_types(&context)?;
        let capabilities = discover_capabilities(&context, &input_types)?;

        Ok(Self {
            context,
            capabilities,
            input_types: input_types.into_iter().collect(),
            config: None,
            status: RecorderStatus::default(),
            scene: None,
            sources: Vec::new(),
            output: None,
        })
    }

    fn refreshed_capabilities(&self) -> RecorderCapabilities {
        let mut capabilities = self.capabilities.clone();
        capabilities
            .audio_sources
            .retain(|source| source.kind != AudioSourceKind::Application);
        capabilities.diagnostics.retain(|diagnostic| {
            !diagnostic.starts_with("Application audio capture is available")
                && !diagnostic.starts_with("PipeWire application capture is available")
                && !diagnostic.starts_with("PipeWire application audio is available")
                && !diagnostic.starts_with("WASAPI playback devices could not be enumerated")
        });

        if self.input_types.contains("wasapi_process_output_capture") {
            #[cfg(windows)]
            {
                let applications = enumerate_windows();
                if applications.is_empty() {
                    capabilities.diagnostics.push(
                        "Application audio capture is available, but no visible titled windows were enumerated."
                            .into(),
                    );
                } else {
                    capabilities
                        .audio_sources
                        .extend(applications.into_iter().map(
                            |(selector, label, process_id, executable)| AudioSourceCapability {
                                id: format!("application:{selector}"),
                                label,
                                kind: AudioSourceKind::Application,
                                process_id: Some(process_id),
                                available: true,
                                detail: Some(format!(
                                    "WASAPI process loopback · {executable} · PID {process_id}"
                                )),
                            },
                        ));
                }
            }
        }

        if self.input_types.contains("wasapi_output_capture") {
            capabilities
                .audio_sources
                .retain(|source| source.kind != AudioSourceKind::PlaybackDevice);
            #[cfg(windows)]
            {
                match enumerate_windows_playback_devices() {
                    Ok(devices) => capabilities.audio_sources.extend(devices.into_iter().map(
                        |(device_id, label)| {
                            playback_device_capability(
                                &device_id,
                                label,
                                true,
                                Some("WASAPI render endpoint".into()),
                            )
                        },
                    )),
                    Err(error) => capabilities.diagnostics.push(format!(
                        "WASAPI playback devices could not be enumerated: {error:#}"
                    )),
                }
            }
        }
        if let Some(config) = &self.config {
            let configured_playback_ids = config
                .audio_routes
                .iter()
                .filter(|route| route.source_id.starts_with("playback:"))
                .map(|route| route.source_id.as_str());
            for source_id in configured_playback_ids {
                if !capabilities
                    .audio_sources
                    .iter()
                    .any(|source| source.id == source_id)
                {
                    let endpoint_id = source_id.strip_prefix("playback:").unwrap_or(source_id);
                    capabilities.audio_sources.push(playback_device_capability(
                        endpoint_id,
                        format!("Unavailable playback device ({endpoint_id})"),
                        false,
                        Some(
                            "This Windows render endpoint is no longer active. Refresh or reconnect the device before enabling this route."
                                .into(),
                        ),
                    ));
                }
            }
        }

        if self
            .input_types
            .contains("pipewire_audio_application_capture")
        {
            #[cfg(target_os = "linux")]
            {
                match enumerate_pipewire_applications() {
                    Ok(applications) if applications.is_empty() => capabilities.diagnostics.push(
                        "PipeWire application capture is available. Start an application and refresh to discover its audio stream; custom executable/app-name selectors can also be added manually."
                            .into(),
                    ),
                    Ok(applications) => capabilities.audio_sources.extend(applications),
                    Err(error) => capabilities.diagnostics.push(format!(
                        "PipeWire application audio is available, but active applications could not be enumerated: {error}. You can still add an executable or app name manually."
                    )),
                }
            }
        }
        capabilities
    }

    fn rebuild_graph(&mut self, requested_config: &RecorderConfig) -> Result<()> {
        let requested_config = self
            .capabilities
            .normalize_config(&requested_config.clone().normalize());
        let config = if requested_config.mode == RecorderMode::Automatic {
            requested_config.automatic_capture_config()
        } else {
            requested_config
        };
        self.capabilities
            .validate_config(&config)
            .map_err(anyhow::Error::msg)?;
        let resolved = self.resolve_capture_settings(&config)?;
        self.stop_if_active()?;

        let video_info = libobs_wrapper::data::video::ObsVideoInfoBuilder::new()
            .fps_num(resolved.fps.numerator)
            .fps_den(resolved.fps.denominator)
            .base_width(resolved.output_width)
            .base_height(resolved.output_height)
            .output_width(resolved.output_width)
            .output_height(resolved.output_height)
            .build();
        self.context
            .reset_video(video_info)
            .context("reset libobs video context")?;

        let mut scene = self
            .context
            .scene("Clip Engine Recorder", Some(0))
            .context("create recorder scene")?;
        let screen_source = self
            .create_screen_source(&resolved.screen_id)
            .context("create display capture source")?;
        let screen_item = scene
            .add_source(screen_source.clone())
            .context("add display capture source")?;
        let _ = screen_item.fit_source_to_screen();

        let mut sources = vec![screen_source];
        let mut tracks = BTreeMap::new();
        let excluded_applications = system_audio_exclusion_selectors(&config);
        for route in config.audio_routes.iter().filter(|route| route.enabled) {
            let (source, track) =
                if route.source_id.starts_with("system:") && !excluded_applications.is_empty() {
                    self.create_excluding_system_audio_source(&excluded_applications, route.track)
                        .with_context(|| {
                            format!(
                                "create isolated system audio source for track {}",
                                route.track
                            )
                        })?
                } else {
                    self.create_audio_source(&route.source_id, route.track)
                        .with_context(|| format!("create audio source {}", route.source_id))?
                };
            set_audio_mixers(&source, track)?;
            scene
                .add_source(source.clone())
                .with_context(|| format!("add audio source {}", route.source_id))?;
            sources.push(source);
            tracks.insert(track, audio_track_name(route));
        }

        let staging_directory = staging_directory(&config);
        fs::create_dir_all(&staging_directory)
            .with_context(|| format!("create replay directory {}", staging_directory.display()))?;
        let mut output_settings = self.context.data().context("create replay settings")?;
        output_settings.set_string("directory", path_string(&staging_directory))?;
        output_settings.set_string("format", "clip-engine-replay-%CCYY-%MM-%DD-%hh-%mm-%ss-%r")?;
        output_settings.set_string("extension", config.container_format.as_str())?;
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

        let video_encoder_id = resolved.video_encoder_id.clone();
        let mut video_settings = self
            .context
            .data()
            .context("create video encoder settings")?;
        let video_capability = self
            .capabilities
            .video_encoders
            .iter()
            .find(|encoder| encoder.id == video_encoder_id);
        let applied_video =
            apply_video_encoder_settings(&mut video_settings, &config, video_capability)?;
        let video_encoder = ObsVideoEncoder::new_from_info(
            ObjectInfo::new(
                video_encoder_id.clone(),
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
        let audio_capability = self
            .capabilities
            .audio_encoders
            .iter()
            .find(|encoder| encoder.id == audio_encoder_id);
        let mut audio_diagnostics = Vec::new();
        for (track, track_name) in tracks {
            let audio_encoder = self
                .select_audio_encoder_builder(&audio_encoder_id)
                .context("select audio encoder")?;
            let mut audio_settings = self
                .context
                .data()
                .context("create audio encoder settings")?;
            set_int_encoder_property(
                &mut audio_settings,
                audio_capability,
                &["bitrate", "bitrate_kbps"],
                i64::from(config.audio_bitrate_kbps),
                "audio bitrate",
                &mut audio_diagnostics,
            )?;
            audio_encoder
                .apply_to_context(
                    &mut output,
                    &track_name,
                    Some(audio_settings),
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
        self.status.effective_settings = Some(EffectiveRecorderSettings {
            mode: config.mode,
            video_encoder: video_encoder_id,
            video_codec: video_capability
                .map(|encoder| encoder.codec.clone())
                .unwrap_or_default(),
            output_width: resolved.output_width,
            output_height: resolved.output_height,
            fps: resolved.fps,
            rate_control: applied_video.rate_control,
            quality_level: applied_video.quality_level,
            video_bitrate_kbps: applied_video.video_bitrate_kbps,
            max_bitrate_kbps: applied_video.max_bitrate_kbps,
            container_format: config.container_format.clone(),
            diagnostics: resolved
                .diagnostics
                .into_iter()
                .chain(applied_video.diagnostics)
                .chain(audio_diagnostics)
                .collect(),
        });
        Ok(())
    }

    fn resolve_capture_settings(&self, config: &RecorderConfig) -> Result<ResolvedCaptureSettings> {
        let screen = if config.screen_id.trim().is_empty() {
            self.capabilities.screens.first()
        } else {
            self.capabilities
                .screens
                .iter()
                .find(|screen| screen.id == config.screen_id)
        };
        let screen_id = screen.map(|screen| screen.id.clone()).unwrap_or_else(|| {
            if config.screen_id.trim().is_empty() {
                "0".into()
            } else {
                config.screen_id.clone()
            }
        });
        let output_width = if config.match_display {
            screen
                .map(|screen| screen.width)
                .unwrap_or(config.output_width)
        } else {
            config.output_width
        };
        let output_height = if config.match_display {
            screen
                .map(|screen| screen.height)
                .unwrap_or(config.output_height)
        } else {
            config.output_height
        };
        let fps = if config.match_display_fps {
            screen
                .and_then(|screen| screen.refresh_hz)
                .filter(|refresh| refresh.is_finite() && *refresh > 0.0)
                .map(|refresh| {
                    let refresh = refresh.min(240.0);
                    Rational::new((refresh * 1_000.0).round() as u32, 1_000)
                })
                .or_else(|| {
                    screen.is_none().then(|| {
                        self.capabilities
                            .frame_rates
                            .iter()
                            .flat_map(|range| range.native.iter().copied())
                            .max_by(|left, right| left.as_f64().total_cmp(&right.as_f64()))
                    })?
                })
                .unwrap_or(config.fps)
        } else {
            config.fps
        };
        let video_encoder_id = self
            .select_video_encoder(&config.video_encoder)
            .context("select video encoder")?;
        let mut diagnostics = Vec::new();
        if config.match_display && screen.is_none() {
            diagnostics.push(
                "The selected display was not reported; using the configured output size.".into(),
            );
        }
        if config.match_display_fps && screen.is_none() {
            diagnostics.push(
                "The selected display refresh rate was not reported; using the configured frame rate."
                    .into(),
            );
        }
        Ok(ResolvedCaptureSettings {
            screen_id,
            output_width,
            output_height,
            fps,
            video_encoder_id,
            diagnostics,
        })
    }

    fn stop_if_active(&mut self) -> Result<()> {
        if let Some(output) = self.output.as_mut() {
            if output.is_active().context("query replay output state")? {
                if let Err(error) = Self::stop_replay_output(output) {
                    self.status.replay_active = false;
                    self.status.state = RecorderState::Error;
                    self.status.last_error = Some(format!("Stop replay buffer failed: {error:#}"));
                    return Err(error).context("stop replay buffer");
                }
            }
        }
        self.status.replay_active = false;
        if !matches!(self.status.state, RecorderState::Error) {
            self.status.state = RecorderState::Stopped;
        }
        Ok(())
    }

    fn stop_replay_output(output: &mut ObsReplayBufferOutputRef) -> Result<()> {
        let mut stop_signals = output.signals().on_stop()?;
        let mut deactivate_signals = output.signals().on_deactivate()?;
        let output_ptr = output.as_ptr();
        let runtime = output.runtime().clone();

        runtime
            .run_with_obs_result(move || unsafe {
                // Safety: the smart pointer keeps the OBS output alive until
                // the stop request has been dispatched on the OBS runtime thread.
                sys::obs_output_stop(output_ptr.get_ptr());
            })
            .context("request replay buffer stop")?;

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stop_signal = None;
        let mut deactivated = false;
        while Instant::now() < deadline {
            if stop_signal.is_none() {
                if let Ok(signal) = stop_signals.try_recv() {
                    stop_signal = Some(signal);
                }
            }
            if !deactivated && deactivate_signals.try_recv().is_ok() {
                deactivated = true;
            }
            if stop_signal.is_some() && deactivated {
                if stop_signal != Some(ObsOutputStopSignal::Success) {
                    anyhow::bail!(
                        "OBS reported replay buffer stop failure: {}",
                        stop_signal.expect("stop signal checked")
                    );
                }
                return Ok(());
            }
            thread::sleep(Duration::from_millis(10));
        }

        anyhow::bail!("timed out waiting for OBS to stop the replay buffer")
    }

    fn save_replay_buffer(
        output: &ObsReplayBufferOutputRef,
        replay_seconds: u32,
    ) -> Result<PathBuf> {
        let mut saved_signals = output.replay_signals().on_saved()?;
        let mut stop_signals = output.signals().on_stop()?;
        let output_ptr = output.as_ptr();
        let runtime = output.runtime().clone();
        let proc_handler = runtime
            .run_with_obs_result(move || {
                let proc_handler =
                    unsafe { sys::obs_output_get_proc_handler(output_ptr.get_ptr()) };
                if proc_handler.is_null() {
                    return Err(ObsError::OutputSaveBufferFailure(
                        "Failed to get proc handler.".to_string(),
                    ));
                }
                Ok(Sendable(proc_handler))
            })
            .context("get replay buffer procedure handler")??;

        unsafe {
            runtime
                .call_proc_handler(&proc_handler, "save")
                .context("call replay buffer save procedure")?;
        }

        // OBS must wait for the next encoded packet, then mux the complete
        // replay buffer on a worker thread before emitting "saved". Allow at
        // least one minute for slow disks, antivirus scans, and long replays.
        let save_timeout_seconds = u64::from(replay_seconds).saturating_add(30).max(60);
        let deadline = Instant::now() + Duration::from_secs(save_timeout_seconds);
        loop {
            if saved_signals.try_recv().is_ok() {
                break;
            }
            if let Ok(stop_signal) = stop_signals.try_recv() {
                if stop_signal != ObsOutputStopSignal::Success {
                    anyhow::bail!("OBS stopped the replay buffer before saving ({stop_signal})");
                }
                anyhow::bail!("OBS stopped the replay buffer before saving the replay");
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for OBS to save the replay buffer after {save_timeout_seconds}s"
                );
            }
            thread::sleep(Duration::from_millis(10));
        }

        let mut calldata = unsafe {
            runtime
                .call_proc_handler(&proc_handler, "get_last_replay")
                .context("get last replay path")?
        };
        Ok(PathBuf::from(calldata.get_string("path")?))
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
                let monitor_id =
                    windows_monitor_device_id(screen_id).unwrap_or_else(|| screen_id.to_owned());
                settings.set_string("monitor_id", monitor_id.as_str())?;
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

    fn create_excluding_system_audio_source(
        &self,
        selectors: &[String],
        track: u8,
    ) -> Result<(ObsSourceRef, u8)> {
        if !self
            .input_types
            .contains("pipewire_audio_application_capture")
        {
            anyhow::bail!(
                "excluding application audio from the system track requires the linux-pipewire-audio OBS plugin"
            );
        }
        let settings_json = pipewire_application_exclusion_settings(selectors).to_string();
        let settings = ObsData::from_json(&settings_json, self.context.runtime().clone())
            .map_err(|error| anyhow::anyhow!(error.to_string()))
            .context("create PipeWire system audio exclusion settings")?;
        let source = ObsSourceRef::new(
            "pipewire_audio_application_capture",
            format!("Clip Engine System Audio Track {track} (excluding apps)"),
            Some(settings.into_immutable()),
            None,
            self.context.runtime().clone(),
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok((source, track))
    }

    fn create_audio_source(&self, source_id: &str, track: u8) -> Result<(ObsSourceRef, u8)> {
        let mut settings = self
            .context
            .data()
            .context("create audio source settings")?;
        let (source_type, name) = if cfg!(windows) {
            if let Some(window) = source_id.strip_prefix("application:") {
                if window.trim().is_empty() {
                    anyhow::bail!(
                        "application audio source is missing its Windows window selector"
                    );
                }
                let window = resolve_windows_audio_selector(window);
                settings.set_string("window", window.as_str())?;
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
            } else if let Some(device_id) = playback_device_id(source_id) {
                if device_id.trim().is_empty() {
                    anyhow::bail!(
                        "playback-device audio source is missing its Windows endpoint ID"
                    );
                }
                settings.set_string("device_id", device_id)?;
                settings.set_bool("use_device_timing", true)?;
                (
                    "wasapi_output_capture",
                    format!("Clip Engine Playback Device Track {track}"),
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
            if self.input_types.contains("pipewire_audio_input_capture") {
                // PW_ID_ANY makes the PipeWire plugin follow the session's default
                // input device, while TargetName keeps the setting compatible with
                // plugin versions that persist a device name.
                settings.set_int("TargetId", i64::from(u32::MAX))?;
                settings.set_string("TargetName", "")?;
                (
                    "pipewire_audio_input_capture",
                    format!("Clip Engine Microphone Track {track}"),
                )
            } else {
                settings.set_string("device_id", source_id.trim_start_matches("microphone:"))?;
                (
                    "pulse_input_capture",
                    format!("Clip Engine Microphone Track {track}"),
                )
            }
        } else if source_id.starts_with("system:") {
            if self.input_types.contains("pipewire_audio_output_capture") {
                settings.set_int("TargetId", i64::from(u32::MAX))?;
                settings.set_string("TargetName", "")?;
                (
                    "pipewire_audio_output_capture",
                    format!("Clip Engine System Audio Track {track}"),
                )
            } else {
                settings.set_string("device_id", source_id.trim_start_matches("system:"))?;
                (
                    "pulse_output_capture",
                    format!("Clip Engine System Audio Track {track}"),
                )
            }
        } else if let Some(application) = source_id.strip_prefix("application:") {
            if application.trim().is_empty() {
                anyhow::bail!("application audio source is missing its application selector");
            }
            if !self
                .input_types
                .contains("pipewire_audio_application_capture")
            {
                anyhow::bail!(
                    "PipeWire application audio requires the linux-pipewire-audio OBS plugin"
                );
            }
            // The plugin's MatchPriorty spelling is part of its public settings
            // ABI. Set the correctly-spelled alias as well for newer builds.
            settings.set_int("CaptureMode", 0)?;
            settings.set_int("MatchPriorty", 0)?;
            settings.set_int("MatchPriority", 0)?;
            settings.set_bool("ExceptApp", false)?;
            settings.set_string("TargetName", application)?;
            (
                "pipewire_audio_application_capture",
                format!("Clip Engine Application Audio Track {track}"),
            )
        } else {
            anyhow::bail!("unknown audio source {source_id}");
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
        let encoders = self.context.available_video_encoders()?;
        let requested = requested.trim();
        let selected = if requested.is_empty() || requested.eq_ignore_ascii_case("auto") {
            self.capabilities
                .video_encoders
                .iter()
                .min_by_key(|encoder| encoder_preference_score(encoder))
                .map(|encoder| encoder.id.clone())
        } else {
            Some(
                encoders
                    .iter()
                    .find_map(|encoder| {
                        let id: ObsString = encoder.get_encoder_id().clone().into();
                        (id == requested).then(|| id.to_string())
                    })
                    .ok_or_else(|| anyhow::anyhow!("video encoder {requested} is not available"))?,
            )
        };
        if encoders.is_empty() {
            anyhow::bail!("OBS reported no video encoders");
        }
        selected.ok_or_else(|| anyhow::anyhow!("OBS reported no video encoders"))
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

#[cfg(windows)]
fn windows_monitor_device_id(screen_id: &str) -> Option<String> {
    use std::{ffi::c_void, mem::size_of};
    use windows_sys::Win32::{
        Graphics::Gdi::{
            EnumDisplayDevicesW, GetMonitorInfoW, DISPLAY_DEVICEW, HMONITOR, MONITORINFO,
            MONITORINFOEXW,
        },
        UI::WindowsAndMessaging::EDD_GET_DEVICE_INTERFACE_NAME,
    };

    let handle = screen_id.parse::<usize>().ok()? as *mut c_void as HMONITOR;
    let mut monitor_info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
    monitor_info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let monitor_info_ptr = (&mut monitor_info as *mut MONITORINFOEXW).cast::<MONITORINFO>();
    if unsafe { GetMonitorInfoW(handle, monitor_info_ptr) } == 0 {
        return None;
    }
    let device_name_end = monitor_info
        .szDevice
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(monitor_info.szDevice.len());
    let device_name = String::from_utf16_lossy(&monitor_info.szDevice[..device_name_end]);

    let mut display_device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
    display_device.cb = size_of::<DISPLAY_DEVICEW>() as u32;
    if unsafe {
        EnumDisplayDevicesW(
            monitor_info.szDevice.as_ptr(),
            0,
            &mut display_device,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
    } == 0
    {
        return (!device_name.is_empty()).then_some(device_name);
    }

    let end = display_device
        .DeviceID
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(display_device.DeviceID.len());
    let device_id = String::from_utf16_lossy(&display_device.DeviceID[..end]);
    Some(if device_id.is_empty() {
        device_name
    } else {
        device_id
    })
}

#[cfg(not(windows))]
fn windows_monitor_device_id(_screen_id: &str) -> Option<String> {
    None
}

struct ResolvedCaptureSettings {
    screen_id: String,
    output_width: u32,
    output_height: u32,
    fps: Rational,
    video_encoder_id: String,
    diagnostics: Vec<String>,
}

struct AppliedVideoSettings {
    rate_control: String,
    quality_level: Option<u32>,
    video_bitrate_kbps: Option<u32>,
    max_bitrate_kbps: Option<u32>,
    diagnostics: Vec<String>,
}

fn apply_video_encoder_settings(
    settings: &mut libobs_wrapper::data::ObsData,
    config: &RecorderConfig,
    capability: Option<&EncoderCapability>,
) -> Result<AppliedVideoSettings> {
    let (rate_control, mut diagnostics) = resolve_rate_control(config, capability);
    let quality_supported =
        encoder_property_key(capability, &["target_quality", "cqp", "cq", "qp", "crf"]).is_some();
    let bitrate_supported =
        encoder_property_key(capability, &["bitrate", "bitrate_kbps"]).is_some();
    let max_bitrate_supported =
        encoder_property_key(capability, &["max_bitrate", "max_bitrate_kbps"]).is_some();

    let rate_control_value = match rate_control {
        RateControl::Cbr => "CBR",
        RateControl::Cqp => "CQP",
        RateControl::Vbr => "VBR",
        // OBS NVENC exposes CQVBR under different names across releases. VBR is
        // the compatible fallback while the canonical setting remains visible.
        RateControl::Cqvbr => "CQVBR",
    };
    let _ = set_string_encoder_property(
        settings,
        capability,
        &["rate_control", "rc"],
        rate_control_value,
        "rate control",
        &mut diagnostics,
    )?;

    let quality_level =
        if matches!(rate_control, RateControl::Cqp | RateControl::Cqvbr) && quality_supported {
            let quality_keys = if matches!(rate_control, RateControl::Cqvbr) {
                &["target_quality", "cqp", "cq", "qp", "crf"][..]
            } else {
                &["cqp", "target_quality", "cq", "qp", "crf"][..]
            };
            let key = encoder_property_key(capability, quality_keys);
            if let Some(key) = key {
                let value = bounded_encoder_integer(
                    capability,
                    key,
                    i64::from(config.quality_level),
                    "quality",
                    &mut diagnostics,
                );
                settings.set_int(key, value)?;
                Some(u32::try_from(value).unwrap_or(config.quality_level))
            } else {
                None
            }
        } else {
            None
        };

    let video_bitrate_kbps = if bitrate_supported
        && matches!(
            rate_control,
            RateControl::Cbr | RateControl::Vbr | RateControl::Cqvbr
        ) {
        let key = encoder_property_key(capability, &["bitrate", "bitrate_kbps"])
            .expect("bitrate_supported guarantees a key");
        let value = bounded_encoder_integer(
            capability,
            key,
            i64::from(config.video_bitrate_kbps),
            "video bitrate",
            &mut diagnostics,
        );
        settings.set_int(key, value)?;
        Some(u32::try_from(value).unwrap_or(config.video_bitrate_kbps))
    } else {
        None
    };
    let max_bitrate_kbps = if config.max_bitrate_kbps > 0 && max_bitrate_supported {
        let key = encoder_property_key(capability, &["max_bitrate", "max_bitrate_kbps"])
            .expect("max_bitrate_supported guarantees a key");
        let value = bounded_encoder_integer(
            capability,
            key,
            i64::from(config.max_bitrate_kbps),
            "maximum bitrate",
            &mut diagnostics,
        );
        settings.set_int(key, value)?;
        Some(u32::try_from(value).unwrap_or(config.max_bitrate_kbps))
    } else {
        None
    };

    let _ = set_int_encoder_property(
        settings,
        capability,
        &["keyint_sec", "keyframe_interval", "keyframe_interval_sec"],
        i64::from(config.keyframe_interval_seconds),
        "keyframe interval",
        &mut diagnostics,
    )?;
    let _ = set_string_encoder_property(
        settings,
        capability,
        &["preset", "preset2"],
        &config.preset,
        "preset",
        &mut diagnostics,
    )?;
    let _ = set_string_encoder_property(
        settings,
        capability,
        &["tune", "tuning"],
        &config.tuning,
        "tuning",
        &mut diagnostics,
    )?;
    let multipass = match config.multipass {
        Multipass::Disabled => "disabled",
        Multipass::QuarterResolution => "qres",
        Multipass::FullResolution => "fullres",
    };
    let _ = set_string_encoder_property(
        settings,
        capability,
        &["multipass", "multi_pass"],
        multipass,
        "multipass",
        &mut diagnostics,
    )?;
    let _ = set_string_encoder_property(
        settings,
        capability,
        &["profile"],
        &config.profile,
        "profile",
        &mut diagnostics,
    )?;
    let _ = set_bool_encoder_property(
        settings,
        capability,
        &["lookahead", "rc-lookahead", "look_ahead"],
        config.lookahead,
        "look-ahead",
        &mut diagnostics,
    )?;
    let _ = set_bool_encoder_property(
        settings,
        capability,
        &["adaptive_quantization", "spatial-aq", "spatial_aq"],
        config.adaptive_quantization,
        "adaptive quantization",
        &mut diagnostics,
    )?;
    let _ = set_int_encoder_property(
        settings,
        capability,
        &["bframes", "bf", "b_frames"],
        i64::from(config.b_frames),
        "B-frames",
        &mut diagnostics,
    )?;
    let b_frame_ref_value = match config.b_frame_ref_mode.to_ascii_lowercase().as_str() {
        "each" => 1,
        "middle" => 2,
        _ => 0,
    };
    let _ = set_integer_or_string_encoder_property(
        settings,
        capability,
        &["bf_ref_mode", "b_ref_mode", "bframe_ref_mode"],
        b_frame_ref_value,
        &config.b_frame_ref_mode,
        "B-frame reference mode",
        &mut diagnostics,
    )?;
    let split_encode_value = match config.split_encode.to_ascii_lowercase().as_str() {
        "disabled" | "off" => 1,
        "enabled" | "on" | "two" => 2,
        "three" => 3,
        "four" => 4,
        _ => 0,
    };
    let _ = set_integer_or_string_encoder_property(
        settings,
        capability,
        &["split_encode", "split-encode"],
        split_encode_value,
        &config.split_encode,
        "split encode",
        &mut diagnostics,
    )?;
    let _ = set_int_encoder_property(
        settings,
        capability,
        &["gpu", "device"],
        i64::from(config.gpu),
        "GPU selection",
        &mut diagnostics,
    )?;
    let _ = set_bool_encoder_property(
        settings,
        capability,
        &["rescale"],
        config.rescale_output,
        "encoder rescale",
        &mut diagnostics,
    )?;

    if config.mode == RecorderMode::Advanced {
        apply_custom_encoder_options(
            settings,
            capability,
            &config.custom_encoder_options,
            &mut diagnostics,
        )?;
    }

    Ok(AppliedVideoSettings {
        rate_control: rate_control.label().into(),
        quality_level,
        video_bitrate_kbps,
        max_bitrate_kbps,
        diagnostics,
    })
}

fn resolve_rate_control(
    config: &RecorderConfig,
    capability: Option<&EncoderCapability>,
) -> (RateControl, Vec<String>) {
    let rate_control_supported =
        encoder_property_key(capability, &["rate_control", "rc"]).is_some();
    let quality_supported =
        encoder_property_key(capability, &["target_quality", "cqp", "cq", "qp", "crf"]).is_some();
    let mut diagnostics = Vec::new();
    let mut rate_control = config.rate_control;
    let quality_mode_requested =
        matches!(config.rate_control, RateControl::Cqp | RateControl::Cqvbr);
    if !rate_control_supported {
        rate_control = RateControl::Cbr;
        diagnostics.push(
            "The selected encoder does not expose rate control; its native rate-control default was retained."
                .into(),
        );
    } else if config.mode == RecorderMode::Automatic && !quality_supported {
        rate_control = RateControl::Cbr;
        diagnostics.push(
            "The selected encoder does not expose quality-based rate control; Automatic mode fell back to CBR."
                .into(),
        );
    } else if quality_mode_requested && !quality_supported {
        rate_control = RateControl::Cbr;
        diagnostics.push(
            "The selected encoder does not expose the requested quality control; Advanced mode fell back to CBR."
                .into(),
        );
    }
    (rate_control, diagnostics)
}

fn apply_custom_encoder_options(
    settings: &mut libobs_wrapper::data::ObsData,
    capability: Option<&EncoderCapability>,
    options: &str,
    diagnostics: &mut Vec<String>,
) -> Result<()> {
    if !options.trim().is_empty() {
        if let Some(key) = encoder_property_key(
            capability,
            &["opts", "x264opts", "ffmpeg_opts", "vaapi_opts"],
        ) {
            let normalized = options
                .split(['\n', ';', ','])
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            settings.set_string(key, normalized)?;
            return Ok(());
        }
    }
    for item in options
        .split(['\n', ';', ','])
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let Some((key, value)) = item.split_once('=') else {
            diagnostics.push(format!("Ignored custom encoder option without '=': {item}"));
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if encoder_property_key(capability, &[key]).is_none() {
            diagnostics.push(format!(
                "Ignored unsupported custom encoder property '{key}'."
            ));
            continue;
        }
        match capability.and_then(|capability| {
            capability
                .settings
                .iter()
                .find(|setting| setting.key == key)
                .map(|setting| setting.kind)
        }) {
            Some(EncoderSettingKind::Boolean) => {
                if let Ok(value) = value.parse::<bool>() {
                    settings.set_bool(key, value)?;
                } else {
                    diagnostics.push(format!("Ignored invalid boolean value for '{key}'."));
                }
            }
            Some(EncoderSettingKind::Integer) => {
                if let Ok(value) = value.parse::<i64>() {
                    settings.set_int(key, value)?;
                } else {
                    diagnostics.push(format!("Ignored invalid integer value for '{key}'."));
                }
            }
            Some(EncoderSettingKind::Float) => {
                if let Ok(value) = value.parse::<f64>() {
                    settings.set_double(key, value)?;
                } else {
                    diagnostics.push(format!("Ignored invalid numeric value for '{key}'."));
                }
            }
            _ => {
                settings.set_string(key, value)?;
            }
        }
    }
    Ok(())
}

fn encoder_property_key<'a>(
    capability: Option<&'a EncoderCapability>,
    keys: &[&'a str],
) -> Option<&'a str> {
    let Some(capability) = capability else {
        return keys.first().copied();
    };
    if capability.settings.is_empty() {
        return keys.first().copied();
    }
    keys.iter().copied().find(|key| {
        capability
            .settings
            .iter()
            .any(|setting| setting.key == *key)
    })
}

fn set_string_encoder_property(
    settings: &mut libobs_wrapper::data::ObsData,
    capability: Option<&EncoderCapability>,
    keys: &[&str],
    value: &str,
    _label: &str,
    _diagnostics: &mut Vec<String>,
) -> Result<bool> {
    let Some(key) = encoder_property_key(capability, keys) else {
        return Ok(false);
    };
    let Some(setting) = capability.and_then(|capability| {
        capability
            .settings
            .iter()
            .find(|setting| setting.key == key)
    }) else {
        settings.set_string(key, value)?;
        return Ok(true);
    };
    if setting.kind == EncoderSettingKind::List {
        let option_index = setting
            .options
            .iter()
            .position(|option| option.eq_ignore_ascii_case(value))
            .or_else(|| {
                setting
                    .options
                    .iter()
                    .position(|option| semantic_option_matches(option, value))
            })
            .or_else(|| {
                setting
                    .option_values
                    .iter()
                    .position(|option| option.eq_ignore_ascii_case(value))
            });
        let option_index = option_index.unwrap_or(0);
        let native_value = setting
            .option_values
            .get(option_index)
            .or_else(|| setting.options.get(option_index))
            .map(String::as_str)
            .unwrap_or(value);
        settings.set_string(key, native_value)?;
    } else {
        settings.set_string(key, value)?;
    }
    Ok(true)
}

fn semantic_option_matches(option: &str, requested: &str) -> bool {
    let option = option.to_ascii_lowercase().replace([' ', '_', '-'], "");
    let requested = requested.to_ascii_lowercase();
    match requested.as_str() {
        "cqp" => {
            option == "crf"
                || option == "cq"
                || option.contains("cqp")
                || option.contains("constantqp")
        }
        "cqvbr" => option.contains("cqvbr") || option.contains("qualityvbr"),
        "cbr" => option == "cbr" || option.contains("constantbitrate"),
        "vbr" => option == "vbr" || option.contains("variablebitrate"),
        _ => false,
    }
}

fn set_int_encoder_property(
    settings: &mut libobs_wrapper::data::ObsData,
    capability: Option<&EncoderCapability>,
    keys: &[&str],
    value: i64,
    label: &str,
    diagnostics: &mut Vec<String>,
) -> Result<bool> {
    let Some(key) = encoder_property_key(capability, keys) else {
        return Ok(false);
    };
    let value = bounded_encoder_integer(capability, key, value, label, diagnostics);
    settings.set_int(key, value)?;
    Ok(true)
}

fn set_integer_or_string_encoder_property(
    settings: &mut libobs_wrapper::data::ObsData,
    capability: Option<&EncoderCapability>,
    keys: &[&str],
    integer_value: i64,
    string_value: &str,
    label: &str,
    diagnostics: &mut Vec<String>,
) -> Result<bool> {
    let Some(key) = encoder_property_key(capability, keys) else {
        return Ok(false);
    };
    let kind = capability.and_then(|capability| {
        capability
            .settings
            .iter()
            .find(|setting| setting.key == key)
            .map(|setting| setting.kind)
    });
    match kind {
        Some(EncoderSettingKind::Text) => set_string_encoder_property(
            settings,
            capability,
            &[key],
            string_value,
            label,
            diagnostics,
        )
        .map(|_| true),
        Some(EncoderSettingKind::List)
            if capability.is_some_and(|capability| {
                capability
                    .settings
                    .iter()
                    .find(|setting| setting.key == key)
                    .is_some_and(|setting| {
                        setting
                            .option_values
                            .iter()
                            .any(|option| option.parse::<i64>().ok() == Some(integer_value))
                    })
            }) =>
        {
            let value = bounded_encoder_integer(capability, key, integer_value, label, diagnostics);
            settings.set_int(key, value)?;
            Ok(true)
        }
        Some(EncoderSettingKind::List) => set_string_encoder_property(
            settings,
            capability,
            &[key],
            string_value,
            label,
            diagnostics,
        )
        .map(|_| true),
        Some(EncoderSettingKind::Float) => {
            settings.set_double(key, integer_value as f64)?;
            Ok(true)
        }
        _ => {
            let value = bounded_encoder_integer(capability, key, integer_value, label, diagnostics);
            settings.set_int(key, value)?;
            Ok(true)
        }
    }
}

fn bounded_encoder_integer(
    capability: Option<&EncoderCapability>,
    key: &str,
    value: i64,
    label: &str,
    diagnostics: &mut Vec<String>,
) -> i64 {
    let Some(setting) = capability.and_then(|capability| {
        capability
            .settings
            .iter()
            .find(|setting| setting.key == key)
    }) else {
        return value;
    };
    let minimum = setting.min.map(|minimum| minimum.ceil() as i64);
    let maximum = setting.max.map(|maximum| maximum.floor() as i64);
    let bounded = value
        .max(minimum.unwrap_or(i64::MIN))
        .min(maximum.unwrap_or(i64::MAX));
    if bounded != value {
        diagnostics.push(format!(
            "The active encoder clamped {label} from {value} to {bounded}."
        ));
    }
    bounded
}

fn set_bool_encoder_property(
    settings: &mut libobs_wrapper::data::ObsData,
    capability: Option<&EncoderCapability>,
    keys: &[&str],
    value: bool,
    _label: &str,
    _diagnostics: &mut Vec<String>,
) -> Result<bool> {
    let Some(key) = encoder_property_key(capability, keys) else {
        return Ok(false);
    };
    settings.set_bool(key, value)?;
    Ok(true)
}

fn prepare_obs_muxer() -> Result<()> {
    let executable = env::current_exe().context("locate recorder executable")?;
    let executable_directory = executable
        .parent()
        .context("recorder executable has no parent directory")?;
    let muxer_name = if cfg!(windows) {
        "obs-ffmpeg-mux.exe"
    } else {
        "obs-ffmpeg-mux"
    };
    let destination = executable_directory.join(muxer_name);
    prepare_obs_encoder_helpers(executable_directory)?;
    if destination.is_file() {
        return Ok(());
    }

    let mut candidates = Vec::new();
    if let Some(root) = select_resource_root() {
        candidates.extend([
            root.join("bin").join("64bit").join(muxer_name),
            root.join("bin").join(muxer_name),
            root.join(muxer_name),
        ]);
    } else if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|directory| directory.join(muxer_name)));
    }
    let source = candidates
        .iter()
        .find(|candidate| candidate.is_file())
        .with_context(|| {
            format!(
                "OBS runtime does not contain {muxer_name} next to the recorder or in its runtime PATH"
            )
        })?;

    if fs::symlink_metadata(&destination).is_ok() {
        fs::remove_file(&destination)
            .with_context(|| format!("remove stale {}", destination.display()))?;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, &destination).with_context(|| {
            format!(
                "link OBS mux helper {} to {}",
                source.display(),
                destination.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    {
        fs::copy(source, &destination).with_context(|| {
            format!(
                "copy OBS mux helper {} to {}",
                source.display(),
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn prepare_obs_encoder_helpers(executable_directory: &Path) -> Result<()> {
    let helper_names = if cfg!(windows) {
        vec!["obs-nvenc-test.exe", "obs-qsv-test.exe"]
    } else if cfg!(target_os = "linux") {
        vec!["obs-nvenc-test"]
    } else {
        Vec::new()
    };

    for helper_name in helper_names {
        let destination = executable_directory.join(helper_name);
        if destination.is_file() {
            continue;
        }

        let mut candidates = Vec::new();
        if let Some(root) = select_resource_root() {
            candidates.extend([
                root.join("bin").join("64bit").join(helper_name),
                root.join("bin").join(helper_name),
                root.join(helper_name),
            ]);
        }
        if let Some(path) = env::var_os("PATH") {
            candidates.extend(env::split_paths(&path).map(|directory| directory.join(helper_name)));
        }
        let Some(source) = candidates.iter().find(|candidate| candidate.is_file()) else {
            continue;
        };

        if fs::symlink_metadata(&destination).is_ok() {
            if let Err(error) = fs::remove_file(&destination) {
                if is_read_only_filesystem_error(&error) {
                    eprintln!(
                        "OBS encoder helper {helper_name} cannot be staged on a read-only filesystem; \
                         package it beside the recorder executable"
                    );
                    continue;
                }
                return Err(anyhow::Error::new(error)
                    .context(format!("remove stale {}", destination.display())));
            }
        }
        let stage_result: std::io::Result<()> = {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(source, &destination)
            }
            #[cfg(not(unix))]
            {
                fs::copy(source, &destination).map(|_| ())
            }
        };
        if let Err(error) = stage_result {
            if is_read_only_filesystem_error(&error) {
                eprintln!(
                    "OBS encoder helper {helper_name} cannot be staged on a read-only filesystem; \
                     package it beside the recorder executable"
                );
                continue;
            }
            return Err(anyhow::Error::new(error).context(format!(
                "stage OBS encoder helper {} beside {}",
                source.display(),
                destination.display()
            )));
        }
    }
    Ok(())
}

fn is_read_only_filesystem_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
    )
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
        self.refreshed_capabilities()
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
        match self.save_replay_inner() {
            Ok(replay) => {
                self.status.last_error = None;
                self.status.last_replay_path = Some(path_string(&replay.path));
                Ok(replay)
            }
            Err(error) => {
                self.status.last_error = Some(format!("Replay save failed: {error:#}"));
                Err(error)
            }
        }
    }
}

impl ObsBackend {
    fn save_replay_inner(&mut self) -> Result<ReplayFile> {
        if !self.status.replay_active {
            anyhow::bail!("replay buffer is not running");
        }
        let config = self
            .config
            .as_ref()
            .context("recorder configuration is missing")?
            .clone();
        let output = self.output.as_ref().context("replay output is missing")?;
        let source_path = Self::save_replay_buffer(output, config.replay_seconds)
            .context("save replay buffer")?;
        let source_path = wait_for_stable_file(&source_path)?;
        let destination_directory = output_directory(&config);
        let destination = handoff_replay(&source_path, &destination_directory)?;
        Ok(ReplayFile {
            path: destination,
            duration_seconds: config.replay_seconds,
        })
    }
}

fn discover_capabilities(
    context: &ObsContext,
    input_types: &[String],
) -> Result<RecorderCapabilities> {
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
            backend: detect_backend(input_types).0,
        })
        .collect::<Vec<_>>();
    let (backend, mut diagnostics) = detect_backend(input_types);

    let mut video_encoders = Vec::new();
    for encoder in context.available_video_encoders()? {
        let id: ObsString = encoder.get_encoder_id().clone().into();
        let id = id.to_string();
        video_encoders.push(video_encoder_capability(&encoder, id));
    }
    if !video_encoders
        .iter()
        .any(|encoder| encoder.id.to_ascii_lowercase().contains("nvenc"))
    {
        diagnostics.push(if encoder_plugin_available("obs-nvenc") {
            "The bundled obs-nvenc plugin is present, but no NVIDIA encoder was exposed. Install or update the NVIDIA driver and verify that libnvidia-encode.so.1 is available.".into()
        } else {
            "The OBS runtime does not contain obs-nvenc, so NVIDIA encoders cannot be listed. Rebuild the bundled runtime with the NVENC plugin.".into()
        });
    }
    if !video_encoders
        .iter()
        .any(|encoder| encoder.id.to_ascii_lowercase().contains("qsv"))
    {
        diagnostics.push(if encoder_plugin_available("obs-qsv11") {
            "The bundled obs-qsv11 plugin is present, but no Intel Quick Sync encoder was exposed. Install or update the Intel graphics driver and ensure the encoder capability-test helper can run.".into()
        } else {
            "The OBS runtime does not contain obs-qsv11, so Intel Quick Sync encoders cannot be listed. Rebuild the bundled runtime with the QSV plugin.".into()
        });
    }
    #[cfg(target_os = "linux")]
    if !video_encoders
        .iter()
        .any(|encoder| encoder.id.to_ascii_lowercase().contains("vaapi"))
    {
        diagnostics.push(if encoder_plugin_available("obs-ffmpeg") {
            "The bundled obs-ffmpeg plugin is present, but no VAAPI encoder was exposed. Install the matching Mesa/VAAPI driver for the GPU.".into()
        } else {
            "The OBS runtime does not contain obs-ffmpeg, so AMD and Intel VAAPI encoders cannot be listed.".into()
        });
    }
    #[cfg(windows)]
    if !video_encoders
        .iter()
        .any(|encoder| encoder.id.to_ascii_lowercase().contains("amf"))
    {
        diagnostics.push(if encoder_plugin_available("obs-ffmpeg") {
            "The bundled obs-ffmpeg plugin is present, but no AMD AMF encoder was exposed. Install or update the AMD graphics driver.".into()
        } else {
            "The OBS runtime does not contain obs-ffmpeg, so AMD AMF encoders cannot be listed."
                .into()
        });
    }
    let mut audio_encoders = Vec::new();
    for encoder in context.available_audio_encoders()? {
        let id: ObsString = encoder.get_encoder_id().clone().into();
        let id = id.to_string();
        audio_encoders.push(audio_encoder_capability(&encoder, id));
    }

    let mut audio_sources = Vec::new();
    let has_pipewire_output = input_types
        .iter()
        .any(|id| id == "pipewire_audio_output_capture");
    if has_pipewire_output || input_types.iter().any(|id| id == "pulse_output_capture") {
        audio_sources.push(AudioSourceCapability {
            id: "system:default".into(),
            label: "System audio".into(),
            kind: AudioSourceKind::System,
            process_id: None,
            available: true,
            detail: Some(if has_pipewire_output {
                "PipeWire default output".into()
            } else {
                "PulseAudio/PipeWire default output monitor".into()
            }),
        });
    }
    let has_pipewire_input = input_types
        .iter()
        .any(|id| id == "pipewire_audio_input_capture");
    if has_pipewire_input || input_types.iter().any(|id| id == "pulse_input_capture") {
        audio_sources.push(AudioSourceCapability {
            id: "microphone:default".into(),
            label: "Default microphone".into(),
            kind: AudioSourceKind::Microphone,
            process_id: None,
            available: true,
            detail: Some(if has_pipewire_input {
                "PipeWire default input".into()
            } else {
                "PulseAudio/PipeWire default input".into()
            }),
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
        #[cfg(windows)]
        {
            match enumerate_windows_playback_devices() {
                Ok(devices) => {
                    audio_sources.extend(devices.into_iter().map(|(device_id, label)| {
                        playback_device_capability(
                            &device_id,
                            label,
                            true,
                            Some("WASAPI render endpoint".into()),
                        )
                    }));
                }
                Err(error) => diagnostics.push(format!(
                    "WASAPI playback devices could not be enumerated: {error:#}"
                )),
            }
        }
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
                audio_sources.extend(applications.into_iter().map(
                    |(selector, label, process_id, executable)| AudioSourceCapability {
                        id: format!("application:{selector}"),
                        label,
                        kind: AudioSourceKind::Application,
                        process_id: Some(process_id),
                        available: true,
                        detail: Some(format!(
                            "WASAPI process loopback · {executable} · PID {process_id}"
                        )),
                    },
                ));
            }
        }
        #[cfg(not(windows))]
        diagnostics.push(
            "Application audio capture is available through window selectors; application entries are refreshed by the desktop client.".into(),
        );
    }

    let has_pipewire_application = input_types
        .iter()
        .any(|id| id == "pipewire_audio_application_capture");
    if has_pipewire_application {
        #[cfg(target_os = "linux")]
        {
            match enumerate_pipewire_applications() {
                Ok(applications) if applications.is_empty() => diagnostics.push(
                    "PipeWire application capture is available. Start an application and refresh to discover its audio stream; custom executable/app-name selectors can also be added manually."
                        .into(),
                ),
                Ok(applications) => audio_sources.extend(applications),
                Err(error) => diagnostics.push(format!(
                    "PipeWire application audio is available, but active applications could not be enumerated: {error}. You can still add an executable or app name manually."
                )),
            }
        }
        #[cfg(not(target_os = "linux"))]
        diagnostics.push(
            "The PipeWire application audio source is present, but this platform does not provide the PipeWire session enumerator.".into(),
        );
    } else if matches!(backend, CaptureBackend::PipeWire | CaptureBackend::X11) {
        diagnostics.push(
            "Per-application Linux audio requires the linux-pipewire-audio OBS plugin; system and microphone routes remain available without it."
                .into(),
        );
    }

    let reported_max_fps = screens
        .iter()
        .filter_map(|screen| screen.refresh_hz)
        .filter(|refresh| refresh.is_finite() && *refresh > 0.0)
        .fold(0.0_f64, f64::max);
    let max_fps = if reported_max_fps > 0.0 {
        reported_max_fps.ceil().min(240.0) as u32
    } else {
        240
    };
    let mut native = [30, 60, 120, 144, 165, 240]
        .into_iter()
        .filter(|fps| *fps <= max_fps)
        .map(|fps| Rational::new(fps, 1))
        .collect::<Vec<_>>();
    for refresh in screens.iter().filter_map(|screen| screen.refresh_hz) {
        let refresh = refresh.min(240.0);
        if refresh.is_finite() && refresh >= 1.0 {
            let native_rate = Rational::new((refresh * 1_000.0).round() as u32, 1_000);
            if !native.contains(&native_rate) {
                native.push(native_rate);
            }
        }
    }
    native.sort_by(|left, right| left.as_f64().total_cmp(&right.as_f64()));

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
        audio_isolation_available: cfg!(target_os = "linux") && has_pipewire_application,
        diagnostics,
    })
}

fn system_audio_exclusion_selectors(config: &RecorderConfig) -> Vec<String> {
    if config.system_audio_mode != SystemAudioMode::ExcludeApplications {
        return Vec::new();
    }

    let mut seen = BTreeSet::new();
    config
        .audio_routes
        .iter()
        .filter(|route| route.enabled)
        .filter_map(|route| route.source_id.strip_prefix("application:"))
        .map(str::trim)
        .filter(|selector| !selector.is_empty())
        .filter(|selector| seen.insert(selector.to_ascii_lowercase()))
        .map(str::to_owned)
        .collect()
}

fn pipewire_application_exclusion_settings(selectors: &[String]) -> serde_json::Value {
    let apps = selectors
        .iter()
        .map(|selector| {
            serde_json::json!({
                "hidden": false,
                "selected": false,
                "value": selector,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "CaptureMode": 1,
        "MatchPriorty": 0,
        "MatchPriority": 0,
        "ExceptApp": true,
        "apps": apps,
    })
}

fn video_encoder_capability(encoder: &ObsVideoEncoderBuilder, id: String) -> EncoderCapability {
    let (codec, family) = encoder_codec_family(&id);
    EncoderCapability {
        label: encoder_display_label(&id),
        hardware: is_hardware_encoder(&id),
        codec: codec.into(),
        family: family.into(),
        settings: discover_encoder_settings(encoder.get_properties()),
        id,
    }
}

fn audio_encoder_capability(encoder: &ObsAudioEncoderBuilder, id: String) -> EncoderCapability {
    EncoderCapability {
        label: id.clone(),
        hardware: false,
        codec: "audio".into(),
        family: "audio".into(),
        settings: discover_encoder_settings(encoder.get_properties()),
        id,
    }
}

fn discover_encoder_settings(
    properties: std::result::Result<
        std::collections::HashMap<String, ObsProperty>,
        libobs_wrapper::utils::ObsError,
    >,
) -> Vec<EncoderSettingCapability> {
    properties
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(key, property)| {
            let (kind, min, max, step, options, option_values) = match property {
                ObsProperty::Bool => (
                    EncoderSettingKind::Boolean,
                    None,
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                ),
                ObsProperty::Int(property) => (
                    EncoderSettingKind::Integer,
                    Some(f64::from(*property.min())),
                    Some(f64::from(*property.max())),
                    Some(f64::from(*property.step())),
                    Vec::new(),
                    Vec::new(),
                ),
                ObsProperty::Float(property) => (
                    EncoderSettingKind::Float,
                    Some(*property.min()),
                    Some(*property.max()),
                    Some(*property.step()),
                    Vec::new(),
                    Vec::new(),
                ),
                ObsProperty::Text(_) => (
                    EncoderSettingKind::Text,
                    None,
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                ),
                ObsProperty::List(property) => {
                    let items = property
                        .items()
                        .iter()
                        .filter(|item| !item.disabled())
                        .collect::<Vec<_>>();
                    (
                        EncoderSettingKind::List,
                        None,
                        None,
                        None,
                        items.iter().map(|item| item.name().clone()).collect(),
                        items
                            .iter()
                            .map(|item| {
                                let value = format!("{:?}", item.value());
                                obs_list_value_string(&value)
                            })
                            .collect(),
                    )
                }
                _ => return None,
            };
            Some(EncoderSettingCapability {
                key,
                kind,
                options,
                option_values,
                min,
                max,
                step,
                description: None,
            })
        })
        .collect()
}

fn obs_list_value_string(value: &str) -> String {
    if let Some(value) = value
        .strip_prefix("String(\"")
        .and_then(|value| value.strip_suffix("\")"))
    {
        return value.to_string();
    }
    for prefix in ["Int(", "Float(", "Bool("] {
        if let Some(value) = value
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix(')'))
        {
            return value.to_string();
        }
    }
    value.to_string()
}

fn encoder_codec_family(id: &str) -> (&'static str, &'static str) {
    let id = id.to_ascii_lowercase();
    if id.contains("av1") {
        ("av1", "AV1")
    } else if id.contains("hevc") || id.contains("h265") {
        ("hevc", "HEVC")
    } else if id.contains("264") || id.contains("x264") {
        ("h264", "H.264")
    } else {
        ("unknown", "Other")
    }
}

fn encoder_display_label(id: &str) -> String {
    let normalized = id.to_ascii_lowercase();
    if normalized.contains("nvenc") {
        if normalized.contains("av1") {
            return "NVIDIA NVENC AV1".into();
        }
        if normalized.contains("hevc") || normalized.contains("h265") {
            return "NVIDIA NVENC HEVC".into();
        }
        if normalized.contains("264") {
            return "NVIDIA NVENC H.264".into();
        }
    }
    if normalized.contains("aom") && normalized.contains("av1") {
        return "AOM AV1".into();
    }
    if normalized.contains("svt") && normalized.contains("av1") {
        return "SVT-AV1".into();
    }
    if normalized.contains("x264") {
        return "x264".into();
    }
    id.into()
}

fn encoder_preference_score(encoder: &EncoderCapability) -> (u8, u8, String) {
    let codec_score = if encoder.hardware {
        match encoder.codec.as_str() {
            "av1" => 0,
            "hevc" => 1,
            "h264" => 2,
            _ => 3,
        }
    } else {
        // Software H.264 is the safest fallback for a high-FPS replay buffer;
        // software AV1/HEVC encoders can be dramatically more expensive.
        match encoder.codec.as_str() {
            "h264" => 0,
            "hevc" => 1,
            "av1" => 2,
            _ => 3,
        }
    };
    let hardware_score = if encoder.hardware { 0 } else { 1 };
    (hardware_score, codec_score, encoder.id.clone())
}

#[cfg(target_os = "linux")]
fn enumerate_pipewire_applications() -> Result<Vec<AudioSourceCapability>> {
    let mut applications = BTreeMap::new();
    let mut command_succeeded = false;

    if let Ok(output) = Command::new("pw-dump").output() {
        if output.status.success() {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                command_succeeded = true;
                collect_pipewire_applications(&value, &mut applications);
            }
        }
    }

    // pipewire-pulse exposes the same stream metadata through pactl. It is a
    // useful fallback on CachyOS sessions where pw-dump is sandboxed or absent.
    if applications.is_empty() {
        if let Ok(output) = Command::new("pactl")
            .args(["-f", "json", "list", "sink-inputs"])
            .output()
        {
            if output.status.success() {
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                    command_succeeded = true;
                    collect_pulse_applications(&value, &mut applications);
                }
            }
        }
    }

    if !command_succeeded {
        anyhow::bail!("pw-dump and pactl did not return application stream metadata");
    }
    Ok(applications.into_values().collect())
}

#[cfg(target_os = "linux")]
fn collect_pipewire_applications(
    value: &serde_json::Value,
    applications: &mut BTreeMap<String, AudioSourceCapability>,
) {
    let Some(objects) = value.as_array() else {
        return;
    };
    for object in objects {
        let Some(properties) = object
            .get("info")
            .and_then(|info| info.get("props"))
            .and_then(serde_json::Value::as_object)
        else {
            continue;
        };
        collect_application_properties(properties, applications);
    }
}

#[cfg(target_os = "linux")]
fn collect_pulse_applications(
    value: &serde_json::Value,
    applications: &mut BTreeMap<String, AudioSourceCapability>,
) {
    let Some(objects) = value.as_array() else {
        return;
    };
    for object in objects {
        let Some(properties) = object
            .get("properties")
            .and_then(serde_json::Value::as_object)
        else {
            continue;
        };
        collect_application_properties(properties, applications);
    }
}

#[cfg(target_os = "linux")]
fn collect_application_properties(
    properties: &serde_json::Map<String, serde_json::Value>,
    applications: &mut BTreeMap<String, AudioSourceCapability>,
) {
    if property_string(properties, "media.class").as_deref() != Some("Stream/Output/Audio") {
        return;
    }
    let binary = property_string(properties, "application.process.binary");
    let application_name = property_string(properties, "application.name");
    let Some(target) = binary
        .clone()
        .or_else(|| application_name.clone())
        .or_else(|| property_string(properties, "node.name"))
    else {
        return;
    };
    let target = target.trim();
    if target.is_empty() {
        return;
    }
    let key = target.to_ascii_lowercase();
    let label = binary
        .or_else(|| application_name.clone())
        .or_else(|| property_string(properties, "node.name"))
        .unwrap_or_else(|| target.to_string());
    let process_id = property_string(properties, "application.process.id")
        .and_then(|value| value.parse::<u32>().ok());
    let detail = match (application_name, process_id) {
        (Some(application_name), Some(process_id)) => {
            format!("PipeWire application capture · {application_name} · PID {process_id}")
        }
        (_, Some(process_id)) => format!("PipeWire application capture · PID {process_id}"),
        _ => "PipeWire application capture · executable/app-name match".into(),
    };
    applications
        .entry(key)
        .or_insert_with(|| AudioSourceCapability {
            id: format!("application:{target}"),
            label,
            kind: AudioSourceKind::Application,
            process_id,
            available: true,
            detail: Some(detail),
        });
}

#[cfg(target_os = "linux")]
fn property_string(
    properties: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    let value = properties.get(key)?;
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(windows)]
fn enumerate_windows() -> Vec<(String, String, u32, String)> {
    use std::ffi::c_void;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HWND},
        System::{
            ProcessStatus::GetProcessImageFileNameW,
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowTextW,
            GetWindowThreadProcessId, IsWindowVisible,
        },
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
        let mut class = vec![0u16; 256];
        let class_length = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
        let class = String::from_utf16_lossy(&class[..class_length.max(0) as usize]);
        let executable = process_executable(process_id);
        let selector = encode_windows_selector(&title, &class, &executable);
        let windows = unsafe { &mut *(lparam as *mut Vec<(String, String, u32, String)>) };
        if !windows
            .iter()
            .any(|(existing, _, _, _)| existing == &selector)
        {
            windows.push((selector, title, process_id, executable));
        }
        1
    }

    fn process_executable(process_id: u32) -> String {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
        if process.is_null() {
            return "unknown".into();
        }
        let mut path = vec![0u16; 1_024];
        let length =
            unsafe { GetProcessImageFileNameW(process, path.as_mut_ptr(), path.len() as u32) };
        unsafe {
            CloseHandle(process);
        }
        if length == 0 {
            return "unknown".into();
        }
        let path = String::from_utf16_lossy(&path[..length as usize]);
        path.rsplit(['\\', '/'])
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or("unknown")
            .to_string()
    }

    let mut windows = Vec::new();
    unsafe {
        EnumWindows(
            Some(collect_window),
            (&mut windows as *mut Vec<(String, String, u32, String)>).cast::<c_void>() as isize,
        );
    }
    windows
}

#[cfg(windows)]
fn enumerate_windows_playback_devices() -> Result<Vec<(String, String)>> {
    let initialization = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let changed_mode = HRESULT(0x8001_0106_u32 as i32);
    if initialization.is_err() && initialization != changed_mode {
        anyhow::bail!("COM initialization failed: {initialization:?}");
    }
    let should_uninitialize = initialization.is_ok();

    let result = (|| {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }?;
        let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }?;
        let count = unsafe { collection.GetCount() }?;
        let mut devices = Vec::with_capacity(count as usize);

        for index in 0..count {
            let device = unsafe { collection.Item(index) }?;
            let device_id = unsafe { device.GetId() }?;
            let device_id_result = unsafe { device_id.to_string() };
            unsafe {
                CoTaskMemFree(Some(device_id.as_ptr() as *const core::ffi::c_void));
            }
            let device_id = device_id_result
                .map_err(|error| anyhow::anyhow!("invalid endpoint ID: {error}"))?;
            if device_id.is_empty() {
                continue;
            }

            let label = windows_playback_device_friendly_name(&device)
                .unwrap_or_else(|_| device_id.clone());
            devices.push((device_id, label));
        }

        Ok(deduplicate_playback_devices(devices))
    })();

    if should_uninitialize {
        unsafe {
            CoUninitialize();
        }
    }
    result
}

fn playback_device_capability(
    device_id: &str,
    label: String,
    available: bool,
    detail: Option<String>,
) -> AudioSourceCapability {
    AudioSourceCapability {
        id: format!("playback:{device_id}"),
        label,
        kind: AudioSourceKind::PlaybackDevice,
        process_id: None,
        available,
        detail,
    }
}

#[cfg(windows)]
fn windows_playback_device_friendly_name(
    device: &windows::Win32::Media::Audio::IMMDevice,
) -> Result<String> {
    let property_store = unsafe { device.OpenPropertyStore(STGM_READ) }?;
    let mut value = PROPVARIANT::default();
    let result = (|| {
        unsafe { property_store.GetValue(&PKEY_Device_FriendlyName) }?;
        let mut buffer = [0u16; 512];
        unsafe { PropVariantToString(&value, &mut buffer) }?;
        Ok(String::from_utf16_lossy(&buffer)
            .trim_end_matches('\0')
            .trim()
            .to_string())
    })();
    unsafe {
        let _ = PropVariantClear(&mut value);
    }
    result
}

#[cfg(any(windows, test))]
fn deduplicate_playback_devices(devices: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut seen = BTreeSet::new();
    devices
        .into_iter()
        .filter(|(id, _)| seen.insert(id.clone()))
        .collect()
}

fn playback_device_id(source_id: &str) -> Option<&str> {
    source_id.strip_prefix("playback:")
}

#[cfg(windows)]
fn resolve_windows_audio_selector(value: &str) -> String {
    let requested = value.trim();
    enumerate_windows()
        .into_iter()
        .find(|(_, label, _, executable)| {
            label.eq_ignore_ascii_case(requested)
                || executable.eq_ignore_ascii_case(requested)
                || executable
                    .strip_suffix(".exe")
                    .is_some_and(|name| name.eq_ignore_ascii_case(requested))
        })
        .map(|(selector, _, _, _)| selector)
        .unwrap_or_else(|| requested.to_string())
}

#[cfg(not(windows))]
fn resolve_windows_audio_selector(value: &str) -> String {
    value.trim().to_string()
}

#[cfg(windows)]
fn encode_windows_selector(title: &str, class: &str, executable: &str) -> String {
    fn encode(value: &str) -> String {
        value.replace('#', "#22").replace(':', "#3A")
    }

    format!("{}:{}:{}", encode(title), encode(class), encode(executable))
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

fn audio_track_name(route: &AudioRoute) -> String {
    let name = route.track_name.trim();
    if name.is_empty() {
        format!("Track {}", route.track)
    } else {
        name.to_string()
    }
}

fn is_obs_runtime_root(root: &Path) -> bool {
    (root.join("data").is_dir() && root.join("obs-plugins").is_dir())
        || (root.join("share").join("obs").is_dir()
            && root.join("lib").join("obs-plugins").is_dir())
}

fn encoder_plugin_available(module: &str) -> bool {
    if let Some(root) = select_resource_root() {
        return encoder_plugin_in_root(&root, module);
    }

    #[cfg(target_os = "linux")]
    {
        system_obs_plugin_directory()
            .join(format!("{module}.so"))
            .is_file()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = module;
        false
    }
}

fn encoder_plugin_in_root(root: &Path, module: &str) -> bool {
    #[cfg(target_os = "linux")]
    let candidates = [
        root.join("obs-plugins").join(format!("{module}.so")),
        root.join("lib")
            .join("obs-plugins")
            .join(format!("{module}.so")),
    ];
    #[cfg(windows)]
    let candidates = [
        root.join("obs-plugins")
            .join("64bit")
            .join(format!("{module}.dll")),
        root.join("bin").join("64bit").join(format!("{module}.dll")),
    ];
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (root, module);
        return false;
    }

    #[cfg(any(target_os = "linux", windows))]
    candidates.iter().any(|candidate| candidate.is_file())
}

#[cfg(target_os = "linux")]
fn system_obs_plugin_directory() -> PathBuf {
    [
        PathBuf::from("/usr/lib/obs-plugins"),
        PathBuf::from("/usr/lib64/obs-plugins"),
        PathBuf::from(format!(
            "/usr/lib/{}-linux-gnu/obs-plugins",
            env::consts::ARCH
        )),
    ]
    .into_iter()
    .find(|path| path.is_dir())
    .unwrap_or_else(|| PathBuf::from("/usr/lib/obs-plugins"))
}

fn select_resource_root() -> Option<PathBuf> {
    if let Some(path) = env::var_os("CLIP_ENGINE_OBS_ROOT") {
        return Some(PathBuf::from(path));
    }
    let executable = env::current_exe().ok()?;
    let parent = executable.parent()?;
    let bundled = parent.join("obs");
    is_obs_runtime_root(&bundled).then_some(bundled)
}

fn discover_startup_paths() -> StartupPaths {
    #[cfg(windows)]
    {
        let root = select_resource_root().unwrap_or_else(|| PathBuf::from("."));
        let standard_layout =
            root.join("bin").join("64bit").is_dir() && root.join("share").join("obs").is_dir();
        let libobs_data = if standard_layout {
            root.join("share").join("obs").join("libobs")
        } else {
            root.join("data").join("libobs")
        };
        let plugin_bin = if standard_layout {
            root.join("bin").join("64bit")
        } else {
            root.join("obs-plugins").join("64bit")
        };
        let plugin_data = if standard_layout {
            root.join("share").join("obs").join("obs-plugins")
        } else {
            root.join("data").join("obs-plugins")
        };
        StartupPaths::new(
            ObsPath::new(&path_string(&libobs_data)),
            ObsPath::new(&path_string(&plugin_bin)),
            ObsPath::new(&path_string(&plugin_data.join("%module%"))),
        )
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(root) = select_resource_root() {
            let standard_layout = root.join("lib").join("obs-plugins").is_dir()
                && root.join("share").join("obs").is_dir();
            let libobs_data = if standard_layout {
                root.join("share").join("obs").join("libobs")
            } else {
                root.join("data").join("libobs")
            };
            let plugin_bin = if standard_layout {
                root.join("lib").join("obs-plugins")
            } else {
                root.join("obs-plugins")
            };
            let plugin_data = if standard_layout {
                root.join("share").join("obs").join("obs-plugins")
            } else {
                root.join("data").join("obs-plugins")
            };
            let required_plugins = ["obs-ffmpeg", "obs-nvenc", "obs-qsv11"];
            let use_system_plugins = cfg!(debug_assertions)
                && required_plugins
                    .iter()
                    .any(|module| !encoder_plugin_in_root(&root, module))
                && required_plugins.iter().all(|module| {
                    system_obs_plugin_directory()
                        .join(format!("{module}.so"))
                        .is_file()
                });
            let (plugin_bin, plugin_data) = if use_system_plugins {
                // Local source builds often use a slim libobs install without
                // hardware encoder plugins. Use the host OBS plugin set in debug builds so
                // development matches the installed OBS application; release
                // packages must carry the pinned plugins in their own runtime.
                (
                    system_obs_plugin_directory(),
                    PathBuf::from("/usr/share/obs/obs-plugins"),
                )
            } else {
                (plugin_bin, plugin_data)
            };
            return StartupPaths::new(
                ObsPath::new(&path_string(&libobs_data)),
                ObsPath::new(&path_string(&plugin_bin)),
                ObsPath::new(&path_string(&plugin_data.join("%module%"))),
            );
        }
        let plugin_bin = system_obs_plugin_directory();
        StartupPaths::new(
            ObsPath::new("/usr/share/obs/libobs"),
            ObsPath::new(&path_string(&plugin_bin)),
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

    #[test]
    fn audio_track_names_use_custom_values_or_track_fallbacks() {
        let named = AudioRoute {
            source_id: "system:default".into(),
            track: 1,
            track_name: "  Game mix  ".into(),
            enabled: true,
        };
        let unnamed = AudioRoute {
            source_id: "microphone:default".into(),
            track: 2,
            track_name: String::new(),
            enabled: true,
        };
        assert_eq!(audio_track_name(&named), "Game mix");
        assert_eq!(audio_track_name(&unnamed), "Track 2");
    }

    #[test]
    fn playback_device_route_ids_are_opaque_and_deduplicated() {
        let endpoint_id =
            r#"\\?\SWD#MMDEVAPI#{0.0.0.00000000}.{01234567-89ab-cdef-0123-456789abcdef}"#;
        let source_id = format!("playback:{endpoint_id}");
        assert_eq!(playback_device_id(&source_id), Some(endpoint_id));
        assert_eq!(playback_device_id("system:default"), None);
        assert_eq!(playback_device_id("playback:"), Some(""));

        let devices = deduplicate_playback_devices(vec![
            ("first".into(), "First".into()),
            ("first".into(), "Duplicate first".into()),
            ("second".into(), "Second".into()),
        ]);
        assert_eq!(
            devices,
            vec![
                ("first".to_string(), "First".to_string()),
                ("second".to_string(), "Second".to_string())
            ]
        );
        let capability = playback_device_capability(
            endpoint_id,
            "Virtual output".into(),
            true,
            Some("WASAPI render endpoint".into()),
        );
        assert_eq!(capability.id, source_id);
        assert_eq!(capability.kind, AudioSourceKind::PlaybackDevice);
        assert!(capability.available);
    }

    #[cfg(windows)]
    #[test]
    fn windows_active_render_endpoints_have_stable_ids() {
        let devices = enumerate_windows_playback_devices().unwrap();
        let mut ids = BTreeSet::new();
        for (id, label) in devices {
            assert!(!id.is_empty());
            assert!(!label.is_empty());
            assert!(ids.insert(id));
        }
    }

    #[test]
    fn system_audio_exclusion_uses_enabled_application_selectors_once() {
        let config = RecorderConfig {
            system_audio_mode: SystemAudioMode::ExcludeApplications,
            audio_routes: vec![
                clip_engine_recorder_protocol::AudioRoute {
                    source_id: "system:default".into(),
                    track: 1,
                    track_name: String::new(),
                    enabled: true,
                },
                clip_engine_recorder_protocol::AudioRoute {
                    source_id: "application:Discord".into(),
                    track: 2,
                    track_name: String::new(),
                    enabled: true,
                },
                clip_engine_recorder_protocol::AudioRoute {
                    source_id: "application:discord".into(),
                    track: 3,
                    track_name: String::new(),
                    enabled: false,
                },
                clip_engine_recorder_protocol::AudioRoute {
                    source_id: "application:spotify".into(),
                    track: 4,
                    track_name: String::new(),
                    enabled: true,
                },
            ],
            ..RecorderConfig::default()
        };
        assert_eq!(
            system_audio_exclusion_selectors(&config),
            vec!["Discord".to_string(), "spotify".to_string()]
        );

        let settings =
            pipewire_application_exclusion_settings(&system_audio_exclusion_selectors(&config));
        assert_eq!(settings["CaptureMode"], 1);
        assert_eq!(settings["ExceptApp"], true);
        assert_eq!(
            settings["apps"][0]["value"],
            serde_json::Value::String("Discord".into())
        );
        assert_eq!(
            settings["apps"][1]["value"],
            serde_json::Value::String("spotify".into())
        );
    }

    #[test]
    fn mixed_system_audio_does_not_build_exclusion_selectors() {
        let config = RecorderConfig {
            audio_routes: vec![clip_engine_recorder_protocol::AudioRoute {
                source_id: "application:discord".into(),
                track: 2,
                track_name: String::new(),
                enabled: true,
            }],
            ..RecorderConfig::default()
        };
        assert!(system_audio_exclusion_selectors(&config).is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pipewire_application_metadata_becomes_stable_audio_sources() {
        let value = serde_json::json!([
            {
                "info": {
                    "props": {
                        "media.class": "Stream/Output/Audio",
                        "application.name": "Spotify",
                        "application.process.binary": "spotify",
                        "application.process.id": 1234
                    }
                }
            },
            {
                "info": {
                    "props": {
                        "media.class": "Stream/Output/Audio",
                        "application.name": "Spotify playback",
                        "application.process.binary": "spotify",
                        "application.process.id": 1234
                    }
                }
            },
            {
                "info": {
                    "props": {
                        "media.class": "Audio/Sink",
                        "node.name": "Speakers"
                    }
                }
            }
        ]);
        let mut applications = BTreeMap::new();
        collect_pipewire_applications(&value, &mut applications);
        let applications = applications.into_values().collect::<Vec<_>>();
        assert_eq!(applications.len(), 1);
        assert_eq!(applications[0].id, "application:spotify");
        assert_eq!(applications[0].label, "spotify");
        assert_eq!(applications[0].process_id, Some(1234));
        assert_eq!(applications[0].kind, AudioSourceKind::Application);
    }

    #[test]
    fn automatic_rate_control_falls_back_when_quality_is_unsupported() {
        let capability = EncoderCapability {
            id: "example_cbr".into(),
            label: "Example".into(),
            hardware: true,
            codec: "h264".into(),
            family: "H.264".into(),
            settings: vec![EncoderSettingCapability {
                key: "rate_control".into(),
                kind: EncoderSettingKind::List,
                options: vec!["CBR".into()],
                option_values: vec!["CBR".into()],
                min: None,
                max: None,
                step: None,
                description: None,
            }],
        };
        let (rate_control, diagnostics) =
            resolve_rate_control(&RecorderConfig::default(), Some(&capability));
        assert_eq!(rate_control, RateControl::Cbr);
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("fell back")));
    }

    #[test]
    fn automatic_encoder_preference_prioritizes_hardware_codec_order() {
        let hardware_av1 = EncoderCapability {
            id: "obs_nvenc_av1".into(),
            label: "hardware AV1".into(),
            hardware: true,
            codec: "av1".into(),
            family: "AV1".into(),
            settings: Vec::new(),
        };
        let hardware_hevc = EncoderCapability {
            id: "obs_nvenc_hevc".into(),
            label: "hardware HEVC".into(),
            hardware: true,
            codec: "hevc".into(),
            family: "HEVC".into(),
            settings: Vec::new(),
        };
        let hardware_h264 = EncoderCapability {
            id: "obs_nvenc_h264".into(),
            label: "hardware H.264".into(),
            hardware: true,
            codec: "h264".into(),
            family: "H.264".into(),
            settings: Vec::new(),
        };
        assert!(encoder_preference_score(&hardware_av1) < encoder_preference_score(&hardware_hevc));
        assert!(
            encoder_preference_score(&hardware_hevc) < encoder_preference_score(&hardware_h264)
        );
    }

    #[test]
    fn automatic_encoder_uses_h264_as_the_software_fallback() {
        let software_av1 = EncoderCapability {
            id: "ffmpeg_svt_av1".into(),
            label: "software AV1".into(),
            hardware: false,
            codec: "av1".into(),
            family: "AV1".into(),
            settings: Vec::new(),
        };
        let software_h264 = EncoderCapability {
            id: "obs_x264".into(),
            label: "software H.264".into(),
            hardware: false,
            codec: "h264".into(),
            family: "H.264".into(),
            settings: Vec::new(),
        };
        assert!(encoder_preference_score(&software_h264) < encoder_preference_score(&software_av1));
    }

    #[test]
    fn encoder_list_values_are_normalized_from_obs_properties() {
        assert_eq!(obs_list_value_string(r#"String("p5")"#), "p5");
        assert_eq!(obs_list_value_string("Int(2)"), "2");
        assert_eq!(obs_list_value_string("Bool(true)"), "true");
        assert_eq!(obs_list_value_string("Invalid"), "Invalid");
    }

    #[test]
    fn encoder_labels_are_human_readable_for_hardware_codecs() {
        assert_eq!(
            encoder_display_label("obs_nvenc_av1_tex"),
            "NVIDIA NVENC AV1"
        );
        assert_eq!(
            encoder_display_label("obs_nvenc_hevc_tex"),
            "NVIDIA NVENC HEVC"
        );
        assert_eq!(
            encoder_display_label("obs_nvenc_h264_tex"),
            "NVIDIA NVENC H.264"
        );
    }

    #[test]
    fn unsupported_encoder_properties_are_not_selected() {
        let capability = EncoderCapability {
            id: "example".into(),
            label: "Example".into(),
            hardware: false,
            codec: "h264".into(),
            family: "H.264".into(),
            settings: vec![EncoderSettingCapability {
                key: "bitrate".into(),
                kind: EncoderSettingKind::Integer,
                options: Vec::new(),
                option_values: Vec::new(),
                min: Some(1.0),
                max: Some(100_000.0),
                step: Some(1.0),
                description: None,
            }],
        };
        assert_eq!(
            encoder_property_key(Some(&capability), &["cqp", "crf"]),
            None
        );
        assert_eq!(
            encoder_property_key(Some(&capability), &["bitrate", "bitrate_kbps"]),
            Some("bitrate")
        );
    }
}
