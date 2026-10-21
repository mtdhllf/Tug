use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

/// 简单的令牌桶式限速器；`None` 表示不限速。
#[derive(Clone, Default)]
pub struct RateLimiter {
    inner: Option<Arc<Inner>>,
}

struct Inner {
    bytes_per_second: u64,
    next: Mutex<Instant>,
}

impl RateLimiter {
    /// 以字节/秒创建限速器。
    pub fn new(bytes_per_second: Option<u64>) -> Self {
        match bytes_per_second {
            Some(rate) if rate > 0 => Self {
                inner: Some(Arc::new(Inner {
                    bytes_per_second: rate,
                    next: Mutex::new(Instant::now()),
                })),
            },
            _ => Self { inner: None },
        }
    }

    /// 为 `bytes` 字节预留时间片，必要时等待。
    pub async fn acquire(&self, bytes: u64) {
        let Some(inner) = &self.inner else {
            return;
        };
        if bytes == 0 {
            return;
        }

        let delay = Duration::from_secs_f64(bytes as f64 / inner.bytes_per_second as f64);
        let wake = {
            let mut next = inner.next.lock().await;
            let now = Instant::now();
            let wake = (*next).max(now);
            *next = wake + delay;
            wake
        };
        tokio::time::sleep_until(wake).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unlimited_is_noop() {
        let limiter = RateLimiter::new(None);
        limiter.acquire(1024).await;
    }

    #[tokio::test]
    async fn zero_rate_is_treated_as_unlimited() {
        let limiter = RateLimiter::new(Some(0));
        limiter.acquire(1024).await;
    }
}
