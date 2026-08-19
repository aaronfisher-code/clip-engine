pub mod auth;
pub mod cloud;
pub mod database;
pub mod engine;
pub mod media;
pub mod models;
pub mod paths;
pub mod recorder;
pub mod updater;

/// Branded product name for window titles, the installer, and other proper-noun uses.
/// Crate, identifier, and data-dir names stay `clip-engine`.
pub const PRODUCT_NAME: &str = "Dabs Clip Engine";

/// Shorter name for sentences where "Dabs Clip Engine" is awkward ("your Clip Engine login").
pub const APP_NAME: &str = "Clip Engine";

pub use clip_engine_recorder_protocol::*;
pub use engine::Engine;
pub use media::{
    export_options, format_file_size, publish_options, safe_base_name, PublishOption,
    MAX_PUBLISH_BYTES,
};
pub use models::*;
pub use paths::AppPaths;
pub use recorder::RecorderSupervisor;
pub use updater::{install_desktop_update, AvailableUpdate, UpdatePackage};
