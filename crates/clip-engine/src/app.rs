use crate::player::Player;
use crate::startup;
use crate::theme;
use crate::tray::{TrayAction, TrayController};
use crate::window_state::WindowPersistence;
use clip_engine_core::cloud::{AccessRequest, AdminUser, CloudClip, CloudUser, PasswordReset};
use clip_engine_core::models::{AppConfig, Clip, PublishJob, Selection};
use clip_engine_core::paths::{default_inbox_dir, video_dir};
use clip_engine_core::{
    export_options, format_file_size, install_desktop_update, safe_base_name, AudioRoute,
    AudioSourceCapability, AudioSourceKind, AvailableUpdate, CaptureBackend, EncoderCapability,
    EncoderSettingCapability, Engine, Hotkey, Multipass, PublishOption, RateControl, Rational,
    RecorderCapabilities, RecorderConfig, RecorderMode, RecorderState, RecorderStatus,
    SystemAudioMode, APP_NAME, PRODUCT_NAME,
};
use eframe::egui::{
    self, Align, Color32, ColorImage, CornerRadius, CursorIcon, Layout, Pos2, Rect, RichText,
    Sense, StrokeKind, TextureHandle, TextureOptions, Ui, UiBuilder, Vec2,
};
use raw_window_handle::HasDisplayHandle;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthMode {
    Request,
    Login,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccessFilter {
    Pending,
    Active,
    Revoked,
    Denied,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RecorderTab {
    Video,
    Audio,
}

enum Message {
    Error(String),
    Notice(String),
    Refresh,
    Imported(Vec<String>),
    User(CloudUser),
    LoggedOut,
    CloudClips(Vec<CloudClip>),
    AccessRequest(Option<AccessRequest>),
    Admin(Vec<AdminUser>, Vec<AccessRequest>),
    PasswordReset(PasswordReset),
    Busy(bool),
    ExportProgress(f64),
    ExportDone(PathBuf),
    UpdateAvailable {
        update: Option<AvailableUpdate>,
        manual: bool,
    },
    UpdateProgress {
        received: u64,
        total: u64,
    },
    UpdateDownloaded(PathBuf),
    RecorderRefreshed {
        capabilities: RecorderCapabilities,
        status: Box<RecorderStatus>,
    },
    RecorderImported(Vec<String>),
}

struct EditorState {
    clip_id: String,
    start: f64,
    end: f64,
    tracks: Vec<i64>,
    muted: bool,
    export_height: i64,
    export_fps: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TimeField {
    In,
    Out,
}

struct TimeEdit {
    field: TimeField,
    text: String,
    requested_focus: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TimelineDrag {
    Playhead,
    In,
    Out,
}

#[derive(Clone, Copy)]
struct TimelineDragState {
    kind: TimelineDrag,
    time: f64,
    was_playing: bool,
}

#[derive(Clone)]
enum PublishModal {
    Name {
        clip_id: String,
        title: String,
        selection: Selection,
        quality_label: String,
        focus_title: bool,
    },
    Job {
        id: String,
    },
}

#[derive(Clone)]
enum ExportModal {
    Working {
        quality_label: String,
        progress: f64,
    },
    Done {
        path: PathBuf,
    },
}

#[derive(Clone)]
enum UpdateModal {
    Prompt,
    Downloading { received: u64, total: u64 },
    Installing,
}

pub struct ClipApp {
    engine: Engine,
    player: Option<Player>,
    config: AppConfig,
    default_inbox: Option<PathBuf>,
    clips: Vec<Clip>,
    jobs: Vec<PublishJob>,
    cloud_clips: Vec<CloudClip>,
    selected_id: Option<String>,
    library_open: bool,
    user: Option<CloudUser>,
    access_request: Option<AccessRequest>,
    show_auth: Option<AuthMode>,
    show_access: bool,
    account_open: bool,
    admin_users: Vec<AdminUser>,
    admin_requests: Vec<AccessRequest>,
    access_filter: AccessFilter,
    access_query: String,
    created_reset: Option<PasswordReset>,
    pending_delete_job: Option<String>,
    pending_delete_clip: Option<String>,
    publish_modal: Option<PublishModal>,
    export_modal: Option<ExportModal>,
    editor: Option<EditorState>,
    thumbs: HashMap<String, TextureHandle>,
    notice: Option<String>,
    notice_until: Option<Instant>,
    error: Option<String>,
    error_until: Option<Instant>,
    busy: bool,
    last_refresh: Instant,
    tx: Sender<Message>,
    rx: Receiver<Message>,
    auth_username: String,
    auth_display: String,
    auth_password: String,
    auth_confirm: String,
    reset_token: String,
    forgot_step: u8,
    player_error: Option<String>,
    device_name: String,
    session_media: Option<String>,
    timeline_drag: Option<TimelineDragState>,
    timeline_settling: bool,
    time_edit: Option<TimeEdit>,
    drop_hovering: bool,
    available_update: Option<AvailableUpdate>,
    update_modal: Option<UpdateModal>,
    update_checking: bool,
    show_pending: bool,
    show_recorder: bool,
    recorder_config: RecorderConfig,
    recorder_capabilities: RecorderCapabilities,
    recorder_status: RecorderStatus,
    recorder_loaded: bool,
    recorder_loading: bool,
    recorder_application_selection: Option<String>,
    recorder_playback_selection: Option<String>,
    recorder_tab: RecorderTab,
    recorder_hotkey_listening: bool,
    last_recorder_refresh: Instant,
    window: WindowPersistence,
    tray: Option<TrayController>,
    tray_recording: bool,
    allow_exit: bool,
    launch_at_login: bool,
}

impl ClipApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        engine: Engine,
        background: bool,
        launch_at_login: bool,
        startup_error: Option<String>,
    ) -> Self {
        theme::apply(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        let config = engine.config().unwrap_or_else(|_| AppConfig {
            source_directory: String::new(),
            authenticated: false,
            pending_access_request: false,
            r2_configured: false,
            public_base_url: Some("https://clips.dab.dev".into()),
            api_base_url: "https://api.clips.dab.dev".into(),
            media_base_url: None,
            platform: std::env::consts::OS.into(),
            export: clip_engine_core::ExportConfig {
                width: 1920,
                height: 1080,
                fps: 120,
                codec: "libx264".into(),
                crf: 20,
            },
            recorder: clip_engine_core::RecorderConfig::default(),
        });
        let clips = engine.clips().unwrap_or_default();
        let jobs = engine.jobs().unwrap_or_default();
        let selected_id = None;
        let default_inbox = default_inbox_dir().ok();
        for clip in &clips {
            if Path::new(&clip.source_path).is_file() {
                let _ = engine.prepare_preview(&clip.id, false);
            }
        }
        let clips = engine.clips().unwrap_or(clips);
        let recorder_capabilities = engine.recorder_capabilities();
        let recorder_status = engine.recorder_status();
        let (tray, tray_error) = match crate::tray::load_icons().and_then(TrayController::new) {
            Ok(tray) => (Some(tray), None),
            Err(error) => (None, Some(format!("System tray unavailable: {error:#}"))),
        };
        if background && tray_error.is_some() {
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        let player = match Player::new(&cc.egui_ctx, cc.gl.is_some(), cc.display_handle().ok()) {
            Ok(player) => Some(player),
            Err(error) => {
                let mut app = Self {
                    player: None,
                    player_error: Some(error.to_string()),
                    engine,
                    config: config.clone(),
                    default_inbox: default_inbox.clone(),
                    clips,
                    jobs,
                    cloud_clips: Vec::new(),
                    selected_id,
                    library_open: true,
                    user: None,
                    access_request: None,
                    show_auth: Some(AuthMode::Request),
                    show_access: false,
                    account_open: false,
                    admin_users: Vec::new(),
                    admin_requests: Vec::new(),
                    access_filter: AccessFilter::Pending,
                    access_query: String::new(),
                    created_reset: None,
                    pending_delete_job: None,
                    pending_delete_clip: None,
                    publish_modal: None,
                    export_modal: None,
                    editor: None,
                    thumbs: HashMap::new(),
                    notice: None,
                    notice_until: None,
                    error: startup_error.or(tray_error),
                    error_until: None,
                    busy: false,
                    last_refresh: Instant::now(),
                    tx,
                    rx,
                    auth_username: String::new(),
                    auth_display: String::new(),
                    auth_password: String::new(),
                    auth_confirm: String::new(),
                    reset_token: String::new(),
                    forgot_step: 0,
                    device_name: format!("{} desktop", std::env::consts::OS),
                    session_media: None,
                    timeline_drag: None,
                    timeline_settling: false,
                    time_edit: None,
                    drop_hovering: false,
                    available_update: None,
                    update_modal: None,
                    update_checking: false,
                    show_pending: false,
                    show_recorder: false,
                    recorder_config: config.recorder.clone(),
                    recorder_capabilities: recorder_capabilities.clone(),
                    recorder_status: recorder_status.clone(),
                    recorder_loaded: false,
                    recorder_loading: false,
                    recorder_application_selection: None,
                    recorder_playback_selection: None,
                    recorder_tab: RecorderTab::Video,
                    recorder_hotkey_listening: false,
                    last_recorder_refresh: Instant::now() - Duration::from_secs(10),
                    window: WindowPersistence::load(),
                    tray,
                    tray_recording: false,
                    allow_exit: false,
                    launch_at_login,
                };
                if app.error.is_some() {
                    app.error_until = Some(Instant::now() + Duration::from_secs(8));
                }
                app.sync_tray_recording();
                if background {
                    app.start_recorder();
                }
                app.schedule_update_check(false);
                app.import_startup_files();
                return app;
            }
        };
        let mut app = Self {
            engine,
            player,
            config: config.clone(),
            default_inbox,
            clips,
            jobs,
            cloud_clips: Vec::new(),
            selected_id,
            library_open: true,
            user: None,
            access_request: None,
            show_auth: None,
            show_access: false,
            account_open: false,
            admin_users: Vec::new(),
            admin_requests: Vec::new(),
            access_filter: AccessFilter::Pending,
            access_query: String::new(),
            created_reset: None,
            pending_delete_job: None,
            pending_delete_clip: None,
            publish_modal: None,
            export_modal: None,
            editor: None,
            thumbs: HashMap::new(),
            notice: None,
            notice_until: None,
            error: startup_error.or(tray_error),
            error_until: None,
            busy: false,
            last_refresh: Instant::now(),
            tx,
            rx,
            auth_username: String::new(),
            auth_display: String::new(),
            auth_password: String::new(),
            auth_confirm: String::new(),
            reset_token: String::new(),
            forgot_step: 0,
            player_error: None,
            device_name: format!("{} desktop", std::env::consts::OS),
            session_media: None,
            timeline_drag: None,
            timeline_settling: false,
            time_edit: None,
            drop_hovering: false,
            available_update: None,
            update_modal: None,
            update_checking: false,
            show_pending: false,
            show_recorder: false,
            recorder_config: config.recorder.clone(),
            recorder_capabilities,
            recorder_status,
            recorder_loaded: false,
            recorder_loading: false,
            recorder_application_selection: None,
            recorder_playback_selection: None,
            recorder_tab: RecorderTab::Video,
            recorder_hotkey_listening: false,
            last_recorder_refresh: Instant::now() - Duration::from_secs(10),
            window: WindowPersistence::load(),
            tray,
            tray_recording: false,
            allow_exit: false,
            launch_at_login,
        };
        if app.error.is_some() {
            app.error_until = Some(Instant::now() + Duration::from_secs(8));
        }
        app.sync_tray_recording();
        app.bootstrap_session();
        app.ensure_valid_selection();
        app.schedule_update_check(false);
        app.import_startup_files();
        if background {
            app.start_recorder();
        }
        app
    }

    fn bootstrap_session(&mut self) {
        let engine = self.engine.clone();
        let tx = self.tx.clone();
        if self.config.authenticated {
            self.engine.spawn(async move {
                match engine.me().await {
                    Ok(user) => {
                        let _ = tx.send(Message::User(user));
                        match engine.cloud_clips().await {
                            Ok(clips) => {
                                let _ = tx.send(Message::CloudClips(clips));
                            }
                            Err(error) => {
                                let _ = engine.logout().await;
                                let _ = tx.send(Message::LoggedOut);
                                let _ = tx.send(Message::Error(error.to_string()));
                            }
                        }
                    }
                    Err(_) => {
                        let _ = engine.logout().await;
                        let _ = tx.send(Message::LoggedOut);
                        let _ = tx.send(Message::Notice(
                            "Your saved login expired. Sign in again to publish.".into(),
                        ));
                    }
                }
            });
        } else if self.config.pending_access_request {
            let engine = self.engine.clone();
            let tx = self.tx.clone();
            self.engine.spawn(async move {
                match engine.access_request_status().await {
                    Ok(request) => {
                        let _ = tx.send(Message::AccessRequest(Some(request)));
                    }
                    Err(_) => {
                        let _ = engine.clear_access_request();
                        let _ = tx.send(Message::AccessRequest(None));
                    }
                }
            });
        } else {
            self.show_auth = Some(AuthMode::Request);
        }
    }

    fn schedule_update_check(&mut self, manual: bool) {
        if self.update_checking {
            return;
        }
        if std::env::var_os("CLIP_ENGINE_SKIP_UPDATES").is_some() {
            return;
        }
        self.update_checking = true;
        if manual {
            self.dismiss_error();
        }
        let engine = self.engine.clone();
        let tx = self.tx.clone();
        self.engine.spawn(async move {
            match engine.check_desktop_update(manual).await {
                Ok(update) => {
                    let _ = tx.send(Message::UpdateAvailable { update, manual });
                }
                Err(error) => {
                    if manual {
                        let _ = tx.send(Message::Error(format!("{error:#}")));
                    } else {
                        let _ = tx.send(Message::UpdateAvailable {
                            update: None,
                            manual: false,
                        });
                    }
                }
            }
        });
    }

    fn start_update_download(&mut self) {
        let Some(update) = self.available_update.clone() else {
            return;
        };
        self.update_modal = Some(UpdateModal::Downloading {
            received: 0,
            total: update.size.max(1),
        });
        let engine = self.engine.clone();
        let tx = self.tx.clone();
        self.engine.spawn(async move {
            let progress_tx = tx.clone();
            match engine
                .download_desktop_update(&update, move |received, total| {
                    let _ = progress_tx.send(Message::UpdateProgress { received, total });
                })
                .await
            {
                Ok(path) => {
                    let _ = tx.send(Message::UpdateDownloaded(path));
                }
                Err(error) => {
                    let _ = tx.send(Message::Error(format!("{error:#}")));
                }
            }
        });
    }

    fn set_notice(&mut self, value: impl Into<String>) {
        self.notice = Some(value.into());
        self.notice_until = Some(Instant::now() + Duration::from_secs(5));
    }

    fn dismiss_notice(&mut self) {
        self.notice = None;
        self.notice_until = None;
    }

    fn set_error(&mut self, value: impl Into<String>) {
        self.error = Some(value.into());
        self.error_until = Some(Instant::now() + Duration::from_secs(5));
    }

    fn dismiss_error(&mut self) {
        self.error = None;
        self.error_until = None;
    }

    fn expire_toasts(&mut self, ctx: &egui::Context) {
        if let Some(until) = self.error_until {
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.dismiss_error();
            } else {
                ctx.request_repaint_after(remaining);
            }
        }
        if let Some(until) = self.notice_until {
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.dismiss_notice();
            } else {
                ctx.request_repaint_after(remaining);
            }
        }
    }

    fn pump(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Error(value) => {
                    self.set_error(value);
                    self.busy = false;
                    self.recorder_loading = false;
                    self.update_checking = false;
                    if self.show_recorder {
                        let mut capabilities = self.engine.recorder_capabilities();
                        ensure_playback_device_labels(&mut capabilities);
                        self.recorder_capabilities = capabilities;
                        self.recorder_status = self.engine.recorder_status();
                    }
                    if matches!(self.export_modal, Some(ExportModal::Working { .. })) {
                        self.export_modal = None;
                    }
                    if matches!(self.update_modal, Some(UpdateModal::Downloading { .. })) {
                        self.update_modal = Some(UpdateModal::Prompt);
                    }
                }
                Message::Notice(value) => {
                    self.set_notice(value);
                    self.busy = false;
                }
                Message::Refresh => {
                    self.reload_library();
                    self.busy = false;
                }
                Message::Imported(ids) => {
                    self.reload_library();
                    self.busy = false;
                    if let Some(id) = ids.into_iter().next() {
                        self.selected_id = Some(id);
                        self.editor = None;
                        self.time_edit = None;
                    }
                }
                Message::User(user) => {
                    self.user = Some(user);
                    self.show_auth = None;
                    self.access_request = None;
                    self.show_pending = false;
                    self.config = self.engine.config().unwrap_or(self.config.clone());
                    self.busy = false;
                }
                Message::LoggedOut => {
                    self.user = None;
                    self.access_request = None;
                    self.show_pending = false;
                    self.cloud_clips.clear();
                    self.show_auth = Some(AuthMode::Login);
                    self.config = self.engine.config().unwrap_or(self.config.clone());
                    self.busy = false;
                }
                Message::CloudClips(clips) => self.cloud_clips = clips,
                Message::AccessRequest(request) => {
                    self.access_request = request;
                    if self.access_request.is_none() {
                        self.show_pending = false;
                        self.show_auth = Some(AuthMode::Request);
                    } else {
                        self.show_pending = true;
                    }
                    self.config = self.engine.config().unwrap_or(self.config.clone());
                    self.busy = false;
                }
                Message::Admin(users, requests) => {
                    self.admin_users = users;
                    self.admin_requests = requests;
                    self.show_access = true;
                    self.busy = false;
                }
                Message::PasswordReset(reset) => {
                    self.created_reset = Some(reset);
                    self.busy = false;
                }
                Message::Busy(busy) => self.busy = busy,
                Message::ExportProgress(progress) => {
                    if let Some(ExportModal::Working {
                        progress: current, ..
                    }) = &mut self.export_modal
                    {
                        *current = progress;
                    }
                }
                Message::ExportDone(path) => {
                    self.export_modal = Some(ExportModal::Done { path });
                    self.busy = false;
                }
                Message::UpdateAvailable { update, manual } => {
                    self.update_checking = false;
                    match update {
                        Some(update) => {
                            let snoozed = self
                                .engine
                                .snoozed_update_version()
                                .is_some_and(|version| version == update.version);
                            self.available_update = Some(update);
                            if manual || !snoozed {
                                self.update_modal = Some(UpdateModal::Prompt);
                            }
                        }
                        None => {
                            self.available_update = None;
                            if manual {
                                self.set_notice(format!(
                                    "{} {} is current.",
                                    APP_NAME,
                                    Engine::current_version()
                                ));
                            }
                        }
                    }
                }
                Message::UpdateProgress { received, total } => {
                    if let Some(UpdateModal::Downloading { .. }) = &mut self.update_modal {
                        self.update_modal = Some(UpdateModal::Downloading { received, total });
                    }
                }
                Message::UpdateDownloaded(path) => {
                    if let Some(update) = self.available_update.clone() {
                        self.update_modal = Some(UpdateModal::Installing);
                        if let Some(player) = &self.player {
                            player.pause();
                        }
                        self.engine.shutdown_recorder();
                        match install_desktop_update(&path, update.package) {
                            Ok(()) => std::process::exit(0),
                            Err(error) => {
                                self.update_modal = Some(UpdateModal::Prompt);
                                self.set_error(error.to_string());
                            }
                        }
                    }
                }
                Message::RecorderRefreshed {
                    mut capabilities,
                    status,
                } => {
                    ensure_playback_device_labels(&mut capabilities);
                    self.recorder_config = capabilities.normalize_config(&self.recorder_config);
                    self.recorder_capabilities = capabilities;
                    ensure_default_audio_routes(
                        &mut self.recorder_config,
                        &self.recorder_capabilities.audio_sources,
                    );
                    ensure_audio_route_names(
                        &mut self.recorder_config,
                        &self.recorder_capabilities.audio_sources,
                    );
                    self.recorder_status = *status;
                    self.recorder_loaded = true;
                    self.recorder_loading = false;
                    self.busy = false;
                }
                Message::RecorderImported(ids) => {
                    self.reload_library();
                    self.busy = false;
                    let count = ids.len();
                    if let Some(id) = ids.into_iter().next() {
                        self.selected_id = Some(id);
                        self.editor = None;
                        self.time_edit = None;
                    }
                    if count == 1 {
                        self.set_notice("Replay saved and added to your library.");
                    } else if count > 1 {
                        self.set_notice(format!(
                            "{count} replays saved and added to your library."
                        ));
                    }
                }
            }
        }
    }

    fn reload_library(&mut self) {
        if let Ok(clips) = self.engine.clips() {
            self.clips = clips;
        }
        if let Ok(jobs) = self.engine.jobs() {
            self.jobs = jobs;
        }
        if let Ok(config) = self.engine.config() {
            self.config = config;
        }
        let pending = self
            .clips
            .iter()
            .filter(|clip| clip.preview_status == "pending")
            .map(|clip| clip.id.clone())
            .collect::<Vec<_>>();
        for id in pending {
            if self
                .clips
                .iter()
                .any(|clip| clip.id == id && Path::new(&clip.source_path).is_file())
            {
                let _ = self.engine.prepare_preview(&id, false);
            }
        }
        self.ensure_valid_selection();
    }

    fn run_async<F>(&mut self, future: F)
    where
        F: std::future::Future<Output = Result<Message, anyhow::Error>> + Send + 'static,
    {
        self.busy = true;
        self.dismiss_error();
        let tx = self.tx.clone();
        self.engine.spawn(async move {
            let _ = tx.send(Message::Busy(true));
            match future.await {
                Ok(message) => {
                    let _ = tx.send(message);
                }
                Err(error) => {
                    let _ = tx.send(Message::Error(format!("{error:#}")));
                }
            }
        });
    }

    fn inbox_clips(&self) -> Vec<&Clip> {
        let inbox = PathBuf::from(&self.config.source_directory);
        self.clips
            .iter()
            .filter(|clip| {
                clip_belongs_in_inbox(
                    Path::new(&clip.source_path),
                    &inbox,
                    self.default_inbox.as_deref(),
                )
            })
            .collect()
    }

    fn visible_clips(&self) -> Vec<&Clip> {
        self.inbox_clips()
    }

    fn ensure_valid_selection(&mut self) {
        let visible = self
            .visible_clips()
            .into_iter()
            .map(|clip| clip.id.clone())
            .collect::<Vec<_>>();
        if self
            .selected_id
            .as_ref()
            .is_some_and(|id| visible.iter().any(|clip_id| clip_id == id))
        {
            return;
        }
        self.selected_id = visible.first().cloned();
        self.editor = None;
        self.time_edit = None;
        self.bind_session_media(None);
    }
}

