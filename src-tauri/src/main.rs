#[cfg(target_os = "linux")]
fn configure_linux_webview() {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|value| value == "wayland");
    let nvidia = std::path::Path::new("/proc/driver/nvidia/version").is_file()
        || std::env::var("__GLX_VENDOR_LIBRARY_NAME").is_ok_and(|value| value == "nvidia");
    if wayland && nvidia && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    configure_linux_webview();
    clip_engine_lib::run();
}
