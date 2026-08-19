use clip_engine_recorder_protocol::Hotkey;
use global_hotkey::{
    hotkey::HotKey as GlobalHotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use std::{
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use crate::backend::RecorderBackend;

type SharedBackend = Arc<Mutex<Box<dyn RecorderBackend>>>;
type ConfigureResult = std::result::Result<(), String>;

enum Command {
    Configure {
        hotkey: Option<Hotkey>,
        notify_on_save: bool,
        result: Sender<ConfigureResult>,
    },
    Shutdown,
}

struct HotkeyStateSnapshot {
    registered: bool,
    error: Option<String>,
    notify_on_save: bool,
}

impl Default for HotkeyStateSnapshot {
    fn default() -> Self {
        Self {
            registered: false,
            error: None,
            notify_on_save: true,
        }
    }
}

pub struct HotkeyController {
    command_tx: Sender<Command>,
    state: Arc<Mutex<HotkeyStateSnapshot>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HotkeyController {
    pub fn new(backend: SharedBackend) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(HotkeyStateSnapshot::default()));
        let thread_state = state.clone();
        let thread = thread::Builder::new()
            .name("clip-engine-recorder-hotkey".into())
            .spawn(move || run_hotkey_loop(command_rx, backend, thread_state))
            .expect("recorder hotkey thread should spawn");
        Self {
            command_tx,
            state,
            thread: Some(thread),
        }
    }

    pub fn configure(&self, hotkey: Option<Hotkey>, notify_on_save: bool) -> ConfigureResult {
        let (result_tx, result_rx) = mpsc::channel();
        self.command_tx
            .send(Command::Configure {
                hotkey,
                notify_on_save,
                result: result_tx,
            })
            .map_err(|_| "The recorder hotkey thread stopped.".to_string())?;
        result_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "The recorder hotkey registration timed out.".to_string())?
    }

    pub fn status(&self) -> (bool, Option<String>) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (state.registered, state.error.clone())
    }
}

impl Drop for HotkeyController {
    fn drop(&mut self) {
        let _ = self.command_tx.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_hotkey_loop(
    command_rx: Receiver<Command>,
    backend: SharedBackend,
    state: Arc<Mutex<HotkeyStateSnapshot>>,
) {
    let manager = GlobalHotKeyManager::new();
    let manager_error = manager.as_ref().err().map(ToString::to_string);
    let manager = manager.ok();
    let mut active: Option<GlobalHotKey> = None;

    loop {
        if !pump_platform_messages() {
            return;
        }
        while let Ok(command) = command_rx.try_recv() {
            match command {
                Command::Configure {
                    hotkey,
                    notify_on_save,
                    result,
                } => {
                    let outcome = configure_hotkey(
                        manager.as_ref(),
                        &mut active,
                        hotkey,
                        manager_error.as_deref(),
                    );
                    let mut snapshot = state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    snapshot.registered = outcome.is_ok() && active.is_some();
                    snapshot.error = outcome.as_ref().err().cloned();
                    snapshot.notify_on_save = notify_on_save;
                    let _ = result.send(outcome);
                }
                Command::Shutdown => return,
            }
        }

        match GlobalHotKeyEvent::receiver().try_recv() {
            Ok(event) if event.state == HotKeyState::Pressed => {
                if active
                    .as_ref()
                    .is_some_and(|hotkey| hotkey.id() == event.id)
                {
                    let notify = state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .notify_on_save;
                    let outcome = {
                        let mut backend = backend
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        backend.save_replay()
                    };
                    match outcome {
                        Ok(replay) => {
                            eprintln!(
                                "recorder hotkey replay saved: {} ({}s)",
                                replay.path.display(),
                                replay.duration_seconds
                            );
                            if notify {
                                crate::notify::replay_saved(&replay.path, replay.duration_seconds);
                            }
                        }
                        Err(error) => {
                            eprintln!("recorder hotkey replay save failed: {error:#}");
                            if notify {
                                crate::notify::replay_save_failed(&error);
                            }
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn pump_platform_messages() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_QUIT,
    };
    let mut message = MSG {
        hwnd: std::ptr::null_mut(),
        message: 0,
        wParam: 0,
        lParam: 0,
        time: 0,
        pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
    };
    while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        if message.message == WM_QUIT {
            return false;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    true
}

#[cfg(not(windows))]
fn pump_platform_messages() -> bool {
    true
}

fn configure_hotkey(
    manager: Option<&GlobalHotKeyManager>,
    active: &mut Option<GlobalHotKey>,
    requested: Option<Hotkey>,
    manager_error: Option<&str>,
) -> ConfigureResult {
    let Some(requested) = requested else {
        if let (Some(manager), Some(previous)) = (manager, active.take()) {
            manager
                .unregister(previous)
                .map_err(|error| format!("unregister replay hotkey: {error}"))?;
        } else {
            *active = None;
        }
        return Ok(());
    };
    let requested = parse_hotkey(requested)?;
    if active.as_ref().is_some_and(|active| *active == requested) {
        return Ok(());
    }
    let Some(manager) = manager else {
        return Err(manager_error
            .unwrap_or("The current desktop session does not support global hotkeys.")
            .to_string());
    };
    if let Some(previous) = active.take() {
        let _ = manager.unregister(previous);
    }
    manager
        .register(requested)
        .map_err(|error| format!("register replay hotkey: {error}"))?;
    *active = Some(requested);
    Ok(())
}

fn parse_hotkey(hotkey: Hotkey) -> std::result::Result<GlobalHotKey, String> {
    hotkey.validate().map_err(|error| error.to_string())?;
    let mut value = String::new();
    if hotkey.ctrl {
        value.push_str("Ctrl+");
    }
    if hotkey.alt {
        value.push_str("Alt+");
    }
    if hotkey.shift {
        value.push_str("Shift+");
    }
    if hotkey.meta {
        value.push_str("Super+");
    }
    value.push_str(&hotkey.key);
    value
        .parse()
        .map_err(|error| format!("parse replay hotkey {value}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_replay_hotkey() {
        assert!(parse_hotkey(Hotkey::default()).is_ok());
    }

    #[test]
    fn rejects_empty_replay_hotkey() {
        assert!(parse_hotkey(Hotkey {
            key: String::new(),
            ..Hotkey::default()
        })
        .is_err());
    }
}