impl ClipApp {
    fn poll_tray(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let actions = self
            .tray
            .as_ref()
            .map(TrayController::poll)
            .unwrap_or_default();
        for action in actions {
            match action {
                TrayAction::Show => self.show_window(ctx, frame),
                TrayAction::StartRecorder if !self.busy => self.start_recorder(),
                TrayAction::StopRecorder if !self.busy => self.stop_recorder(),
                TrayAction::SaveReplay if !self.busy => self.save_recorder_replay(),
                TrayAction::Quit => {
                    self.allow_exit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                _ => {}
            }
        }

        if self.tray.is_some()
            && ctx.input(|input| input.viewport().close_requested())
            && !self.allow_exit
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(ctx, frame);
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn show_window(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(window) = frame.winit_window() {
            window.set_minimized(false);
            window.set_visible(true);
            window.focus_window();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    }

    fn hide_window(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Some(window) = frame.winit_window() {
            window.set_visible(false);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn sync_tray_recording(&mut self) {
        let recording = self.recorder_status.replay_active;
        if recording == self.tray_recording {
            return;
        }
        if let Some(tray) = &self.tray {
            tray.set_recording(recording);
        }
        self.tray_recording = recording;
    }
}

impl eframe::App for ClipApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.timeline_settling = false;
        if let Some(window) = frame.winit_window() {
            if self.window.observe(window) {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
        self.pump();
        self.poll_tray(ctx, frame);
        self.poll_recorder();
        self.sync_tray_recording();
        self.expire_toasts(ctx);
        let previewing = self
            .clips
            .iter()
            .any(|clip| matches!(clip.preview_status.as_str(), "pending" | "processing"));
        let processing = previewing
            || self
                .jobs
                .iter()
                .any(|job| matches!(job.status.as_str(), "queued" | "transcoding" | "uploading"));
        let refresh_every = if self
            .jobs
            .iter()
            .any(|job| matches!(job.status.as_str(), "queued" | "transcoding" | "uploading"))
        {
            Duration::from_millis(200)
        } else {
            Duration::from_millis(900)
        };
        if processing && self.last_refresh.elapsed() > refresh_every {
            self.reload_library();
            self.last_refresh = Instant::now();
        }
        if processing {
            ctx.request_repaint();
        }
        if self.recorder_loading {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        self.stop_at_out_point();
        if self.timeline_drag.is_none()
            && self.player.as_ref().is_some_and(|player| {
                player.playing() || player.buffering() || player.wants_redraw()
            })
            || matches!(self.export_modal, Some(ExportModal::Working { .. }))
            || matches!(
                self.update_modal,
                Some(UpdateModal::Downloading { .. } | UpdateModal::Installing)
            )
            || self.update_checking
        {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        self.engine.shutdown_recorder();
        self.window.flush();
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ingest_dropped_files(&ctx);
        if self.drop_hovering {
            ctx.request_repaint();
        }

        egui::Panel::top("topbar")
            .frame(theme::top_frame())
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(36.0);
                    if theme::library_menu_button(ui, self.library_open).clicked() {
                        self.library_open = !self.library_open;
                    }
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(PRODUCT_NAME)
                                .family(theme::medium())
                                .size(16.0),
                        );
                        ui.label(
                            RichText::new(format!("v{}", Engine::current_version()))
                                .color(theme::MUTED)
                                .size(11.5),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let label = if self.config.authenticated {
                            self.user
                                .as_ref()
                                .map(|user| user.display_name.clone())
                                .unwrap_or_else(|| "Connected".into())
                        } else if self
                            .access_request
                            .as_ref()
                            .is_some_and(|request| request.status == "pending")
                        {
                            "Approval pending".into()
                        } else {
                            "Sign in to publish".into()
                        };
                        if ui.button(label).clicked() {
                            if self.config.authenticated {
                                self.account_open = !self.account_open;
                            } else if self.access_request.is_some() {
                                self.show_pending = true;
                            } else {
                                self.show_auth = Some(AuthMode::Login);
                            }
                        }
                        if self.user.as_ref().is_some_and(|user| user.role == "owner")
                            && ui.button("Manage access").clicked()
                        {
                            let engine = self.engine.clone();
                            self.run_async(async move {
                                Ok(Message::Admin(
                                    engine.admin_users().await?,
                                    engine.admin_access_requests().await?,
                                ))
                            });
                        }
                        if ui
                            .button("Recorder")
                            .on_hover_text("Configure and control the libobs replay recorder")
                            .clicked()
                        {
                            self.show_recorder = !self.show_recorder;
                            if self.show_recorder {
                                self.refresh_recorder();
                            } else {
                                self.recorder_hotkey_listening = false;
                            }
                        }
                    });
                });
                if self.account_open {
                    if let Some(user) = self.user.clone() {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("@{}", user.username)).color(theme::MUTED),
                            );
                            if ui.button("Sign out this device").clicked() {
                                self.sign_out_this_device();
                            }
                            let update_label = if self.available_update.is_some() {
                                "Update available"
                            } else if self.update_checking {
                                "Checking…"
                            } else {
                                "Check for updates"
                            };
                            if ui
                                .add_enabled(!self.update_checking, egui::Button::new(update_label))
                                .clicked()
                            {
                                if self.available_update.is_some() {
                                    self.update_modal = Some(UpdateModal::Prompt);
                                } else {
                                    self.schedule_update_check(true);
                                }
                            }
                        });
                    }
                }
            });

        if self.library_open {
            let library_width = (ctx.content_rect().width() * 0.24).clamp(280.0, 400.0);
            egui::Panel::left("library")
                .resizable(true)
                .default_size(library_width)
                .min_size(240.0)
                .max_size(480.0)
                .frame(theme::side_frame())
                .show(ui, |ui| {
                    self.library_panel(ui);
                });
        }

        egui::CentralPanel::default()
            .frame(theme::central_frame())
            .show(ui, |ui| {
                if self.show_recorder {
                    self.recorder_panel(ui);
                } else {
                    self.status_banner(ui);
                    if let Some(clip_id) = self.selected_id.clone() {
                        if let Some(index) = self.clips.iter().position(|clip| clip.id == clip_id) {
                            let clip = self.clips[index].clone();
                            self.editor_panel(ui, &ctx, &clip);
                        }
                    } else {
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                RichText::new("Upload some clips to get started")
                                    .family(theme::medium())
                                    .size(18.0)
                                    .color(theme::MUTED),
                            );
                        });
                    }
                }
            });

        if self.show_auth.is_some() {
            self.auth_modal(&ctx);
        }
        if self.show_pending
            && self.access_request.is_some()
            && !self.config.authenticated
            && self.show_auth.is_none()
        {
            self.pending_modal(&ctx);
        }
        if self.show_access {
            self.access_modal(&ctx);
        }
        if self.created_reset.is_some() {
            self.reset_modal(&ctx);
        }
        if self.pending_delete_job.is_some() {
            self.delete_version_modal(&ctx);
        }
        if self.pending_delete_clip.is_some() {
            self.delete_clip_modal(&ctx);
        }
        if self.publish_modal.is_some() {
            self.publish_flow_modal(&ctx);
        }
        if self.export_modal.is_some() {
            self.export_flow_modal(&ctx);
        }
        if self.update_modal.is_some() {
            self.update_flow_modal(&ctx);
        }

        if self.drop_hovering {
            theme::window_drop_overlay(&ctx);
        }
    }
}

impl ClipApp {
    fn refresh_recorder(&mut self) {
        self.recorder_loading = true;
        if self.busy {
            return;
        }
        let engine = self.engine.clone();
        self.run_async(async move {
            let (capabilities, status) = engine.refresh_recorder()?;
            Ok(Message::RecorderRefreshed {
                capabilities,
                status: Box::new(status),
            })
        });
    }

    fn apply_recorder_config(&mut self) {
        self.recorder_config = self
            .recorder_capabilities
            .normalize_config(&self.recorder_config);
        let engine = self.engine.clone();
        let config = self.recorder_config.clone();
        self.run_async(async move {
            engine.apply_recorder_config(config)?;
            let (capabilities, status) = engine.refresh_recorder()?;
            Ok(Message::RecorderRefreshed {
                capabilities,
                status: Box::new(status),
            })
        });
    }

    fn start_recorder(&mut self) {
        self.recorder_config = self
            .recorder_capabilities
            .normalize_config(&self.recorder_config);
        let engine = self.engine.clone();
        let config = self.recorder_config.clone();
        self.run_async(async move {
            engine.apply_recorder_config(config)?;
            engine.start_recorder()?;
            let (capabilities, status) = engine.refresh_recorder()?;
            Ok(Message::RecorderRefreshed {
                capabilities,
                status: Box::new(status),
            })
        });
    }

    fn stop_recorder(&mut self) {
        let engine = self.engine.clone();
        self.run_async(async move {
            engine.stop_recorder()?;
            let (capabilities, status) = engine.refresh_recorder()?;
            Ok(Message::RecorderRefreshed {
                capabilities,
                status: Box::new(status),
            })
        });
    }

    fn save_recorder_replay(&mut self) {
        let engine = self.engine.clone();
        self.run_async(async move {
            engine.save_recorder_replay()?;
            let clips = engine.import_recorder_replays().await?;
            Ok(Message::RecorderImported(
                clips.into_iter().map(|clip| clip.id).collect(),
            ))
        });
    }

    fn poll_recorder(&mut self) {
        if self.last_recorder_refresh.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_recorder_refresh = Instant::now();
        let engine = self.engine.clone();
        let tx = self.tx.clone();
        let refresh = (self.show_recorder || self.recorder_status.replay_active) && !self.busy;
        self.engine.spawn(async move {
            if refresh {
                if let Ok((capabilities, status)) = engine.refresh_recorder() {
                    let _ = tx.send(Message::RecorderRefreshed {
                        capabilities,
                        status: Box::new(status),
                    });
                }
            }
            if let Ok(clips) = engine.import_recorder_replays().await {
                if !clips.is_empty() {
                    let _ = tx.send(Message::RecorderImported(
                        clips.into_iter().map(|clip| clip.id).collect(),
                    ));
                }
            }
        });
    }

