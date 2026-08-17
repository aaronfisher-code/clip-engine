use eframe::egui::{self, ColorImage, PaintCallback, Rect, TextureOptions};
use eframe::egui_glow::CallbackFn;
use glow::HasContext;
use raw_window_handle::{DisplayHandle, RawDisplayHandle};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MPV_FORMAT_FLAG: i32 = 3;
const MPV_FORMAT_DOUBLE: i32 = 5;
const MPV_RENDER_PARAM_INVALID: u32 = 0;
const MPV_RENDER_PARAM_API_TYPE: u32 = 1;
const MPV_RENDER_PARAM_OPENGL_INIT_PARAMS: u32 = 2;
const MPV_RENDER_PARAM_OPENGL_FBO: u32 = 3;
const MPV_RENDER_PARAM_FLIP_Y: u32 = 4;
const MPV_RENDER_PARAM_X11_DISPLAY: u32 = 8;
const MPV_RENDER_PARAM_WL_DISPLAY: u32 = 9;
const MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME: u32 = 12;
const MPV_RENDER_PARAM_SW_SIZE: u32 = 17;
const MPV_RENDER_PARAM_SW_FORMAT: u32 = 18;
const MPV_RENDER_PARAM_SW_STRIDE: u32 = 19;
const MPV_RENDER_PARAM_SW_POINTER: u32 = 20;
const MPV_RENDER_UPDATE_FRAME: u64 = 1;

#[repr(C)]
struct MpvHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct MpvRenderContext {
    _private: [u8; 0],
}

#[repr(C)]
struct MpvRenderParam {
    type_: u32,
    data: *mut c_void,
}

