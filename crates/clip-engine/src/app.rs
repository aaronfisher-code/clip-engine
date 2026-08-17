use crate::player::Player;
use crate::theme;
use clip_engine_core::cloud::{AccessRequest, AdminUser, CloudClip, CloudUser, PasswordReset};
use clip_engine_core::models::{AppConfig, Clip, PublishJob, Selection};
use clip_engine_core::paths::{default_inbox_dir, path_is_within, video_dir};
use clip_engine_core::{
    export_options, format_file_size, install_desktop_update, safe_base_name, AvailableUpdate,
    Engine, PublishOption, APP_NAME, PRODUCT_NAME,
};
use eframe::egui::{
    self, Align, Color32, ColorImage, CornerRadius, CursorIcon, Layout, Pos2, Rect, RichText,
    Sense, StrokeKind, TextureHandle, TextureOptions, Ui, UiBuilder, Vec2,
};
use raw_window_handle::HasDisplayHandle;
use std::collections::HashMap;
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
    timeline_drag: Option<TimelineDrag>,
    time_edit: Option<TimeEdit>,
    drop_hovering: bool,
    available_update: Option<AvailableUpdate>,
    update_modal: Option<UpdateModal>,
    update_checking: bool,
    show_pending: bool,
}

impl ClipApp {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: Engine) -> Self {
        theme::apply(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        let config = engine.config().unwrap_or_else(|_| AppConfig {
            source_directory: String::new(),
            audio_track_labels: vec![
                "Game / System".into(),
                "Discord".into(),
                "Microphone".into(),
            ],
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
        });
        let clips = engine.clips().unwrap_or_default();
        let jobs = engine.jobs().unwrap_or_default();
        let selected_id = None;
        for clip in &clips {
            if Path::new(&clip.source_path).is_file() {
                let _ = engine.prepare_preview(&clip.id, false);
            }
        }
        let clips = engine.clips().unwrap_or(clips);
        let player = match Player::new(&cc.egui_ctx, cc.gl.is_some(), cc.display_handle().ok()) {
            Ok(player) => Some(player),
            Err(error) => {
                let mut app = Self {
                    player: None,
                    player_error: Some(error.to_string()),
                    engine,
                    config,
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
                    error: None,
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
                    time_edit: None,
                    drop_hovering: false,
                    available_update: None,
                    update_modal: None,
                    update_checking: false,
                    show_pending: false,
                };
                app.schedule_update_check(false);
                app.import_startup_files();
                return app;
            }
        };
        let mut app = Self {
            engine,
            player,
            config: config.clone(),
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
            error: None,
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
            time_edit: None,
            drop_hovering: false,
            available_update: None,
            update_modal: None,
            update_checking: false,
            show_pending: false,
        };
        app.bootstrap_session();
        app.ensure_valid_selection();
        app.schedule_update_check(false);
        app.import_startup_files();
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
                    self.update_checking = false;
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
                        match install_desktop_update(&path, update.package) {
                            Ok(()) => std::process::exit(0),
                            Err(error) => {
                                self.update_modal = Some(UpdateModal::Prompt);
                                self.set_error(error.to_string());
                            }
                        }
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
        let default_inbox = default_inbox_dir().ok();
        self.clips
            .iter()
            .filter(|clip| {
                clip_belongs_in_inbox(
                    Path::new(&clip.source_path),
                    &inbox,
                    default_inbox.as_deref(),
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

impl eframe::App for ClipApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump();
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
        self.ingest_dropped_files(ctx);
        if self.drop_hovering {
            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("topbar")
            .frame(theme::top_frame())
            .show(ctx, |ui| {
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
                        });
                    }
                }
            });

        if self.library_open {
            let library_width = (ctx.screen_rect().width() * 0.24).clamp(280.0, 400.0);
            egui::SidePanel::left("library")
                .resizable(true)
                .default_width(library_width)
                .min_width(240.0)
                .max_width(480.0)
                .frame(theme::side_frame())
                .show(ctx, |ui| {
                    self.library_panel(ui);
                });
        }

        egui::CentralPanel::default()
            .frame(theme::central_frame())
            .show(ctx, |ui| {
                self.status_banner(ui);
                if let Some(clip_id) = self.selected_id.clone() {
                    if let Some(index) = self.clips.iter().position(|clip| clip.id == clip_id) {
                        let clip = self.clips[index].clone();
                        self.editor_panel(ui, ctx, &clip);
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
            });

        if self.show_auth.is_some() {
            self.auth_modal(ctx);
        }
        if self.show_pending
            && self.access_request.is_some()
            && !self.config.authenticated
            && self.show_auth.is_none()
        {
            self.pending_modal(ctx);
        }
        if self.show_access {
            self.access_modal(ctx);
        }
        if self.created_reset.is_some() {
            self.reset_modal(ctx);
        }
        if self.pending_delete_job.is_some() {
            self.delete_version_modal(ctx);
        }
        if self.pending_delete_clip.is_some() {
            self.delete_clip_modal(ctx);
        }
        if self.publish_modal.is_some() {
            self.publish_flow_modal(ctx);
        }
        if self.export_modal.is_some() {
            self.export_flow_modal(ctx);
        }
        if self.update_modal.is_some() {
            self.update_flow_modal(ctx);
        }

        if self.drop_hovering {
            theme::window_drop_overlay(ctx);
        }

        self.stop_at_out_point();

        if self
            .player
            .as_ref()
            .is_some_and(|player| player.playing() || player.buffering() || player.wants_redraw())
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
}

impl ClipApp {
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
                        RichText::new("Downloaded from the published GitHub Release.")
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
        if response.clicked() {
            self.selected_id = Some(clip.id.clone());
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
                    file.path
                        .clone()
                        .or_else(|| (!file.name.is_empty()).then(|| PathBuf::from(&file.name)))
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
        if !Path::new(&clip.source_path).is_file() {
            self.bind_session_media(None);
            ui.centered_and_justified(|ui| {
                theme::card().show(ui, |ui| {
                    ui.set_max_width(460.0);
                    ui.label(
                        RichText::new("Recording is missing from disk")
                            .family(theme::medium())
                            .size(18.0),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(clip.source_path.clone())
                            .color(theme::MUTED)
                            .small(),
                    );
                });
            });
            return;
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

        let time = self.playback_time();
        if !ctx.wants_keyboard_input() {
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
        let time = self.playback_time();
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
        if let Some(player) = &mut self.player {
            if loaded {
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
        let time = self.playback_time();
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
                            let label = track_name(track, &self.config.audio_track_labels);
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
                            egui::Button::new("Remove from library"),
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

        let dragging = self.timeline_drag;
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
            self.timeline_drag = kind.or(Some(TimelineDrag::Playhead));
        }

        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                self.apply_timeline_drag(time_at(pos.x), duration);
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
            self.timeline_drag = None;
        }
    }

    fn apply_timeline_drag(&mut self, time: f64, duration: f64) {
        match self.timeline_drag.unwrap_or(TimelineDrag::Playhead) {
            TimelineDrag::In => {
                let start = self.editor.as_mut().map(|editor| {
                    editor.start = time.min(editor.end - 0.05).max(0.0);
                    editor.start
                });
                if let Some(start) = start {
                    self.seek_preview(start);
                }
            }
            TimelineDrag::Out => {
                let end = self.editor.as_mut().map(|editor| {
                    editor.end = time.max(editor.start + 0.05).min(duration);
                    editor.end
                });
                if let Some(end) = end {
                    self.seek_preview(end);
                }
            }
            TimelineDrag::Playhead => {
                self.seek_preserving_play_state(time);
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
                }
                UpdateModal::Installing => {
                    ui.label(
                        RichText::new(format!("The installer is starting. {APP_NAME} will close."))
                            .color(theme::MUTED),
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
        egui::Window::new("Remove from library")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                ui.label(format!(
                    "Are you sure you want to remove \"{name}\" from the library?"
                ));
                ui.label(
                    RichText::new(
                        format!(
                            "This only removes it from {APP_NAME}. The original recording on disk is not deleted."
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
                                RichText::new("Remove from library")
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
    if !path.is_file() {
        return false;
    }
    if path_is_within(path, inbox) {
        return true;
    }
    if let Some(default_inbox) = default_inbox {
        if path_is_within(path, default_inbox) {
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

fn track_name(track: &clip_engine_core::AudioTrack, labels: &[String]) -> String {
    if let Some(title) = track
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let generic = title.eq_ignore_ascii_case("audio")
            || title.to_ascii_lowercase().starts_with("track")
            || title.to_ascii_lowercase().starts_with("audio track");
        if !generic {
            return title.to_string();
        }
    }
    if let Some(label) = labels
        .get(track.ordinal as usize)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        return label.to_string();
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
    use super::{is_cli_flag, media_paths_from_args};
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
}
