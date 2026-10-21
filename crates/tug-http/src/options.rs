use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Url;
use reqwest::header::{HeaderName, HeaderValue};

use crate::error::{HttpError, Result};

/// 默认分片大小：8 MiB。
pub const DEFAULT_SEGMENT_SIZE: u64 = 8 * 1024 * 1024;

/// 最小分片大小：64 KiB（小文件多连接时使用）。
pub const MIN_SEGMENT_SIZE: u64 = 64 * 1024;

/// 单个下载任务的参数。
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// 原始 URL。
    pub url: String,
    /// 输出目录。
    pub output_dir: PathBuf,
    /// 覆盖服务器给出的文件名（可选）。
    pub file_name: Option<String>,
    /// 并发连接数（1–32）。
    pub connections: usize,
    /// 期望分片大小；实际会按文件大小自适应。
    pub segment_size: u64,
    /// 额外请求头。
    pub headers: Vec<(String, String)>,
    /// User-Agent；默认 `Tug/<version>`。
    pub user_agent: Option<String>,
    /// 显式 HTTP 代理（CLI/高级用法）；设置后不再使用系统代理。
    pub proxy: Option<String>,
    /// 是否使用系统代理；显式 `proxy` 优先。
    pub use_system_proxy: bool,
    /// 限速（字节/秒）。
    pub rate_limit_bps: Option<u64>,
    /// 期望的 SHA-256（十六进制，大小写不敏感）。
    pub checksum_sha256: Option<String>,
    /// 忽略已有续传数据，从头下载。
    pub restart: bool,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            url: String::new(),
            output_dir: PathBuf::from("."),
            file_name: None,
            connections: 8,
            segment_size: DEFAULT_SEGMENT_SIZE,
            headers: Vec::new(),
            user_agent: None,
            proxy: None,
            use_system_proxy: true,
            rate_limit_bps: None,
            checksum_sha256: None,
            restart: false,
        }
    }
}

impl DownloadOptions {
    /// 以 URL 和输出目录创建默认参数。
    pub fn new(url: impl Into<String>, output_dir: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            output_dir: output_dir.into(),
            ..Self::default()
        }
    }

    /// 校验参数，尽早给出可读错误。
    pub fn validate(&self) -> Result<()> {
        let url = Url::parse(&self.url)
            .map_err(|err| HttpError::InvalidOptions(format!("URL 非法: {err}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(HttpError::InvalidOptions(format!(
                "仅支持 http/https，收到 {}",
                url.scheme()
            )));
        }
        if self.output_dir.as_os_str().is_empty() {
            return Err(HttpError::InvalidOptions("输出目录不能为空".into()));
        }
        if !(1..=32).contains(&self.connections) {
            return Err(HttpError::InvalidOptions(format!(
                "连接数必须在 1–32 之间，收到 {}",
                self.connections
            )));
        }
        if !(MIN_SEGMENT_SIZE..=1024 * 1024 * 1024).contains(&self.segment_size) {
            return Err(HttpError::InvalidOptions(format!(
                "分片大小必须在 64 KiB–1 GiB 之间，收到 {}",
                self.segment_size
            )));
        }
        if let Some(rate) = self.rate_limit_bps
            && rate == 0
        {
            return Err(HttpError::InvalidOptions("限速必须大于 0".into()));
        }
        if let Some(checksum) = &self.checksum_sha256
            && (checksum.len() != 64 || !checksum.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(HttpError::InvalidOptions(
                "SHA-256 必须是 64 位十六进制字符串".into(),
            ));
        }
        for (name, value) in &self.headers {
            HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
                HttpError::InvalidOptions(format!("请求头名称非法 {name}: {err}"))
            })?;
            HeaderValue::from_str(value).map_err(|err| {
                HttpError::InvalidOptions(format!("请求头 {name} 的值非法: {err}"))
            })?;
        }
        Ok(())
    }
}

/// 下载进度快照。
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    /// 已写入字节数（含续传部分）。
    pub downloaded: u64,
    /// 总字节数；服务器未给出时为 `None`。
    pub total: Option<u64>,
    /// 瞬时速度（字节/秒）。
    pub speed_bps: u64,
    /// 预计剩余秒数。
    pub eta_secs: Option<u64>,
    /// 当前活跃连接数。
    pub active_connections: usize,
    /// 已完成分片数。
    pub completed_segments: usize,
    /// 总分片数。
    pub total_segments: usize,
}

/// 进度回调；由调用方决定如何节流/渲染。
pub type ProgressCallback = Arc<dyn Fn(Progress) + Send + Sync + 'static>;

/// 下载完成后的摘要。
#[derive(Debug, Clone)]
pub struct DownloadSummary {
    /// 最终文件路径。
    pub path: PathBuf,
    /// 文件总大小。
    pub total: u64,
    /// 本次启动时已复用的字节数。
    pub resumed_from: u64,
    /// 总耗时。
    pub elapsed: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> DownloadOptions {
        DownloadOptions::new("https://example.com/file.bin", "D:/downloads")
    }

    #[test]
    fn accepts_defaults() {
        valid().validate().expect("defaults valid");
    }

    #[test]
    fn rejects_non_http_scheme() {
        let mut options = valid();
        options.url = "ftp://example.com/file".into();
        assert!(options.validate().is_err());
    }

    #[test]
    fn rejects_bad_connections() {
        let mut options = valid();
        options.connections = 0;
        assert!(options.validate().is_err());
        options.connections = 33;
        assert!(options.validate().is_err());
    }

    #[test]
    fn rejects_bad_checksum() {
        let mut options = valid();
        options.checksum_sha256 = Some("abc".into());
        assert!(options.validate().is_err());
    }

    #[test]
    fn rejects_bad_header() {
        let mut options = valid();
        options.headers = vec![("Bad Header".into(), "x".into())];
        assert!(options.validate().is_err());
    }
}
