//! BitTorrent / 磁力链接适配层。
//!
//! `engine` 封装 librqbit；service 只依赖这个 crate，不直接依赖 librqbit。

mod engine;

pub use engine::{
    DEFAULT_TRACKERS, TorrentAddOptions, TorrentConfig, TorrentFileInfo, TorrentHandle,
    TorrentPeerInfo, TorrentRuntimeDetails, TorrentSession, TorrentSnapshot, with_default_trackers,
};

/// 当前 crate 版本，供 CLI / 诊断面板展示。
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