    fn capture_recorder_hotkey(&mut self, ctx: &egui::Context) {
        if !self.recorder_hotkey_listening {
            return;
        }
        let captured = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                let egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                } = event
                else {
                    return None;
                };
                if *key == egui::Key::Escape || *physical_key == Some(egui::Key::Escape) {
                    return Some(None);
                }
                recorder_hotkey_from_event(*key, *physical_key, *modifiers).map(Some)
            })
        });
        let Some(captured) = captured else {
            return;
        };
        let Some(hotkey) = captured else {
            self.recorder_hotkey_listening = false;
            return;
        };
        if self.recorder_config.hotkey.is_some() {
            let label = hotkey.to_string();
            self.recorder_config.hotkey = Some(hotkey);
            self.recorder_hotkey_listening = false;
            self.set_notice(format!(
                "Save key changed to {label}. Save settings to apply it."
            ));
        } else {
            self.recorder_hotkey_listening = false;
        }
    }

    fn recorder_panel(&mut self, ui: &mut Ui) {
        self.capture_recorder_hotkey(ui.ctx());
        if self.recorder_loading && !self.recorder_loaded {
            ui.heading(
                RichText::new("Replay recorder")
                    .family(theme::medium())
                    .color(theme::TEXT),
            );
            ui.label(
                RichText::new(
                    "Set this up once, then press your save key whenever something worth keeping happens. The editor stays separate from the recorder helper.",
                )
                .color(theme::MUTED)
                .size(12.0),
            );
            ui.add_space(24.0);
            theme::card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add_space(18.0);
                    ui.add(egui::Spinner::new().size(28.0));
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("Preparing replay recorder…")
                            .family(theme::medium())
                            .color(theme::TEXT),
                    );
                    ui.label(
                        RichText::new("Detecting displays, audio devices, and available encoders.")
                            .color(theme::MUTED)
                            .size(12.0),
                    );
                    ui.add_space(18.0);
                });
            });
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.heading(
                    RichText::new("Replay recorder")
                        .family(theme::medium())
                        .color(theme::TEXT),
                );
                ui.label(
                    RichText::new(
                        "Set this up once, then press your save key whenever something worth keeping happens. The editor stays separate from the recorder helper.",
                    )
                    .color(theme::MUTED)
                    .size(12.0),
                );
                ui.add_space(10.0);

                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let (status_label, status_color) = match self.recorder_status.state {
                        RecorderState::Running => ("Running", theme::OK),
                        RecorderState::Starting => ("Starting", theme::ACCENT),
                        RecorderState::Stopping => ("Stopping", theme::ACCENT),
                        RecorderState::Error => ("Needs attention", theme::DANGER),
                        RecorderState::Stopped => ("Stopped", theme::MUTED),
                    };
                    let refresh_label = if self.recorder_loading {
                        "Refreshing…"
                    } else {
                        "Refresh"
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Recorder status")
                                .family(theme::medium())
                                .color(theme::MUTED),
                        );
                        ui.label(
                            RichText::new(format!("● {status_label}"))
                                .family(theme::medium())
                                .color(status_color),
                        );
                        if self.recorder_status.replay_active {
                            ui.label(RichText::new("replay buffer active").color(theme::OK));
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    !self.busy,
                                    egui::Button::new(
                                        RichText::new(refresh_label).color(theme::TEXT),
                                    )
                                    .fill(theme::PANEL),
                                )
                                .clicked()
                            {
                                self.refresh_recorder();
                            }
                        });
                    });
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Capture actions")
                                .family(theme::medium())
                                .color(theme::MUTED),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if self.recorder_status.replay_active {
                                if ui
                                    .add_enabled(
                                        !self.busy,
                                        egui::Button::new(
                                            RichText::new("Save last replay")
                                                .family(theme::medium())
                                                .color(theme::INK),
                                        )
                                        .fill(theme::ACCENT),
                                    )
                                    .clicked()
                                {
                                    self.save_recorder_replay();
                                }
                                if ui
                                    .add_enabled(
                                        !self.busy,
                                        egui::Button::new(
                                            RichText::new("Stop recording").color(theme::TEXT),
                                        )
                                        .fill(theme::PANEL),
                                    )
                                    .clicked()
                                {
                                    self.stop_recorder();
                                }
                            } else if ui
                                .add_enabled(
                                    !self.busy,
                                    egui::Button::new(
                                        RichText::new("Start recording")
                                            .family(theme::medium())
                                            .color(theme::INK),
                                    )
                                    .fill(theme::ACCENT),
                                )
                                .clicked()
                            {
                                self.start_recorder();
                            }
                        });
                    });
                    if let Some(error) = &self.recorder_status.last_error {
                        ui.label(RichText::new(error).color(theme::DANGER).size(12.0));
                    }
                    if let Some(path) = &self.recorder_status.last_replay_path {
                        let name = Path::new(path)
                            .file_name()
                            .and_then(OsStr::to_str)
                            .unwrap_or(path);
                        ui.label(
                            RichText::new(format!("Last saved clip: {name}"))
                                .color(theme::OK)
                                .size(12.0),
                        );
                    }
                    if let Some(error) = &self.recorder_status.hotkey_error {
                        ui.label(
                            RichText::new(format!("Global hotkey: {error}"))
                                .color(theme::DANGER)
                                .size(12.0),
                        );
                    } else if self.recorder_status.hotkey_registered {
                        ui.label(
                            RichText::new(format!(
                                "Global save hotkey registered: {}",
                                self.recorder_config
                                    .hotkey
                                    .as_ref()
                                    .map(ToString::to_string)
                                    .unwrap_or_else(|| "disabled".into())
                            ))
                            .color(theme::OK)
                            .size(12.0),
                        );
                    }
                });

                if !self.recorder_capabilities.diagnostics.is_empty() {
                    ui.add_space(10.0);
                    theme::card().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let backend = match self.recorder_capabilities.backend {
                            CaptureBackend::WindowsGraphicsCapture => "Windows capture",
                            CaptureBackend::X11 => "X11",
                            CaptureBackend::PipeWire => "PipeWire / Wayland",
                            CaptureBackend::Unknown => "Unavailable",
                        };
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Capture backend")
                                    .family(theme::medium())
                                    .color(theme::MUTED),
                            );
                            ui.label(RichText::new(backend).color(theme::TEXT));
                        });
                        ui.collapsing(
                            RichText::new("Backend diagnostics").family(theme::medium()),
                            |ui| {
                                for diagnostic in &self.recorder_capabilities.diagnostics {
                                    ui.label(
                                        RichText::new(diagnostic).color(theme::MUTED).size(12.0),
                                    );
                                }
                            },
                        );
                    });
                }

                ui.add_space(10.0);
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("General setup").family(theme::medium()));
                    ui.label(
                        RichText::new(
                            "Pick a display, decide how much replay history to keep, and leave the rest on automatic.",
                        )
                        .color(theme::MUTED)
                        .size(12.0),
                    );
                    ui.add_space(4.0);
                    let screens = self.recorder_capabilities.screens.clone();
                    ui.horizontal(|ui| {
                        ui.label("Display");
                        let selected = screens
                            .iter()
                            .find(|screen| screen.id == self.recorder_config.screen_id)
                            .map(|screen| screen.label.clone())
                            .unwrap_or_else(|| {
                                if screens.is_empty() {
                                    if self.recorder_capabilities.backend == CaptureBackend::Unknown
                                        && !self.recorder_capabilities.diagnostics.is_empty()
                                    {
                                        "Unavailable — see diagnostics".into()
                                    } else {
                                        "No screen reported".into()
                                    }
                                } else {
                                    "Choose a screen".into()
                                }
                            });
                        egui::ComboBox::from_id_salt("recorder-screen")
                            .selected_text(selected)
                            .show_ui(ui, |ui| {
                                for screen in &screens {
                                    ui.selectable_value(
                                        &mut self.recorder_config.screen_id,
                                        screen.id.clone(),
                                        format!(
                                            "{} ({}×{}{})",
                                            screen.label,
                                            screen.width,
                                            screen.height,
                                            screen
                                                .refresh_hz
                                                .map(|hz| format!(", {:.0} Hz", hz))
                                                .unwrap_or_default()
                                        ),
                                    );
                                }
                            });
                    });
                    let selected_screen = screens
                        .iter()
                        .find(|screen| screen.id == self.recorder_config.screen_id);
                    ui.horizontal(|ui| {
                        ui.checkbox(
                            &mut self.recorder_config.match_display,
                            "Use display resolution",
                        );
                        if self.recorder_config.match_display {
                            if let Some(screen) = selected_screen {
                                ui.label(
                                    RichText::new(format!(
                                        "{}×{}",
                                        screen.width, screen.height
                                    ))
                                    .color(theme::MUTED)
                                    .size(12.0),
                                );
                            }
                        } else {
                            ui.label("Custom size");
                            ui.add(
                                egui::DragValue::new(&mut self.recorder_config.output_width)
                                    .range(320..=16_384)
                                    .prefix("W "),
                            );
                            ui.add(
                                egui::DragValue::new(&mut self.recorder_config.output_height)
                                    .range(180..=16_384)
                                    .prefix("H "),
                            );
                        }
                    });
                    let mut fps = self.recorder_config.fps.as_f64();
                    ui.horizontal(|ui| {
                        ui.label("Frame rate");
                        ui.checkbox(
                            &mut self.recorder_config.match_display_fps,
                            "Match display refresh rate",
                        );
                        if !self.recorder_config.match_display_fps {
                            ui.add(
                                egui::DragValue::new(&mut fps)
                                    .range(1.0..=1_000.0)
                                    .speed(1.0)
                                    .suffix(" fps"),
                            );
                        } else if let Some(refresh_hz) =
                            selected_screen.and_then(|screen| screen.refresh_hz)
                        {
                            ui.label(
                                RichText::new(format!("display: {:.2} Hz", refresh_hz.min(240.0)))
                                    .color(theme::MUTED)
                                    .size(12.0),
                            );
                        }
                        let reported = self
                            .recorder_capabilities
                            .frame_rates
                            .iter()
                            .map(|range| {
                                format!("{:.0}–{:.0}", range.min.as_f64(), range.max.as_f64())
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        ui.label(
                            RichText::new(if reported.is_empty() {
                                "Refresh to discover supported rates".into()
                            } else {
                                format!("supported: {reported}")
                            })
                            .color(theme::MUTED)
                            .size(12.0),
                        );
                    });
                    self.recorder_config.fps = rational_from_decimal(fps);
                    ui.horizontal(|ui| {
                        ui.label("Replay history");
                        ui.add(
                            egui::DragValue::new(&mut self.recorder_config.replay_seconds)
                                .range(1..=3_600)
                                .suffix(" seconds"),
                        );
                    });
                    ui.add_space(10.0);
                    ui.separator();
                    theme::section_title(ui, "Save key");
                    theme::helper_text(
                        ui,
                        "Press this shortcut at any time to save the last part of your replay.",
                    );
                    let mut enabled = self.recorder_config.hotkey.is_some();
                    if ui
                        .checkbox(&mut enabled, "Enable global save key")
                        .changed()
                    {
                        self.recorder_config.hotkey = enabled.then_some(Default::default());
                        if !enabled {
                            self.recorder_hotkey_listening = false;
                        }
                    }
                    let hotkey_label = self
                        .recorder_config
                        .hotkey
                        .as_ref()
                        .map(ToString::to_string);
                    let listening = self.recorder_hotkey_listening;
                    let mut listen_clicked = false;
                    let mut cancel_clicked = false;
                    if self.recorder_config.hotkey.is_some() {
                        ui.horizontal(|ui| {
                            ui.label("Shortcut");
                            ui.label(
                                RichText::new(hotkey_label.as_deref().unwrap_or_default())
                                    .family(theme::medium())
                                    .color(theme::TEXT),
                            );
                            let listen_label = if listening {
                                "Listening…"
                            } else {
                                "Listen"
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(listen_label).color(theme::TEXT),
                                    )
                                    .fill(if listening {
                                        theme::ACCENT
                                    } else {
                                        theme::PANEL
                                    }),
                                )
                                .clicked()
                            {
                                listen_clicked = true;
                            }
                            if listening && ui.button("Cancel").clicked() {
                                cancel_clicked = true;
                            }
                        });
                        if listening {
                            theme::helper_text(
                                ui,
                                "Press the key or key combination you want to use. Escape cancels listening.",
                            );
                        }
                    }
                    if listen_clicked {
                        self.recorder_hotkey_listening = !self.recorder_hotkey_listening;
                    }
                    if cancel_clicked {
                        self.recorder_hotkey_listening = false;
                    }
                    theme::helper_text(
                        ui,
                        "F8 is the default. Use Listen to capture a key or command, then save settings to apply it. Windows and X11 support global shortcuts; Wayland compositors may limit them.",
                    );
                    ui.checkbox(
                        &mut self.recorder_config.notify_on_save,
                        "Desktop notification when a clip is saved",
                    );
                    theme::helper_text(
                        ui,
                        "Plays a short system sound and shows a desktop notification for successful saves and failures, including when the buffer is not running. Save settings to apply.",
                    );
                });

                ui.add_space(10.0);
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let mut enabled = self.launch_at_login;
                    if ui
                        .checkbox(&mut enabled, "Launch Clip Engine at login")
                        .changed()
                    {
                        let result = startup::set_enabled(enabled).and_then(|()| {
                            self.engine.database.put_setting(
                                startup::LAUNCH_AT_LOGIN_SETTING,
                                if enabled { "true" } else { "false" },
                            )
                        });
                        match result {
                            Ok(()) => {
                                self.launch_at_login = enabled;
                                self.set_notice(if enabled {
                                    "Clip Engine will start hidden in the tray at login."
                                } else {
                                    "Clip Engine will not start automatically at login."
                                });
                            }
                            Err(error) => {
                                self.set_error(format!("Could not update launch at login: {error:#}"));
                            }
                        }
                    }
                    theme::helper_text(
                        ui,
                        "When enabled, Clip Engine starts hidden and begins the replay buffer automatically.",
                    );
                });

                // Keep default system and microphone routes ready even before the
                // user opens the Audio tab.
                ensure_default_audio_routes(
                    &mut self.recorder_config,
                    &self.recorder_capabilities.audio_sources,
                );
                ensure_audio_route_names(
                    &mut self.recorder_config,
                    &self.recorder_capabilities.audio_sources,
                );

                ui.add_space(10.0);
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let tab_width = ((ui.available_width() - 6.0) / 2.0).max(120.0);
                    theme::section_title(ui, "Recorder settings");
                    theme::helper_text(ui, match self.recorder_tab {
                        RecorderTab::Video => {
                            "Choose the picture quality and format for your replay clips."
                        }
                        RecorderTab::Audio => {
                            "Choose which sounds are included and keep them on separate tracks."
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if theme::tab(
                            ui,
                            self.recorder_tab == RecorderTab::Video,
                            "Video",
                            tab_width,
                        ) {
                            self.recorder_tab = RecorderTab::Video;
                        }
                        if theme::tab(
                            ui,
                            self.recorder_tab == RecorderTab::Audio,
                            "Audio",
                            tab_width,
                        ) {
                            self.recorder_tab = RecorderTab::Audio;
                        }
                    });
                    ui.add_space(2.0);
                });

                if self.recorder_tab == RecorderTab::Video {
                ui.add_space(4.0);
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    theme::section_title(ui, "Video quality");
                    theme::helper_text(
                        ui,
                        "Automatic is recommended for most players. Choose Advanced only if you need to control a specific encoder.",
                    );
                    ui.horizontal(|ui| {
                        ui.label("Mode");
                        if ui
                            .selectable_label(
                                self.recorder_config.mode == RecorderMode::Automatic,
                                "Automatic",
                            )
                            .clicked()
                        {
                            self.recorder_config.mode = RecorderMode::Automatic;
                        }
                        ui.selectable_value(
                            &mut self.recorder_config.mode,
                            RecorderMode::Advanced,
                            "Advanced",
                        );
                    });
                    if self.recorder_config.mode == RecorderMode::Advanced {
                        self.recorder_config = self
                            .recorder_capabilities
                            .normalize_config(&self.recorder_config);
                    }
                    if self.recorder_config.mode == RecorderMode::Automatic {
                        ui.label(
                            RichText::new(
                                "Clip Engine picks the best available hardware encoder, uses display-native settings, and keeps MKV as the safe default.",
                            )
                            .color(theme::MUTED)
                            .size(12.0),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Reset automatic defaults").clicked() {
                                reset_automatic_encoding(&mut self.recorder_config);
                            }
                            if let Some(effective) = &self.recorder_status.effective_settings {
                                ui.label(
                                    RichText::new(format!(
                                        "Active: {} · {}×{} · {:.2} fps · {}",
                                        effective.video_encoder,
                                        effective.output_width,
                                        effective.output_height,
                                        effective.fps.as_f64(),
                                        effective.rate_control
                                    ))
                                    .color(theme::OK)
                                    .size(12.0),
                                );
                            }
                        });
                    }
                    if self.recorder_config.mode == RecorderMode::Advanced {
                        ui.add_space(6.0);
                        advanced_encoder_settings(
                            ui,
                            &self.recorder_capabilities,
                            &mut self.recorder_config,
                        );
                    }
                    if let Some(effective) = &self.recorder_status.effective_settings {
                        if !effective.diagnostics.is_empty() {
                            let count = effective.diagnostics.len();
                            ui.collapsing(
                                format!(
                                    "Encoder notes · {count} setting{}",
                                    if count == 1 { "" } else { "s" }
                                ),
                                |ui| {
                                    for diagnostic in &effective.diagnostics {
                                        ui.label(
                                            RichText::new(diagnostic)
                                                .color(theme::MUTED)
                                                .size(11.0),
                                        );
                                    }
                                },
                            );
                        }
                    }
                });
                }

                ui.add_space(4.0);
                if self.recorder_tab == RecorderTab::Audio {
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let (audio_label_width, audio_control_width) = settings_column_widths(ui);
                    let audio_track_width = 110.0;
                    ui.label(RichText::new("Audio").family(theme::medium()));
                    ui.add_space(6.0);
                    theme::section_title(ui, "Audio quality");
                    audio_quality_settings(
                        ui,
                        &self.recorder_capabilities,
                        &mut self.recorder_config,
                    );
                    ui.separator();
                    let isolation_available =
                        self.recorder_capabilities.audio_isolation_available;
                    let mut exclude_application_audio = self.recorder_config.system_audio_mode
                        == SystemAudioMode::ExcludeApplications;
                    let can_change_isolation = isolation_available || exclude_application_audio;
                    let mut isolation_changed = false;
                    egui::Grid::new("recorder-system-audio-settings")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            encoder_settings_label(ui, "System audio mix", audio_label_width);
                            encoder_settings_control_area(ui, |ui| {
                                isolation_changed = ui
                                    .add_enabled(
                                        can_change_isolation,
                                        egui::Checkbox::new(
                                            &mut exclude_application_audio,
                                            "Exclude enabled application tracks from System audio",
                                        ),
                                    )
                                    .changed();
                            });
                            ui.end_row();
                        });
                    if isolation_changed {
                        self.recorder_config.system_audio_mode = if exclude_application_audio {
                            SystemAudioMode::ExcludeApplications
                        } else {
                            SystemAudioMode::Mixed
                        };
                    }
                    ui.add_space(6.0);
                    let audio_sources = self.recorder_capabilities.audio_sources.clone();
                    let default_sources = audio_sources
                        .iter()
                        .filter(|source| {
                            matches!(
                                source.kind,
                                AudioSourceKind::System | AudioSourceKind::Microphone
                            )
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    let playback_sources = audio_sources
                        .iter()
                        .filter(|source| source.kind == AudioSourceKind::PlaybackDevice)
                        .cloned()
                        .collect::<Vec<_>>();
                    let application_sources = audio_sources
                        .iter()
                        .filter(|source| source.kind == AudioSourceKind::Application)
                        .cloned()
                        .collect::<Vec<_>>();
                    if audio_sources.is_empty() {
                        ui.label(
                            RichText::new(
                                "No audio sources were reported. Refresh after starting \
                                 PipeWire/PulseAudio or enabling WASAPI sources.",
                            )
                            .color(theme::MUTED)
                            .size(12.0),
                        );
                    }
                    ui.add_space(8.0);
                    theme::section_title(ui, "Audio tracks");
                    for (source_index, source) in default_sources.iter().enumerate() {
                        let Some(index) = self
                            .recorder_config
                            .audio_routes
                            .iter()
                            .position(|route| route.source_id == source.id)
                        else {
                            continue;
                        };
                        let route = &mut self.recorder_config.audio_routes[index];
                        if source_index > 0 {
                            ui.separator();
                        }
                        theme::section_title(ui, &source.label);
                        egui::Grid::new(("recorder-default-source-settings", source.id.as_str()))
                            .num_columns(2)
                            .spacing([12.0, 8.0])
                            .show(ui, |ui| {
                                encoder_settings_label(ui, "Enabled", audio_label_width);
                                encoder_settings_control_area(ui, |ui| {
                                    ui.checkbox(&mut route.enabled, "");
                                });
                                ui.end_row();
                                encoder_settings_label(ui, "Track name", audio_label_width);
                                encoder_settings_control_area(ui, |ui| {
                                    ui.add_sized(
                                        [audio_control_width, 22.0],
                                        egui::TextEdit::singleline(&mut route.track_name),
                                    );
                                });
                                ui.end_row();
                                encoder_settings_label(ui, "Audio track", audio_label_width);
                                encoder_settings_control_area(ui, |ui| {
                                    audio_track_selector(
                                        ui,
                                        ("recorder-default-audio-track", source.id.as_str()),
                                        &mut route.track,
                                        audio_track_width,
                                    );
                                });
                                ui.end_row();
                            });
                    }
                    if default_sources.is_empty() && !audio_sources.is_empty() {
                        ui.label(
                            RichText::new("No default system or microphone sources are available.")
                            .color(theme::MUTED)
                            .size(11.0),
                        );
                    }

                    let available_playback_sources = playback_sources
                        .iter()
                        .filter(|source| source.available)
                        .collect::<Vec<_>>();
                    let selection_is_available = self
                        .recorder_playback_selection
                        .as_ref()
                        .is_some_and(|selection| {
                            available_playback_sources
                                .iter()
                                .any(|source| &source.id == selection)
                        });
                    if !selection_is_available {
                        self.recorder_playback_selection = None;
                    }
                    let mut playback_selection = self.recorder_playback_selection.clone();
                    let show_playback_devices = self.recorder_capabilities.backend
                        == CaptureBackend::WindowsGraphicsCapture
                        || !playback_sources.is_empty();
                    if show_playback_devices {
                        ui.separator();
                        ui.add_space(8.0);

                        let playback_routes = self
                            .recorder_config
                            .audio_routes
                            .iter()
                            .enumerate()
                            .filter(|(_, route)| route.source_id.starts_with("playback:"))
                            .map(|(index, _)| index)
                            .collect::<Vec<_>>();
                        let mut remove_playback_routes = Vec::new();
                        for index in playback_routes {
                            let mut remove = false;
                            let route = &mut self.recorder_config.audio_routes[index];
                            let selected_source = playback_sources
                                .iter()
                                .find(|source| source.id == route.source_id);
                            let route_heading = if route.track_name.trim().is_empty() {
                                "Playback device track".into()
                            } else {
                                route.track_name.clone()
                            };
                            ui.separator();
                            theme::section_title(ui, &route_heading);
                            egui::Grid::new(("recorder-playback-route-settings", index))
                                .num_columns(2)
                                .spacing([12.0, 8.0])
                                .show(ui, |ui| {
                                    encoder_settings_label(ui, "Device", audio_label_width);
                                    encoder_settings_control_area(ui, |ui| {
                                        let selected_text = selected_source
                                            .map(|source| source.label.clone())
                                            .unwrap_or_else(|| {
                                                "Unavailable playback device".into()
                                            });
                                        egui::ComboBox::from_id_salt((
                                            "recorder-playback-route",
                                            index,
                                        ))
                                        .width(audio_control_width)
                                        .selected_text(selected_text)
                                        .show_ui(ui, |ui| {
                                            for source in &available_playback_sources {
                                                ui.selectable_value(
                                                    &mut route.source_id,
                                                    source.id.clone(),
                                                    source.label.clone(),
                                                );
                                            }
                                        });
                                    });
                                    ui.end_row();
                                    encoder_settings_label(ui, "Track name", audio_label_width);
                                    encoder_settings_control_area(ui, |ui| {
                                        ui.add_sized(
                                            [audio_control_width, 22.0],
                                            egui::TextEdit::singleline(&mut route.track_name),
                                        );
                                    });
                                    ui.end_row();
                                    encoder_settings_label(ui, "Audio track", audio_label_width);
                                    encoder_settings_control_area(ui, |ui| {
                                        audio_track_selector(
                                            ui,
                                            ("recorder-playback-track", index),
                                            &mut route.track,
                                            audio_track_width,
                                        );
                                    });
                                    ui.end_row();
                                    encoder_settings_label(ui, "Enabled", audio_label_width);
                                    encoder_settings_control_area(ui, |ui| {
                                        ui.checkbox(&mut route.enabled, "");
                                        ui.add_space(8.0);
                                        if ui.button("Remove").clicked() {
                                            remove = true;
                                        }
                                    });
                                    ui.end_row();
                                });
                            if remove {
                                remove_playback_routes.push(index);
                            }
                        }
                        remove_playback_routes.sort_unstable_by(|left, right| right.cmp(left));
                        for index in remove_playback_routes {
                            self.recorder_config.audio_routes.remove(index);
                        }

                    }

                    let selection_is_available = self
                        .recorder_application_selection
                        .as_ref()
                        .is_some_and(|selection| {
                            application_sources
                                .iter()
                                .any(|source| &source.id == selection)
                        });
                    if !selection_is_available {
                        self.recorder_application_selection = None;
                    }
                    let mut application_selection = self.recorder_application_selection.clone();

                    let application_routes = self
                        .recorder_config
                        .audio_routes
                        .iter()
                        .enumerate()
                        .filter(|(_, route)| {
                            route.source_id.starts_with("application:")
                        })
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>();
                    let mut remove_application_routes = Vec::new();
                    for index in application_routes {
                        let mut remove = false;
                        let route = &mut self.recorder_config.audio_routes[index];
                        let selected_source = application_sources
                            .iter()
                            .find(|source| source.id == route.source_id);
                        let route_heading = if route.track_name.trim().is_empty() {
                            selected_source
                                .map(|source| source.label.clone())
                                .unwrap_or_else(|| "Application track".into())
                        } else {
                            route.track_name.clone()
                        };
                        ui.separator();
                        theme::section_title(ui, &route_heading);
                        egui::Grid::new(("recorder-application-route-settings", index))
                            .num_columns(2)
                            .spacing([12.0, 8.0])
                            .show(ui, |ui| {
                                encoder_settings_label(ui, "Source", audio_label_width);
                                encoder_settings_control_area(ui, |ui| {
                                    if application_sources.is_empty() {
                                        ui.add_sized(
                                            [audio_control_width, 22.0],
                                            egui::TextEdit::singleline(&mut route.source_id)
                                                .hint_text(
                                                    "application:spotify or application:discord.exe",
                                                ),
                                        );
                                    } else {
                                        let selected_text = selected_source
                                            .map(|source| source.label.clone())
                                            .unwrap_or_else(|| "Manual selector…".into());
                                        egui::ComboBox::from_id_salt((
                                            "recorder-application-route",
                                            index,
                                        ))
                                        .width(audio_control_width)
                                        .selected_text(selected_text)
                                        .show_ui(ui, |ui| {
                                            for source in &application_sources {
                                                ui.selectable_value(
                                                    &mut route.source_id,
                                                    source.id.clone(),
                                                    source.label.clone(),
                                                );
                                            }
                                            ui.separator();
                                            ui.selectable_value(
                                                &mut route.source_id,
                                                "application:".into(),
                                                "Manual selector…",
                                            );
                                        });
                                    }
                                });
                                ui.end_row();
                                if !application_sources.is_empty() && selected_source.is_none() {
                                    encoder_settings_label(ui, "Selector", audio_label_width);
                                    encoder_settings_control_area(ui, |ui| {
                                        ui.add_sized(
                                            [audio_control_width, 22.0],
                                            egui::TextEdit::singleline(&mut route.source_id)
                                                .hint_text(
                                                    "application:spotify or application:discord.exe",
                                                ),
                                        );
                                    });
                                    ui.end_row();
                                }
                                encoder_settings_label(ui, "Track name", audio_label_width);
                                encoder_settings_control_area(ui, |ui| {
                                    ui.add_sized(
                                        [audio_control_width, 22.0],
                                        egui::TextEdit::singleline(&mut route.track_name),
                                    );
                                });
                                ui.end_row();
                                encoder_settings_label(ui, "Audio track", audio_label_width);
                                encoder_settings_control_area(ui, |ui| {
                                    audio_track_selector(
                                        ui,
                                        ("recorder-application-track", index),
                                        &mut route.track,
                                        audio_track_width,
                                    );
                                });
                                ui.end_row();
                                encoder_settings_label(ui, "Enabled", audio_label_width);
                                encoder_settings_control_area(ui, |ui| {
                                    ui.checkbox(&mut route.enabled, "");
                                    ui.add_space(8.0);
                                    if ui.button("Remove").clicked() {
                                        remove = true;
                                    }
                                });
                                ui.end_row();
                            });
                        if remove {
                            remove_application_routes.push(index);
                        }
                    }
                    remove_application_routes.sort_unstable_by(|left, right| right.cmp(left));
                    for index in remove_application_routes {
                        self.recorder_config.audio_routes.remove(index);
                    }
                    ui.separator();
                    if show_playback_devices && available_playback_sources.is_empty() {
                        ui.label(
                            RichText::new(
                                "No active Windows playback devices were reported. Connect or enable the endpoint, then refresh the recorder.",
                            )
                            .color(theme::MUTED)
                            .size(11.0),
                        );
                    }
                    egui::Grid::new("recorder-append-audio-tracks")
                        .num_columns(2)
                        .spacing([12.0, 8.0])
                        .show(ui, |ui| {
                            if show_playback_devices && !available_playback_sources.is_empty() {
                                encoder_settings_label(
                                    ui,
                                    "Append playback device",
                                    audio_label_width,
                                );
                                encoder_settings_control_area(ui, |ui| {
                                    let add_button_width = 58.0;
                                    let selected_text = playback_selection
                                        .as_ref()
                                        .and_then(|selection| {
                                            available_playback_sources
                                                .iter()
                                                .find(|source| &source.id == selection)
                                        })
                                        .map(|source| source.label.clone())
                                        .unwrap_or_else(|| "Choose a device…".into());
                                    egui::ComboBox::from_id_salt("recorder-playback-device")
                                        .width(
                                            (audio_control_width - add_button_width - 12.0)
                                                .max(120.0),
                                        )
                                        .selected_text(selected_text)
                                        .show_ui(ui, |ui| {
                                            for source in &available_playback_sources {
                                                ui.selectable_value(
                                                    &mut playback_selection,
                                                    Some(source.id.clone()),
                                                    source.label.clone(),
                                                );
                                            }
                                        });
                                    if ui
                                        .add_enabled(
                                            playback_selection.is_some(),
                                            egui::Button::new("Add"),
                                        )
                                        .clicked()
                                    {
                                        if let Some(source_id) = playback_selection.clone() {
                                            if let Some(route) = self
                                                .recorder_config
                                                .audio_routes
                                                .iter_mut()
                                                .find(|route| route.source_id == source_id)
                                            {
                                                route.enabled = true;
                                            } else {
                                                let track =
                                                    next_audio_track(&self.recorder_config);
                                                let track_name = available_playback_sources
                                                    .iter()
                                                    .find(|source| source.id == source_id)
                                                    .map(|source| source.label.clone())
                                                    .unwrap_or_default();
                                                self.recorder_config.audio_routes.push(AudioRoute {
                                                    source_id,
                                                    track,
                                                    track_name,
                                                    enabled: true,
                                                });
                                            }
                                            playback_selection = None;
                                        }
                                    }
                                });
                                ui.end_row();
                            }
                            if !application_sources.is_empty() {
                                encoder_settings_label(
                                    ui,
                                    "Append application track",
                                    audio_label_width,
                                );
                                encoder_settings_control_area(ui, |ui| {
                                    let add_button_width = 58.0;
                                    let selected_text = application_selection
                                        .as_ref()
                                        .and_then(|selection| {
                                            application_sources
                                                .iter()
                                                .find(|source| &source.id == selection)
                                        })
                                        .map(|source| source.label.clone())
                                        .unwrap_or_else(|| "Choose an app…".into());
                                    egui::ComboBox::from_id_salt("recorder-open-application")
                                        .width(
                                            (audio_control_width - add_button_width - 12.0)
                                                .max(120.0),
                                        )
                                        .selected_text(selected_text)
                                        .show_ui(ui, |ui| {
                                            for source in &application_sources {
                                                ui.selectable_value(
                                                    &mut application_selection,
                                                    Some(source.id.clone()),
                                                    source.label.clone(),
                                                );
                                            }
                                        });
                                    if ui
                                        .add_enabled(
                                            application_selection.is_some(),
                                            egui::Button::new("Add"),
                                        )
                                        .clicked()
                                    {
                                        if let Some(source_id) = application_selection.clone() {
                                            if let Some(route) = self
                                                .recorder_config
                                                .audio_routes
                                                .iter_mut()
                                                .find(|route| route.source_id == source_id)
                                            {
                                                route.enabled = true;
                                            } else {
                                                let track =
                                                    next_audio_track(&self.recorder_config);
                                                let track_name = application_sources
                                                    .iter()
                                                    .find(|source| source.id == source_id)
                                                    .map(|source| source.label.clone())
                                                    .unwrap_or_default();
                                                self.recorder_config.audio_routes.push(AudioRoute {
                                                    source_id,
                                                    track,
                                                    track_name,
                                                    enabled: true,
                                                });
                                            }
                                            application_selection = None;
                                        }
                                    }
                                });
                                ui.end_row();
                            }
                            encoder_settings_label(
                                ui,
                                "Custom application track",
                                audio_label_width,
                            );
                            encoder_settings_control_area(ui, |ui| {
                                if ui.button("Add custom selector").clicked() {
                                    let track = next_audio_track(&self.recorder_config);
                                    self.recorder_config.audio_routes.push(AudioRoute {
                                        source_id: "application:".into(),
                                        track,
                                        track_name: String::new(),
                                        enabled: false,
                                    });
                                }
                            });
                            ui.end_row();
                        });
                    self.recorder_playback_selection = playback_selection;
                    self.recorder_application_selection = application_selection;
                    let mut used_tracks = HashSet::new();
                    if self
                        .recorder_config
                        .audio_routes
                        .iter()
                        .filter(|route| route.enabled)
                        .any(|route| !used_tracks.insert(route.track))
                    {
                        ui.label(
                            RichText::new(
                                "Enabled sources must use different tracks. Assign each one a unique track before saving.",
                            )
                            .color(theme::DANGER)
                            .size(11.0),
                        );
                    }
                });
                }

                ui.add_space(10.0);
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("Apply recorder settings")
                                    .family(theme::medium())
                                    .color(theme::TEXT),
                            );
                            theme::helper_text(ui, "Changes take effect when you save.");
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    !self.busy,
                                    egui::Button::new(
                                        RichText::new("Save settings")
                                            .family(theme::medium())
                                            .color(theme::INK),
                                    )
                                    .fill(theme::ACCENT),
                                )
                                .clicked()
                            {
                                self.apply_recorder_config();
                            }
                        });
                    });
                });

            });
    }

    fn status_banner(&mut self, ui: &mut Ui) {
        let tracked_job = match &self.publish_modal {
            Some(PublishModal::Job { id }) => Some(id.as_str()),
            _ => None,
        };
        let active = self.selected_id.as_ref().and_then(|clip_id| {
            self.jobs.iter().find(|job| {
                job.clip_id == *clip_id
                    && matches!(job.status.as_str(), "queued" | "transcoding" | "uploading")
                    && tracked_job != Some(job.id.as_str())
            })
        });
        if let Some(job) = active.cloned() {
            let stage = publish_stage_label(&job);
            theme::card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(stage)
                            .family(theme::medium())
                            .color(theme::ACCENT),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{:.0}%", job.progress * 100.0))
                                .monospace()
                                .color(theme::TEXT),
                        );
                    });
                });
                ui.add_space(6.0);
                theme::progress_bar(ui, job.progress as f32);
                ui.add_space(2.0);
                ui.label(
                    RichText::new("You can keep editing while this runs.")
                        .color(theme::MUTED)
                        .size(12.0),
                );
            });
            ui.add_space(8.0);
            return;
        }
        if let Some(update) = self.available_update.clone() {
            if !matches!(
                self.update_modal,
                Some(UpdateModal::Downloading { .. } | UpdateModal::Installing)
            ) {
                theme::card().show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{} {} is ready.", APP_NAME, update.version))
                                .family(theme::medium())
                                .color(theme::ACCENT),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Install").clicked() {
                                self.update_modal = Some(UpdateModal::Prompt);
                            }
                            if ui
                                .add(egui::Button::new(RichText::new("Later").size(12.0)))
                                .clicked()
                            {
                                let _ = self.engine.snooze_update(&update.version);
                                self.available_update = None;
                                self.update_modal = None;
                            }
                        });
                    });
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(update_restart_note())
                            .color(theme::MUTED)
                            .size(12.0),
                    );
                });
                ui.add_space(8.0);
            }
        }
        if let Some(error) = self.error.clone() {
            theme::card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.colored_label(theme::DANGER, error);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new(RichText::new("Dismiss").size(12.0)))
                            .clicked()
                        {
                            self.dismiss_error();
                        }
                    });
                });
            });
            ui.add_space(8.0);
        } else if let Some(notice) = self.notice.clone() {
            theme::card().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.colored_label(theme::OK, notice);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new(RichText::new("Dismiss").size(12.0)))
                            .clicked()
                        {
                            self.dismiss_notice();
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }
    }

    fn library_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Library").family(theme::medium()).size(18.0));
            ui.label(
                RichText::new(format!("{} clips", self.clips.len()))
                    .color(theme::MUTED)
                    .size(12.0),
            );
        });
        ui.add_space(8.0);
        if theme::import_drop_zone(ui, self.drop_hovering, 72.0).clicked() {
            self.import_recordings();
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Inbox folder").color(theme::MUTED).size(11.5));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Scan").clicked() {
                    let engine = self.engine.clone();
                    self.run_async(async move {
                        engine.scan_clips().await?;
                        Ok(Message::Refresh)
                    });
                }
                let path = self.config.source_directory.clone();
                let path_response =
                    theme::folder_path_field(ui, &path).on_hover_cursor(CursorIcon::PointingHand);
                if path_response.clicked() {
                    self.pick_inbox_folder();
                }
            });
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let clips = self.inbox_clips().into_iter().cloned().collect::<Vec<_>>();
                for clip in clips {
                    self.library_clip_row(ui, &clip);
                    ui.add_space(6.0);
                }
            });
    }

    fn published_version_count(&self, clip_id: &str) -> usize {
        self.jobs
            .iter()
            .filter(|job| job.clip_id == clip_id && job.status == "complete")
            .count()
    }

    fn library_clip_row(&mut self, ui: &mut Ui, clip: &Clip) {
        let selected = self.selected_id.as_deref() == Some(&clip.id);
        let versions = self.published_version_count(&clip.id);
        let clip_id = clip.id.clone();
        let clip_name = clip.name.clone();
        let clip_path = PathBuf::from(&clip.source_path);
        let desired = Vec2::new(ui.available_width(), 62.0);
        let id = ui.id().with(&clip.id);
        let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
        let response = ui.interact(rect, id, Sense::click());
        let fill = if selected {
            theme::CARD_HOVER
        } else if response.hovered() {
            theme::CARD
        } else {
            theme::PANEL
        };
        ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(6),
            egui::Stroke::new(1.0, if selected { theme::ACCENT } else { theme::LINE }),
            StrokeKind::Inside,
        );
        if selected {
            ui.painter().rect_filled(
                Rect::from_min_max(rect.left_top(), Pos2::new(rect.left() + 3.0, rect.bottom())),
                CornerRadius::ZERO,
                theme::ACCENT,
            );
        }
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(10.0, 8.0)))
                .sense(Sense::hover()),
            |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 2.0);
                ui.horizontal_centered(|ui| {
                    self.ensure_thumb(ui.ctx(), clip);
                    let thumb_size = Vec2::new(80.0, 45.0);
                    let thumb_rect = if let Some(texture) = self.thumbs.get(&clip.id) {
                        ui.add(
                            egui::Image::new((texture.id(), thumb_size))
                                .corner_radius(4.0)
                                .sense(Sense::hover()),
                        )
                        .rect
                    } else {
                        let (thumb, _) = ui.allocate_exact_size(thumb_size, Sense::hover());
                        ui.painter()
                            .rect_filled(thumb, CornerRadius::same(4), theme::BG);
                        thumb
                    };
                    if versions > 0 {
                        theme::published_tick_overlay(ui, thumb_rect, versions);
                    }
                    let meta = format!(
                        "{}p{}  ·  {}",
                        clip.height,
                        clip.fps.round(),
                        format_duration_compact(clip.duration),
                    );
                    let text_width = ui.available_width().max(40.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(text_width, ui.available_height()),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(text_width);
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(clip.name.clone())
                                            .family(theme::medium())
                                            .size(13.0),
                                    )
                                    .truncate()
                                    .selectable(false)
                                    .sense(Sense::hover()),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(meta).color(theme::MUTED).size(11.5),
                                    )
                                    .truncate()
                                    .selectable(false)
                                    .sense(Sense::hover()),
                                );
                            });
                        },
                    );
                });
            },
        );
        let mut open_in_explorer = false;
        let mut delete_clip = false;
        response.context_menu(|ui| {
            ui.label(
                RichText::new(clip_name.clone())
                    .family(theme::medium())
                    .size(12.5),
            );
            ui.separator();
            if ui.button("Open in file explorer").clicked() {
                open_in_explorer = true;
                ui.close();
            }
            if ui
                .button(RichText::new("Delete from device").color(theme::DANGER))
                .clicked()
            {
                delete_clip = true;
                ui.close();
            }
        });
        if response.secondary_clicked() {
            self.selected_id = Some(clip_id.clone());
        }
        if response.clicked() {
            self.selected_id = Some(clip_id.clone());
            self.show_recorder = false;
            self.recorder_hotkey_listening = false;
        }
        if open_in_explorer {
            self.open_clip_in_file_explorer(&clip_path);
        }
        if delete_clip {
            self.pending_delete_clip = Some(clip_id);
        }
    }

    fn open_clip_in_file_explorer(&mut self, path: &Path) {
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if let Err(error) = open::that(directory) {
            self.set_error(format!("Could not open file explorer: {error}"));
        }
    }

    fn ensure_thumb(&mut self, ctx: &egui::Context, clip: &Clip) {
        if clip.preview_status != "ready" {
            self.thumbs.remove(&clip.id);
            return;
        }
        if self.thumbs.contains_key(&clip.id) {
            return;
        }
        let path = self.engine.thumbnail_path(clip);
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(image) = image::load_from_memory(&bytes) {
                let rgba = image.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                let texture = ctx.load_texture(clip.id.clone(), color, TextureOptions::LINEAR);
                self.thumbs.insert(clip.id.clone(), texture);
            }
        }
    }

    fn pick_inbox_folder(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose inbox folder");
        if !self.config.source_directory.is_empty() {
            dialog = dialog.set_directory(PathBuf::from(&self.config.source_directory));
        }
        let Some(folder) = dialog.pick_folder() else {
            return;
        };
        match self.engine.set_source_directory(folder) {
            Ok(path) => {
                self.config.source_directory = path.to_string_lossy().to_string();
                self.set_notice(format!("Inbox set to {}", self.config.source_directory));
                self.dismiss_error();
                self.ensure_valid_selection();
                let engine = self.engine.clone();
                self.run_async(async move {
                    engine.scan_clips().await?;
                    Ok(Message::Refresh)
                });
            }
            Err(error) => {
                self.set_error(error.to_string());
            }
        }
    }

    fn ingest_dropped_files(&mut self, ctx: &egui::Context) {
        self.drop_hovering = files_being_dropped(ctx);
        let dropped = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| {
                    let path = file.path();
                    (!path.as_os_str().is_empty()).then(|| path.to_path_buf())
                })
                .collect::<Vec<_>>()
        });
        if dropped.is_empty() {
            return;
        }
        let files = collect_import_paths(dropped);
        if files.is_empty() {
            self.set_error(
                "Drop a video recording (mkv, mp4, mov, webm, avi, or m4v).".to_string(),
            );
            return;
        }
        self.import_paths(files);
    }

    fn import_recordings(&mut self) {
        let files = rfd::FileDialog::new()
            .add_filter(
                "Video recordings",
                &["mkv", "mp4", "mov", "webm", "avi", "m4v"],
            )
            .pick_files();
        if let Some(files) = files {
            self.import_paths(files);
        }
    }

    fn import_startup_files(&mut self) {
        let files = media_paths_from_args(std::env::args_os());
        if files.is_empty() {
            return;
        }
        self.import_paths(files);
    }

    fn import_paths(&mut self, files: Vec<PathBuf>) {
        if files.is_empty() {
            return;
        }
        let engine = self.engine.clone();
        self.run_async(async move {
            let clips = engine.import_clips(files).await?;
            Ok(Message::Imported(
                clips.into_iter().map(|clip| clip.id).collect(),
            ))
        });
    }

    fn bind_session_media(&mut self, media: Option<String>) {
        if self.session_media.as_deref() == media.as_deref() {
            return;
        }
        if let Some(player) = &mut self.player {
            player.unload();
        }
        self.session_media = media;
    }

    fn playback_time(&self) -> f64 {
        self.player
            .as_ref()
            .filter(|player| player.has_video() || player.playing() || player.buffering())
            .map(|player| player.time())
            .unwrap_or(0.0)
    }

    fn timeline_time(&self) -> f64 {
        self.timeline_drag
            .map(|drag| drag.time)
            .unwrap_or_else(|| self.playback_time())
    }

    fn activate_player(&mut self, play: bool) {
        let Some(media) = self.session_media.clone() else {
            return;
        };
        if let Some(player) = &mut self.player {
            if player.loaded_path() != Some(media.as_str()) {
                if let Err(error) = player.load(&media) {
                    self.set_error(error.to_string());
                    return;
                }
            }
            if play {
                player.play();
            } else {
                player.pause();
            }
        }
    }

    fn toggle_playback(&mut self) {
        if self
            .player
            .as_ref()
            .is_some_and(|player| player.wants_to_play())
        {
            if let Some(player) = &self.player {
                player.pause();
            }
        } else {
            self.start_playback();
        }
    }

    fn request_play(&mut self) {
        if self
            .player
            .as_ref()
            .is_some_and(|player| player.wants_to_play())
        {
            return;
        }
        self.start_playback();
    }

    fn start_playback(&mut self) {
        if let Some(editor) = &self.editor {
            if self.playback_time() + 0.01 >= editor.end {
                let start = editor.start;
                self.activate_player(false);
                if let Some(player) = &mut self.player {
                    player.seek_and_play(start);
                }
                return;
            }
        }
        self.activate_player(true);
    }

    fn stop_at_out_point(&mut self) {
        if self.timeline_drag.is_some() {
            return;
        }
        let Some(end) = self.editor.as_ref().map(|editor| editor.end) else {
            return;
        };
        if let Some(player) = &mut self.player {
            player.stop_at(end);
        }
    }

    fn step_frame(&mut self, delta: f64) {
        self.activate_player(false);
        if let Some(player) = &mut self.player {
            player.seek_relative(delta);
        }
    }

    fn seek_preview(&mut self, time: f64) {
        self.activate_player(false);
        if let Some(player) = &mut self.player {
            player.seek(time);
        }
    }

    fn seek_preserving_play_state(&mut self, time: f64) {
        let playing = self
            .player
            .as_ref()
            .is_some_and(|player| player.wants_to_play());
        self.activate_player(playing);
        if let Some(player) = &mut self.player {
            if playing {
                player.seek_and_play(time);
            } else {
                player.seek(time);
            }
        }
    }

    fn seek_to_clip_start(&mut self) {
        let start = self
            .editor
            .as_ref()
            .map(|editor| editor.start)
            .unwrap_or(0.0);
        self.seek_preserving_play_state(start);
    }

    fn trim_time_value(&mut self, ui: &mut Ui, field: TimeField, displayed: &str, duration: f64) {
        let editing = self
            .time_edit
            .as_ref()
            .is_some_and(|edit| edit.field == field);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(100.0, 28.0), Sense::hover());
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                if editing {
                    self.trim_time_edit(ui, duration);
                } else {
                    self.trim_time_label(ui, field, displayed, duration);
                }
            });
        });
    }

    fn trim_time_edit(&mut self, ui: &mut Ui, duration: f64) {
        let mut text = self
            .time_edit
            .as_ref()
            .map(|edit| edit.text.clone())
            .unwrap_or_default();
        let response = ui.add(
            egui::TextEdit::singleline(&mut text)
                .font(egui::FontId::monospace(13.0))
                .desired_width(100.0)
                .clip_text(false)
                .horizontal_align(Align::Center)
                .margin(egui::Margin::symmetric(6, 4)),
        );
        if let Some(edit) = &mut self.time_edit {
            edit.text = text;
        }
        let already_focused = self
            .time_edit
            .as_ref()
            .is_some_and(|edit| edit.requested_focus);
        if !already_focused {
            response.request_focus();
            if let Some(edit) = &mut self.time_edit {
                edit.requested_focus = true;
            }
        }
        let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
        let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if escape {
            self.time_edit = None;
        } else if enter || (already_focused && response.lost_focus()) {
            self.commit_time_edit(duration);
        }
    }

    fn trim_time_label(&mut self, ui: &mut Ui, field: TimeField, displayed: &str, duration: f64) {
        let response = ui
            .add(
                egui::Label::new(
                    RichText::new(displayed)
                        .monospace()
                        .size(13.0)
                        .color(theme::MUTED),
                )
                .sense(Sense::click()),
            )
            .on_hover_text("Click to edit time");
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }
        if response.clicked() || response.double_clicked() {
            if self
                .time_edit
                .as_ref()
                .is_some_and(|edit| edit.field != field)
            {
                self.commit_time_edit(duration);
            }
            self.time_edit = Some(TimeEdit {
                field,
                text: displayed.to_string(),
                requested_focus: false,
            });
        }
    }

    fn commit_time_edit(&mut self, duration: f64) {
        let Some(edit) = self.time_edit.take() else {
            return;
        };
        let Some(parsed) = parse_time(&edit.text) else {
            return;
        };
        let time = {
            let Some(editor) = &mut self.editor else {
                return;
            };
            match edit.field {
                TimeField::In => {
                    editor.start = parsed.min(editor.end - 0.05).max(0.0);
                    editor.start
                }
                TimeField::Out => {
                    editor.end = parsed.max(editor.start + 0.05).min(duration);
                    editor.end
                }
            }
        };
        self.seek_preview(time);
    }

    fn editor_panel(&mut self, ui: &mut Ui, ctx: &egui::Context, clip: &Clip) {
        if self
            .editor
            .as_ref()
            .is_none_or(|editor| editor.clip_id != clip.id)
        {
            let options = export_options(
                clip.width,
                clip.height,
                clip.fps,
                clip.duration,
                !clip.audio_tracks.is_empty(),
            );
            let (export_height, export_fps) = options
                .first()
                .map(|option| (option.height, option.fps))
                .unwrap_or((720, 30));
            self.editor = Some(EditorState {
                clip_id: clip.id.clone(),
                start: 0.0,
                end: clip.duration,
                tracks: clip
                    .audio_tracks
                    .iter()
                    .map(|track| track.stream_index)
                    .collect(),
                muted: false,
                export_height,
                export_fps,
            });
            self.time_edit = None;
        }
        self.bind_session_media(Some(clip.source_path.clone()));
        let jobs = self
            .jobs
            .iter()
            .filter(|job| job.clip_id == clip.id)
            .cloned()
            .collect::<Vec<_>>();
        let frame_step = 1.0 / clip.fps.max(1.0);
        let size = ui.available_size();
        let wide = size.x >= 1040.0;
        let gap = 14.0;
        let inspector_w = if wide {
            (size.x * 0.30).clamp(300.0, 400.0)
        } else {
            0.0
        };
        let stage_w = if inspector_w > 0.0 {
            size.x - inspector_w - gap
        } else {
            size.x
        };

        if let Some(error) = &self.player_error {
            ui.colored_label(theme::DANGER, error);
        }

        if wide {
            ui.allocate_ui(Vec2::new(size.x, size.y), |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(size.y);
                    ui.allocate_ui_with_layout(
                        Vec2::new(stage_w, size.y),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(stage_w);
                            ui.set_max_height(size.y);
                            self.editor_stage(ui, ctx, clip);
                        },
                    );
                    ui.allocate_ui_with_layout(
                        Vec2::new(inspector_w, size.y),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(inspector_w);
                            ui.set_min_height(size.y);
                            ui.set_max_height(size.y);
                            self.editor_inspector(ui, clip, &jobs, true);
                        },
                    );
                });
            });
        } else {
            let stage_h = (size.y * 0.64).clamp(260.0, (size.y - 220.0).max(260.0));
            egui::ScrollArea::vertical()
                .id_salt("editor-page")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let width = ui.available_width();
                    ui.set_width(width);
                    ui.allocate_ui_with_layout(
                        Vec2::new(width, stage_h),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_width(width);
                            ui.set_min_height(stage_h);
                            self.editor_stage(ui, ctx, clip);
                        },
                    );
                    ui.add_space(10.0);
                    self.editor_inspector(ui, clip, &jobs, false);
                });
        }

        let time = self.timeline_time();
        if !ctx.egui_wants_keyboard_input() {
            // Read keys first: play/seek can request a repaint, which deadlocks inside `ui.input`.
            let (space, left, right, mark_in, mark_out, shift) = ui.input(|input| {
                (
                    input.key_pressed(egui::Key::Space),
                    input.key_pressed(egui::Key::ArrowLeft),
                    input.key_pressed(egui::Key::ArrowRight),
                    input.key_pressed(egui::Key::I),
                    input.key_pressed(egui::Key::O),
                    input.modifiers.shift,
                )
            });
            if space {
                self.toggle_playback();
            }
            if left {
                self.step_frame(if shift { -1.0 } else { -frame_step });
            }
            if right {
                self.step_frame(if shift { 1.0 } else { frame_step });
            }
            if mark_in {
                if let Some(editor) = &mut self.editor {
                    editor.start = time.min(editor.end - 0.05).max(0.0);
                }
            }
            if mark_out {
                if let Some(editor) = &mut self.editor {
                    editor.end = time.max(editor.start + 0.05).min(clip.duration);
                }
            }
        }
    }

    fn editor_stage(&mut self, ui: &mut Ui, _ctx: &egui::Context, clip: &Clip) {
        let width = ui.available_width();
        let height = ui.available_height();
        ui.set_min_width(width);
        ui.set_min_height(height);
        let time = self.timeline_time();
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.set_width(width);
            ui.add_space(14.0);
            let row_height = 34.0;
            let row_width = ui.available_width().max(1.0);
            let (row, _) = ui.allocate_exact_size(Vec2::new(row_width, row_height), Sense::hover());
            let mut mark_in = false;
            let mut mark_out = false;
            let times = self
                .editor
                .as_ref()
                .map(|editor| (format_time(editor.start), format_time(editor.end)));
            let button_w = 156.0_f32.min((row.width() * 0.32).max(1.0));
            let left = Rect::from_min_max(row.min, Pos2::new(row.left() + button_w, row.bottom()));
            let right = Rect::from_min_max(Pos2::new(row.right() - button_w, row.top()), row.max);
            let center = Rect::from_min_max(
                Pos2::new(left.right(), row.top()),
                Pos2::new(right.left(), row.bottom()),
            );
            ui.scope_builder(UiBuilder::new().max_rect(left), |ui| {
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    mark_in = theme::hotkey_button(ui, "I", "Mark start", true).clicked();
                });
            });
            ui.scope_builder(UiBuilder::new().max_rect(right), |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    mark_out = theme::hotkey_button(ui, "O", "Mark end", false).clicked();
                });
            });
            if let Some((start, end)) = times {
                ui.scope_builder(UiBuilder::new().max_rect(center), |ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let cluster = 100.0 + 8.0 + 24.0 + 8.0 + 100.0;
                        let pad = ((center.width() - cluster) * 0.5).max(0.0);
                        if pad.is_finite() {
                            ui.add_space(pad);
                        }
                        self.trim_time_value(ui, TimeField::In, &start, clip.duration);
                        let (arrow, _) =
                            ui.allocate_exact_size(Vec2::new(24.0, 28.0), Sense::hover());
                        ui.painter().text(
                            arrow.center(),
                            egui::Align2::CENTER_CENTER,
                            "->",
                            egui::FontId::monospace(13.0),
                            theme::MUTED,
                        );
                        self.trim_time_value(ui, TimeField::Out, &end, clip.duration);
                    });
                });
            }
            if mark_in {
                if let Some(editor) = &mut self.editor {
                    editor.start = time.min(editor.end - 0.05).max(0.0);
                }
            }
            if mark_out {
                if let Some(editor) = &mut self.editor {
                    editor.end = time.max(editor.start + 0.05).min(clip.duration);
                }
            }
            self.timeline(ui, clip.duration, time);
            self.editor_transport(ui, clip.duration, false);
            let leftover = ui.available_size();
            ui.allocate_ui_with_layout(leftover, Layout::top_down(Align::Min), |ui| {
                ui.set_width(width);
                ui.set_min_height(leftover.y);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(clip.name.clone())
                            .family(theme::medium())
                            .size(16.0),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{}×{}  ·  {} fps  ·  original",
                                clip.width,
                                clip.height,
                                clip.fps.round()
                            ))
                            .color(theme::MUTED)
                            .size(12.0),
                        );
                    });
                });
                self.editor_preview(ui, clip, true);
            });
        });
    }

    fn editor_preview(&mut self, ui: &mut Ui, clip: &Clip, mix_editor_audio: bool) {
        self.ensure_thumb(ui.ctx(), clip);
        let thumb = self.thumbs.get(&clip.id).cloned();
        let available = ui.available_size();
        let stage_size = Vec2::new(available.x.max(2.0), available.y.max(2.0));
        let (stage, response) = ui.allocate_exact_size(stage_size, Sense::click());
        ui.painter()
            .rect_filled(stage, CornerRadius::same(8), Color32::BLACK);
        ui.painter().rect_stroke(
            stage,
            CornerRadius::same(8),
            egui::Stroke::new(1.0, theme::LINE),
            StrokeKind::Inside,
        );
        let rect = stage.shrink(1.0);
        let loaded = self
            .player
            .as_ref()
            .is_some_and(|player| player.loaded_path().is_some());
        let show_video = self
            .player
            .as_ref()
            .is_some_and(|player| player.has_video());
        let buffering = self
            .player
            .as_ref()
            .is_some_and(|player| player.buffering());
        let scrubbing = self.timeline_drag.is_some() || self.timeline_settling;
        if let Some(player) = &mut self.player {
            if loaded {
                if scrubbing {
                    player.paint_frozen(ui, rect.shrink(1.0));
                } else {
                    player.pump_events();
                    player.flush_seek();
                    if mix_editor_audio {
                        let _ = player.apply_audio(
                            &self
                                .editor
                                .as_ref()
                                .map(|editor| editor.tracks.clone())
                                .unwrap_or_default(),
                        );
                        player.set_mute(self.editor.as_ref().is_some_and(|editor| editor.muted));
                    }
                    player.start_if_ready();
                    player.paint(ui, rect.shrink(1.0));
                }
            }
        }
        if let Some(player) = &mut self.player {
            if let Some(error) = player.take_error() {
                self.set_error(error);
            }
        }
        if !show_video {
            if let Some(texture) = thumb {
                let size = texture.size_vec2();
                let fitted = theme::fit_contain(
                    rect.width(),
                    rect.height(),
                    (size.x / size.y.max(1.0)).max(0.01),
                );
                let image_rect = Rect::from_center_size(rect.center(), fitted);
                ui.painter().image(
                    texture.id(),
                    image_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
                if !buffering {
                    theme::poster_play_overlay(ui, image_rect);
                }
            } else if !buffering {
                theme::poster_play_overlay(ui, rect);
            }
        }
        if buffering {
            theme::buffering_overlay(ui, rect);
        }
        if response.clicked() {
            if show_video {
                self.toggle_playback();
            } else {
                self.request_play();
            }
        }
    }

    fn editor_transport(&mut self, ui: &mut Ui, duration: f64, watching: bool) {
        let time = self.timeline_time();
        let (display_time, display_duration) = if let Some(editor) = &self.editor {
            let length = (editor.end - editor.start).max(0.0);
            ((time - editor.start).clamp(0.0, length), length)
        } else {
            (time, duration)
        };
        theme::inset().show(ui, |ui| {
            ui.horizontal(|ui| {
                let playing = self
                    .player
                    .as_ref()
                    .is_some_and(|player| player.wants_to_play());
                if theme::transport_icon_button(
                    ui,
                    Vec2::new(36.0, 36.0),
                    false,
                    "Return to start",
                    theme::paint_to_start_icon,
                )
                .clicked()
                {
                    self.seek_to_clip_start();
                }
                if theme::transport_icon_button(
                    ui,
                    Vec2::new(44.0, 36.0),
                    true,
                    if playing { "Pause" } else { "Play" },
                    if playing {
                        theme::paint_pause_icon
                    } else {
                        theme::paint_play_icon
                    },
                )
                .clicked()
                {
                    self.toggle_playback();
                }
                if watching {
                    let muted = self.player.as_ref().is_some_and(|player| player.muted());
                    if ui.selectable_label(muted, "Mute").clicked() {
                        if let Some(player) = &self.player {
                            player.set_mute(!muted);
                        }
                    }
                } else if let Some(editor) = &mut self.editor {
                    if ui.selectable_label(editor.muted, "Mute").clicked() {
                        editor.muted = !editor.muted;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{}  /  {}",
                            format_time(display_time),
                            format_time(display_duration)
                        ))
                        .monospace()
                        .size(14.0),
                    );
                });
            });
        });
    }

    fn editor_inspector(&mut self, ui: &mut Ui, clip: &Clip, jobs: &[PublishJob], scroll: bool) {
        let mut add_contents = |ui: &mut Ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Audio mix").family(theme::medium()).size(14.5));
                    ui.label(
                        RichText::new("Checked tracks mix live. Video stays the original decode.")
                            .color(theme::MUTED)
                            .size(12.0),
                    );
                    ui.add_space(8.0);
                    if let Some(editor) = &mut self.editor {
                        for track in &clip.audio_tracks {
                            let mut enabled = editor.tracks.contains(&track.stream_index);
                            let label = track_name(track);
                            theme::inset().show(ui, |ui| {
                                if ui
                                    .checkbox(
                                        &mut enabled,
                                        format!(
                                            "{label}    {}  ·  track {}",
                                            track.codec.to_uppercase(),
                                            track.ordinal + 1
                                        ),
                                    )
                                    .changed()
                                {
                                    if enabled {
                                        if !editor.tracks.contains(&track.stream_index) {
                                            editor.tracks.push(track.stream_index);
                                        }
                                    } else {
                                        editor.tracks.retain(|value| *value != track.stream_index);
                                    }
                                    if let Some(player) = &mut self.player {
                                        player.clear_audio();
                                    }
                                }
                            });
                            ui.add_space(6.0);
                        }
                        if clip.audio_tracks.is_empty() {
                            ui.label("This recording has no audio tracks.");
                        }
                    }
                });
                ui.add_space(10.0);
                theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Export").family(theme::medium()).size(14.5));
                    ui.label(
                        RichText::new(
                            "Save a local file any time. Publishing a share link needs an account, and those links expire in 30 days.",
                        )
                        .color(theme::MUTED)
                        .size(12.0),
                    );
                    ui.add_space(8.0);
                    let options = self.editor.as_ref().map(|editor| {
                        export_options(
                            clip.width,
                            clip.height,
                            clip.fps,
                            editor.end - editor.start,
                            !editor.tracks.is_empty(),
                        )
                    });
                    if let (Some(editor), Some(options)) = (&mut self.editor, options.as_ref()) {
                        if !options.iter().any(|option| {
                            option.height == editor.export_height && option.fps == editor.export_fps
                        }) {
                            if let Some(option) = options.first() {
                                editor.export_height = option.height;
                                editor.export_fps = option.fps;
                            }
                        }
                    }
                    let (export_height, export_fps) = self
                        .editor
                        .as_ref()
                        .map(|editor| (editor.export_height, editor.export_fps))
                        .unwrap_or((0, 0));
                    let selected = options.as_ref().and_then(|options| {
                        options.iter().find(|option| {
                            option.height == export_height && option.fps == export_fps
                        })
                    });
                    if let Some(options) = &options {
                        if options.is_empty() {
                            ui.label(
                                RichText::new(
                                    "No export qualities are available for this selection.",
                                )
                                .color(theme::DANGER)
                                .size(12.0),
                            );
                        } else {
                            let selected_text = selected
                                .map(quality_choice_label)
                                .unwrap_or_else(|| "Choose quality".into());
                            egui::ComboBox::from_id_salt("publish-quality")
                                .width(ui.available_width())
                                .selected_text(selected_text)
                                .show_ui(ui, |ui| {
                                    for option in options {
                                        let is_selected = self.editor.as_ref().is_some_and(|editor| {
                                            editor.export_height == option.height
                                                && editor.export_fps == option.fps
                                        });
                                        let response = ui.selectable_label(
                                            is_selected,
                                            quality_choice_label(option),
                                        );
                                        if option.heavier_than_1080p120() {
                                            response.clone().on_hover_text(
                                                "High bandwidth quality — some slower connections may struggle to play this back smoothly.",
                                            );
                                        }
                                        if response.clicked() {
                                            let height = option.height;
                                            let fps = option.fps;
                                            if let Some(editor) = &mut self.editor {
                                                editor.export_height = height;
                                                editor.export_fps = fps;
                                            }
                                        }
                                    }
                                });
                            if let Some(option) = selected {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(format!(
                                        "{}×{} after transcoding, about {}",
                                        option.width,
                                        option.height,
                                        format_file_size(option.estimated_bytes)
                                    ))
                                    .color(theme::MUTED)
                                    .size(12.0),
                                );
                                if option.heavier_than_1080p120() {
                                    ui.label(
                                        RichText::new(
                                            "You've selected a high bandwidth quality which some slower connections may struggle to play back smoothly.",
                                        )
                                        .color(theme::ACCENT)
                                        .size(12.0),
                                    );
                                }
                                if !option.within_publish_limit() {
                                    ui.label(
                                        RichText::new(
                                            "This quality is over 200 MB, so it can be saved locally but not published.",
                                        )
                                        .color(theme::ACCENT)
                                        .size(12.0),
                                    );
                                }
                            }
                        }
                    }
                    ui.add_space(8.0);
                    let can_export = !self.busy
                        && selected.is_some()
                        && self.publish_modal.is_none()
                        && self.export_modal.is_none();
                    let can_publish = can_export
                        && self.config.authenticated
                        && selected.is_some_and(PublishOption::within_publish_limit);
                    ui.horizontal(|ui| {
                        let gap = ui.spacing().item_spacing.x;
                        let button_width = ((ui.available_width() - gap) / 2.0).max(0.0);
                        let mut publish_response = ui.add_enabled(
                            can_publish,
                            egui::Button::new(
                                RichText::new("Publish")
                                    .color(if can_publish {
                                        theme::INK
                                    } else {
                                        theme::TEXT
                                    })
                                    .family(theme::medium()),
                            )
                            .fill(if can_publish {
                                theme::ACCENT
                            } else {
                                theme::LINE
                            })
                            .min_size(Vec2::new(button_width, 32.0)),
                        );
                        if !self.config.authenticated {
                            publish_response =
                                publish_response.on_hover_text("Sign in to publish a share link.");
                        } else if selected.is_some_and(|option| !option.within_publish_limit()) {
                            publish_response = publish_response.on_hover_text(
                                "This quality is over 200 MB. Choose a lower quality or shorten the trim to publish.",
                            );
                        }
                        if publish_response.clicked() {
                            if let (Some(editor), Some(option)) = (&self.editor, selected) {
                                self.publish_modal = Some(PublishModal::Name {
                                    clip_id: clip.id.clone(),
                                    title: default_clip_title(&clip.name),
                                    selection: Selection {
                                        start: editor.start,
                                        end: editor.end,
                                        audio_stream_indexes: editor.tracks.clone(),
                                        export: Some(option.profile()),
                                    },
                                    quality_label: option.quality_label(),
                                    focus_title: true,
                                });
                            }
                        }
                        if ui
                            .add_enabled(
                                can_export,
                                egui::Button::new(
                                    RichText::new("Export")
                                        .color(if can_publish {
                                            theme::TEXT
                                        } else {
                                            theme::INK
                                        })
                                        .family(theme::medium()),
                                )
                                .fill(if !can_export {
                                    theme::LINE
                                } else if can_publish {
                                    theme::CARD_HOVER
                                } else {
                                    theme::ACCENT
                                })
                                .min_size(Vec2::new(button_width, 32.0)),
                            )
                            .clicked()
                        {
                            let export =
                                self.editor.as_ref().zip(selected).map(|(editor, option)| {
                                    (
                                        clip.id.clone(),
                                        clip.name.clone(),
                                        Selection {
                                            start: editor.start,
                                            end: editor.end,
                                            audio_stream_indexes: editor.tracks.clone(),
                                            export: Some(option.profile()),
                                        },
                                        option.clone(),
                                    )
                                });
                            if let Some((clip_id, clip_name, selection, option)) = export {
                                self.start_local_export(clip_id, &clip_name, selection, &option);
                            }
                        }
                    });
                    ui.add_space(6.0);
                    if ui
                        .add_sized(
                            [ui.available_width(), 28.0],
                            egui::Button::new("Delete from device"),
                        )
                        .clicked()
                    {
                        self.pending_delete_clip = Some(clip.id.clone());
                    }
                });
                let history = jobs
                    .iter()
                    .filter(|job| matches!(job.status.as_str(), "complete" | "failed"))
                    .collect::<Vec<_>>();
                if !history.is_empty() {
                    ui.add_space(10.0);
                    ui.label(RichText::new("Versions").family(theme::medium()).size(14.5));
                    ui.add_space(6.0);
                    for job in history {
                        theme::card().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let status_color = match job.status.as_str() {
                                "complete" => theme::OK,
                                "failed" => theme::DANGER,
                                _ => theme::ACCENT,
                            };
                            ui.horizontal(|ui| {
                                ui.colored_label(
                                    status_color,
                                    RichText::new(job.status.to_uppercase())
                                        .size(11.5)
                                        .family(theme::medium()),
                                );
                                if job.status == "complete" {
                                    ui.label(
                                        RichText::new(expiry_label(job.expires_at.as_deref()))
                                            .color(theme::MUTED)
                                            .size(12.0),
                                    );
                                }
                            });
                            if let Some(url) = &job.url {
                                ui.label(RichText::new(url.clone()).small().color(theme::MUTED));
                                ui.horizontal(|ui| {
                                    if ui.button("Copy link").clicked() {
                                        ui.ctx().copy_text(url.clone());
                                        self.set_notice("Published link copied.");
                                    }
                                    if ui.button("Open").clicked() {
                                        let _ = open::that(url);
                                    }
                                    if job.status == "complete"
                                        && ui.button("Delete version").clicked()
                                    {
                                        self.pending_delete_job = Some(job.id.clone());
                                    }
                                });
                            }
                            if let Some(error) = &job.error {
                                ui.colored_label(theme::DANGER, error);
                            }
                        });
                        ui.add_space(6.0);
                    }
                }
                });
        };
        if scroll {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, add_contents);
        } else {
            add_contents(ui);
        }
    }

    fn timeline(&mut self, ui: &mut Ui, duration: f64, time: f64) {
        let height = 54.0;
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), height),
            Sense::click_and_drag(),
        );
        let time = self.timeline_drag.map(|drag| drag.time).unwrap_or(time);
        let track = Rect::from_min_max(
            Pos2::new(rect.left() + 1.0, rect.top() + 16.0),
            Pos2::new(rect.right() - 1.0, rect.bottom() - 16.0),
        );
        let duration = duration.max(0.001);
        let x_for = |value: f64| rect.left() + (value / duration) as f32 * rect.width();
        let time_at =
            |x: f32| (((x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64) * duration;
        let handle_hit = 14.0;
        let near_handle = |pos: Pos2, start: f64, end: f64| -> Option<TimelineDrag> {
            if pos.y < track.center().y {
                return None;
            }
            let dist_in = (pos.x - x_for(start)).abs();
            let dist_out = (pos.x - x_for(end)).abs();
            if dist_in <= handle_hit && dist_in <= dist_out {
                Some(TimelineDrag::In)
            } else if dist_out <= handle_hit {
                Some(TimelineDrag::Out)
            } else {
                None
            }
        };

        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), Color32::from_rgb(12, 13, 16));
        ui.painter()
            .rect_filled(track, CornerRadius::ZERO, Color32::from_rgb(46, 51, 62));

        let selection = self
            .editor
            .as_ref()
            .map(|editor| (editor.start, editor.end));
        if let Some((start, end)) = selection {
            let start_x = x_for(start).clamp(track.left(), track.right());
            let end_x = x_for(end).clamp(track.left(), track.right());
            if start_x > track.left() + 0.5 {
                ui.painter().rect_filled(
                    Rect::from_min_max(track.min, Pos2::new(start_x, track.bottom())),
                    CornerRadius::ZERO,
                    Color32::BLACK,
                );
            }
            if end_x < track.right() - 0.5 {
                ui.painter().rect_filled(
                    Rect::from_min_max(Pos2::new(end_x, track.top()), track.max),
                    CornerRadius::ZERO,
                    Color32::BLACK,
                );
            }
            ui.painter().rect_filled(
                Rect::from_min_max(
                    Pos2::new(start_x, track.top()),
                    Pos2::new(end_x, track.bottom()),
                ),
                CornerRadius::ZERO,
                theme::ACCENT.gamma_multiply(0.38),
            );
        }
        ui.painter().rect_stroke(
            track,
            CornerRadius::ZERO,
            egui::Stroke::new(1.0, theme::LINE),
            StrokeKind::Inside,
        );
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(6),
            egui::Stroke::new(1.0, theme::LINE),
            StrokeKind::Inside,
        );

        let play_x = x_for(time.clamp(0.0, duration));
        theme::paint_playhead(ui.painter(), play_x, track);

        let dragging = self.timeline_drag.map(|drag| drag.kind);
        if let Some((start, end)) = selection {
            theme::paint_trim_handle(
                ui.painter(),
                x_for(start),
                track,
                true,
                dragging == Some(TimelineDrag::In),
            );
            theme::paint_trim_handle(
                ui.painter(),
                x_for(end),
                track,
                false,
                dragging == Some(TimelineDrag::Out),
            );
        }

        let pointer = response
            .interact_pointer_pos()
            .or_else(|| response.hover_pos().filter(|_| response.hovered()));
        if let (Some(pos), Some((start, end))) = (pointer, selection) {
            if near_handle(pos, start, end).is_some()
                || dragging == Some(TimelineDrag::In)
                || dragging == Some(TimelineDrag::Out)
            {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
        }

        if response.drag_started() {
            self.time_edit = None;
            let kind = pointer
                .and_then(|pos| selection.and_then(|(start, end)| near_handle(pos, start, end)));
            let kind = kind.unwrap_or(TimelineDrag::Playhead);
            let was_playing = self
                .player
                .as_ref()
                .is_some_and(|player| player.wants_to_play());
            self.timeline_drag = Some(TimelineDragState {
                kind,
                time,
                was_playing,
            });
            if let Some(player) = &self.player {
                player.set_scrubbing(true);
            }
            self.activate_player(false);
        }

        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                self.apply_timeline_drag(time_at(pos.x), duration);
                ui.ctx().request_repaint();
            }
        } else if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let t = time_at(pos.x);
                let handle =
                    selection.and_then(|(start, end)| match near_handle(pos, start, end) {
                        Some(TimelineDrag::In) => Some(start),
                        Some(TimelineDrag::Out) => Some(end),
                        _ => None,
                    });
                if let Some(handle) = handle {
                    self.seek_preview(handle);
                } else {
                    self.seek_preserving_play_state(t);
                }
            }
        }

        if response.drag_stopped() {
            self.finish_timeline_drag();
        }
    }

    fn apply_timeline_drag(&mut self, time: f64, duration: f64) {
        let kind = self
            .timeline_drag
            .map(|drag| drag.kind)
            .unwrap_or(TimelineDrag::Playhead);
        let target = match kind {
            TimelineDrag::In => self.editor.as_mut().map(|editor| {
                editor.start = time.min(editor.end - 0.05).max(0.0);
                editor.start
            }),
            TimelineDrag::Out => self.editor.as_mut().map(|editor| {
                editor.end = time.max(editor.start + 0.05).min(duration);
                editor.end
            }),
            TimelineDrag::Playhead => Some(time.clamp(0.0, duration)),
        };
        let Some(target) = target else {
            return;
        };
        if let Some(drag) = &mut self.timeline_drag {
            drag.time = target;
        }
    }

    fn finish_timeline_drag(&mut self) {
        let Some(drag) = self.timeline_drag.take() else {
            return;
        };
        self.timeline_settling = true;
        if let Some(player) = &self.player {
            player.set_scrubbing(false);
        }
        self.activate_player(false);
        if let Some(player) = &mut self.player {
            if drag.kind == TimelineDrag::Playhead && drag.was_playing {
                player.seek_and_play(drag.time);
            } else {
                player.seek(drag.time);
            }
        }
    }

    fn auth_modal(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new(match self.forgot_step {
            1 => "Forgot your password?",
            2 => "Choose a new password",
            _ if self.show_auth == Some(AuthMode::Login) => "Sign in",
            _ => "Create account",
        })
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            if self.forgot_step == 0 {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            self.show_auth == Some(AuthMode::Request),
                            "Create account",
                        )
                        .clicked()
                    {
                        self.show_auth = Some(AuthMode::Request);
                    }
                    if ui
                        .selectable_label(self.show_auth == Some(AuthMode::Login), "Sign in")
                        .clicked()
                    {
                        self.show_auth = Some(AuthMode::Login);
                    }
                });
                ui.label("Username");
                ui.text_edit_singleline(&mut self.auth_username);
                if self.show_auth == Some(AuthMode::Request) {
                    ui.label("Display name");
                    ui.text_edit_singleline(&mut self.auth_display);
                }
                ui.label("Password");
                ui.add(egui::TextEdit::singleline(&mut self.auth_password).password(true));
                if self.show_auth == Some(AuthMode::Request) {
                    ui.label("Confirm password");
                    ui.add(egui::TextEdit::singleline(&mut self.auth_confirm).password(true));
                }
                if self.show_auth == Some(AuthMode::Login)
                    && ui.link("Forgot my password").clicked()
                {
                    self.forgot_step = 1;
                }
                let access_pending = self
                    .access_request
                    .as_ref()
                    .is_some_and(|request| request.status == "pending");
                let request_label = if access_pending {
                    "Awaiting owner approval"
                } else if self.busy {
                    "Access requested"
                } else {
                    "Request access"
                };
                if ui
                    .add_enabled(
                        !self.busy && !access_pending,
                        egui::Button::new(if self.show_auth == Some(AuthMode::Login) {
                            "Sign in"
                        } else {
                            request_label
                        }),
                    )
                    .clicked()
                {
                    self.submit_auth();
                }
                if self.show_auth == Some(AuthMode::Request) && (self.busy || access_pending) {
                    ui.label(
                        egui::RichText::new("Your request is pending owner approval.")
                            .color(theme::MUTED)
                            .size(12.0),
                    );
                    if access_pending && ui.button("Sign out this device").clicked() {
                        self.sign_out_this_device();
                    }
                }
            } else if self.forgot_step == 1 {
                ui.label("Username");
                ui.text_edit_singleline(&mut self.auth_username);
                ui.label("Forgotten-password token");
                ui.text_edit_multiline(&mut self.reset_token);
                ui.horizontal(|ui| {
                    if ui.button("Back").clicked() {
                        self.forgot_step = 0;
                    }
                    if ui.button("Continue").clicked() {
                        let engine = self.engine.clone();
                        let token = self.reset_token.clone();
                        let username = self.auth_username.clone();
                        let tx = self.tx.clone();
                        self.busy = true;
                        self.engine.spawn(async move {
                            match engine.validate_password_reset(&token, &username).await {
                                Ok(()) => {
                                    let _ = tx.send(Message::Notice(
                                        "Token is valid. Choose a new password.".into(),
                                    ));
                                }
                                Err(error) => {
                                    let _ = tx.send(Message::Error(format!("{error:#}")));
                                }
                            }
                        });
                        self.forgot_step = 2;
                    }
                });
            } else {
                ui.label("New password");
                ui.add(egui::TextEdit::singleline(&mut self.auth_password).password(true));
                ui.label("Confirm new password");
                ui.add(egui::TextEdit::singleline(&mut self.auth_confirm).password(true));
                if ui.button("Reset password and sign in").clicked() {
                    if self.auth_password != self.auth_confirm {
                        self.set_error("Passwords do not match.");
                    } else {
                        let engine = self.engine.clone();
                        let token = self.reset_token.clone();
                        let username = self.auth_username.clone();
                        let password = self.auth_password.clone();
                        let device = self.device_name.clone();
                        self.run_async(async move {
                            Ok(Message::User(
                                engine
                                    .redeem_invite(&token, &username, &password, "", &device)
                                    .await?
                                    .user,
                            ))
                        });
                    }
                }
            }
        });
        if !open {
            self.show_auth = None;
        }
    }

    fn submit_auth(&mut self) {
        if self.show_auth == Some(AuthMode::Request) && self.auth_password != self.auth_confirm {
            self.set_error("Passwords do not match.");
            return;
        }
        let engine = self.engine.clone();
        let username = self.auth_username.clone();
        let password = self.auth_password.clone();
        let display = self.auth_display.clone();
        let device = self.device_name.clone();
        if self.show_auth == Some(AuthMode::Login) {
            self.run_async(async move {
                Ok(Message::User(
                    engine.login(&username, &password, &device).await?.user,
                ))
            });
        } else {
            self.run_async(async move {
                let request = engine
                    .request_access(&username, &display, &password)
                    .await?;
                Ok(Message::AccessRequest(Some(request)))
            });
        }
    }

    fn sign_out_this_device(&mut self) {
        let engine = self.engine.clone();
        self.account_open = false;
        self.show_pending = false;
        self.access_request = None;
        self.show_auth = None;
        self.run_async(async move {
            engine.logout().await?;
            Ok(Message::LoggedOut)
        });
    }

    fn pending_modal(&mut self, ctx: &egui::Context) {
        let Some(request) = self.access_request.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Publishing access status")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("@{}", request.username));
                match request.status.as_str() {
                    "approved" => {
                        ui.heading("Your access was approved");
                        if ui.button("Sign in").clicked() {
                            self.show_pending = false;
                            self.show_auth = Some(AuthMode::Login);
                        }
                    }
                    "denied" => {
                        ui.heading("Your request was declined");
                        if ui.button("Start over").clicked() {
                            let _ = self.engine.clear_access_request();
                            self.access_request = None;
                            self.show_pending = false;
                            self.show_auth = Some(AuthMode::Request);
                        }
                    }
                    _ => {
                        ui.heading("Awaiting owner approval");
                        ui.horizontal(|ui| {
                            if ui.button("Check status").clicked() {
                                let engine = self.engine.clone();
                                self.run_async(async move {
                                    Ok(Message::AccessRequest(Some(
                                        engine.access_request_status().await?,
                                    )))
                                });
                            }
                            if ui.button("Sign out this device").clicked() {
                                self.sign_out_this_device();
                            }
                        });
                    }
                }
            });
        if !open {
            self.show_pending = false;
        }
    }

    fn access_modal(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Publishing access")
            .collapsible(false)
            .resizable(true)
            .default_size([720.0, 480.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (filter, label) in [
                        (AccessFilter::Pending, "Pending"),
                        (AccessFilter::Active, "Active"),
                        (AccessFilter::Revoked, "Revoked"),
                        (AccessFilter::Denied, "Declined"),
                    ] {
                        if ui
                            .selectable_label(self.access_filter == filter, label)
                            .clicked()
                        {
                            self.access_filter = filter;
                        }
                    }
                    ui.text_edit_singleline(&mut self.access_query);
                });
                let query = self.access_query.to_lowercase();
                match self.access_filter {
                    AccessFilter::Pending | AccessFilter::Denied => {
                        let status = if self.access_filter == AccessFilter::Pending {
                            "pending"
                        } else {
                            "denied"
                        };
                        for request in self.admin_requests.clone() {
                            if request.status != status
                                || !format!("{} {}", request.username, request.display_name)
                                    .to_lowercase()
                                    .contains(&query)
                            {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                ui.label(format!(
                                    "{}  @{}",
                                    request.display_name, request.username
                                ));
                                if request.status == "pending" {
                                    if ui.button("Approve").clicked() {
                                        self.review(&request.id, "approved");
                                    }
                                    if ui.button("Decline").clicked() {
                                        self.review(&request.id, "denied");
                                    }
                                }
                            });
                        }
                    }
                    AccessFilter::Active | AccessFilter::Revoked => {
                        let status = if self.access_filter == AccessFilter::Active {
                            "active"
                        } else {
                            "revoked"
                        };
                        for member in self.admin_users.clone() {
                            if member.status != status
                                || !format!("{} {}", member.username, member.display_name)
                                    .to_lowercase()
                                    .contains(&query)
                            {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                ui.label(format!(
                                    "{}  @{}  ·  {} devices",
                                    member.display_name, member.username, member.device_count
                                ));
                                let current = self.user.as_ref().map(|user| user.id.clone());
                                if ui
                                    .add_enabled(
                                        current.as_deref() != Some(&member.id)
                                            && member.status == "active",
                                        egui::Button::new("Reset password"),
                                    )
                                    .clicked()
                                {
                                    let engine = self.engine.clone();
                                    let id = member.id.clone();
                                    self.run_async(async move {
                                        Ok(Message::PasswordReset(
                                            engine.create_password_reset(&id).await?,
                                        ))
                                    });
                                }
                                let next = if member.status == "active" {
                                    "revoked"
                                } else {
                                    "active"
                                };
                                if ui
                                    .add_enabled(
                                        current.as_deref() != Some(&member.id),
                                        egui::Button::new(if member.status == "active" {
                                            "Revoke"
                                        } else {
                                            "Restore"
                                        }),
                                    )
                                    .clicked()
                                {
                                    let engine = self.engine.clone();
                                    let id = member.id.clone();
                                    let status = next.to_string();
                                    self.run_async(async move {
                                        engine.set_user_status(&id, &status).await?;
                                        Ok(Message::Admin(
                                            engine.admin_users().await?,
                                            engine.admin_access_requests().await?,
                                        ))
                                    });
                                }
                            });
                        }
                    }
                }
            });
        if !open {
            self.show_access = false;
        }
    }

    fn review(&mut self, id: &str, decision: &str) {
        let engine = self.engine.clone();
        let id = id.to_string();
        let decision = decision.to_string();
        self.run_async(async move {
            engine.review_access_request(&id, &decision).await?;
            Ok(Message::Admin(
                engine.admin_users().await?,
                engine.admin_access_requests().await?,
            ))
        });
    }

    fn publish_flow_modal(&mut self, ctx: &egui::Context) {
        let Some(modal) = self.publish_modal.clone() else {
            return;
        };
        let job = match &modal {
            PublishModal::Job { id } => self.jobs.iter().find(|job| job.id == *id).cloned(),
            PublishModal::Name { .. } => None,
        };
        let in_progress = job.as_ref().is_some_and(|job| {
            matches!(job.status.as_str(), "queued" | "transcoding" | "uploading")
        });
        let title = match (&modal, job.as_ref()) {
            (PublishModal::Name { .. }, _) => "Name this clip",
            (_, Some(job)) if job.status == "complete" => "Published",
            (_, Some(job)) if job.status == "failed" => "Publish failed",
            _ => "Publishing",
        };
        let mut open = true;
        let mut window = egui::Window::new(title)
            .id(egui::Id::new("publish-flow"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO);
        if !in_progress {
            window = window.open(&mut open);
        }
        window.show(ctx, |ui| {
            ui.set_width(360.0);
            match &modal {
                PublishModal::Name { quality_label, .. } => {
                    ui.label(
                        RichText::new(format!(
                            "Publishing {quality_label}. Share links expire in 30 days."
                        ))
                        .color(theme::MUTED)
                        .size(12.5),
                    );
                    ui.add_space(6.0);
                    ui.label("Clip name");
                    let PublishModal::Name {
                        title, focus_title, ..
                    } = self.publish_modal.as_mut().unwrap()
                    else {
                        return;
                    };
                    let response = ui.add(
                        egui::TextEdit::singleline(title)
                            .desired_width(ui.available_width())
                            .char_limit(160),
                    );
                    if *focus_title {
                        response.request_focus();
                        *focus_title = false;
                    }
                    let submitted = response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.publish_modal = None;
                        }
                        let can_confirm = self
                            .publish_modal
                            .as_ref()
                            .and_then(|modal| match modal {
                                PublishModal::Name { title, .. } => Some(!title.trim().is_empty()),
                                _ => None,
                            })
                            .unwrap_or(false);
                        if ui
                            .add_enabled(
                                can_confirm,
                                egui::Button::new(
                                    RichText::new("Publish")
                                        .color(theme::INK)
                                        .family(theme::medium()),
                                )
                                .fill(if can_confirm {
                                    theme::ACCENT
                                } else {
                                    theme::LINE
                                }),
                            )
                            .clicked()
                            || (submitted && can_confirm)
                        {
                            self.confirm_publish();
                        }
                    });
                }
                PublishModal::Job { .. } => {
                    let Some(job) = job else {
                        ui.label(
                            RichText::new("Starting export…")
                                .color(theme::MUTED)
                                .size(12.5),
                        );
                        ui.add_space(8.0);
                        theme::progress_bar(ui, 0.0);
                        return;
                    };
                    if job.status == "complete" {
                        ui.label("Your clip is live. Copy the link to share it.");
                        if let Some(url) = &job.url {
                            ui.label(RichText::new(url.clone()).small().color(theme::MUTED));
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Copy link")
                                                .color(theme::INK)
                                                .family(theme::medium()),
                                        )
                                        .fill(theme::ACCENT),
                                    )
                                    .clicked()
                                {
                                    ui.ctx().copy_text(url.clone());
                                    self.set_notice("Published link copied.");
                                }
                                if ui.button("Done").clicked() {
                                    self.publish_modal = None;
                                }
                            });
                        } else if ui.button("Done").clicked() {
                            self.publish_modal = None;
                        }
                    } else if job.status == "failed" {
                        ui.colored_label(
                            theme::DANGER,
                            job.error
                                .clone()
                                .unwrap_or_else(|| "Publishing failed.".into()),
                        );
                        ui.add_space(8.0);
                        if ui.button("Close").clicked() {
                            self.publish_modal = None;
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(publish_stage_label(&job))
                                    .family(theme::medium())
                                    .color(theme::ACCENT),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("{:.0}%", job.progress * 100.0))
                                        .monospace()
                                        .color(theme::TEXT),
                                );
                            });
                        });
                        ui.add_space(6.0);
                        theme::progress_bar(ui, job.progress as f32);
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new("Keep this window open until the upload finishes.")
                                .color(theme::MUTED)
                                .size(12.0),
                        );
                    }
                }
            }
        });
        if !open {
            self.publish_modal = None;
        }
    }

    fn confirm_publish(&mut self) {
        let Some(PublishModal::Name {
            clip_id,
            title,
            selection,
            ..
        }) = self.publish_modal.clone()
        else {
            return;
        };
        let title = title.trim().to_string();
        if title.is_empty() {
            return;
        }
        match self.engine.publish_clip(clip_id, title, selection) {
            Ok(job) => {
                self.publish_modal = Some(PublishModal::Job { id: job.id });
                self.dismiss_notice();
                self.reload_library();
            }
            Err(error) => self.set_error(format!("{error:#}")),
        }
    }

    fn start_local_export(
        &mut self,
        clip_id: String,
        clip_name: &str,
        selection: Selection,
        option: &PublishOption,
    ) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Export clip")
            .add_filter("MP4 video", &["mp4"])
            .set_file_name(format!(
                "{}-{}.mp4",
                safe_base_name(clip_name),
                option.quality_label()
            ));
        if let Ok(videos) = video_dir() {
            dialog = dialog.set_directory(videos);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        let output = ensure_mp4_path(path);
        let quality_label = option.quality_label();
        let engine = self.engine.clone();
        let tx = self.tx.clone();
        self.export_modal = Some(ExportModal::Working {
            quality_label,
            progress: 0.0,
        });
        self.run_async(async move {
            engine
                .export_clip_to(&clip_id, selection, output.clone(), |progress| {
                    let _ = tx.send(Message::ExportProgress(progress));
                })
                .await?;
            Ok(Message::ExportDone(output))
        });
    }

    fn update_flow_modal(&mut self, ctx: &egui::Context) {
        let Some(update) = self.available_update.clone() else {
            self.update_modal = None;
            return;
        };
        let Some(modal) = self.update_modal.clone() else {
            return;
        };
        let blocking = !matches!(modal, UpdateModal::Prompt);
        let mut open = true;
        let mut window = egui::Window::new(format!("{} {}", APP_NAME, update.version))
            .id(egui::Id::new("update-flow"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO);
        if !blocking {
            window = window.open(&mut open);
        }
        window.show(ctx, |ui| {
            ui.set_width(380.0);
            match modal {
                UpdateModal::Prompt => {
                    ui.label(
                        RichText::new("A published GitHub Release is newer than this build.")
                            .color(theme::MUTED)
                            .size(12.5),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!(
                            "{}  ·  {}",
                            update.asset_name,
                            format_file_size(update.size)
                        ))
                        .monospace()
                        .size(12.0)
                        .color(theme::TEXT),
                    );
                    let notes = condensed_release_notes(&update.notes);
                    if !notes.is_empty() {
                        ui.add_space(10.0);
                        ui.label(RichText::new(notes).size(13.0));
                    }
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(update_restart_note())
                            .color(theme::MUTED)
                            .size(12.0),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Install now")
                                        .color(theme::INK)
                                        .family(theme::medium()),
                                )
                                .fill(theme::ACCENT),
                            )
                            .clicked()
                        {
                            self.start_update_download();
                        }
                        if ui.button("Later").clicked() {
                            let _ = self.engine.snooze_update(&update.version);
                            self.update_modal = None;
                        }
                        if ui.link("View release").clicked() {
                            let _ = open::that(&update.html_url);
                        }
                    });
                }
                UpdateModal::Downloading { received, total } => {
                    let fraction = (received as f32 / total.max(1) as f32).clamp(0.0, 1.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Downloading installer")
                                .family(theme::medium())
                                .color(theme::ACCENT),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", fraction * 100.0))
                                    .monospace()
                                    .color(theme::TEXT),
                            );
                        });
                    });
                    ui.add_space(6.0);
                    theme::progress_bar(ui, fraction);
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "{} / {}",
                            format_file_size(received),
                            format_file_size(total)
                        ))
                        .color(theme::MUTED)
                        .size(12.0),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(update_restart_note())
                            .color(theme::MUTED)
                            .size(12.0),
                    );
                }
                UpdateModal::Installing => {
                    ui.label(
                        RichText::new("The installer is starting.")
                            .family(theme::medium())
                            .color(theme::ACCENT),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(update_restart_note())
                            .color(theme::MUTED)
                            .size(12.0),
                    );
                }
            }
        });
        if !open && !blocking {
            self.update_modal = None;
        }
    }

    fn export_flow_modal(&mut self, ctx: &egui::Context) {
        let Some(modal) = self.export_modal.clone() else {
            return;
        };
        let in_progress = matches!(modal, ExportModal::Working { .. });
        let title = match &modal {
            ExportModal::Working { .. } => "Exporting",
            ExportModal::Done { .. } => "Exported",
        };
        let mut open = true;
        let mut window = egui::Window::new(title)
            .id(egui::Id::new("export-flow"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO);
        if !in_progress {
            window = window.open(&mut open);
        }
        window.show(ctx, |ui| {
            ui.set_width(360.0);
            match &modal {
                ExportModal::Working {
                    quality_label,
                    progress,
                } => {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Exporting {quality_label}"))
                                .family(theme::medium())
                                .color(theme::ACCENT),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", progress * 100.0))
                                    .monospace()
                                    .color(theme::TEXT),
                            );
                        });
                    });
                    ui.add_space(6.0);
                    theme::progress_bar(ui, *progress as f32);
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("Keep this window open until the file is written.")
                            .color(theme::MUTED)
                            .size(12.0),
                    );
                }
                ExportModal::Done { path } => {
                    ui.label("Your clip was saved.");
                    ui.label(
                        RichText::new(path.display().to_string())
                            .small()
                            .color(theme::MUTED),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Show in folder")
                                        .color(theme::INK)
                                        .family(theme::medium()),
                                )
                                .fill(theme::ACCENT),
                            )
                            .clicked()
                        {
                            let reveal = path
                                .parent()
                                .filter(|parent| !parent.as_os_str().is_empty())
                                .unwrap_or(path);
                            let _ = open::that(reveal);
                        }
                        if ui.button("Done").clicked() {
                            self.export_modal = None;
                        }
                    });
                }
            }
        });
        if !open {
            self.export_modal = None;
        }
    }

    fn delete_clip_modal(&mut self, ctx: &egui::Context) {
        let Some(id) = self.pending_delete_clip.clone() else {
            return;
        };
        let name = self
            .clips
            .iter()
            .find(|clip| clip.id == id)
            .map(|clip| clip.name.clone())
            .unwrap_or_else(|| "this video".into());
        let mut open = true;
        egui::Window::new("Delete from device")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                ui.label(format!(
                    "Are you sure you want to permanently delete \"{name}\" from your device?"
                ));
                ui.label(
                    RichText::new(
                        format!(
                            "This deletes the original recording and removes its {APP_NAME} previews, exports, and library history. Published versions are not deleted."
                        ),
                    )
                    .color(theme::MUTED)
                    .size(12.5),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.pending_delete_clip = None;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Delete from device")
                                    .color(theme::INK)
                                    .family(theme::medium()),
                            )
                            .fill(theme::DANGER),
                        )
                        .clicked()
                    {
                        let engine = self.engine.clone();
                        self.pending_delete_clip = None;
                        if self.selected_id.as_deref() == Some(id.as_str()) {
                            self.selected_id = None;
                            self.editor = None;
                            self.time_edit = None;
                            self.bind_session_media(None);
                        }
                        self.run_async(async move {
                            engine.delete_clip(&id).await?;
                            Ok(Message::Refresh)
                        });
                    }
                });
            });
        if !open {
            self.pending_delete_clip = None;
        }
    }

    fn delete_version_modal(&mut self, ctx: &egui::Context) {
        let Some(id) = self.pending_delete_job.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Delete version")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                ui.label("Are you sure you want to delete this published version?");
                ui.label(
                    RichText::new("The public link will stop working and this cannot be undone.")
                        .color(theme::MUTED)
                        .size(12.5),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.pending_delete_job = None;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Delete version")
                                    .color(theme::INK)
                                    .family(theme::medium()),
                            )
                            .fill(theme::DANGER),
                        )
                        .clicked()
                    {
                        let engine = self.engine.clone();
                        self.pending_delete_job = None;
                        self.run_async(async move {
                            engine.delete_job(&id).await?;
                            Ok(Message::Refresh)
                        });
                    }
                });
            });
        if !open {
            self.pending_delete_job = None;
        }
    }

    fn reset_modal(&mut self, ctx: &egui::Context) {
        let Some(reset) = self.created_reset.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new(format!("Password reset for @{}", reset.username))
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(format!("Single use · expires {}", reset.expires_at));
                ui.text_edit_multiline(&mut reset.url.clone());
                if ui.button("Copy link").clicked() {
                    ui.ctx().copy_text(reset.url.clone());
                    self.set_notice("Private link copied.");
                }
            });
        if !open {
            self.created_reset = None;
        }
    }
}

