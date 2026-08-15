use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data: PathBuf,
    pub database: PathBuf,
    pub source: PathBuf,
    pub previews: PathBuf,
    pub exports: PathBuf,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

impl AppPaths {
    pub fn new(data: PathBuf, source: PathBuf, resource: &Path) -> anyhow::Result<Self> {
        let executable = if cfg!(windows) { ".exe" } else { "" };
        let bundled_ffmpeg = resource
            .join("binaries")
            .join(format!("ffmpeg{executable}"));
        let bundled_ffprobe = resource
            .join("binaries")
            .join(format!("ffprobe{executable}"));
        let ffmpeg = std::env::var_os("CLIP_ENGINE_FFMPEG")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                if cfg!(all(debug_assertions, not(windows))) {
                    PathBuf::from("ffmpeg")
                } else if bundled_ffmpeg.is_file() {
                    bundled_ffmpeg
                } else {
                    PathBuf::from("ffmpeg")
                }
            });
        let ffprobe = std::env::var_os("CLIP_ENGINE_FFPROBE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                if cfg!(all(debug_assertions, not(windows))) {
                    PathBuf::from("ffprobe")
                } else if bundled_ffprobe.is_file() {
                    bundled_ffprobe
                } else {
                    PathBuf::from("ffprobe")
                }
            });
        let value = Self {
            database: data.join("clip-engine.sqlite3"),
            source,
            previews: data.join("previews"),
            exports: data.join("exports"),
            data,
            ffmpeg,
            ffprobe,
        };
        std::fs::create_dir_all(&value.data)?;
        std::fs::create_dir_all(&value.source)?;
        std::fs::create_dir_all(&value.previews)?;
        std::fs::create_dir_all(&value.exports)?;
        Ok(value)
    }
}
