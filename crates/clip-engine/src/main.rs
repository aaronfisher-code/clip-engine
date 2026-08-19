#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clip_engine_core::{Engine, PRODUCT_NAME};
use eframe::egui::{IconData, ViewportBuilder};
use eframe::egui_glow::{GlowConfiguration, HardwareAcceleration};
use eframe::Renderer;
use std::sync::Arc;
use tokio::runtime::Runtime;

#[cfg(windows)]
#[no_mangle]
#[used]
static NvOptimusEnablement: u32 = 1;

#[cfg(windows)]
#[no_mangle]
#[used]
static AmdPowerXpressRequestHighPerformance: i32 = 1;

mod app;
mod player;
mod startup;
mod theme;
mod tray;
mod window_state;

fn main() {
    configure_numeric_locale();
    isolate_linux_input();
    install_panic_hook();
    if let Err(error) = run() {
        let message = format!("{error:#}");
        let _ = write_crash_log(&message);
        show_error_dialog(&format!("{PRODUCT_NAME} could not start"), &message);
        std::process::exit(1);
    }
}

fn configure_numeric_locale() {
    #[cfg(all(unix, not(target_os = "macos")))]
    unsafe {
        // libmpv requires LC_NUMERIC to remain the C locale when its client
        // API is created. Keep child processes on the same setting too.
        std::env::set_var("LC_NUMERIC", "C");
        let _ = libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr());
    }
}

fn run() -> anyhow::Result<()> {
    let instance = single_instance::SingleInstance::new("dev.dab.clip-engine")
        .map_err(|error| anyhow::anyhow!("Could not create the single-instance lock: {error}"))?;
    if !instance.is_single() {
        return Ok(());
    }

    check_runtime_files()?;
    let runtime = Runtime::new().map_err(|error| anyhow::anyhow!("tokio runtime: {error}"))?;
    let engine = Engine::initialize(runtime.handle().clone())
        .map_err(|error| anyhow::anyhow!("Could not start {PRODUCT_NAME}: {error:#}"))?;
    let background = startup::is_background_requested(std::env::args_os());
    let launch_at_login = match engine.database.setting(startup::LAUNCH_AT_LOGIN_SETTING)? {
        Some(value) => value != "false",
        None => {
            engine
                .database
                .put_setting(startup::LAUNCH_AT_LOGIN_SETTING, "true")?;
            true
        }
    };
    let startup_error = startup::set_enabled(launch_at_login)
        .err()
        .map(|error| format!("{error:#}"));
    let _keep_alive = runtime;
    let icon = load_icon();
    let mut glow_options = native_options(icon, background);
    glow_options.renderer = Renderer::Glow;
    glow_options.glow_options.hardware_acceleration = HardwareAcceleration::Required;
    match launch(
        engine.clone(),
        glow_options,
        background,
        launch_at_login,
        startup_error.clone(),
    ) {
        Ok(()) => Ok(()),
        Err(error) if should_try_wgpu(&error) => {
            let mut wgpu_options = native_options(load_icon(), background);
            wgpu_options.renderer = Renderer::Wgpu;
            launch(
                engine,
                wgpu_options,
                background,
                launch_at_login,
                startup_error,
            )
            .map_err(|error| anyhow::anyhow!("{error}\n\n{GPU_HELP}"))
        }
        Err(error) => Err(anyhow::anyhow!("{error}\n\n{GPU_HELP}")),
    }
}

const GPU_HELP: &str = "Clip Engine could not use this PC's graphics. Install the latest graphics driver from Intel, AMD, or NVIDIA (the one that matches the GPU in this machine), then restart. A GPU driver is required; the installer cannot add that for you.";

fn check_runtime_files() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let Some(directory) = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(std::path::PathBuf::from))
        else {
            return Ok(());
        };
        let dll = directory.join("mpv-2.dll");
        if !dll.is_file() {
            anyhow::bail!(
                "mpv-2.dll is missing from the install folder. Reinstall Dabs Clip Engine."
            );
        }
    }
    Ok(())
}