fn update_restart_note() -> String {
    format!(
        "{APP_NAME} will close automatically once the update is installed. The new version takes effect the next time you launch."
    )
}

fn condensed_release_notes(notes: &str) -> String {
    let trimmed = notes.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let first = trimmed.lines().take(6).collect::<Vec<_>>().join("\n");
    if first.chars().count() > 360 {
        let mut short = first.chars().take(360).collect::<String>();
        short.push('…');
        short
    } else {
        first
    }
}

fn quality_choice_label(option: &PublishOption) -> String {
    let label = format!(
        "{}  ·  ~{}",
        option.quality_label(),
        format_file_size(option.estimated_bytes)
    );
    if option.heavier_than_1080p120() {
        format!("{label}  ⚠")
    } else {
        label
    }
}

fn default_clip_title(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Untitled clip")
        .to_string()
}

fn ensure_mp4_path(path: PathBuf) -> PathBuf {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("mp4"))
    {
        Some(true) => path,
        _ => path.with_extension("mp4"),
    }
}

fn publish_stage_label(job: &PublishJob) -> String {
    match job.status.as_str() {
        "queued" => "Queued".into(),
        "transcoding" => job
            .selection
            .as_ref()
            .and_then(|selection| selection.export.as_ref())
            .map(|profile| format!("Exporting {}p{}", profile.height, profile.fps))
            .unwrap_or_else(|| "Exporting".into()),
        "uploading" => "Uploading".into(),
        "complete" => "Published".into(),
        "failed" => "Failed".into(),
        _ => "Working".into(),
    }
}

