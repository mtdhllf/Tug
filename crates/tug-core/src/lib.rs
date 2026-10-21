//! Tug 核心库：应用目录、配置、日志、任务模型与持久化。
//!
//! 这一层不依赖 Tauri，也不依赖具体下载引擎，便于后续 Android 端复用。

pub mod error;
pub mod logging;
pub mod paths;
pub mod settings;
pub mod store;
pub mod task;

pub use error::{Result, TugError};
pub use paths::AppPaths;
pub use settings::{Settings, Theme};
pub use store::TaskStore;
pub use task::{NewTask, TaskRecord, TaskState, TaskView};

/// 产品名。
pub const APP_NAME: &str = "Tug";

/// 当前版本（来自 Cargo.toml）。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
