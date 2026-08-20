mod backend;
mod hotkey;
mod notify;

use anyhow::Context;
use backend::{create_backend, RecorderBackend};
#[cfg(not(windows))]
use clip_engine_recorder_protocol::IPC_TIMEOUT;
use clip_engine_recorder_protocol::{
    is_ipc_timeout, read_frame, write_frame, AudioRoute, ClientMessage, RecorderConfig,
    RecorderError, RecorderEvent, RecorderRequest, RecorderResponse, RecorderState, ServiceMessage,
    DEFAULT_SOCKET_NAME, PROTOCOL_VERSION,
};
use hotkey::HotkeyController;
use interprocess::local_socket::{
    prelude::*, GenericNamespaced, ListenerOptions, Stream as LocalSocketStream,
};
use serde_json::json;
use std::{
    env, fs,
    io::ErrorKind,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type SharedBackend = Arc<Mutex<Box<dyn RecorderBackend>>>;

fn main() {
    initialize_platform_capture();
    if let Err(error) = run() {
        eprintln!("clip-engine-recorder: {error:#}");
        std::process::exit(1);
    }
}

fn initialize_platform_capture() {
    #[cfg(windows)]
    unsafe {
        // This must happen before display enumeration or libobs startup. Without
        // per-monitor awareness Windows can report a scaled 4K desktop as
        // 1920x1080 and win-capture receives mismatched monitor geometry.
        use windows_sys::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

fn run() -> anyhow::Result<()> {
    let options = ServiceOptions::from_args(env::args().skip(1))?;
    if !options.probe && !options.smoke && options.auth_token.is_empty() {
        anyhow::bail!("--auth-token is required for recorder IPC");
    }
    let socket_name = options.socket_name.clone();
    let name = options
        .socket_name
        .clone()
        .to_ns_name::<GenericNamespaced>()
        .context("invalid recorder IPC name")?;
    if options.probe {
        let backend = create_backend();
        println!(
            "{}",
            serde_json::to_string_pretty(&backend.capabilities())
                .context("serialize recorder capabilities")?
        );
        if matches!(
            backend.status().state,
            clip_engine_recorder_protocol::RecorderState::Error
        ) {
            anyhow::bail!(
                "{}",
                backend
                    .status()
                    .last_error
                    .unwrap_or_else(|| "recorder backend initialization failed".into())
            );
        }
        return Ok(());
    }
    if options.smoke {
        let backend = create_backend();
        if matches!(backend.status().state, RecorderState::Error) {
            anyhow::bail!(
                "{}",
                backend
                    .status()
                    .last_error
                    .unwrap_or_else(|| "recorder backend initialization failed".into())
            );
        }
        println!("{}", run_backend_smoke(backend)?);
        return Ok(());
    }
    let mut service = RecorderService::new(options.auth_token);
    let listener = ListenerOptions::new()
        .name(name)
        .create_sync()
        .context("create recorder IPC listener")?;

    eprintln!(
        "clip-engine-recorder listening on {} (libobs={})",
        socket_name,
        cfg!(feature = "obs")
    );

    for connection in listener.incoming() {
        let stream = match connection {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("recorder IPC accept failed: {error}");
                continue;
            }
        };
        match service.serve(stream) {
            Ok(true) => break,
            Ok(false) => {}
            Err(error) => eprintln!("recorder IPC session ended: {error:#}"),
        }
    }
    Ok(())
}

struct ServiceOptions {
    socket_name: String,
    auth_token: String,
    probe: bool,
    smoke: bool,
}

impl ServiceOptions {
    fn from_args(args: impl Iterator<Item = String>) -> anyhow::Result<Self> {
        let mut socket_name = DEFAULT_SOCKET_NAME.to_string();
        let mut auth_token = String::new();
        let mut probe = false;
        let mut smoke = false;
        let mut args = args.peekable();
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--socket" => {
                    socket_name = args.next().context("--socket requires a value")?;
                }
                "--auth-token" => {
                    auth_token = args.next().context("--auth-token requires a value")?;
                }
                "--probe" => {
                    probe = true;
                }
                "--smoke" => {
                    smoke = true;
                }
                "--help" | "-h" => {
                    println!(
                        "Usage: clip-engine-recorder [--socket NAME] [--auth-token TOKEN] [--probe|--smoke]"
                    );
                    std::process::exit(0);
                }
                unknown => anyhow::bail!("unknown argument {unknown}"),
            }
        }
        Ok(Self {
            socket_name,
            auth_token,
            probe,
            smoke,
        })
    }
}

