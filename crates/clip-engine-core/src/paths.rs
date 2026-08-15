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
    pub fn discover() -> anyhow::Result<Self> {
        let data = data_dir()?;
        let source = video_dir()?.join("Clip Engine").join("Inbox");
        let resource = resource_dir();
        Self::new(data, source, &resource)
    }

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
                if bundled_ffmpeg.is_file() {
                    bundled_ffmpeg
                } else {
                    PathBuf::from("ffmpeg")
                }
            });
        let ffprobe = std::env::var_os("CLIP_ENGINE_FFPROBE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                if bundled_ffprobe.is_file() {
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

pub fn data_dir() -> anyhow::Result<PathBuf> {
    if let Some(value) = std::env::var_os("CLIP_ENGINE_DATA_DIR") {
        return Ok(PathBuf::from(value));
    }
    #[cfg(windows)]
    {
        let local = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        Ok(local.join("dev.dab.clip-engine"))
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(home.join("dev.dab.clip-engine"))
    }
}

pub fn video_dir() -> anyhow::Result<PathBuf> {
    if let Some(value) = std::env::var_os("CLIP_ENGINE_SOURCE_DIR") {
        return Ok(PathBuf::from(value));
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        Ok(user.join("Videos"))
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("XDG_VIDEOS_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Videos")))
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(home)
    }
}

pub fn resource_dir() -> PathBuf {
    if let Some(value) = std::env::var_os("CLIP_ENGINE_RESOURCE_DIR") {
        return PathBuf::from(value);
    }
    if let Ok(path) = std::env::current_exe() {
        if let Some(directory) = path.parent() {
            let next_to_exe = directory.join("resources");
            if next_to_exe.is_dir() {
                return next_to_exe;
            }
            let bundled = directory.join("binaries");
            if bundled.is_dir() {
                return directory.to_path_buf();
            }
        }
    }
    PathBuf::from("resources")
}
