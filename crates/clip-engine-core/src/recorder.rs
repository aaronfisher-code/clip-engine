use anyhow::{Context, Result};
#[cfg(not(windows))]
use clip_engine_recorder_protocol::IPC_TIMEOUT;
use clip_engine_recorder_protocol::{
    read_frame, write_frame, CaptureBackend, ClientMessage, RecorderCapabilities, RecorderConfig,
    RecorderEvent, RecorderMode, RecorderRequest, RecorderResponse, RecorderState, RecorderStatus,
    ServiceMessage, DEFAULT_SOCKET_NAME, PROTOCOL_VERSION,
};
use interprocess::{
    local_socket::{prelude::*, ConnectOptions, GenericNamespaced, Stream as LocalSocketStream},
    ConnectWaitMode,
};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use crate::paths::AppPaths;

/// Windows named-pipe helpers must run without opening a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Clone)]
pub struct RecorderSupervisor {
    paths: AppPaths,
    inner: Arc<Mutex<RecorderInner>>,
}

struct RecorderInner {
    child: Option<Child>,
    child_stdout: Option<Arc<Mutex<String>>>,
    child_stdout_thread: Option<thread::JoinHandle<()>>,
    child_stderr: Option<Arc<Mutex<String>>>,
    child_stderr_thread: Option<thread::JoinHandle<()>>,
    stream: Option<LocalSocketStream>,
    socket_name: String,
    auth_token: String,
    next_request_id: u64,
    capabilities: RecorderCapabilities,
    status: RecorderStatus,
    config: RecorderConfig,
}

impl RecorderSupervisor {
    pub fn new(paths: AppPaths, config: RecorderConfig) -> Self {
        let config = config.normalize();
        Self {
            paths,
            inner: Arc::new(Mutex::new(RecorderInner {
                child: None,
                child_stdout: None,
                child_stdout_thread: None,
                child_stderr: None,
                child_stderr_thread: None,
                stream: None,
                socket_name: DEFAULT_SOCKET_NAME.to_string(),
                auth_token: String::new(),
                next_request_id: 1,
                capabilities: RecorderCapabilities::default(),
                status: RecorderStatus::default(),
                config,
            })),
        }
    }

    pub fn config(&self) -> RecorderConfig {
        self.lock().config.clone()
    }

    pub fn status(&self) -> RecorderStatus {
        self.lock().status.clone()
    }

    pub fn capabilities(&self) -> RecorderCapabilities {
        self.lock().capabilities.clone()
    }

    pub fn refresh(&self) -> Result<(RecorderCapabilities, RecorderStatus)> {
        let _ = self.request(RecorderRequest::GetCapabilities)?;
        let _ = self.request(RecorderRequest::GetStatus)?;
        Ok((self.capabilities(), self.status()))
    }

    pub fn apply_config(&self, config: RecorderConfig) -> Result<()> {
        let config = self.capabilities().normalize_config(&config.normalize());
        let applied_config = if config.mode == RecorderMode::Automatic {
            config.automatic_capture_config()
        } else {
            config.clone()
        };
        applied_config.validate().map_err(anyhow::Error::msg)?;
        let response = self.request(RecorderRequest::ApplyConfig {
            config: Box::new(applied_config),
        })?;
        ensure_accepted(response)?;
        self.lock().config = config;
        Ok(())
    }

    pub fn start(&self) -> Result<()> {
        ensure_accepted(self.request(RecorderRequest::Start)?)?;
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        ensure_accepted(self.request(RecorderRequest::Stop)?)?;
        Ok(())
    }

    pub fn save_replay(&self) -> Result<Option<PathBuf>> {
        let response = self.request(RecorderRequest::SaveReplay)?;
        ensure_accepted(response)?;
        Ok(self.status().last_replay_path.map(PathBuf::from))
    }

    pub fn shutdown(&self) {
        let mut inner = self.lock();
        if inner.stream.is_some() {
            let _ = send_request_locked(&mut inner, RecorderRequest::Shutdown);
        }
        inner.stream = None;
        if let Some(mut child) = inner.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(thread) = inner.child_stdout_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = inner.child_stderr_thread.take() {
            let _ = thread.join();
        }
        inner.child_stdout = None;
        inner.child_stderr = None;
        inner.status = RecorderStatus::default();
    }

