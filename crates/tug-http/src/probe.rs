use std::time::Duration;

use reqwest::header::{
    ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, HeaderMap, LAST_MODIFIED,
    RANGE, RETRY_AFTER,
};
use reqwest::{Client, StatusCode};

use crate::error::{HttpError, Result};
use crate::filename::{file_name_from_url, parse_content_disposition, sanitize_file_name};
use crate::options::DownloadOptions;

/// 探测结果，描述远端资源与下载能力。
#[derive(Debug, Clone)]
pub struct DownloadPlan {
    /// 原始 URL。
    pub url: String,
    /// 跟随重定向后的 URL。
    pub final_url: String,
    /// 建议文件名（已清洗）。
    pub file_name: String,
    /// 总字节数；未知时为 `None`。
    pub total: Option<u64>,
    /// 是否支持 Range。
    pub supports_range: bool,
    /// 强 ETag。
    pub etag: Option<String>,
    /// Last-Modified。
    pub last_modified: Option<String>,
    /// Content-Type。
    pub content_type: Option<String>,
}

/// 构建带超时、重定向、Cookie、代理的 HTTP 客户端。
pub fn build_client(options: &DownloadOptions) -> Result<Client> {
    let user_agent = options
        .user_agent
        .clone()
        .unwrap_or_else(|| format!("Tug/{}", crate::crate_version()));

    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .cookie_store(true)
        .user_agent(user_agent);

    if let Some(proxy) = &options.proxy {
        let proxy = reqwest::Proxy::all(proxy)
            .map_err(|err| HttpError::InvalidOptions(format!("代理地址非法: {err}")))?;
        builder = builder.proxy(proxy);
    } else if !options.use_system_proxy {
        builder = builder.no_proxy();
    }

    builder.build().map_err(HttpError::Request)
}

/// 用 `GET Range: bytes=0-0` 探测远端能力。
pub async fn probe(client: &Client, options: &DownloadOptions) -> Result<DownloadPlan> {
    let mut request = client
        .get(&options.url)
        .header(RANGE, "bytes=0-0")
        .header(ACCEPT_ENCODING, "identity");
    for (name, value) in &options.headers {
        request = request.header(name, value);
    }

    let response = request.send().await?;
    let status = response.status();
    let final_url = response.url().clone();
    let headers = response.headers();

    let (total, supports_range) = match status {
        StatusCode::PARTIAL_CONTENT => {
            let total = headers
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range)
                .and_then(|(_, _, total)| total);
            (total, total.is_some())
        }
        StatusCode::OK => (content_length(headers), false),
        StatusCode::RANGE_NOT_SATISFIABLE => {
            let total = headers
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_unsatisfied_total);
            match total {
                Some(0) => (Some(0), true),
                _ => {
                    return Err(HttpError::Status {
                        status,
                        url: final_url.to_string(),
                    });
                }
            }
        }
        status if status.is_success() => (content_length(headers), false),
        _ => {
            return Err(HttpError::Status {
                status,
                url: final_url.to_string(),
            });
        }
    };

    let file_name = parse_content_disposition(headers)
        .or_else(|| file_name_from_url(&final_url))
        .map(|name| sanitize_file_name(&name))
        .unwrap_or_else(|| "download".to_owned());

    Ok(DownloadPlan {
        url: options.url.clone(),
        final_url: final_url.to_string(),
        file_name,
        total,
        supports_range,
        etag: header_string(headers, ETAG),
        last_modified: header_string(headers, LAST_MODIFIED),
        content_type: header_string(headers, CONTENT_TYPE),
    })
}

/// 解析 `Content-Range: bytes start-end/total`；`total` 可能为 `*`。
pub(crate) fn parse_content_range(value: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = value.strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let total = if total == "*" {
        None
    } else {
        Some(total.parse().ok()?)
    };
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total))
}

/// 解析 416 的 `Content-Range: bytes */total`。
pub(crate) fn parse_unsatisfied_total(value: &str) -> Option<u64> {
    value.strip_prefix("bytes */")?.parse().ok()
}

/// 解析 `Retry-After`（仅支持秒数形式）。
pub(crate) fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(seconds))
}

fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn header_string(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_total() {
        assert_eq!(
            parse_content_range("bytes 0-0/12345"),
            Some((0, 0, Some(12345)))
        );
        assert_eq!(
            parse_content_range("bytes 100-199/200"),
            Some((100, 199, Some(200)))
        );
    }

    #[test]
    fn parses_unknown_total() {
        assert_eq!(parse_content_range("bytes 0-0/*"), Some((0, 0, None)));
    }

    #[test]
    fn rejects_malformed_range() {
        assert_eq!(parse_content_range("items 0-0/1"), None);
        assert_eq!(parse_content_range("bytes 0-0"), None);
    }

    #[test]
    fn parses_unsatisfied_total() {
        assert_eq!(parse_unsatisfied_total("bytes */0"), Some(0));
        assert_eq!(parse_unsatisfied_total("bytes */42"), Some(42));
        assert_eq!(parse_unsatisfied_total("bytes 0-0/42"), None);
    }
}