fn run_backend_smoke(mut backend: Box<dyn RecorderBackend>) -> anyhow::Result<serde_json::Value> {
    let capabilities = backend.capabilities();
    let screen = capabilities
        .screens
        .first()
        .context("recorder smoke requires an enumerated display")?;
    let fps = capabilities
        .frame_rates
        .iter()
        .map(|range| range.native.first().copied().unwrap_or(range.min))
        .next()
        .context("recorder smoke requires a reported frame-rate range")?;
    let output_directory = env::temp_dir().join(format!(
        "clip-engine-recorder-smoke-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut config = RecorderConfig {
        screen_id: screen.id.clone(),
        output_width: screen.width.clamp(320, 1_920),
        output_height: screen.height.clamp(180, 1_080),
        fps,
        replay_seconds: 1,
        output_directory: output_directory.to_string_lossy().into_owned(),
        hotkey: None,
        ..RecorderConfig::default()
    };
    config.audio_routes = capabilities
        .audio_sources
        .iter()
        .filter(|source| source.available)
        .take(6)
        .enumerate()
        .map(|(index, source)| AudioRoute {
            source_id: source.id.clone(),
            track: u8::try_from(index + 1).expect("six audio tracks fit in u8"),
            track_name: source.label.clone(),
            enabled: true,
        })
        .collect();

    backend
        .apply_config(config.clone())
        .context("apply recorder smoke configuration")?;
    let effective_settings = backend.status().effective_settings.clone();
    backend.start().context("start recorder smoke capture")?;
    thread::sleep(Duration::from_secs(u64::from(config.replay_seconds) + 1));
    let replay = backend
        .save_replay()
        .context("save recorder smoke replay")?;
    backend.stop().context("stop recorder smoke capture")?;

    let size = fs::metadata(&replay.path)
        .with_context(|| format!("inspect smoke replay {}", replay.path.display()))?
        .len();
    if size == 0 {
        anyhow::bail!("recorder smoke replay is empty: {}", replay.path.display());
    }

    Ok(json!({
        "backend": capabilities.backend,
        "outputDirectory": output_directory,
        "replayPath": replay.path,
        "replayBytes": size,
        "audioRouteCount": config.audio_routes.len(),
        "audioRoutes": config.audio_routes,
        "effectiveSettings": effective_settings,
    }))
}

struct RecorderService {
    auth_token: String,
    backend: SharedBackend,
    hotkeys: HotkeyController,
    notify_on_save: bool,
}

impl RecorderService {
    fn new(auth_token: String) -> Self {
        let backend = Arc::new(Mutex::new(create_backend()));
        let hotkeys = HotkeyController::new(backend.clone());
        Self {
            auth_token,
            backend,
            hotkeys,
            notify_on_save: true,
        }
    }

    fn serve(&mut self, mut stream: LocalSocketStream) -> anyhow::Result<bool> {
        verify_same_user(&stream)?;
        let mut authenticated = false;
        #[cfg(not(windows))]
        stream
            .set_nonblocking(false)
            .context("set recorder IPC blocking mode")?;
        #[cfg(not(windows))]
        {
            stream
                .set_recv_timeout(Some(IPC_TIMEOUT))
                .context("set recorder IPC receive timeout")?;
            stream
                .set_send_timeout(Some(IPC_TIMEOUT))
                .context("set recorder IPC send timeout")?;
        }

        loop {
            let message = match read_frame::<_, ClientMessage>(&mut stream) {
                Ok(message) => message,
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::UnexpectedEof | ErrorKind::BrokenPipe
                    ) =>
                {
                    return Ok(false);
                }
                Err(error) if is_ipc_timeout(&error) => continue,
                Err(error) => {
                    return Err(error).context("read recorder IPC message");
                }
            };
            let (response, events, should_shutdown) = self.handle(message, &mut authenticated);
            for event in events {
                write_frame(&mut stream, &ServiceMessage::Event(event))
                    .context("write recorder IPC event")?;
            }
            write_frame(&mut stream, &response).context("write recorder IPC response")?;
            if should_shutdown {
                return Ok(true);
            }
        }
    }

    fn handle(
        &mut self,
        message: ClientMessage,
        authenticated: &mut bool,
    ) -> (ServiceMessage, Vec<RecorderEvent>, bool) {
        let request_id = message.request_id;
        if !*authenticated {
            let RecorderRequest::Hello {
                protocol_version,
                auth_token,
            } = message.request
            else {
                return (
                    response_error(
                        request_id,
                        RecorderError::new(
                            "unauthenticated",
                            "The recorder must receive Hello first.",
                        ),
                    ),
                    Vec::new(),
                    false,
                );
            };
            if protocol_version != PROTOCOL_VERSION {
                return (
                    response_error(
                        request_id,
                        RecorderError::new(
                            "protocol",
                            format!(
                                "Unsupported recorder protocol {protocol_version}; expected {PROTOCOL_VERSION}."
                            ),
                        ),
                    ),
                    Vec::new(),
                    false,
                );
            }
            if auth_token != self.auth_token {
                return (
                    response_error(
                        request_id,
                        RecorderError::new("auth", "Recorder IPC authentication failed."),
                    ),
                    Vec::new(),
                    false,
                );
            }
            *authenticated = true;
            return (
                ServiceMessage::Response {
                    request_id,
                    response: RecorderResponse::Hello {
                        protocol_version: PROTOCOL_VERSION,
                        service_version: env!("CARGO_PKG_VERSION").to_string(),
                    },
                },
                vec![RecorderEvent::StatusChanged(self.status())],
                false,
            );
        }

        match message.request {
            RecorderRequest::Hello { .. } => (
                response_error(
                    request_id,
                    RecorderError::new("protocol", "Hello may only be sent once."),
                ),
                Vec::new(),
                false,
            ),
            RecorderRequest::GetStatus => (
                response(request_id, RecorderResponse::Status(self.status())),
                Vec::new(),
                false,
            ),
            RecorderRequest::GetCapabilities => (
                response(
                    request_id,
                    RecorderResponse::Capabilities(self.capabilities()),
                ),
                Vec::new(),
                false,
            ),
            RecorderRequest::ApplyConfig { config } => {
                let config = (*config).normalize();
                let hotkey = config.hotkey.clone();
                self.notify_on_save = config.notify_on_save;
                let result = self
                    .backend
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .apply_config(config);
                let result = result.and_then(|()| {
                    self.hotkeys
                        .configure(hotkey, self.notify_on_save)
                        .map_err(|error| anyhow::anyhow!(error))
                });
                match result {
                    Ok(()) => (
                        response(request_id, RecorderResponse::Accepted),
                        vec![RecorderEvent::StatusChanged(self.status())],
                        false,
                    ),
                    Err(error) => (
                        response_error(request_id, backend_error(error)),
                        vec![RecorderEvent::StatusChanged(self.status())],
                        false,
                    ),
                }
            }
            RecorderRequest::Start => {
                let result = self
                    .backend
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .start();
                match result {
                    Ok(()) => (
                        response(request_id, RecorderResponse::Accepted),
                        vec![RecorderEvent::StatusChanged(self.status())],
                        false,
                    ),
                    Err(error) => (
                        response_error(request_id, backend_error(error)),
                        vec![RecorderEvent::StatusChanged(self.status())],
                        false,
                    ),
                }
            }
            RecorderRequest::Stop => {
                let result = self
                    .backend
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .stop();
                match result {
                    Ok(()) => (
                        response(request_id, RecorderResponse::Accepted),
                        vec![RecorderEvent::StatusChanged(self.status())],
                        false,
                    ),
                    Err(error) => (
                        response_error(request_id, backend_error(error)),
                        vec![RecorderEvent::StatusChanged(self.status())],
                        false,
                    ),
                }
            }
            RecorderRequest::SaveReplay => {
                let result = {
                    let mut backend = self
                        .backend
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    backend.save_replay()
                };
                match result {
                    Ok(replay) => {
                        eprintln!(
                            "recorder replay saved: {} ({}s)",
                            replay.path.display(),
                            replay.duration_seconds
                        );
                        if self.notify_on_save {
                            notify::replay_saved(&replay.path, replay.duration_seconds);
                        }
                        (
                            response(request_id, RecorderResponse::Accepted),
                            vec![
                                RecorderEvent::ReplaySaved {
                                    path: replay.path.to_string_lossy().to_string(),
                                    duration_seconds: replay.duration_seconds,
                                },
                                RecorderEvent::StatusChanged(self.status()),
                            ],
                            false,
                        )
                    }
                    Err(error) => {
                        eprintln!("recorder replay save failed: {error:#}");
                        if self.notify_on_save {
                            notify::replay_save_failed(&error);
                        }
                        (
                            response_error(request_id, backend_error(error)),
                            vec![RecorderEvent::StatusChanged(self.status())],
                            false,
                        )
                    }
                }
            }
            RecorderRequest::Ping => (
                response(request_id, RecorderResponse::Pong),
                Vec::new(),
                false,
            ),
            RecorderRequest::Shutdown => (
                response(request_id, RecorderResponse::Accepted),
                vec![RecorderEvent::StatusChanged(self.status())],
                true,
            ),
        }
    }

    fn status(&self) -> clip_engine_recorder_protocol::RecorderStatus {
        let mut status = self
            .backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .status();
        let (registered, error) = self.hotkeys.status();
        status.hotkey_registered = registered;
        status.hotkey_error = error;
        status
    }

    fn capabilities(&self) -> clip_engine_recorder_protocol::RecorderCapabilities {
        self.backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .capabilities()
    }
}