fn native_options(icon: Arc<IconData>, background: bool) -> eframe::NativeOptions {
    let viewport = window_state::WindowState::load().apply(
        ViewportBuilder::default()
            .with_min_inner_size(window_state::MIN_INNER_SIZE)
            .with_title(PRODUCT_NAME)
            .with_app_id("dev.dab.clip-engine")
            .with_drag_and_drop(true)
            .with_visible(!background)
            .with_icon(icon),
    );
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport,
        renderer: Renderer::Glow,
        glow_options: GlowConfiguration {
            vsync: true,
            hardware_acceleration: HardwareAcceleration::Required,
            shader_version: None,
        },
        ..Default::default()
    };
    // winit has no Wayland file-drop events. Prefer X11 so the OS can deliver drops.
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("DISPLAY").is_some() {
        options.event_loop_builder = Some(Box::new(|event_loop| {
            use winit::platform::x11::EventLoopBuilderExtX11 as _;
            event_loop.with_x11();
        }));
    }
    options
}

fn launch(
    engine: Engine,
    options: eframe::NativeOptions,
    background: bool,
    launch_at_login: bool,
    startup_error: Option<String>,
) -> Result<(), eframe::Error> {
    eframe::run_native(
        "clip-engine",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::ClipApp::new(
                cc,
                engine,
                background,
                launch_at_login,
                startup_error,
            )))
        }),
    )
}

fn should_try_wgpu(error: &eframe::Error) -> bool {
    match error {
        eframe::Error::OpenGL(_)
        | eframe::Error::Glutin(_)
        | eframe::Error::NoGlutinConfigs(_, _) => true,
        other => {
            let text = other.to_string().to_ascii_lowercase();
            text.contains("opengl") || text.contains("glutin")
        }
    }
}

fn load_icon() -> Arc<IconData> {
    let bytes = include_bytes!("../assets/clip-engine.png");
    match image::load_from_memory(bytes) {
        Ok(image) => {
            let rgba = image.to_rgba8();
            Arc::new(IconData {
                rgba: rgba.as_raw().clone(),
                width: rgba.width(),
                height: rgba.height(),
            })
        }
        Err(_) => Arc::new(IconData {
            rgba: vec![0, 0, 0, 255],
            width: 1,
            height: 1,
        }),
    }
}

fn isolate_linux_input() {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // SAFETY: no threads exist yet. AT-SPI/GTK a11y panics abort the AppImage.
        unsafe {
            std::env::set_var("NO_AT_BRIDGE", "1");
            std::env::set_var("GTK_A11Y", "none");
            std::env::set_var("QT_ACCESSIBILITY", "0");
        }
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info.to_string();
        let _ = write_crash_log(&message);
        show_error_dialog(&format!("{PRODUCT_NAME} crashed"), &message);
        previous(info);
    }));
}

fn write_crash_log(message: &str) -> std::io::Result<()> {
    let directory = clip_engine_core::paths::data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("dev.dab.clip-engine"));
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("crash.log"),
        format!("{PRODUCT_NAME} failed to start\n\n{message}\n"),
    )
}

fn show_error_dialog(title: &str, message: &str) {
    eprintln!("{title}: {message}");
    #[cfg(windows)]
    {
        fn wide(value: &str) -> Vec<u16> {
            use std::os::windows::ffi::OsStrExt;
            std::ffi::OsStr::new(value)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        }
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBoxW(
                hwnd: *mut core::ffi::c_void,
                text: *const u16,
                caption: *const u16,
                ty: u32,
            ) -> i32;
        }
        let title = wide(title);
        let message = wide(message);
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                0x0000_0010,
            );
        }
        return;
    }
    #[cfg(not(windows))]
    {
        let _ = rfd::MessageDialog::new()
            .set_title(title)
            .set_description(message)
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn second_instance_is_rejected() {
        let key = format!("dev.dab.clip-engine-test-{}", std::process::id());
        let first = single_instance::SingleInstance::new(&key).unwrap();
        let second = single_instance::SingleInstance::new(&key).unwrap();
        assert!(first.is_single());
        assert!(!second.is_single());
    }
}
