use std::io;

/// Tug 的统一错误类型。
#[derive(Debug, thiserror::Error)]
pub enum TugError {
    /// 文件系统或网络 IO 错误。
    #[error("IO 错误: {0}")]
    Io(#[from] io::Error),

    /// 配置缺失或格式错误。
    #[error("配置错误: {0}")]
    Config(String),

    /// 运行环境缺少必要的系统目录。
    #[error("缺少系统目录: {0}")]
    MissingDir(&'static str),

    /// SQLite 操作失败。
    #[error("数据库错误: {0}")]
    Database(#[from] rusqlite::Error),
}

/// 核心库统一使用的结果类型。
pub type Result<T> = std::result::Result<T, TugError>;
