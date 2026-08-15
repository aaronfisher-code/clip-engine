use clip_engine_core::Engine;
use eframe::egui::{IconData, ViewportBuilder};
use eframe::Renderer;
use std::sync::Arc;
use tokio::runtime::Runtime;

mod app;
mod player;
mod theme;

fn main() -> eframe::Result<()> {
    let runtime = Runtime::new().expect("tokio runtime");
    let engine = Engine::initialize(runtime.handle().clone()).expect("initialize Clip Engine");
    let _keep_alive = runtime;
    let icon = load_icon();
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 620.0])
            .with_title("DAB Clip Engine")
            .with_icon(icon),
        vsync: true,
        renderer: Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "clip-engine",
        options,
        Box::new(move |cc| Ok(Box::new(app::ClipApp::new(cc, engine)))),
    )
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
