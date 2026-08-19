use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::path::PathBuf;

pub const BACKGROUND_ARG: &str = "--background";
pub const LAUNCH_AT_LOGIN_SETTING: &str = "launch_at_login";

pub fn is_background_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .any(|arg| arg.as_ref() == OsStr::new(BACKGROUND_ARG))
}

pub fn startup_arguments() -> [&'static str; 1] {
    [BACKGROUND_ARG]
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    let launcher = launcher()?;
    if enabled {
        launcher.enable().context("enable launch at login")
    } else {
        launcher.disable().context("disable launch at login")
    }
}

fn launcher() -> Result<auto_launcher::AutoLaunch> {
    let executable = executable_path()?;
    let executable = quoted_executable(&executable)?;
    let args = startup_arguments();

    #[cfg(target_os = "linux")]
    {
        Ok(auto_launcher::AutoLaunch::new(
            "Dabs Clip Engine",
            &executable,
            auto_launcher::LinuxLaunchMode::XdgAutostart,
            &args,
        ))
    }

    #[cfg(windows)]
    {
        Ok(auto_launcher::AutoLaunch::new(
            "Dabs Clip Engine",
            &executable,
            auto_launcher::WindowsEnableMode::CurrentUser,
            &args,
        ))
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = args;
        anyhow::bail!("launch at login is not supported on this platform")
    }
}

fn executable_path() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        let appimage = PathBuf::from(appimage);
        if appimage.is_file() {
            return Ok(appimage);
        }
    }

    std::env::current_exe().context("discover the application path")
}

fn quoted_executable(path: &std::path::Path) -> Result<String> {
    let path = path
        .to_str()
        .context("the application path is not valid UTF-8")?;
    #[cfg(windows)]
    let escaped = path.replace('"', "\\\"");
    #[cfg(target_os = "linux")]
    let escaped = path
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$");
    #[cfg(not(any(target_os = "linux", windows)))]
    let escaped = path.to_string();
    Ok(format!("\"{escaped}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_argument_is_detected_without_matching_video_paths() {
        assert!(is_background_requested(["clip-engine", BACKGROUND_ARG]));
        assert!(!is_background_requested(["clip-engine", "recording.mp4"]));
    }

    #[test]
    fn startup_command_uses_background_mode() {
        assert_eq!(startup_arguments(), [BACKGROUND_ARG]);
    }

    #[test]
    fn startup_command_quotes_paths_with_spaces() {
        let path =
            quoted_executable(std::path::Path::new("/opt/Dabs Clip Engine/clip-engine")).unwrap();
        assert_eq!(path, "\"/opt/Dabs Clip Engine/clip-engine\"");
    }
}
