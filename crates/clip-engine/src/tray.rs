use anyhow::{Context, Result};
use std::sync::mpsc::{self, Receiver, Sender};
use tray_icon::TrayIcon;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem},
    Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
};

const SHOW_ID: &str = "clip-engine.show";
const START_ID: &str = "clip-engine.start";
const STOP_ID: &str = "clip-engine.stop";
const SAVE_ID: &str = "clip-engine.save";
const QUIT_ID: &str = "clip-engine.quit";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    StartRecorder,
    StopRecorder,
    SaveReplay,
    Quit,
}

#[derive(Clone)]
struct MenuIds {
    show: MenuId,
    start: MenuId,
    stop: MenuId,
    save: MenuId,
    quit: MenuId,
}

pub struct TrayIcons {
    idle: Icon,
    recording: Icon,
}

enum TrayCommand {
    SetRecording(bool),
}

pub struct TrayController {
    menu_ids: MenuIds,
    #[cfg(not(target_os = "linux"))]
    _icon: TrayIcon,
    #[cfg(not(target_os = "linux"))]
    idle_icon: Icon,
    #[cfg(not(target_os = "linux"))]
    recording_icon: Icon,
    #[cfg(target_os = "linux")]
    command_tx: Sender<TrayCommand>,
}

impl TrayController {
    pub fn new(icons: TrayIcons) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            Self::new_linux(icons)
        }

        #[cfg(not(target_os = "linux"))]
        {
            let (menu, menu_ids) = build_menu()?;
            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_icon(icons.idle.clone())
                .with_tooltip("Dabs Clip Engine")
                .with_menu_on_left_click(false)
                .build()
                .context("create system tray icon")?;
            Ok(Self {
                menu_ids,
                _icon: tray,
                idle_icon: icons.idle,
                recording_icon: icons.recording,
            })
        }
    }

    #[cfg(target_os = "linux")]
    fn new_linux(icons: TrayIcons) -> Result<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (command_tx, command_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("clip-engine-tray".into())
            .spawn(move || {
                if let Err(error) = gtk::init() {
                    let _ = ready_tx.send(Err(format!("initialize GTK tray: {error}")));
                    return;
                }

                let (menu, menu_ids) = match build_menu() {
                    Ok(menu) => menu,
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!("build tray menu: {error:#}")));
                        return;
                    }
                };
                let tray = TrayIconBuilder::new()
                    .with_menu(Box::new(menu))
                    .with_icon(icons.idle.clone())
                    .with_title("Clip Engine")
                    .build();
                match tray {
                    Ok(tray) => {
                        let _ = ready_tx.send(Ok(menu_ids));
                        install_icon_update_source(tray.clone(), icons, command_rx);
                        gtk::main();
                        drop(tray);
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!("create system tray icon: {error}")));
                    }
                }
            })
            .context("start Linux tray thread")?;

        let menu_ids = ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .context("wait for Linux tray initialization")?
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            menu_ids,
            command_tx,
        })
    }

    pub fn set_recording(&self, recording: bool) {
        #[cfg(target_os = "linux")]
        {
            let _ = self.command_tx.send(TrayCommand::SetRecording(recording));
        }

        #[cfg(not(target_os = "linux"))]
        {
            let icon = if recording {
                self.recording_icon.clone()
            } else {
                self.idle_icon.clone()
            };
            let _ = self._icon.set_icon(Some(icon));
        }
    }

    pub fn poll(&self) -> Vec<TrayAction> {
        let mut actions = Vec::new();

        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                actions.push(TrayAction::Show);
            }
        }

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let action = if event.id == self.menu_ids.show {
                Some(TrayAction::Show)
            } else if event.id == self.menu_ids.start {
                Some(TrayAction::StartRecorder)
            } else if event.id == self.menu_ids.stop {
                Some(TrayAction::StopRecorder)
            } else if event.id == self.menu_ids.save {
                Some(TrayAction::SaveReplay)
            } else if event.id == self.menu_ids.quit {
                Some(TrayAction::Quit)
            } else {
                None
            };
            if let Some(action) = action {
                actions.push(action);
            }
        }

        actions
    }
}