#[repr(C)]
struct MpvOpenglInitParams {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
struct MpvOpenglFbo {
    fbo: i32,
    w: i32,
    h: i32,
    internal_format: i32,
}

#[repr(C)]
struct MpvEvent {
    event_id: i32,
    error: i32,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventEndFile {
    reason: c_int,
    error: c_int,
}

const MPV_EVENT_NONE: i32 = 0;
const MPV_EVENT_END_FILE: i32 = 7;
const MPV_EVENT_PLAYBACK_RESTART: i32 = 21;
const MPV_END_FILE_REASON_ERROR: i32 = 4;

#[link(name = "mpv")]
unsafe extern "C" {
    fn mpv_create() -> *mut MpvHandle;
    fn mpv_initialize(ctx: *mut MpvHandle) -> c_int;
    fn mpv_terminate_destroy(ctx: *mut MpvHandle);
    fn mpv_set_option_string(
        ctx: *mut MpvHandle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int;
    fn mpv_command(ctx: *mut MpvHandle, args: *mut *const c_char) -> c_int;
    fn mpv_set_property_string(
        ctx: *mut MpvHandle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int;
    fn mpv_set_property(
        ctx: *mut MpvHandle,
        name: *const c_char,
        format: i32,
        data: *mut c_void,
    ) -> c_int;
    fn mpv_get_property(
        ctx: *mut MpvHandle,
        name: *const c_char,
        format: i32,
        data: *mut c_void,
    ) -> c_int;
    fn mpv_get_property_string(ctx: *mut MpvHandle, name: *const c_char) -> *mut c_char;
    fn mpv_free(data: *mut c_void);
    fn mpv_error_string(error: c_int) -> *const c_char;
    fn mpv_wait_event(ctx: *mut MpvHandle, timeout: f64) -> *mut MpvEvent;
    fn mpv_render_context_create(
        res: *mut *mut MpvRenderContext,
        mpv: *mut MpvHandle,
        params: *mut MpvRenderParam,
    ) -> c_int;
    fn mpv_render_context_free(ctx: *mut MpvRenderContext);
    fn mpv_render_context_render(ctx: *mut MpvRenderContext, params: *mut MpvRenderParam) -> c_int;
    fn mpv_render_context_update(ctx: *mut MpvRenderContext) -> u64;
    fn mpv_render_context_report_swap(ctx: *mut MpvRenderContext);
    fn mpv_render_context_set_update_callback(
        ctx: *mut MpvRenderContext,
        callback: unsafe extern "C" fn(*mut c_void),
        callback_ctx: *mut c_void,
    );
}

#[cfg(unix)]
#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetProcAddress(procname: *const c_char) -> *mut c_void;
}

#[cfg(windows)]
#[link(name = "opengl32")]
unsafe extern "system" {
    fn wglGetProcAddress(proc: *const c_char) -> *mut c_void;
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, proc: *const c_char) -> *mut c_void;
}

struct GlTarget {
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    width: i32,
    height: i32,
}

struct RenderShared {
    render: AtomicPtr<MpvRenderContext>,
    target: Mutex<Option<GlTarget>>,
    awaiting_frame: AtomicBool,
    awaiting_seek_frame: AtomicBool,
    seek_restarted: AtomicBool,
    resume_after_seek: AtomicBool,
    pending_play: AtomicBool,
}

enum VideoBackend {
    OpenGl,
    Software,
}

pub struct Player {
    handle: *mut MpvHandle,
    render: *mut MpvRenderContext,
    shared: Arc<RenderShared>,
    callback: Option<Arc<CallbackFn>>,
    backend: VideoBackend,
    sw_pixels: Vec<u8>,
    sw_size: [i32; 2],
    sw_texture: Option<egui::TextureHandle>,
    update_ctx: *mut egui::Context,
    loaded_path: Option<String>,
    pending_seek: Option<f64>,
    last_issued_seek: Option<f64>,
    last_seek: Instant,
    seek_in_flight: bool,
    load_started: Option<Instant>,
    error: Option<String>,
    audio_signature: Option<Vec<i64>>,
}

unsafe impl Send for Player {}

impl Player {
    pub fn new(
        ctx: &egui::Context,
        opengl_available: bool,
        display: Option<DisplayHandle<'_>>,
    ) -> anyhow::Result<Self> {
        unsafe {
            let handle = mpv_create();
            if handle.is_null() {
                anyhow::bail!("Could not create the libmpv player");
            }
            set_option(handle, "vo", "libmpv")?;
            if opengl_available {
                set_option(handle, "hwdec", "auto-copy-safe")
                    .or_else(|_| set_option(handle, "hwdec", "auto-copy"))
                    .or_else(|_| set_option(handle, "hwdec", "auto-safe"))?;
                let _ = set_option(handle, "gpu-hwdec-interop", "auto");
            } else {
                let _ = set_option(handle, "hwdec", "no");
            }
            set_option(handle, "pause", "yes")?;
            set_option(handle, "hr-seek", "always")?;
            set_option(handle, "keep-open", "yes")?;
            set_option(handle, "idle", "yes")?;
            set_option(handle, "osc", "no")?;
            set_option(handle, "osd-level", "0")?;
            set_option(handle, "input-default-bindings", "no")?;
            set_option(handle, "input-vo-keyboard", "no")?;
            set_option(handle, "audio-display", "no")?;
            set_option(handle, "interpolation", "no")?;
            set_option(handle, "video-sync", "display-desync")?;
            set_option(handle, "video-timing-offset", "0")?;
            set_option(handle, "vd-lavc-dr", "no")?;
            set_option(handle, "framedrop", "no")?;
            set_option(handle, "hr-seek-framedrop", "no")?;
            set_option(handle, "rebase-start-time", "yes")?;
            set_option(handle, "cache", "auto")?;
            set_option(handle, "cache-pause", "yes")?;
            let _ = set_option(handle, "terminal", "no");
            let _ = set_option(handle, "msg-level", "all=error");
            let _ = set_option(handle, "mute", "no");
            let _ = set_option(handle, "volume", "100");
            if cfg!(windows) {
                let _ = set_option(handle, "ao", "wasapi");
            } else {
                let _ = set_option(handle, "ao", "pipewire,pulse,alsa");
            }
            check(mpv_initialize(handle))?;

            let mut backend = VideoBackend::Software;
            let mut render = ptr::null_mut();
            if opengl_available {
                if let Ok(created) = create_opengl_render(handle, display) {
                    render = created;
                    backend = VideoBackend::OpenGl;
                }
            }
            if render.is_null() {
                let _ = set_property(handle, "hwdec", "no");
                render = create_software_render(handle)?;
                backend = VideoBackend::Software;
            }

            let update_ctx = Box::into_raw(Box::new(ctx.clone()));
            mpv_render_context_set_update_callback(render, on_mpv_update, update_ctx.cast());

            let shared = Arc::new(RenderShared {
                render: AtomicPtr::new(render),
                target: Mutex::new(None),
                awaiting_frame: AtomicBool::new(false),
                awaiting_seek_frame: AtomicBool::new(false),
                seek_restarted: AtomicBool::new(false),
                resume_after_seek: AtomicBool::new(false),
                pending_play: AtomicBool::new(false),
            });
            let callback = matches!(backend, VideoBackend::OpenGl).then(|| {
                let paint_shared = shared.clone();
                Arc::new(CallbackFn::new(move |info, painter| {
                    paint_video(&paint_shared, info, painter);
                }))
            });

            Ok(Self {
                handle,
                render,
                shared,
                callback,
                backend,
                sw_pixels: Vec::new(),
                sw_size: [0, 0],
                sw_texture: None,
                update_ctx,
                loaded_path: None,
                pending_seek: None,
                last_issued_seek: None,
                last_seek: Instant::now() - Duration::from_secs(1),
                seek_in_flight: false,
                load_started: None,
                error: None,
                audio_signature: None,
            })
        }
    }

