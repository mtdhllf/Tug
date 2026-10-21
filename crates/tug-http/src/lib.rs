//! HTTP/HTTPS 分段下载引擎。
//!
//! 能力边界（M1）：
//! - `Range` 探测与重定向跟随；
//! - 多连接分段下载、失败重试、限速；
//! - 断点续传（sidecar 元数据 + `If-Range` 校验）；
//! - 无 `Range` 服务器自动降级单流；
//! - 可选 SHA-256 校验。
//!
//! 暂不包含：代理认证界面、Cookie 持久化、BT（见 M3）。

pub mod engine;
pub mod error;
pub mod filename;
pub mod meta;
pub mod options;
pub mod probe;
pub mod segments;
pub mod writer;

mod rate;

pub use engine::{CancelToken, download};
pub use error::{HttpError, Result};
pub use filename::{guess_file_name, sanitize_file_name, sidecar_path, unique_path};
pub use meta::ResumeMeta;
pub use options::{DownloadOptions, DownloadSummary, Progress, ProgressCallback};
pub use probe::{DownloadPlan, probe};
pub use segments::{Segment, build_segments, effective_segment_size};

/// 当前 crate 版本，供 CLI / 诊断面板展示。
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