fn verify_same_user(stream: &LocalSocketStream) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        if let Some(peer_uid) = stream.peer_creds()?.euid() {
            let current_uid = unsafe { libc::geteuid() };
            if peer_uid != current_uid {
                anyhow::bail!("recorder IPC peer is not owned by the current user");
            }
        }
    }
    #[cfg(not(unix))]
    let _ = stream;
    Ok(())
}

fn response(request_id: u64, response: RecorderResponse) -> ServiceMessage {
    ServiceMessage::Response {
        request_id,
        response,
    }
}

fn response_error(request_id: u64, error: RecorderError) -> ServiceMessage {
    response(request_id, RecorderResponse::Error(error))
}

fn backend_error(error: anyhow::Error) -> RecorderError {
    RecorderError::new("backend", format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clip_engine_recorder_protocol::{ClientMessage, RecorderResponse};

    #[test]
    fn service_rejects_commands_before_authentication() {
        let mut service = RecorderService::new("test-token".into());
        let mut authenticated = false;
        let (message, events, should_shutdown) = service.handle(
            ClientMessage {
                request_id: 1,
                request: RecorderRequest::Ping,
            },
            &mut authenticated,
        );
        assert!(events.is_empty());
        assert!(!should_shutdown);
        assert!(!authenticated);
        assert!(matches!(
            message,
            ServiceMessage::Response {
                response: RecorderResponse::Error(RecorderError { code, .. }),
                ..
            } if code == "unauthenticated"
        ));
    }

    #[test]
    fn service_accepts_matching_hello_token() {
        let mut service = RecorderService::new("test-token".into());
        let mut authenticated = false;
        let (message, events, should_shutdown) = service.handle(
            ClientMessage {
                request_id: 1,
                request: RecorderRequest::Hello {
                    protocol_version: PROTOCOL_VERSION,
                    auth_token: "test-token".into(),
                },
            },
            &mut authenticated,
        );
        assert!(!events.is_empty());
        assert!(!should_shutdown);
        assert!(authenticated);
        assert!(matches!(
            message,
            ServiceMessage::Response {
                response: RecorderResponse::Hello { .. },
                ..
            }
        ));
    }

    #[test]
    fn replay_save_releases_backend_lock_before_status_event() {
        let mut service = RecorderService::new("test-token".into());
        let mut authenticated = true;
        let (message, _, _) = service.handle(
            ClientMessage {
                request_id: 1,
                request: RecorderRequest::SaveReplay,
            },
            &mut authenticated,
        );
        assert!(matches!(
            message,
            ServiceMessage::Response {
                response: RecorderResponse::Error(RecorderError { code, .. }),
                ..
            } if code == "backend"
        ));
    }
}