    pub fn paint(&mut self, ui: &mut egui::Ui, rect: Rect) {
        match self.backend {
            VideoBackend::OpenGl => {
                if let Some(callback) = &self.callback {
                    ui.painter().add(PaintCallback {
                        rect,
                        callback: callback.clone(),
                    });
                }
            }
            VideoBackend::Software => self.paint_software(ui, rect),
        }
    }

    fn paint_software(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let width = (rect.width().round() as i32).clamp(2, 1920);
        let height = (rect.height().round() as i32).clamp(2, 1080);
        let stride = (width as usize) * 4;
        if self.sw_size != [width, height] {
            self.sw_size = [width, height];
            self.sw_pixels.resize(stride * height as usize, 0);
        }
        unsafe {
            let flags = mpv_render_context_update(self.render);
            let has_frame = flags & MPV_RENDER_UPDATE_FRAME != 0;
            let seek_ready = self.shared.seek_restarted.load(Ordering::SeqCst);
            let force_render =
                self.shared.awaiting_seek_frame.load(Ordering::Relaxed) && seek_ready;
            if has_frame || force_render || self.sw_texture.is_none() {
                let mut size = [width, height];
                let mut stride_value = stride;
                let format = CString::new("rgba").expect("rgba");
                let mut params = [
                    MpvRenderParam {
                        type_: MPV_RENDER_PARAM_SW_SIZE,
                        data: size.as_mut_ptr().cast(),
                    },
                    MpvRenderParam {
                        type_: MPV_RENDER_PARAM_SW_FORMAT,
                        data: format.as_ptr().cast_mut().cast(),
                    },
                    MpvRenderParam {
                        type_: MPV_RENDER_PARAM_SW_STRIDE,
                        data: (&mut stride_value as *mut usize).cast(),
                    },
                    MpvRenderParam {
                        type_: MPV_RENDER_PARAM_SW_POINTER,
                        data: self.sw_pixels.as_mut_ptr().cast(),
                    },
                    MpvRenderParam {
                        type_: MPV_RENDER_PARAM_INVALID,
                        data: ptr::null_mut(),
                    },
                ];
                if check(mpv_render_context_render(self.render, params.as_mut_ptr())).is_ok() {
                    let image = ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &self.sw_pixels,
                    );
                    match &mut self.sw_texture {
                        Some(texture) => texture.set(image, TextureOptions::LINEAR),
                        None => {
                            self.sw_texture = Some(ui.ctx().load_texture(
                                "clip-engine-video",
                                image,
                                TextureOptions::LINEAR,
                            ));
                        }
                    }
                    if has_frame {
                        self.shared.awaiting_frame.store(false, Ordering::Relaxed);
                    }
                    if seek_ready && (has_frame || force_render) {
                        self.shared
                            .awaiting_seek_frame
                            .store(false, Ordering::Relaxed);
                        self.shared.seek_restarted.store(false, Ordering::Relaxed);
                    }
                    mpv_render_context_report_swap(self.render);
                }
            }
        }
        if let Some(texture) = &self.sw_texture {
            ui.painter().image(
                texture.id(),
                rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }

    pub fn load(&mut self, path: &str) -> anyhow::Result<()> {
        if self.loaded_path.as_deref() == Some(path) {
            return Ok(());
        }
        if !is_remote_media(path) {
            let source = std::path::Path::new(path);
            if !source.is_file() {
                anyhow::bail!("This recording is no longer on disk:\n{path}");
            }
            if source.metadata().map(|meta| meta.len()).unwrap_or(0) == 0 {
                anyhow::bail!("This recording is empty and cannot be played:\n{path}");
            }
        }
        let _ = set_property(self.handle, "pause", "yes");
        let _ = set_property(self.handle, "lavfi-complex", "");
        let _ = set_property(self.handle, "aid", "auto");
        let _ = set_property(self.handle, "mute", "no");
        let _ = set_property(self.handle, "volume", "100");
        command(self.handle, &["loadfile", path, "replace"])?;
        self.loaded_path = Some(path.to_string());
        self.audio_signature = None;
        self.pending_seek = None;
        self.last_issued_seek = None;
        self.seek_in_flight = false;
        self.error = None;
        self.load_started = Some(Instant::now());
        self.shared.pending_play.store(false, Ordering::Relaxed);
        self.shared.awaiting_frame.store(true, Ordering::SeqCst);
        self.shared
            .awaiting_seek_frame
            .store(false, Ordering::Relaxed);
        self.shared.seek_restarted.store(false, Ordering::Relaxed);
        self.request_redraw();
        Ok(())
    }

    pub fn wants_redraw(&self) -> bool {
        self.pending_seek.is_some()
            || self.seek_in_flight
            || self.shared.awaiting_frame.load(Ordering::Relaxed)
            || self.shared.awaiting_seek_frame.load(Ordering::Relaxed)
    }

    fn request_redraw(&self) {
        if !self.update_ctx.is_null() {
            unsafe {
                (*self.update_ctx).request_repaint();
            }
        }
    }

    pub fn playing(&self) -> bool {
        self.loaded_path.is_some() && !flag(self.handle, "pause").unwrap_or(true)
    }

    pub fn loaded_path(&self) -> Option<&str> {
        self.loaded_path.as_deref()
    }

    pub fn unload(&mut self) {
        self.shared
            .resume_after_seek
            .store(false, Ordering::Relaxed);
        self.shared.pending_play.store(false, Ordering::Relaxed);
        self.shared.awaiting_frame.store(false, Ordering::SeqCst);
        self.shared
            .awaiting_seek_frame
            .store(false, Ordering::Relaxed);
        self.shared.seek_restarted.store(false, Ordering::Relaxed);
        let _ = set_property(self.handle, "pause", "yes");
        let _ = command(self.handle, &["stop"]);
        self.loaded_path = None;
        self.audio_signature = None;
        self.pending_seek = None;
        self.last_issued_seek = None;
        self.seek_in_flight = false;
        self.load_started = None;
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    pub fn has_video(&self) -> bool {
        self.loaded_path.is_some() && !self.shared.awaiting_frame.load(Ordering::Relaxed)
    }

    pub fn buffering(&self) -> bool {
        self.loaded_path.is_some()
            && (self.shared.awaiting_frame.load(Ordering::Relaxed)
                || flag(self.handle, "paused-for-cache").unwrap_or(false))
    }

    pub fn play(&self) {
        if self.shared.awaiting_frame.load(Ordering::Relaxed) {
            self.shared.pending_play.store(true, Ordering::Relaxed);
            return;
        }
        self.shared.pending_play.store(false, Ordering::Relaxed);
        self.set_playback_sync(true);
        let _ = set_property(self.handle, "pause", "no");
    }

    pub fn pause(&self) {
        self.shared
            .resume_after_seek
            .store(false, Ordering::Relaxed);
        self.shared.pending_play.store(false, Ordering::Relaxed);
        self.set_playback_sync(false);
        let _ = set_property(self.handle, "pause", "yes");
    }

    pub fn wants_to_play(&self) -> bool {
        self.loaded_path.is_some()
            && (self.playing() || self.shared.pending_play.load(Ordering::Relaxed))
    }

    pub fn start_if_ready(&mut self) {
        if self.shared.awaiting_frame.load(Ordering::Relaxed) {
            return;
        }
        if !self.shared.pending_play.load(Ordering::Relaxed) {
            return;
        }
        if self.pending_seek.is_some() || self.seek_in_flight {
            return;
        }
        self.shared.pending_play.store(false, Ordering::Relaxed);
        self.set_playback_sync(true);
        let _ = set_property(self.handle, "pause", "no");
    }

    pub fn set_mute(&self, muted: bool) {
        let _ = set_property(self.handle, "mute", if muted { "yes" } else { "no" });
    }

    pub fn muted(&self) -> bool {
        flag(self.handle, "mute").unwrap_or(false)
    }

    fn set_playback_sync(&self, playing: bool) {
        let mode = if playing { "audio" } else { "display-desync" };
        let _ = set_property(self.handle, "video-sync", mode);
    }

    pub fn time(&self) -> f64 {
        if let Some(time) = self.pending_seek {
            return time;
        }
        if self.shared.awaiting_seek_frame.load(Ordering::Relaxed) {
            if let Some(time) = self.last_issued_seek {
                return time;
            }
        }
        number(self.handle, "time-pos").unwrap_or(0.0)
    }

    pub fn seek(&mut self, time: f64) {
        let time = time.max(0.0);
        self.pending_seek = Some(time);
        self.last_issued_seek = Some(time);
        self.request_redraw();
    }

    pub fn stop_at(&mut self, end: f64) {
        if self.pending_seek.is_some() {
            return;
        }
        if !self.wants_to_play() {
            return;
        }
        let time = self.time();
        if let Some(target) = self.last_issued_seek {
            if target < end - 0.001 && (time - target).abs() > 0.12 && time > target + 0.12 {
                return;
            }
            if (time - target).abs() <= 0.12 || time >= target {
                self.last_issued_seek = None;
            } else {
                return;
            }
        }
        if time + 0.001 < end {
            return;
        }
        self.pause();
        if time > end + 0.02 {
            self.seek(end);
        }
    }

    pub fn seek_and_play(&mut self, time: f64) {
        self.shared.resume_after_seek.store(true, Ordering::Relaxed);
        self.seek(time);
        self.play();
    }

    pub fn seek_relative(&mut self, delta: f64) {
        self.seek(self.time() + delta);
    }

    pub fn pump_events(&mut self) {
        self.check_load_timeout();
        unsafe {
            loop {
                let event = mpv_wait_event(self.handle, 0.0);
                if event.is_null() || (*event).event_id == MPV_EVENT_NONE {
                    break;
                }
                match (*event).event_id {
                    MPV_EVENT_END_FILE => {
                        let data = (*event).data as *const MpvEventEndFile;
                        if !data.is_null() && (*data).reason == MPV_END_FILE_REASON_ERROR {
                            let message = if (*data).error < 0 {
                                CStr::from_ptr(mpv_error_string((*data).error))
                                    .to_string_lossy()
                                    .into_owned()
                            } else {
                                "The recording could not be played.".into()
                            };
                            self.error = Some(message);
                            self.shared.awaiting_frame.store(false, Ordering::SeqCst);
                            self.shared
                                .awaiting_seek_frame
                                .store(false, Ordering::Relaxed);
                            self.shared.seek_restarted.store(false, Ordering::Relaxed);
                            self.shared.pending_play.store(false, Ordering::Relaxed);
                            self.seek_in_flight = false;
                            self.load_started = None;
                        }
                    }
                    MPV_EVENT_PLAYBACK_RESTART if self.seek_in_flight => {
                        self.seek_in_flight = false;
                        self.shared.seek_restarted.store(true, Ordering::SeqCst);
                    }
                    _ => {}
                }
                self.request_redraw();
            }
        }
    }

    fn check_load_timeout(&mut self) {
        if !self.shared.awaiting_frame.load(Ordering::Relaxed) {
            return;
        }
        let Some(started) = self.load_started else {
            return;
        };
        if started.elapsed() < Duration::from_secs(8) {
            return;
        }
        self.error = Some(
            "Playback stalled. The recording may be incomplete, missing, or unreadable.".into(),
        );
        self.unload();
    }

    pub fn flush_seek(&mut self) {
        if self.seek_in_flight && self.last_seek.elapsed() > Duration::from_millis(1000) {
            self.seek_in_flight = false;
        }
        if self.shared.awaiting_seek_frame.load(Ordering::Relaxed)
            && self.last_seek.elapsed() > Duration::from_millis(1000)
        {
            self.shared
                .awaiting_seek_frame
                .store(false, Ordering::Relaxed);
            self.shared.seek_restarted.store(false, Ordering::Relaxed);
        }
        let Some(time) = self.pending_seek else {
            return;
        };
        if self.shared.awaiting_frame.load(Ordering::Relaxed) {
            self.request_redraw();
            return;
        }
        if self.seek_in_flight {
            self.request_redraw();
            return;
        }
        let playing = self.playing()
            || self.shared.resume_after_seek.load(Ordering::Relaxed)
            || self.shared.pending_play.load(Ordering::Relaxed);
        if playing && self.last_seek.elapsed() < Duration::from_millis(40) {
            self.request_redraw();
            return;
        }
        let issued = if playing {
            command(
                self.handle,
                &["seek", &format!("{time:.6}"), "absolute+exact"],
            )
            .is_ok()
        } else {
            self.set_playback_sync(false);
            set_property_f64(self.handle, "time-pos", time).is_ok()
                || command(
                    self.handle,
                    &["seek", &format!("{time:.6}"), "absolute+exact"],
                )
                .is_ok()
        };
        if issued {
            self.pending_seek = None;
            self.last_seek = Instant::now();
            self.seek_in_flight = true;
            self.shared.seek_restarted.store(false, Ordering::SeqCst);
            self.shared
                .awaiting_seek_frame
                .store(true, Ordering::Relaxed);
            if self.shared.resume_after_seek.load(Ordering::Relaxed)
                && !self.shared.awaiting_frame.load(Ordering::Relaxed)
            {
                self.play();
            }
            self.request_redraw();
        }
    }

    pub fn apply_audio(&mut self, stream_indexes: &[i64]) -> anyhow::Result<()> {
        let available = self.audio_tracks();
        if available.is_empty() {
            return Ok(());
        }
        let mapped = map_requested(&available, stream_indexes);
        let selection_changed = self.audio_signature.as_deref() != Some(stream_indexes);
        if self.audio_already_matches(&mapped) {
            self.audio_signature = Some(stream_indexes.to_vec());
            return Ok(());
        }
        match mapped.as_slice() {
            [] => {
                set_property(self.handle, "lavfi-complex", "")?;
                set_property(self.handle, "aid", "no")?;
            }
            [id] => {
                set_property(self.handle, "lavfi-complex", "")?;
                set_property(self.handle, "aid", &id.to_string())?;
            }
            ids => {
                let inputs = ids
                    .iter()
                    .map(|id| format!("[aid{id}]"))
                    .collect::<String>();
                let graph = format!(
                    "{inputs}amix=inputs={}:duration=longest:normalize=1[ao]",
                    ids.len()
                );
                set_property(self.handle, "lavfi-complex", "")?;
                if set_property(self.handle, "lavfi-complex", &graph).is_err() {
                    set_property(self.handle, "aid", &ids[0].to_string())?;
                }
            }
        }
        let _ = set_property(self.handle, "mute", "no");
        let _ = set_property(self.handle, "volume", "100");
        self.audio_signature = Some(stream_indexes.to_vec());
        if selection_changed && self.pending_seek.is_none() {
            self.seek(self.time());
        }
        Ok(())
    }

    fn audio_already_matches(&self, mapped: &[i64]) -> bool {
        let lavfi = string(self.handle, "lavfi-complex").unwrap_or_default();
        match mapped {
            [] => lavfi.is_empty() && string(self.handle, "aid").as_deref() == Some("no"),
            [id] => {
                lavfi.is_empty()
                    && number(self.handle, "aid").map(|value| value as i64) == Some(*id)
            }
            ids => {
                let expected = ids
                    .iter()
                    .map(|id| format!("[aid{id}]"))
                    .collect::<String>();
                lavfi.starts_with(&format!("{expected}amix=inputs={}", ids.len()))
            }
        }
    }

    pub fn clear_audio(&mut self) {
        self.audio_signature = None;
        self.request_redraw();
    }

    fn audio_tracks(&self) -> Vec<(i64, i64)> {
        let count = number(self.handle, "track-list/count").unwrap_or(0.0) as i64;
        let mut tracks = Vec::new();
        for index in 0..count {
            let kind = string(self.handle, &format!("track-list/{index}/type")).unwrap_or_default();
            if kind != "audio" {
                continue;
            }
            let Some(id) = number(self.handle, &format!("track-list/{index}/id")) else {
                continue;
            };
            let ff_index =
                number(self.handle, &format!("track-list/{index}/ff-index")).unwrap_or(id) as i64;
            tracks.push((ff_index, id as i64));
        }
        tracks
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe {
            self.shared.render.store(ptr::null_mut(), Ordering::SeqCst);
            if !self.render.is_null() {
                mpv_render_context_free(self.render);
            }
            if !self.handle.is_null() {
                mpv_terminate_destroy(self.handle);
            }
            if !self.update_ctx.is_null() {
                drop(Box::from_raw(self.update_ctx));
            }
        }
    }
}

unsafe extern "C" fn gl_get_proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    #[cfg(unix)]
    {
        let pointer = eglGetProcAddress(name);
        if !pointer.is_null() {
            return pointer;
        }
        let lib = libc::dlopen(c"libGL.so.1".as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL);
        if !lib.is_null() {
            let symbol = libc::dlsym(lib, name);
            if !symbol.is_null() {
                return symbol;
            }
        }
    }
    #[cfg(windows)]
    {
        let pointer = wglGetProcAddress(name);
        if !pointer.is_null() {
            return pointer;
        }
        let module = LoadLibraryA(b"opengl32.dll\0".as_ptr());
        if !module.is_null() {
            return GetProcAddress(module, name);
        }
    }
    ptr::null_mut()
}

unsafe extern "C" fn on_mpv_update(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    let context = &*(ctx as *const egui::Context);
    context.request_repaint();
}

unsafe fn create_opengl_render(
    handle: *mut MpvHandle,
    display: Option<DisplayHandle<'_>>,
) -> anyhow::Result<*mut MpvRenderContext> {
    let mut init = MpvOpenglInitParams {
        get_proc_address: gl_get_proc_address,
        get_proc_address_ctx: ptr::null_mut(),
    };
    let api = CString::new("opengl")?;
    let mut display_param = MpvRenderParam {
        type_: MPV_RENDER_PARAM_INVALID,
        data: ptr::null_mut(),
    };
    if let Some(display) = display {
        match display.as_raw() {
            RawDisplayHandle::Wayland(wayland) => {
                display_param.type_ = MPV_RENDER_PARAM_WL_DISPLAY;
                display_param.data = wayland.display.as_ptr();
            }
            RawDisplayHandle::Xlib(xlib) => {
                if let Some(display) = xlib.display {
                    display_param.type_ = MPV_RENDER_PARAM_X11_DISPLAY;
                    display_param.data = display.as_ptr();
                }
            }
            _ => {}
        }
    }
    let mut params = [
        MpvRenderParam {
            type_: MPV_RENDER_PARAM_API_TYPE,
            data: api.as_ptr().cast_mut().cast(),
        },
        MpvRenderParam {
            type_: MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
            data: (&mut init as *mut MpvOpenglInitParams).cast(),
        },
        display_param,
        MpvRenderParam {
            type_: MPV_RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        },
    ];
    let mut render = ptr::null_mut();
    check(mpv_render_context_create(
        &mut render,
        handle,
        params.as_mut_ptr(),
    ))?;
    Ok(render)
}

fn create_software_render(handle: *mut MpvHandle) -> anyhow::Result<*mut MpvRenderContext> {
    let api = CString::new("sw")?;
    let mut params = [
        MpvRenderParam {
            type_: MPV_RENDER_PARAM_API_TYPE,
            data: api.as_ptr().cast_mut().cast(),
        },
        MpvRenderParam {
            type_: MPV_RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        },
    ];
    let mut render = ptr::null_mut();
    unsafe {
        check(mpv_render_context_create(
            &mut render,
            handle,
            params.as_mut_ptr(),
        ))?;
    }
    Ok(render)
}

fn paint_video(
    shared: &RenderShared,
    info: egui::PaintCallbackInfo,
    painter: &eframe::egui_glow::Painter,
) {
    let render = shared.render.load(Ordering::SeqCst);
    if render.is_null() {
        return;
    }
    let viewport = info.viewport_in_pixels();
    let clip = info.clip_rect_in_pixels();
    let width = viewport.width_px.max(2);
    let height = viewport.height_px.max(2);
    let gl = painter.gl();
    unsafe {
        let window_fbo = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
        let mut target = shared
            .target
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let recreate = target
            .as_ref()
            .is_none_or(|current| current.width != width || current.height != height);
        if recreate {
            if let Some(old) = target.take() {
                gl.delete_framebuffer(old.fbo);
                gl.delete_texture(old.texture);
            }
            let texture = gl.create_texture().expect("video texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width,
                height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            let fbo = gl.create_framebuffer().expect("video framebuffer");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            *target = Some(GlTarget {
                fbo,
                texture,
                width,
                height,
            });
        }
        let Some(current) = target.as_ref() else {
            return;
        };
        let flags = mpv_render_context_update(render);
        let has_frame = flags & MPV_RENDER_UPDATE_FRAME != 0;
        let seek_ready = shared.seek_restarted.load(Ordering::SeqCst);
        let force_render = shared.awaiting_seek_frame.load(Ordering::Relaxed) && seek_ready;
        gl.disable(glow::SCISSOR_TEST);
        gl.viewport(0, 0, width, height);
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(current.fbo));
        if recreate {
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        let mut fbo = MpvOpenglFbo {
            fbo: current.fbo.0.get() as i32,
            w: width,
            h: height,
            internal_format: 0,
        };
        let mut flip: i32 = 1;
        let mut block: i32 = 0;
        let mut params = [
            MpvRenderParam {
                type_: MPV_RENDER_PARAM_OPENGL_FBO,
                data: (&mut fbo as *mut MpvOpenglFbo).cast(),
            },
            MpvRenderParam {
                type_: MPV_RENDER_PARAM_FLIP_Y,
                data: (&mut flip as *mut i32).cast(),
            },
            MpvRenderParam {
                type_: MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                data: (&mut block as *mut i32).cast(),
            },
            MpvRenderParam {
                type_: MPV_RENDER_PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];
        if has_frame || recreate || force_render {
            gl.disable(glow::SCISSOR_TEST);
            gl.viewport(0, 0, width, height);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(current.fbo));
            let _ = mpv_render_context_render(render, params.as_mut_ptr());
            if has_frame {
                shared.awaiting_frame.store(false, Ordering::Relaxed);
            }
            if seek_ready && (has_frame || force_render) {
                shared.awaiting_seek_frame.store(false, Ordering::Relaxed);
                shared.seek_restarted.store(false, Ordering::Relaxed);
            }
        }
        let window = if window_fbo == 0 {
            None
        } else {
            std::num::NonZeroU32::new(window_fbo as u32).map(glow::NativeFramebuffer)
        };
        gl.bind_framebuffer(glow::FRAMEBUFFER, window);
        gl.enable(glow::SCISSOR_TEST);
        gl.scissor(
            clip.left_px,
            clip.from_bottom_px,
            clip.width_px.max(0),
            clip.height_px.max(0),
        );
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(current.fbo));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, window);
        gl.blit_framebuffer(
            0,
            0,
            width,
            height,
            viewport.left_px,
            viewport.from_bottom_px,
            viewport.left_px + width,
            viewport.from_bottom_px + height,
            glow::COLOR_BUFFER_BIT,
            glow::LINEAR,
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, window);
        gl.disable(glow::SCISSOR_TEST);
        if has_frame || force_render {
            mpv_render_context_report_swap(render);
        }
    }
}

fn is_remote_media(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://")
}

fn set_option(handle: *mut MpvHandle, name: &str, value: &str) -> anyhow::Result<()> {
    let name = CString::new(name)?;
    let value = CString::new(value)?;
    unsafe { check(mpv_set_option_string(handle, name.as_ptr(), value.as_ptr())) }
}

fn set_property(handle: *mut MpvHandle, name: &str, value: &str) -> anyhow::Result<()> {
    let name = CString::new(name)?;
    let value = CString::new(value)?;
    unsafe {
        check(mpv_set_property_string(
            handle,
            name.as_ptr(),
            value.as_ptr(),
        ))
    }
}

fn set_property_f64(handle: *mut MpvHandle, name: &str, value: f64) -> anyhow::Result<()> {
    let name = CString::new(name)?;
    let mut value = value;
    unsafe {
        check(mpv_set_property(
            handle,
            name.as_ptr(),
            MPV_FORMAT_DOUBLE,
            (&mut value as *mut f64).cast(),
        ))
    }
}

fn command(handle: *mut MpvHandle, args: &[&str]) -> anyhow::Result<()> {
    let owned = args
        .iter()
        .map(|value| CString::new(*value))
        .collect::<Result<Vec<_>, _>>()?;
    let mut pointers = owned
        .iter()
        .map(|value| value.as_ptr())
        .chain(std::iter::once(ptr::null()))
        .collect::<Vec<_>>();
    unsafe { check(mpv_command(handle, pointers.as_mut_ptr())) }
}

fn number(handle: *mut MpvHandle, name: &str) -> Option<f64> {
    let name = CString::new(name).ok()?;
    let mut value = 0.0_f64;
    let status = unsafe {
        mpv_get_property(
            handle,
            name.as_ptr(),
            MPV_FORMAT_DOUBLE,
            (&mut value as *mut f64).cast(),
        )
    };
    (status >= 0).then_some(value)
}

fn flag(handle: *mut MpvHandle, name: &str) -> Option<bool> {
    let name = CString::new(name).ok()?;
    let mut value: i32 = 0;
    let status = unsafe {
        mpv_get_property(
            handle,
            name.as_ptr(),
            MPV_FORMAT_FLAG,
            (&mut value as *mut i32).cast(),
        )
    };
    (status >= 0).then_some(value != 0)
}

fn string(handle: *mut MpvHandle, name: &str) -> Option<String> {
    let name = CString::new(name).ok()?;
    unsafe {
        let pointer = mpv_get_property_string(handle, name.as_ptr());
        if pointer.is_null() {
            return None;
        }
        let value = CStr::from_ptr(pointer).to_string_lossy().into_owned();
        mpv_free(pointer.cast());
        Some(value)
    }
}

fn map_requested(available: &[(i64, i64)], requested: &[i64]) -> Vec<i64> {
    let mut mapped = Vec::new();
    for (ordinal, stream_index) in requested.iter().enumerate() {
        if let Some((_, id)) = available
            .iter()
            .find(|(ff_index, _)| ff_index == stream_index)
        {
            mapped.push(*id);
            continue;
        }
        if let Some((_, id)) = available.get(ordinal) {
            mapped.push(*id);
        }
    }
    mapped.sort_unstable();
    mapped.dedup();
    mapped
}

fn check(status: c_int) -> anyhow::Result<()> {
    if status >= 0 {
        return Ok(());
    }
    let message = unsafe { CStr::from_ptr(mpv_error_string(status)) }
        .to_string_lossy()
        .into_owned();
    anyhow::bail!("libmpv: {message}");
}