fn clip_belongs_in_inbox(path: &Path, inbox: &Path, default_inbox: Option<&Path>) -> bool {
    if path.starts_with(inbox) {
        return true;
    }
    if let Some(default_inbox) = default_inbox {
        if path.starts_with(default_inbox) {
            return false;
        }
    }
    true
}

fn format_duration_compact(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as i64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let secs = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

fn format_time(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "0:00.000".into();
    }
    let minutes = (seconds / 60.0).floor() as i64;
    format!("{minutes}:{:06.3}", seconds % 60.0)
}

fn parse_time(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let parts = text.split(':').collect::<Vec<_>>();
    let seconds = match parts.as_slice() {
        [seconds] => seconds.parse::<f64>().ok()?,
        [minutes, seconds] => minutes.parse::<f64>().ok()? * 60.0 + seconds.parse::<f64>().ok()?,
        [hours, minutes, seconds] => {
            hours.parse::<f64>().ok()? * 3600.0
                + minutes.parse::<f64>().ok()? * 60.0
                + seconds.parse::<f64>().ok()?
        }
        _ => return None,
    };
    seconds.is_finite().then_some(seconds.max(0.0))
}

fn recorder_hotkey_from_event(
    key: egui::Key,
    physical_key: Option<egui::Key>,
    modifiers: egui::Modifiers,
) -> Option<Hotkey> {
    let key = physical_key
        .and_then(recorder_hotkey_key)
        .or_else(|| recorder_hotkey_key(key))?;
    let hotkey = Hotkey {
        key: key.into(),
        ctrl: modifiers.ctrl,
        alt: modifiers.alt,
        shift: modifiers.shift,
        meta: modifiers.mac_cmd,
    };
    hotkey.validate().ok().map(|()| hotkey)
}

