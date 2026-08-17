use anyhow::{Context, Result};
use clip_engine_recorder_protocol::{
    read_frame, write_frame, ClientMessage, RecorderCapabilities, RecorderConfig, RecorderEvent,
    RecorderRequest, RecorderResponse, RecorderState, RecorderStatus, ServiceMessage,
    DEFAULT_SOCKET_NAME, PROTOCOL_VERSION,
};
use interprocess::{
    local_socket::{prelude::*, ConnectOptions, GenericNamespaced, Stream as LocalSocketStream},
    ConnectWaitMode,
};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use crate::paths::AppPaths;

#[derive(Clone)]
pub struct RecorderSupervisor {
    paths: AppPaths,
    inner: Arc<Mutex<RecorderInner>>,
}

struct RecorderInner {
    child: Option<Child>,
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
        Self {
            paths,
            inner: Arc::new(Mutex::new(RecorderInner {
                child: None,
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
        config.validate().map_err(anyhow::Error::msg)?;
        let response = self.request(RecorderRequest::ApplyConfig {
            config: config.clone(),
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
        inner.status = RecorderStatus::default();
    }

    fn request(&self, request: RecorderRequest) -> Result<RecorderResponse> {
        let mut inner = self.lock();
        if let Err(error) = ensure_connected_locked(&self.paths, &mut inner) {
            reset_connection_locked(&mut inner);
            return Err(error);
        }
        match send_request_locked(&mut inner, request) {
            Ok(response) => Ok(response),
            Err(error) => {
                reset_connection_locked(&mut inner);
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

    if let Some(child) = inner.child.as_mut() {
        if child.try_wait()?.is_some() {
            inner.child = None;
        }
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
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(root) = paths.recorder_obs_root() {
            command.env("CLIP_ENGINE_OBS_ROOT", &root);
            configure_obs_library_path(&mut command, &root);
        }
        inner.child = Some(
            command
                .spawn()
                .with_context(|| format!("launch recorder helper {}", binary.display()))?,
        );
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
                if let Some(child) = inner.child.as_mut() {
                    if let Some(status) = child.try_wait()? {
                        anyhow::bail!(
                            "The recorder helper exited before accepting IPC ({status})."
                        );
                    }
                }
                if Instant::now() >= deadline {
                    anyhow::bail!("Timed out connecting to the recorder helper: {error}");
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    };
    stream.set_recv_timeout(Some(Duration::from_secs(30)))?;
    stream.set_send_timeout(Some(Duration::from_secs(30)))?;
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
        RecorderResponse::Hello { .. } => Ok(()),
        other => anyhow::bail!("recorder helper returned an unexpected Hello response: {other:?}"),
    }
}

fn configure_obs_library_path(command: &mut Command, root: &std::path::Path) {
    let mut directories = vec![
        root.join("bin").join("64bit"),
        root.join("lib"),
        root.join("lib64"),
    ];
    if let Some(existing) = std::env::var_os(if cfg!(windows) {
        "PATH"
    } else {
        "LD_LIBRARY_PATH"
    }) {
        directories.extend(std::env::split_paths(&existing));
    }
    let Ok(joined) = std::env::join_paths(directories) else {
        return;
    };
    if cfg!(windows) {
        command.env("PATH", joined);
    } else {
        command.env("LD_LIBRARY_PATH", joined);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_failure_resets_state_for_reconnect() {
        let mut inner = RecorderInner {
            child: None,
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
}