    fn request(&self, request: RecorderRequest) -> Result<RecorderResponse> {
        let mut inner = self.lock();
        if let Err(error) = ensure_connected_locked(&self.paths, &mut inner) {
            record_connection_error(&mut inner, &error);
            return Err(error);
        }
        match send_request_locked(&mut inner, request.clone()) {
            Ok(response) => Ok(response),
            Err(error) if is_retryable_ipc_error(&error) => {
                reset_connection_locked(&mut inner);
                if let Err(connect_error) = ensure_connected_locked(&self.paths, &mut inner) {
                    record_connection_error(&mut inner, &connect_error);
                    return Err(error);
                }
                match send_request_locked(&mut inner, request) {
                    Ok(response) => Ok(response),
                    Err(retry_error) => {
                        let retry_error = with_child_exit_diagnostic(&mut inner, retry_error);
                        record_connection_error(&mut inner, &retry_error);
                        Err(retry_error)
                    }
                }
            }
            Err(error) => {
                let error = with_child_exit_diagnostic(&mut inner, error);
                record_connection_error(&mut inner, &error);
                Err(error)
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, RecorderInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn ensure_connected_locked(paths: &AppPaths, inner: &mut RecorderInner) -> Result<()> {
    if inner.stream.is_some() {
        return Ok(());
    }

    if let Some(error) = exited_child_error(inner)? {
        anyhow::bail!("{error}");
    }

    if inner.child.is_none() {
        let binary = std::env::var_os("CLIP_ENGINE_RECORDER")
            .map(PathBuf::from)
            .unwrap_or_else(|| paths.recorder_binary());
        if !binary.is_file() {
            anyhow::bail!(
                "The recorder helper is not installed at {}. Set CLIP_ENGINE_RECORDER for local development.",
                binary.display()
            );
        }
        inner.socket_name = format!(
            "{}-{}-{}",
            DEFAULT_SOCKET_NAME,
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        );
        inner.auth_token = uuid::Uuid::new_v4().to_string();
        let mut command = Command::new(&binary);
        command
            .arg("--socket")
            .arg(&inner.socket_name)
            .arg("--auth-token")
            .arg(&inner.auth_token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        if let Some(root) = paths.recorder_obs_root() {
            command.env("CLIP_ENGINE_OBS_ROOT", &root);
            configure_obs_library_path(&mut command, &root);
            prepare_obs_muxer(&binary, &root)
                .with_context(|| format!("prepare OBS mux helper next to {}", binary.display()))?;
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("launch recorder helper {}", binary.display()))?;
        let (stdout_log, stdout_thread) = capture_child_output(child.stdout.take());
        let (stderr_log, stderr_thread) = capture_child_output(child.stderr.take());
        inner.child_stdout = Some(stdout_log);
        inner.child_stdout_thread = stdout_thread;
        inner.child_stderr = Some(stderr_log);
        inner.child_stderr_thread = stderr_thread;
        inner.child = Some(child);
    }

    let name = inner
        .socket_name
        .clone()
        .to_ns_name::<GenericNamespaced>()
        .context("invalid recorder IPC name")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let stream = loop {
        let options = ConnectOptions::new()
            .name(name.clone())
            .wait_mode(ConnectWaitMode::Timeout(Duration::from_millis(400)));
        match options.connect_sync() {
            Ok(stream) => break stream,
            Err(error) => {
                if let Some(child_error) = exited_child_error(inner)? {
                    anyhow::bail!("{child_error}");
                }
                if Instant::now() >= deadline {
                    anyhow::bail!("Timed out connecting to the recorder helper: {error}");
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    };
    #[cfg(not(windows))]
    stream.set_nonblocking(false)?;
    #[cfg(not(windows))]
    {
        stream.set_recv_timeout(Some(IPC_TIMEOUT))?;
        stream.set_send_timeout(Some(IPC_TIMEOUT))?;
    }
    inner.stream = Some(stream);
    let auth_token = inner.auth_token.clone();
    let response = send_request_locked(
        inner,
        RecorderRequest::Hello {
            protocol_version: PROTOCOL_VERSION,
            auth_token,
        },
    )?;
    match response {
        RecorderResponse::Hello {
            protocol_version, ..
        } if protocol_version == PROTOCOL_VERSION => Ok(()),
        RecorderResponse::Hello {
            protocol_version, ..
        } => anyhow::bail!(
            "recorder helper negotiated protocol {protocol_version}, expected {PROTOCOL_VERSION}"
        ),
        other => anyhow::bail!("recorder helper returned an unexpected Hello response: {other:?}"),
    }
}

fn configure_obs_library_path(command: &mut Command, root: &std::path::Path) {
    let library_directories = vec![
        root.join("bin").join("64bit"),
        root.join("lib"),
        root.join("lib64"),
    ];
    if cfg!(windows) {
        let mut directories = library_directories;
        if let Some(existing) = std::env::var_os("PATH") {
            directories.extend(std::env::split_paths(&existing));
        }
        if let Ok(joined) = std::env::join_paths(directories) {
            command.env("PATH", joined);
        }
    } else {
        let mut library_directories = library_directories;
        if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
            library_directories.extend(std::env::split_paths(&existing));
        }
        if let Ok(joined) = std::env::join_paths(library_directories) {
            command.env("LD_LIBRARY_PATH", joined);
        }

        // Keep the runtime's executable directories available to any
        // subprocesses launched by OBS.
        let mut path_directories = vec![root.join("bin").join("64bit"), root.join("bin")];
        if let Some(existing) = std::env::var_os("PATH") {
            path_directories.extend(std::env::split_paths(&existing));
        }
        if let Ok(joined) = std::env::join_paths(path_directories) {
            command.env("PATH", joined);
        }
    }
}

fn send_request_locked(
    inner: &mut RecorderInner,
    request: RecorderRequest,
) -> Result<RecorderResponse> {
    let request_id = inner.next_request_id;
    inner.next_request_id = inner.next_request_id.saturating_add(1);
    let stream = inner
        .stream
        .as_mut()
        .context("recorder IPC connection is not open")?;
    write_frame(
        stream,
        &ClientMessage {
            request_id,
            request,
        },
    )
    .context("write recorder IPC request")?;
    loop {
        let message: ServiceMessage = {
            let stream = inner
                .stream
                .as_mut()
                .context("recorder IPC connection is not open")?;
            read_frame(stream).context("read recorder IPC response")?
        };
        match message {
            ServiceMessage::Event(event) => apply_event(inner, event),
            ServiceMessage::Response {
                request_id: response_id,
                response,
            } if response_id == request_id => {
                if let RecorderResponse::Status(status) = &response {
                    inner.status = status.clone();
                }
                if let RecorderResponse::Capabilities(capabilities) = &response {
                    inner.capabilities = capabilities.clone();
                }
                return Ok(response);
            }
            ServiceMessage::Response { .. } => {}
        }
    }
}

fn apply_event(inner: &mut RecorderInner, event: RecorderEvent) {
    match event {
        RecorderEvent::StatusChanged(status) => inner.status = status,
        RecorderEvent::CapabilitiesChanged(capabilities) => inner.capabilities = capabilities,
        RecorderEvent::ReplaySaved {
            path,
            duration_seconds: _,
        } => {
            inner.status.last_replay_path = Some(path);
        }
        RecorderEvent::Log { .. } => {}
    }
}

fn prepare_obs_muxer(binary: &Path, root: &Path) -> Result<()> {
    let binary_directory = binary
        .parent()
        .context("recorder helper has no parent directory")?;
    let muxer_name = if cfg!(windows) {
        "obs-ffmpeg-mux.exe"
    } else {
        "obs-ffmpeg-mux"
    };
    let destination = binary_directory.join(muxer_name);
    if destination.is_file() {
        return Ok(());
    }

    let candidates = [
        root.join("bin").join("64bit").join(muxer_name),
        root.join("bin").join(muxer_name),
        root.join("obs-plugins").join("64bit").join(muxer_name),
        root.join("obs-plugins").join(muxer_name),
    ];
    let source = candidates
        .iter()
        .find(|candidate| candidate.is_file())
        .context("OBS runtime does not contain obs-ffmpeg-mux")?;

    if std::fs::symlink_metadata(&destination).is_ok() {
        std::fs::remove_file(&destination)
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
        std::fs::copy(source, &destination).with_context(|| {
            format!(
                "copy OBS mux helper {} to {}",
                source.display(),
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn ensure_accepted(response: RecorderResponse) -> Result<()> {
    match response {
        RecorderResponse::Accepted => Ok(()),
        RecorderResponse::Error(error) => {
            anyhow::bail!("{}: {}", error.code, error.message)
        }
        other => anyhow::bail!("unexpected recorder response: {other:?}"),
    }
}

fn reset_connection_locked(inner: &mut RecorderInner) {
    inner.stream = None;
    inner.status.state = RecorderState::Error;
    inner.status.replay_active = false;
    inner.status.effective_settings = None;
}

fn record_connection_error(inner: &mut RecorderInner, error: &anyhow::Error) {
    let message = format!("{error:#}");
    reset_connection_locked(inner);
    inner.status.last_error = Some(message.clone());
    inner.capabilities.backend = CaptureBackend::Unknown;
    inner.capabilities.diagnostics = vec![message];
}

fn is_retryable_ipc_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|io_error| {
            matches!(
                io_error.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::NotConnected
            )
        })
    })
}

fn with_child_exit_diagnostic(inner: &mut RecorderInner, error: anyhow::Error) -> anyhow::Error {
    if let Ok(Some(child_error)) = exited_child_error(inner) {
        return anyhow::anyhow!("{error:#}; {child_error}");
    }
    if let Some(output) = child_output_snapshot(inner) {
        return anyhow::anyhow!("{error:#}; recorder output:\n{output}");
    }
    error
}

fn child_output_snapshot(inner: &RecorderInner) -> Option<String> {
    let diagnostics = [&inner.child_stdout, &inner.child_stderr]
        .into_iter()
        .filter_map(|log| {
            log.as_ref()
                .and_then(|log| log.lock().ok().map(|message| message.clone()))
        })
        .filter(|message| !message.is_empty())
        .collect::<Vec<_>>();
    (!diagnostics.is_empty()).then(|| diagnostics.join("\n"))
}

fn capture_child_output<R>(
    output: Option<R>,
) -> (Arc<Mutex<String>>, Option<thread::JoinHandle<()>>)
where
    R: Read + Send + 'static,
{
    const MAX_DIAGNOSTIC_BYTES: usize = 16 * 1024;
    let log = Arc::new(Mutex::new(String::new()));
    let Some(mut output) = output else {
        return (log, None);
    };
    let shared_log = Arc::clone(&log);
    let thread = thread::spawn(move || {
        let mut captured = Vec::new();
        let mut buffer = [0_u8; 4 * 1024];
        loop {
            match output.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    captured.extend_from_slice(&buffer[..read]);
                    if captured.len() > MAX_DIAGNOSTIC_BYTES {
                        let excess = captured.len() - MAX_DIAGNOSTIC_BYTES;
                        captured.drain(..excess);
                    }
                    if let Ok(mut shared_log) = shared_log.lock() {
                        *shared_log = String::from_utf8_lossy(&captured).trim().to_string();
                    }
                }
            }
        }
    });
    (log, Some(thread))
}

fn exited_child_error(inner: &mut RecorderInner) -> Result<Option<String>> {
    let Some(child) = inner.child.as_mut() else {
        return Ok(None);
    };
    let Some(status) = child.try_wait()? else {
        return Ok(None);
    };
    let _child = inner.child.take();
    if let Some(thread) = inner.child_stdout_thread.take() {
        let _ = thread.join();
    }
    if let Some(thread) = inner.child_stderr_thread.take() {
        let _ = thread.join();
    }
    let diagnostics = [inner.child_stdout.take(), inner.child_stderr.take()]
        .into_iter()
        .filter_map(|log| log.and_then(|log| log.lock().ok().map(|message| message.clone())))
        .filter(|message| !message.is_empty())
        .collect::<Vec<_>>();
    let message = if diagnostics.is_empty() {
        format!("The recorder helper exited before accepting IPC ({status}).")
    } else {
        format!(
            "The recorder helper exited before accepting IPC ({status}):\n{}",
            diagnostics.join("\n")
        )
    };
    Ok(Some(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_failure_resets_state_for_reconnect() {
        let mut inner = RecorderInner {
            child: None,
            child_stdout: None,
            child_stdout_thread: None,
            child_stderr: None,
            child_stderr_thread: None,
            stream: None,
            socket_name: DEFAULT_SOCKET_NAME.into(),
            auth_token: "test-token".into(),
            next_request_id: 1,
            capabilities: RecorderCapabilities::default(),
            status: RecorderStatus {
                state: RecorderState::Running,
                replay_active: true,
                ..RecorderStatus::default()
            },
            config: RecorderConfig::default(),
        };

        reset_connection_locked(&mut inner);

        assert_eq!(inner.status.state, RecorderState::Error);
        assert!(!inner.status.replay_active);
        assert!(inner.stream.is_none());
    }

    #[test]
    fn helper_failure_is_exposed_as_capability_diagnostic() {
        let mut inner = RecorderInner {
            child: None,
            child_stdout: None,
            child_stdout_thread: None,
            child_stderr: None,
            child_stderr_thread: None,
            stream: None,
            socket_name: DEFAULT_SOCKET_NAME.into(),
            auth_token: String::new(),
            next_request_id: 1,
            capabilities: RecorderCapabilities::default(),
            status: RecorderStatus::default(),
            config: RecorderConfig::default(),
        };

        record_connection_error(&mut inner, &anyhow::anyhow!("pinned OBS runtime missing"));

        assert_eq!(inner.capabilities.backend, CaptureBackend::Unknown);
        assert_eq!(
            inner.capabilities.diagnostics,
            vec!["pinned OBS runtime missing".to_string()]
        );
        assert_eq!(
            inner.status.last_error.as_deref(),
            Some("pinned OBS runtime missing")
        );
    }
}