fn recorder_hotkey_key(key: egui::Key) -> Option<&'static str> {
    use egui::Key::*;
    Some(match key {
        ArrowDown => "Down",
        ArrowLeft => "Left",
        ArrowRight => "Right",
        ArrowUp => "Up",
        Escape => "Escape",
        Tab => "Tab",
        Backspace => "Backspace",
        Enter => "Enter",
        Space => "Space",
        Insert => "Insert",
        Delete => "Delete",
        Home => "Home",
        End => "End",
        PageUp => "PageUp",
        PageDown => "PageDown",
        Colon => ";",
        Comma => ",",
        Backslash => "\\",
        Slash => "/",
        Pipe => "\\",
        Questionmark => "/",
        Exclamationmark => "1",
        OpenBracket => "[",
        CloseBracket => "]",
        OpenCurlyBracket => "[",
        CloseCurlyBracket => "]",
        Backtick => "`",
        Minus => "-",
        Period => ".",
        Plus | Equals => "=",
        Semicolon => ";",
        Quote => "'",
        Num0 => "0",
        Num1 => "1",
        Num2 => "2",
        Num3 => "3",
        Num4 => "4",
        Num5 => "5",
        Num6 => "6",
        Num7 => "7",
        Num8 => "8",
        Num9 => "9",
        A => "A",
        B => "B",
        C => "C",
        D => "D",
        E => "E",
        F => "F",
        G => "G",
        H => "H",
        I => "I",
        J => "J",
        K => "K",
        L => "L",
        M => "M",
        N => "N",
        O => "O",
        P => "P",
        Q => "Q",
        R => "R",
        S => "S",
        T => "T",
        U => "U",
        V => "V",
        W => "W",
        X => "X",
        Y => "Y",
        Z => "Z",
        F1 => "F1",
        F2 => "F2",
        F3 => "F3",
        F4 => "F4",
        F5 => "F5",
        F6 => "F6",
        F7 => "F7",
        F8 => "F8",
        F9 => "F9",
        F10 => "F10",
        F11 => "F11",
        F12 => "F12",
        F13 => "F13",
        F14 => "F14",
        F15 => "F15",
        F16 => "F16",
        F17 => "F17",
        F18 => "F18",
        F19 => "F19",
        F20 => "F20",
        F21 => "F21",
        F22 => "F22",
        F23 => "F23",
        F24 => "F24",
        IntlBackslash => "\\",
        _ => return None,
    })
}

