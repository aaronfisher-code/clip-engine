use eframe::egui::{Pos2, ViewportBuilder};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use winit::window::Window;

pub const DEFAULT_INNER_SIZE: [f32; 2] = [1440.0, 900.0];
pub const MIN_INNER_SIZE: [f32; 2] = [900.0, 620.0];

const SAVE_DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub x: Option<f32>,
    #[serde(default)]
    pub y: Option<f32>,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub fullscreen: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: DEFAULT_INNER_SIZE[0],
            height: DEFAULT_INNER_SIZE[1],
            x: None,
            y: None,
            maximized: false,
            fullscreen: false,
        }
    }
}

impl WindowState {
    pub fn load() -> Self {
        Self::load_from(&storage_path())
    }

    fn load_from(path: &Path) -> Self {
        let Ok(bytes) = std::fs::read(path) else {
            return Self::default();
        };
        serde_json::from_slice::<Self>(&bytes)
            .unwrap_or_default()
            .sanitized()
    }

    fn sanitized(mut self) -> Self {
        self.width = self.width.max(MIN_INNER_SIZE[0]);
        self.height = self.height.max(MIN_INNER_SIZE[1]);
        self
    }

    pub fn apply(&self, mut viewport: ViewportBuilder) -> ViewportBuilder {
        let state = self.clone().sanitized();
        viewport = viewport
            .with_inner_size([state.width, state.height])
            .with_maximized(state.maximized)
            .with_fullscreen(state.fullscreen);
        if let (Some(x), Some(y)) = (state.x, state.y) {
            viewport = viewport.with_position(Pos2::new(x, y));
        }
        viewport
    }

    fn incorporate(&self, snapshot: Snapshot) -> Self {
        if snapshot.minimized {
            return self.clone();
        }
        let mut next = self.clone();
        next.maximized = snapshot.maximized;
        next.fullscreen = snapshot.fullscreen;
        if !snapshot.maximized && !snapshot.fullscreen {
            next.width = snapshot.inner_width.max(MIN_INNER_SIZE[0]);
            next.height = snapshot.inner_height.max(MIN_INNER_SIZE[1]);
            next.x = snapshot.outer_x;
            next.y = snapshot.outer_y;
        }
        next
    }

    fn write_to(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

#[derive(Clone, Copy)]
struct Snapshot {
    inner_width: f32,
    inner_height: f32,
    outer_x: Option<f32>,
    outer_y: Option<f32>,
    maximized: bool,
    fullscreen: bool,
    minimized: bool,
}

pub struct WindowPersistence {
    state: WindowState,
    last_written: WindowState,
    last_change: Instant,
    path: PathBuf,
}

impl WindowPersistence {
    pub fn load() -> Self {
        let state = WindowState::load();
        Self {
            last_written: state.clone(),
            last_change: Instant::now(),
            state,
            path: storage_path(),
        }
    }

    pub fn observe(&mut self, window: &Window) -> bool {
        let next = self.state.incorporate(snapshot_from_window(window));
        if next != self.state {
            let flags_changed =
                next.maximized != self.state.maximized || next.fullscreen != self.state.fullscreen;
            self.state = next;
            self.last_change = Instant::now();
            if flags_changed {
                self.flush();
            }
        }
        if self.state != self.last_written && self.last_change.elapsed() >= SAVE_DEBOUNCE {
            self.flush();
        }
        self.state != self.last_written
    }

    pub fn flush(&mut self) {
        if self.state == self.last_written {
            return;
        }
        self.state.write_to(&self.path);
        self.last_written = self.state.clone();
    }
}

fn storage_path() -> PathBuf {
    clip_engine_core::paths::data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("dev.dab.clip-engine"))
        .join("window.json")
}

fn snapshot_from_window(window: &Window) -> Snapshot {
    let scale = window.scale_factor().max(0.1);
    let inner = window.inner_size().to_logical::<f32>(scale);
    let outer = window
        .outer_position()
        .ok()
        .map(|pos| pos.to_logical::<f32>(scale));
    Snapshot {
        inner_width: inner.width.round(),
        inner_height: inner.height.round(),
        outer_x: outer.map(|pos| pos.x.round()),
        outer_y: outer.map(|pos| pos.y.round()),
        maximized: window.is_maximized(),
        fullscreen: window.fullscreen().is_some(),
        minimized: window.is_minimized() == Some(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_size_is_kept_while_maximized() {
        let saved = WindowState {
            width: 1280.0,
            height: 720.0,
            x: Some(40.0),
            y: Some(80.0),
            maximized: false,
            fullscreen: false,
        };
        let next = saved.incorporate(Snapshot {
            inner_width: 1920.0,
            inner_height: 1080.0,
            outer_x: Some(0.0),
            outer_y: Some(0.0),
            maximized: true,
            fullscreen: false,
            minimized: false,
        });
        assert_eq!(next.width, 1280.0);
        assert_eq!(next.height, 720.0);
        assert_eq!(next.x, Some(40.0));
        assert_eq!(next.y, Some(80.0));
        assert!(next.maximized);
    }

    #[test]
    fn size_and_position_update_when_restored() {
        let saved = WindowState {
            width: 1280.0,
            height: 720.0,
            x: Some(40.0),
            y: Some(80.0),
            maximized: true,
            fullscreen: false,
        };
        let next = saved.incorporate(Snapshot {
            inner_width: 1600.0,
            inner_height: 900.0,
            outer_x: Some(120.0),
            outer_y: Some(60.0),
            maximized: false,
            fullscreen: false,
            minimized: false,
        });
        assert_eq!(next.width, 1600.0);
        assert_eq!(next.height, 900.0);
        assert_eq!(next.x, Some(120.0));
        assert_eq!(next.y, Some(60.0));
        assert!(!next.maximized);
    }

    #[test]
    fn minimized_windows_are_ignored() {
        let saved = WindowState::default();
        let next = saved.incorporate(Snapshot {
            inner_width: 200.0,
            inner_height: 100.0,
            outer_x: Some(-32000.0),
            outer_y: Some(-32000.0),
            maximized: false,
            fullscreen: false,
            minimized: true,
        });
        assert_eq!(next, saved);
    }

    #[test]
    fn load_roundtrip_preserves_geometry() {
        let directory = std::env::temp_dir().join(format!(
            "clip-engine-window-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("window.json");
        let state = WindowState {
            width: 1712.0,
            height: 988.0,
            x: Some(64.0),
            y: Some(48.0),
            maximized: true,
            fullscreen: false,
        };
        state.write_to(&path);
        let loaded = WindowState::load_from(&path);
        assert_eq!(loaded, state);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
