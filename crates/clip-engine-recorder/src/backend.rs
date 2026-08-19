use clip_engine_recorder_protocol::{
    CaptureBackend, RecorderCapabilities, RecorderConfig, RecorderState, RecorderStatus,
};
use std::path::PathBuf;

#[derive(Debug)]
pub struct ReplayFile {
    pub path: PathBuf,
    pub duration_seconds: u32,
}

pub trait RecorderBackend: Send {
    fn capabilities(&self) -> RecorderCapabilities;
    fn status(&self) -> RecorderStatus;
    fn apply_config(&mut self, config: RecorderConfig) -> anyhow::Result<()>;
    fn start(&mut self) -> anyhow::Result<()>;
    fn stop(&mut self) -> anyhow::Result<()>;
    fn save_replay(&mut self) -> anyhow::Result<ReplayFile>;
}

#[cfg(feature = "obs")]
#[path = "obs.rs"]
mod obs;

#[cfg(feature = "obs")]
pub use obs::ObsBackend;

pub fn create_backend() -> Box<dyn RecorderBackend> {
    #[cfg(feature = "obs")]
    {
        match ObsBackend::new() {
            Ok(backend) => Box::new(backend),
            Err(error) => Box::new(UnavailableBackend::new(format!(
                "libobs initialization failed: {error:#}"
            ))),
        }
    }

    #[cfg(not(feature = "obs"))]
    {
        Box::new(UnavailableBackend::default())
    }
}

struct UnavailableBackend {
    config: Option<RecorderConfig>,
    status: RecorderStatus,
    diagnostic: String,
}

impl UnavailableBackend {
    fn new(diagnostic: String) -> Self {
        Self {
            config: None,
            status: RecorderStatus {
                state: RecorderState::Error,
                last_error: Some(diagnostic.clone()),
                ..RecorderStatus::default()
            },
            diagnostic,
        }
    }
}

impl Default for UnavailableBackend {
    fn default() -> Self {
        Self::new("The recorder was built without the libobs backend.".into())
    }
}

impl RecorderBackend for UnavailableBackend {
    fn capabilities(&self) -> RecorderCapabilities {
        RecorderCapabilities {
            backend: CaptureBackend::Unknown,
            diagnostics: vec![self.diagnostic.clone()],
            ..RecorderCapabilities::default()
        }
    }

    fn status(&self) -> RecorderStatus {
        self.status.clone()
    }

    fn apply_config(&mut self, config: RecorderConfig) -> anyhow::Result<()> {
        let config = config.normalize();
        config.validate().map_err(|error| anyhow::anyhow!(error))?;
        self.config = Some(config);
        self.status.configured = true;
        Ok(())
    }

    fn start(&mut self) -> anyhow::Result<()> {
        self.status.state = RecorderState::Error;
        self.status.last_error =
            Some("The libobs recorder backend is unavailable in this build.".to_string());
        anyhow::bail!("{}", self.status.last_error.as_deref().unwrap())
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        self.status.state = RecorderState::Stopped;
        self.status.replay_active = false;
        Ok(())
    }

    fn save_replay(&mut self) -> anyhow::Result<ReplayFile> {
        self.status.last_error =
            Some("The libobs recorder backend is unavailable in this build.".to_string());
        anyhow::bail!("{}", self.status.last_error.as_deref().unwrap())
    }
}