fn reset_automatic_encoding(config: &mut RecorderConfig) {
    let defaults = RecorderConfig::default();
    config.mode = RecorderMode::Automatic;
    config.video_encoder = defaults.video_encoder;
    config.rate_control = defaults.rate_control;
    config.quality_level = defaults.quality_level;
    config.video_bitrate_kbps = defaults.video_bitrate_kbps;
    config.max_bitrate_kbps = defaults.max_bitrate_kbps;
    config.keyframe_interval_seconds = defaults.keyframe_interval_seconds;
    config.preset = defaults.preset;
    config.tuning = defaults.tuning;
    config.multipass = defaults.multipass;
    config.profile = defaults.profile;
    config.lookahead = defaults.lookahead;
    config.adaptive_quantization = defaults.adaptive_quantization;
    config.b_frames = defaults.b_frames;
    config.b_frame_ref_mode = defaults.b_frame_ref_mode;
    config.split_encode = defaults.split_encode;
    config.gpu = defaults.gpu;
    config.rescale_output = defaults.rescale_output;
    config.container_format = defaults.container_format;
    config.custom_encoder_options = defaults.custom_encoder_options;
    config.audio_encoder = defaults.audio_encoder;
    config.audio_bitrate_kbps = defaults.audio_bitrate_kbps;
}

fn encoder_supports_property(
    capabilities: &RecorderCapabilities,
    selected: &str,
    keys: &[&str],
) -> bool {
    let selected = selected.trim();
    let encoders = if selected.is_empty() || selected.eq_ignore_ascii_case("auto") {
        capabilities.video_encoders.iter().collect::<Vec<_>>()
    } else {
        capabilities
            .video_encoders
            .iter()
            .filter(|encoder| encoder.id == selected)
            .collect::<Vec<_>>()
    };
    encoders.iter().any(|encoder| {
        encoder.settings.is_empty()
            || keys
                .iter()
                .any(|key| encoder.settings.iter().any(|setting| setting.key == *key))
    })
}

fn encoder_setting_capability<'a>(
    capabilities: &'a RecorderCapabilities,
    selected: &str,
    keys: &[&str],
) -> Option<&'a EncoderSettingCapability> {
    let selected = selected.trim();
    if selected.is_empty() || selected.eq_ignore_ascii_case("auto") {
        return None;
    }
    capabilities
        .video_encoders
        .iter()
        .find(|encoder| encoder.id == selected)
        .and_then(|encoder| {
            keys.iter()
                .find_map(|key| encoder.settings.iter().find(|setting| setting.key == *key))
        })
}

fn audio_quality_settings(
    ui: &mut Ui,
    capabilities: &RecorderCapabilities,
    config: &mut RecorderConfig,
) {
    let (label_width, control_width) = settings_column_widths(ui);

    egui::Grid::new("recorder-audio-settings")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            encoder_settings_label(ui, "Audio encoder", label_width);
            encoder_settings_control_area(ui, |ui| {
                egui::ComboBox::from_id_salt("recorder-audio-encoder")
                    .width(control_width)
                    .selected_text(encoder_selected_label(
                        &config.audio_encoder,
                        &capabilities.audio_encoders,
                        true,
                    ))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut config.audio_encoder, "auto".into(), "Automatic");
                        for encoder in &capabilities.audio_encoders {
                            ui.selectable_value(
                                &mut config.audio_encoder,
                                encoder.id.clone(),
                                encoder_display_name(encoder),
                            );
                        }
                    });
            });
            ui.end_row();

            encoder_settings_label(ui, "Bitrate", label_width);
            encoder_settings_control_area(ui, |ui| {
                encoder_settings_add_sized(
                    ui,
                    [control_width, 22.0],
                    egui::DragValue::new(&mut config.audio_bitrate_kbps)
                        .range(1..=10_000)
                        .suffix(" kbps"),
                );
            });
            ui.end_row();
        });
}

