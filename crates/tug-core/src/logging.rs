use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

use crate::error::Result;
use crate::paths::AppPaths;

/// 初始化日志：写入 `%LOCALAPPDATA%\Tug\logs`，按天滚动。
///
/// 返回的 guard 必须在应用生命周期内持有，否则后台写日志线程会提前结束。
/// 同一进程只应调用一次。
pub fn init(paths: &AppPaths) -> Result<WorkerGuard> {
    std::fs::create_dir_all(&paths.log_dir)?;

    let appender = tracing_appender::rolling::daily(&paths.log_dir, "tug.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("tug=info,warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .init();

    Ok(guard)
}
