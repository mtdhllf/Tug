//! 任务编排层：把 `tug-http` 的下载能力包装成可持久化、可暂停/继续的任务。
//!
//! 这一层不依赖 Tauri；桌面端与后续 Android 端共用。

pub mod manager;

pub use manager::{AddTaskInput, ServiceError, TaskEvent, TaskManager};
pub use tug_torrent::{TorrentFileInfo, TorrentPeerInfo, TorrentRuntimeDetails};