fn settings_column_widths(ui: &Ui) -> (f32, f32) {
    let available_width = (ui.available_width() - 16.0).max(1.0);
    let label_width = 220.0_f32.min((available_width - 24.0).max(0.0) * 0.45);
    let control_width = (available_width - label_width - 24.0).max(1.0);
    (label_width, control_width)
}

fn advanced_encoder_settings(
    ui: &mut Ui,
    capabilities: &RecorderCapabilities,
    config: &mut RecorderConfig,
) {
    let video_encoder = config.video_encoder.clone();
    let supports = |keys: &[&str]| encoder_supports_property(capabilities, &video_encoder, keys);
    let available_width = (ui.available_width() - 16.0).max(1.0);
    let label_width = 220.0_f32.min((available_width - 24.0).max(0.0) * 0.45);
    let control_width = (available_width - label_width - 24.0).max(1.0);
    let layout = EncoderSettingsLayout {
        capabilities,
        selected: &video_encoder,
        label_width,
        control_width,
    };

    egui::Grid::new("recorder-encoder-settings")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            encoder_settings_label(ui, "Video encoder", label_width);
            let selected_encoder =
                encoder_selected_label(&config.video_encoder, &capabilities.video_encoders, false);
            encoder_settings_control_area(ui, |ui| {
                egui::ComboBox::from_id_salt("recorder-video-encoder")
                    .width(control_width)
                    .selected_text(selected_encoder)
                    .show_ui(ui, |ui| {
                        for encoder in &capabilities.video_encoders {
                            ui.selectable_value(
                                &mut config.video_encoder,
                                encoder.id.clone(),
                                encoder_display_name(encoder),
                            );
                        }
                    });
            });
            ui.end_row();

            if supports(&["rate_control", "rc"]) {
                encoder_settings_label(ui, "Rate control", label_width);
                encoder_settings_control_area(ui, |ui| {
                    egui::ComboBox::from_id_salt("recorder-rate-control")
                        .width(control_width)
                        .selected_text(config.rate_control.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut config.rate_control,
                                RateControl::Cbr,
                                RateControl::Cbr.label(),
                            );
                            ui.selectable_value(
                                &mut config.rate_control,
                                RateControl::Cqp,
                                RateControl::Cqp.label(),
                            );
                            ui.selectable_value(
                                &mut config.rate_control,
                                RateControl::Vbr,
                                RateControl::Vbr.label(),
                            );
                            ui.selectable_value(
                                &mut config.rate_control,
                                RateControl::Cqvbr,
                                RateControl::Cqvbr.label(),
                            );
                        });
                });
                ui.end_row();
            } else {
                encoder_settings_label(ui, "Rate control", label_width);
                encoder_settings_control_area(ui, |ui| {
                    encoder_settings_add_sized(
                        ui,
                        [control_width, 22.0],
                        egui::Label::new(
                            RichText::new(
                                "Managed by the active encoder (no compatible property reported).",
                            )
                            .color(theme::MUTED)
                            .size(12.0),
                        ),
                    );
                });
                ui.end_row();
            }

            match config.rate_control {
                RateControl::Cqp | RateControl::Cqvbr => {
                    if supports(&["target_quality", "cqp", "cq", "qp", "crf"]) {
                        encoder_settings_label(ui, "Constant QP", label_width);
                        encoder_settings_control_area(ui, |ui| {
                            encoder_settings_add_sized(
                                ui,
                                [control_width, 22.0],
                                egui::DragValue::new(&mut config.quality_level).range(1..=63),
                            );
                        });
                        ui.end_row();
                    }
                }
                RateControl::Cbr | RateControl::Vbr => {}
            }

            if matches!(
                config.rate_control,
                RateControl::Cbr | RateControl::Vbr | RateControl::Cqvbr
            ) && supports(&["bitrate", "bitrate_kbps"])
            {
                encoder_settings_label(ui, "Target bitrate", label_width);
                encoder_settings_control_area(ui, |ui| {
                    encoder_settings_add_sized(
                        ui,
                        [control_width, 22.0],
                        egui::DragValue::new(&mut config.video_bitrate_kbps)
                            .range(1..=1_000_000)
                            .suffix(" kbps"),
                    );
                });
                ui.end_row();
            }

            if matches!(config.rate_control, RateControl::Vbr | RateControl::Cqvbr)
                && supports(&["max_bitrate", "max_bitrate_kbps"])
            {
                encoder_settings_label(ui, "Maximum bitrate", label_width);
                encoder_settings_control_area(ui, |ui| {
                    encoder_settings_add_sized(
                        ui,
                        [control_width, 22.0],
                        egui::DragValue::new(&mut config.max_bitrate_kbps)
                            .range(0..=1_000_000)
                            .suffix(" kbps"),
                    );
                });
                ui.end_row();
            }

            if supports(&["keyint_sec", "keyframe_interval", "keyframe_interval_sec"]) {
                encoder_settings_label(ui, "Keyframe interval (seconds, 0 = auto)", label_width);
                encoder_settings_control_area(ui, |ui| {
                    encoder_settings_add_sized(
                        ui,
                        [control_width, 22.0],
                        egui::DragValue::new(&mut config.keyframe_interval_seconds)
                            .range(0..=60)
                            .suffix(" seconds"),
                    );
                });
                ui.end_row();
            }

            if supports(&["preset", "preset2"]) {
                layout.setting_control(
                    ui,
                    "Preset",
                    "recorder-preset",
                    &mut config.preset,
                    &["preset", "preset2"],
                );
            }
            if supports(&["tune", "tuning"]) {
                layout.setting_control(
                    ui,
                    "Tuning",
                    "recorder-tuning",
                    &mut config.tuning,
                    &["tune", "tuning"],
                );
            }
            if supports(&["multipass", "multi_pass"]) {
                encoder_settings_label(ui, "Multipass Mode", label_width);
                encoder_settings_control_area(ui, |ui| {
                    egui::ComboBox::from_id_salt("recorder-multipass")
                        .width(control_width)
                        .selected_text(config.multipass.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut config.multipass,
                                Multipass::Disabled,
                                Multipass::Disabled.label(),
                            );
                            ui.selectable_value(
                                &mut config.multipass,
                                Multipass::QuarterResolution,
                                Multipass::QuarterResolution.label(),
                            );
                            ui.selectable_value(
                                &mut config.multipass,
                                Multipass::FullResolution,
                                Multipass::FullResolution.label(),
                            );
                        });
                });
                ui.end_row();
            }
            if supports(&["profile"]) {
                layout.setting_control(
                    ui,
                    "Profile",
                    "recorder-profile",
                    &mut config.profile,
                    &["profile"],
                );
            }

            if supports(&["lookahead", "rc-lookahead", "look_ahead"]) {
                encoder_settings_label(ui, "", label_width);
                encoder_settings_control_area(ui, |ui| {
                    encoder_settings_add_sized(
                        ui,
                        [control_width, 22.0],
                        egui::Checkbox::new(&mut config.lookahead, "Look-ahead"),
                    );
                });
                ui.end_row();
            }
            if supports(&["adaptive_quantization", "spatial-aq", "spatial_aq"]) {
                encoder_settings_label(ui, "", label_width);
                encoder_settings_control_area(ui, |ui| {
                    encoder_settings_add_sized(
                        ui,
                        [control_width, 22.0],
                        egui::Checkbox::new(
                            &mut config.adaptive_quantization,
                            "Adaptive quantization",
                        ),
                    );
                });
                ui.end_row();
            }

            if supports(&["bframes", "bf", "b_frames"]) {
                encoder_settings_label(ui, "B-Frames", label_width);
                encoder_settings_control_area(ui, |ui| {
                    encoder_settings_add_sized(
                        ui,
                        [control_width, 22.0],
                        egui::DragValue::new(&mut config.b_frames).range(0..=8),
                    );
                });
                ui.end_row();
            }
            if supports(&["bf_ref_mode", "b_ref_mode", "bframe_ref_mode"]) {
                layout.setting_control(
                    ui,
                    "B-Frame as Reference",
                    "recorder-b-frame-reference",
                    &mut config.b_frame_ref_mode,
                    &["bf_ref_mode", "b_ref_mode", "bframe_ref_mode"],
                );
            }
            if supports(&["split_encode", "split-encode"]) {
                layout.setting_control(
                    ui,
                    "Split Encode",
                    "recorder-split-encode",
                    &mut config.split_encode,
                    &["split_encode", "split-encode"],
                );
            }
            if supports(&["gpu", "device"]) {
                encoder_settings_label(ui, "GPU", label_width);
                encoder_settings_control_area(ui, |ui| {
                    encoder_settings_add_sized(
                        ui,
                        [control_width, 22.0],
                        egui::DragValue::new(&mut config.gpu).range(0..=32),
                    );
                });
                ui.end_row();
            }
            if supports(&["rescale"]) {
                encoder_settings_label(ui, "", label_width);
                encoder_settings_control_area(ui, |ui| {
                    encoder_settings_add_sized(
                        ui,
                        [control_width, 22.0],
                        egui::Checkbox::new(&mut config.rescale_output, "Encoder rescale"),
                    );
                });
                ui.end_row();
            }

            encoder_settings_label(ui, "Container", label_width);
            encoder_settings_control_area(ui, |ui| {
                egui::ComboBox::from_id_salt("recorder-container")
                    .width(control_width)
                    .selected_text(config.container_format.to_uppercase())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut config.container_format,
                            "mkv".into(),
                            "MKV (recommended)",
                        );
                        ui.selectable_value(&mut config.container_format, "mp4".into(), "MP4");
                    });
            });
            ui.end_row();

            encoder_settings_label(ui, "Custom Encoder Options", label_width);
            encoder_settings_control_area(ui, |ui| {
                encoder_settings_add_sized(
                    ui,
                    [control_width, 72.0],
                    egui::TextEdit::multiline(&mut config.custom_encoder_options)
                        .desired_rows(3)
                        .hint_text("Encoder default"),
                );
            });
            ui.end_row();
        });
    ui.label(
        RichText::new(
            "MKV keeps replay files recoverable after an interruption and preserves separate audio tracks.",
        )
        .color(theme::MUTED)
        .size(12.0),
    );
}

fn encoder_settings_label(ui: &mut Ui, label: &str, width: f32) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.add_sized([width, 22.0], egui::Label::new(label).halign(Align::Max));
    });
}

fn encoder_settings_control_area(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.with_layout(Layout::left_to_right(Align::Center), add);
}

fn encoder_settings_add_sized(
    ui: &mut Ui,
    size: [f32; 2],
    widget: impl egui::Widget,
) -> egui::Response {
    ui.allocate_ui_with_layout(
        size.into(),
        Layout::left_to_right(Align::Center)
            .with_main_align(Align::Min)
            .with_main_justify(true),
        |ui| ui.add(widget),
    )
    .inner
}

struct EncoderSettingsLayout<'a> {
    capabilities: &'a RecorderCapabilities,
    selected: &'a str,
    label_width: f32,
    control_width: f32,
}

impl EncoderSettingsLayout<'_> {
    fn setting_control(
        &self,
        ui: &mut Ui,
        label: &str,
        id: &str,
        value: &mut String,
        keys: &[&str],
    ) {
        encoder_settings_label(ui, label, self.label_width);
        let setting = encoder_setting_capability(self.capabilities, self.selected, keys);
        if let Some(setting) = setting.filter(|setting| !setting.options.is_empty()) {
            let selected_text = setting
                .options
                .iter()
                .enumerate()
                .find_map(|(index, option)| {
                    (value.eq_ignore_ascii_case(option)
                        || setting
                            .option_values
                            .get(index)
                            .is_some_and(|native| value.eq_ignore_ascii_case(native)))
                    .then(|| option.clone())
                })
                .unwrap_or_else(|| value.clone());
            encoder_settings_control_area(ui, |ui| {
                egui::ComboBox::from_id_salt(id)
                    .width(self.control_width)
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        for (index, option) in setting.options.iter().enumerate() {
                            let native_value = setting
                                .option_values
                                .get(index)
                                .cloned()
                                .unwrap_or_else(|| option.clone());
                            ui.selectable_value(value, native_value, option);
                        }
                    });
            });
        } else {
            encoder_settings_control_area(ui, |ui| {
                encoder_settings_add_sized(
                    ui,
                    [self.control_width, 22.0],
                    egui::TextEdit::singleline(value).hint_text("Encoder default"),
                );
            });
        }
        ui.end_row();
    }
}

fn encoder_display_name(encoder: &EncoderCapability) -> String {
    if encoder.hardware {
        format!("{} (hardware)", encoder.label)
    } else {
        encoder.label.clone()
    }
}

fn encoder_selected_label(
    selected: &str,
    encoders: &[EncoderCapability],
    allow_automatic: bool,
) -> String {
    if selected.trim().is_empty() || selected.eq_ignore_ascii_case("auto") {
        if allow_automatic {
            "Automatic".into()
        } else {
            "Choose an encoder".into()
        }
    } else {
        encoders
            .iter()
            .find(|encoder| encoder.id == selected)
            .map(encoder_display_name)
            .unwrap_or_else(|| selected.into())
    }
}

fn rational_from_decimal(value: f64) -> Rational {
    let scaled = (value.clamp(1.0, 1_000.0) * 1_000.0).round() as u32;
    Rational::new(scaled, 1_000)
}

fn audio_track_selector(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    track: &mut u8,
    width: f32,
) {
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(format!("Track {}", *track))
        .show_ui(ui, |ui| {
            for option in 1_u8..=6 {
                ui.selectable_value(track, option, format!("Track {option}"));
            }
        });
}

fn ensure_default_audio_routes(config: &mut RecorderConfig, sources: &[AudioSourceCapability]) {
    for source in sources.iter().filter(|source| {
        source.available
            && matches!(
                source.kind,
                AudioSourceKind::System | AudioSourceKind::Microphone
            )
    }) {
        if config
            .audio_routes
            .iter()
            .all(|route| route.source_id != source.id)
        {
            let track = next_audio_track(config);
            config.audio_routes.push(AudioRoute {
                source_id: source.id.clone(),
                track,
                track_name: source.label.clone(),
                enabled: true,
            });
        }
    }
}

fn ensure_audio_route_names(config: &mut RecorderConfig, sources: &[AudioSourceCapability]) {
    for route in &mut config.audio_routes {
        if !route.track_name.trim().is_empty() {
            continue;
        }
        route.track_name = sources
            .iter()
            .find(|source| source.id == route.source_id)
            .map(|source| source.label.clone())
            .unwrap_or_else(|| fallback_audio_route_name(&route.source_id, route.track));
    }
}

fn ensure_playback_device_labels(capabilities: &mut RecorderCapabilities) {
    for source in capabilities
        .audio_sources
        .iter_mut()
        .filter(|source| source.kind == AudioSourceKind::PlaybackDevice)
    {
        if !source.label.trim().is_empty() {
            continue;
        }
        let endpoint_id = source
            .id
            .strip_prefix("playback:")
            .map(str::trim)
            .filter(|endpoint_id| !endpoint_id.is_empty());
        source.label = endpoint_id
            .map(|endpoint_id| format!("Playback endpoint ({endpoint_id})"))
            .unwrap_or_else(|| "Playback device".into());
    }
}

fn fallback_audio_route_name(source_id: &str, track: u8) -> String {
    if let Some(selector) = source_id.strip_prefix("application:") {
        if !selector.trim().is_empty() {
            return selector.trim().to_string();
        }
    }
    if source_id.starts_with("playback:") {
        return "Playback device".into();
    }
    if source_id.starts_with("system:") {
        return "System audio".into();
    }
    if source_id.starts_with("microphone:") {
        return "Default microphone".into();
    }
    format!("Track {track}")
}

fn next_audio_track(config: &RecorderConfig) -> u8 {
    (1_u8..=6)
        .find(|track| {
            !config
                .audio_routes
                .iter()
                .any(|route| route.track == *track)
        })
        .unwrap_or(6)
}

fn files_being_dropped(ctx: &egui::Context) -> bool {
    ctx.input(|input| !input.raw.hovered_files.is_empty())
}

fn is_importable(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mkv" | "mp4" | "mov" | "webm" | "avi" | "m4v"
            )
        })
}

fn collect_import_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                for entry in entries.flatten() {
                    let child = entry.path();
                    if child.is_file() && is_importable(&child) {
                        files.push(child);
                    }
                }
            }
        } else if is_importable(&path) {
            files.push(path);
        }
    }
    files
}

fn is_cli_flag(arg: &OsStr) -> bool {
    arg.to_str().is_some_and(|value| {
        value.starts_with('-') || matches!(value, "/S" | "/NS" | "/UPDATE" | "/P" | "/R")
    })
}

fn media_paths_from_args(args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Vec<PathBuf> {
    collect_import_paths(
        args.into_iter()
            .skip(1)
            .map(|arg| arg.as_ref().to_os_string())
            .filter(|arg| !is_cli_flag(arg))
            .map(PathBuf::from),
    )
}

fn expiry_label(expires_at: Option<&str>) -> String {
    let Some(expires_at) = expires_at else {
        return "Expiry pending".into();
    };
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(expires_at) else {
        return expires_at.into();
    };
    let milliseconds = parsed.timestamp_millis() - chrono::Utc::now().timestamp_millis();
    if milliseconds <= 0 {
        return "Expired".into();
    }
    let days = (milliseconds as f64 / 86_400_000.0).ceil() as i64;
    format!("Expires in {days} day{}", if days == 1 { "" } else { "s" })
}

fn track_name(track: &clip_engine_core::AudioTrack) -> String {
    if let Some(title) = track
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return title.to_string();
    }
    if let Some(language) = track
        .language
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("und"))
    {
        return language.to_string();
    }
    format!("Audio {}", track.ordinal + 1)
}

#[cfg(test)]
mod tests {
    use super::{is_cli_flag, media_paths_from_args, recorder_hotkey_from_event};
    use eframe::egui::{Key, Modifiers};
    use std::ffi::OsStr;
    use std::path::PathBuf;

    #[test]
    fn cli_flags_are_detected() {
        assert!(is_cli_flag(OsStr::new("--help")));
        assert!(is_cli_flag(OsStr::new("/S")));
        assert!(is_cli_flag(OsStr::new("/UPDATE")));
        assert!(!is_cli_flag(OsStr::new(r"C:\clips\round.mkv")));
        assert!(!is_cli_flag(OsStr::new("round.mp4")));
    }

    #[test]
    fn startup_args_keep_video_paths() {
        let paths = media_paths_from_args([
            "clip-engine",
            "/S",
            r"D:\Videos\highlight.mkv",
            "--ignored",
            "notes.txt",
            "take.mp4",
        ]);
        assert_eq!(
            paths,
            vec![
                PathBuf::from(r"D:\Videos\highlight.mkv"),
                PathBuf::from("take.mp4"),
            ]
        );
    }

    #[test]
    fn recorder_hotkey_listener_captures_key_and_modifiers() {
        let hotkey = recorder_hotkey_from_event(
            Key::S,
            Some(Key::S),
            Modifiers {
                ctrl: true,
                alt: true,
                shift: true,
                ..Modifiers::default()
            },
        )
        .unwrap();

        assert_eq!(hotkey.key, "S");
        assert!(hotkey.ctrl);
        assert!(hotkey.alt);
        assert!(hotkey.shift);
        assert!(!hotkey.meta);
        assert_eq!(hotkey.to_string(), "Ctrl+Alt+Shift+S");
    }

    #[test]
    fn recorder_hotkey_listener_uses_physical_punctuation_key() {
        let hotkey = recorder_hotkey_from_event(
            Key::Plus,
            Some(Key::Equals),
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        )
        .unwrap();

        assert_eq!(hotkey.key, "=");
        assert!(hotkey.shift);
    }

    #[test]
    fn recorder_hotkey_listener_ignores_modifier_only_and_unsupported_keys() {
        assert!(recorder_hotkey_from_event(
            Key::ControlLeft,
            Some(Key::ControlLeft),
            Modifiers::CTRL,
        )
        .is_none());
        assert!(
            recorder_hotkey_from_event(Key::F25, Some(Key::F25), Modifiers::default(),).is_none()
        );
    }
}
