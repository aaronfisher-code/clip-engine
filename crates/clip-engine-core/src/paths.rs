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
        let source = default_inbox_dir()?;
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
        let mut value = Self {
            database: data.join("clip-engine.sqlite3"),
            source,
            previews: data.join("previews"),
            exports: data.join("exports"),
            data,
            ffmpeg,
            ffprobe,
        };
        std::fs::create_dir_all(&value.data)?;
        if std::fs::create_dir_all(&value.source).is_err() {
            value.source = value.data.join("Inbox");
            std::fs::create_dir_all(&value.source)?;
        }
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

pub fn default_inbox_dir() -> anyhow::Result<PathBuf> {
    let videos = video_dir()?;
    let branded = videos.join(crate::PRODUCT_NAME).join("Inbox");
    let legacy = videos.join("Clip Engine").join("Inbox");
    if branded.exists() || !legacy.exists() {
        Ok(branded)
    } else {
        Ok(legacy)
    }
}

pub fn path_is_within(path: &Path, directory: &Path) -> bool {
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    let Ok(directory) = directory.canonicalize() else {
        return false;
    };
    path.starts_with(directory)
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
        if let Some(directory) = resource_dir_for_executable(&path) {
            return directory;
        }
    }
    if let Some(directory) = std::env::var_os("APPDIR")
        .map(PathBuf::from)
        .and_then(|appdir| resource_dir_for_appdir(&appdir))
    {
        return directory;
    }
    PathBuf::from("resources")
}

fn resource_dir_for_executable(exe: &Path) -> Option<PathBuf> {
    let directory = exe.parent()?;
    let next_to_exe = directory.join("resources");
    if next_to_exe.is_dir() {
        return Some(next_to_exe);
    }
    if directory.join("binaries").is_dir() {
        return Some(directory.to_path_buf());
    }
    // cargo-packager Linux AppImage/deb: /usr/bin/clip-engine plus
    // /usr/lib/clip-engine/binaries/{ffmpeg,ffprobe}.
    let packager = directory.join("..").join("lib").join("clip-engine");
    if packager.join("binaries").is_dir() {
        return Some(packager.canonicalize().unwrap_or(packager));
    }
    None
}

fn resource_dir_for_appdir(appdir: &Path) -> Option<PathBuf> {
    let packager = appdir.join("usr").join("lib").join("clip-engine");
    packager
        .join("binaries")
        .is_dir()
        .then(|| packager.canonicalize().unwrap_or_else(|_| packager.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_is_within_detects_files_in_a_directory() {
        let directory =
            std::env::temp_dir().join(format!("clip-engine-inbox-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("clip.mkv");
        std::fs::write(&file, b"test").unwrap();
        assert!(path_is_within(&file, &directory));
        assert!(!path_is_within(&file, &directory.join("missing")));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn resource_dir_finds_packager_linux_layout() {
        let root =
            std::env::temp_dir().join(format!("clip-engine-appdir-{}", uuid::Uuid::new_v4()));
        let exe = root.join("usr").join("bin").join("clip-engine");
        let binaries = root
            .join("usr")
            .join("lib")
            .join("clip-engine")
            .join("binaries");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&binaries).unwrap();
        std::fs::write(&exe, b"").unwrap();
        std::fs::write(binaries.join("ffprobe"), b"").unwrap();
        let discovered = resource_dir_for_executable(&exe).unwrap();
        assert_eq!(
            discovered.join("binaries").join("ffprobe"),
            binaries.join("ffprobe")
        );
        assert_eq!(
            resource_dir_for_appdir(&root).as_deref(),
            Some(discovered.as_path())
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
