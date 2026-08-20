use std::path::Path;

#[cfg(not(test))]
const APP_NAME: &str = "Clip Engine";

#[cfg(all(not(test), windows))]
const APP_ID: &str = "dev.dab.clip-engine";

pub fn replay_saved(path: &Path, duration_seconds: u32) {
    show("Replay saved", &saved_body(path, duration_seconds), false);
}

pub fn replay_save_failed(error: &impl std::fmt::Display) {
    show("Replay save failed", &error.to_string(), true);
}

pub(crate) fn saved_body(path: &Path, duration_seconds: u32) -> String {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("replay");
    format!("{file_name} ({duration_seconds}s)")
}

#[cfg(test)]
fn show(_summary: &str, _body: &str, _error: bool) {}

#[cfg(not(test))]
fn show(summary: &str, body: &str, error: bool) {
    let summary = summary.to_string();
    let body = body.to_string();
    let _ = std::thread::Builder::new()
        .name("clip-engine-recorder-notify".into())
        .spawn(move || {
            play_alert(error);
            if let Err(notify_error) = build_notification(&summary, &body, error) {
                eprintln!("recorder notification failed: {notify_error}");
            }
        });
}

#[cfg(not(test))]
fn build_notification(
    summary: &str,
    body: &str,
    error: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut notification = notify_rust::Notification::new();
    notification
        .appname(APP_NAME)
        .summary(summary)
        .body(body)
        .timeout(notify_rust::Timeout::Milliseconds(5_000));
    apply_platform_hints(&mut notification, error);
    notification.show()?;
    Ok(())
}

#[cfg(not(test))]
fn apply_platform_hints(notification: &mut notify_rust::Notification, error: bool) {
    #[cfg(windows)]
    {
        // Match the AppUserModelID registered by the Windows installer so
        // notifications are attributed to Clip Engine instead of PowerShell.
        notification.app_id(APP_ID);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        notification.urgency(if error {
            notify_rust::Urgency::Critical
        } else {
            notify_rust::Urgency::Normal
        });
        notification.sound_name(if error { "dialog-error" } else { "complete" });
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = (notification, error);
    }
}

#[cfg(not(test))]
fn play_alert(error: bool) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONASTERISK, MB_ICONHAND};
        let kind = if error { MB_ICONHAND } else { MB_ICONASTERISK };
        MessageBeep(kind);
    }
    #[cfg(not(windows))]
    {
        let _ = error;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn saved_body_uses_file_name_and_duration() {
        assert_eq!(
            saved_body(Path::new("/tmp/clips/replay-1.mkv"), 30),
            "replay-1.mkv (30s)"
        );
    }
}