#[cfg(target_os = "linux")]
fn install_icon_update_source(tray: TrayIcon, icons: TrayIcons, command_rx: Receiver<TrayCommand>) {
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        while let Ok(command) = command_rx.try_recv() {
            match command {
                TrayCommand::SetRecording(recording) => {
                    let icon = if recording {
                        icons.recording.clone()
                    } else {
                        icons.idle.clone()
                    };
                    let _ = tray.set_icon(Some(icon));
                }
            }
        }
        gtk::glib::ControlFlow::Continue
    });
}

fn build_menu() -> Result<(Menu, MenuIds)> {
    let show = MenuItem::with_id(MenuId::new(SHOW_ID), "Show Clip Engine", true, None);
    let start = MenuItem::with_id(MenuId::new(START_ID), "Start replay buffer", true, None);
    let stop = MenuItem::with_id(MenuId::new(STOP_ID), "Stop replay buffer", true, None);
    let save = MenuItem::with_id(MenuId::new(SAVE_ID), "Save last replay", true, None);
    let quit = MenuItem::with_id(MenuId::new(QUIT_ID), "Quit", true, None);
    let menu = Menu::with_items(&[&show, &start, &stop, &save, &quit])
        .context("add items to tray menu")?;
    Ok((
        menu,
        MenuIds {
            show: show.id().clone(),
            start: start.id().clone(),
            stop: stop.id().clone(),
            save: save.id().clone(),
            quit: quit.id().clone(),
        },
    ))
}

pub fn load_icons() -> Result<TrayIcons> {
    let image = image::load_from_memory(include_bytes!("../assets/clip-engine.png"))
        .context("load tray icon")?
        .to_rgba8();
    let recording_image = recording_icon_image(&image);
    Ok(TrayIcons {
        idle: icon_from_image(&image)?,
        recording: icon_from_image(&recording_image)?,
    })
}

fn icon_from_image(image: &image::RgbaImage) -> Result<Icon> {
    Icon::from_rgba(image.as_raw().clone(), image.width(), image.height())
        .context("create tray icon image")
}

fn recording_icon_image(image: &image::RgbaImage) -> image::RgbaImage {
    let mut recording = image.clone();
    if image.width() == 0 || image.height() == 0 {
        return recording;
    }
    let diameter = (image.width().min(image.height()) / 5).max(8);
    let radius = diameter / 2;
    let outer_radius = radius + 2;
    let center_x = image.width().saturating_sub(outer_radius + 2);
    let center_y = outer_radius + 2;
    let outer_radius_squared = (outer_radius * outer_radius) as i64;
    let radius_squared = (radius * radius) as i64;

    for y in
        center_y.saturating_sub(outer_radius)..=(center_y + outer_radius).min(image.height() - 1)
    {
        for x in
            center_x.saturating_sub(outer_radius)..=(center_x + outer_radius).min(image.width() - 1)
        {
            let dx = x as i64 - center_x as i64;
            let dy = y as i64 - center_y as i64;
            let distance_squared = dx * dx + dy * dy;
            if distance_squared <= outer_radius_squared {
                let pixel = recording.get_pixel_mut(x, y);
                *pixel = if distance_squared <= radius_squared {
                    image::Rgba([235, 45, 55, 255])
                } else {
                    image::Rgba([35, 20, 22, 255])
                };
            }
        }
    }
    recording
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_icon_has_a_red_status_dot() {
        let image = image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 255]));
        let recording = recording_icon_image(&image);
        assert!(recording
            .pixels()
            .any(|pixel| pixel[0] > 200 && pixel[1] < 80 && pixel[2] < 80));
    }
}
