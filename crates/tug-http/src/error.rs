use std::time::Duration;

use reqwest::StatusCode;

/// HTTP 下载引擎的统一错误。
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// 网络请求失败（连接、超时、body 中断等）。
    #[error("网络请求失败: {0}")]
    Request(#[from] reqwest::Error),

    /// 本地文件读写失败。
    #[error("文件读写失败: {0}")]
    Io(#[from] std::io::Error),

    /// 服务器返回了不可恢复的状态码。
    #[error("服务器返回 {status}: {url}")]
    Status { status: StatusCode, url: String },

    /// 可重试的临时错误（429/5xx/短读）。
    #[error("服务器暂时不可用: {status}")]
    Transient {
        status: StatusCode,
        retry_after: Option<Duration>,
    },

    /// 探测时支持 Range，但实际请求返回 200。
    #[error("服务器不再支持范围请求: {0}")]
    RangeUnsupported(String),

    /// 远端文件已变化（ETag/长度不一致）。
    #[error("远端文件已变化（ETag 或大小不一致），请使用 --restart 重新下载")]
    RemoteChanged,

    /// 分片读取长度不足。
    #[error("分片数据不完整: 期望 {expected} 字节，实际 {actual} 字节")]
    ShortRead { expected: u64, actual: u64 },

    /// SHA-256 校验失败。
    #[error("SHA-256 校验失败: 期望 {expected}，实际 {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    /// 磁盘空间不足。
    #[error("磁盘空间不足: 需要 {needed} 字节，可用 {available} 字节")]
    DiskFull { needed: u64, available: u64 },

    /// 参数非法。
    #[error("参数错误: {0}")]
    InvalidOptions(String),

    /// 元数据损坏或版本不兼容。
    #[error("续传元数据错误: {0}")]
    Meta(String),

    /// 用户取消。
    #[error("任务已取消")]
    Cancelled,

    /// 内部错误（不应发生）。
    #[error("内部错误: {0}")]
    Internal(String),
}

/// HTTP 引擎统一结果类型。
pub type Result<T> = std::result::Result<T, HttpError>;
