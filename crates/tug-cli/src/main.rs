use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use tug_core::{APP_NAME, APP_VERSION, AppPaths, Settings, TaskState};
use tug_http::{CancelToken, DownloadOptions, Progress, download};
use tug_service::{AddTaskInput, TaskManager};

#[derive(Parser)]
#[command(name = "tug", version = APP_VERSION, about = "Tug — 多协议下载器")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 显示应用目录
    Paths,
    /// 显示当前设置（首次运行会生成默认配置）
    Settings,
    /// 下载 HTTP/HTTPS 资源
    Download(DownloadArgs),
    /// 下载 BT 种子/磁力（M3a 验证用，数据目录隔离）
    Bt(BtArgs),
}

#[derive(Args)]
struct DownloadArgs {
    /// 下载地址
    url: String,
    /// 输出目录（默认取设置中的下载目录）
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    /// 并发连接数（1–32，默认取设置）
    #[arg(short = 'n', long)]
    connections: Option<usize>,
    /// 覆盖服务器给出的文件名
    #[arg(long)]
    name: Option<String>,
    /// 限速（KiB/s）
    #[arg(long)]
    limit: Option<u64>,
    /// 期望的 SHA-256（64 位十六进制）
    #[arg(long)]
    checksum: Option<String>,
    /// 忽略已有续传数据，从头下载
    #[arg(long)]
    restart: bool,
    /// 额外请求头，格式 "Name: Value"，可重复
    #[arg(long = "header")]
    headers: Vec<String>,
    /// HTTP 代理，例如 http://127.0.0.1:7890
    #[arg(long)]
    proxy: Option<String>,
}

#[derive(Args)]
struct BtArgs {
    /// 磁力链接、.torrent 路径或 URL
    source: String,
    /// 输出目录
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
    /// 最长运行秒数
    #[arg(long, default_value_t = 120)]
    seconds: u64,
    /// 运行到第 N 秒时自动暂停 2 秒再继续（验证暂停/继续）
    #[arg(long)]
    pause_after: Option<u64>,
    /// 隔离的数据目录（默认放在系统临时目录）
    #[arg(long)]
    data: Option<PathBuf>,
    /// 继续该数据目录中已有的任务（用于验证重启 fastresume）
    #[arg(long)]
    resume_id: Option<i64>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Paths => show_paths(),
        Command::Settings => show_settings(),
        Command::Download(args) => run_download(args).await,
        Command::Bt(args) => run_bt(args).await,
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("错误: {message}");
            ExitCode::FAILURE
        }
    }
}

fn show_paths() -> Result<(), String> {
    let paths = AppPaths::discover().map_err(|err| err.to_string())?;
    println!("config:   {}", paths.config_dir.display());
    println!("data:     {}", paths.data_dir.display());
    println!("logs:     {}", paths.log_dir.display());
    println!("download: {}", paths.default_download_dir.display());
    Ok(())
}

fn show_settings() -> Result<(), String> {
    let paths = AppPaths::discover().map_err(|err| err.to_string())?;
    let settings = Settings::load(&paths).map_err(|err| err.to_string())?;
    let text = serde_json::to_string_pretty(&settings).map_err(|err| err.to_string())?;
    println!("{text}");
    Ok(())
}

async fn run_download(args: DownloadArgs) -> Result<(), String> {
    let paths = AppPaths::discover().map_err(|err| err.to_string())?;
    let settings = Settings::load(&paths).map_err(|err| err.to_string())?;

    let output_dir = args.output.unwrap_or(settings.download_dir);
    let headers = parse_headers(&args.headers)?;

    let mut options = DownloadOptions::new(args.url, output_dir);
    options.file_name = args.name;
    options.connections = args
        .connections
        .unwrap_or(settings.connections_per_task as usize);
    options.headers = headers;
    options.proxy = args.proxy;
    options.rate_limit_bps = args.limit.map(|kib| kib.saturating_mul(1024));
    options.checksum_sha256 = args.checksum;
    options.restart = args.restart;

    let cancel = CancelToken::new();
    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\n收到中断，正在停止…");
            cancel_for_signal.cancel();
        }
    });

    let progress: tug_http::ProgressCallback = Arc::new(|progress: Progress| {
        print_progress(&progress);
    });

    println!("{APP_NAME} {APP_VERSION}");
    match download(options, Some(progress), cancel).await {
        Ok(summary) => {
            println!();
            println!("完成: {}", summary.path.display());
            println!(
                "大小: {}，复用: {}，耗时: {:.1}s",
                format_bytes(summary.total),
                format_bytes(summary.resumed_from),
                summary.elapsed.as_secs_f64()
            );
            Ok(())
        }
        Err(err) => {
            println!();
            Err(err.to_string())
        }
    }
}

async fn run_bt(args: BtArgs) -> Result<(), String> {
    let filter =
        std::env::var("RUST_LOG").unwrap_or_else(|_| "tug_service=info,librqbit=warn".to_owned());
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

    let root = args.data.unwrap_or_else(|| {
        std::env::temp_dir().join(format!(
            "tug-bt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_secs())
                .unwrap_or(0)
        ))
    });
    std::fs::create_dir_all(&root).map_err(|err| err.to_string())?;
    let paths = AppPaths::under_root(&root);
    paths.ensure().map_err(|err| err.to_string())?;

    let mut settings = Settings::initial(&paths);
    if let Some(output) = args.output.clone() {
        settings.download_dir = output;
    }
    let download_dir = settings.download_dir.clone();
    let settings = Arc::new(RwLock::new(settings));
    let manager = TaskManager::new(&paths, Arc::clone(&settings)).map_err(|err| err.to_string())?;

    println!("data:   {}", root.display());
    println!("output: {}", download_dir.display());
    println!("source: {}", args.source);

    let task = if let Some(id) = args.resume_id {
        manager.resume(id).await.map_err(|err| err.to_string())?
    } else {
        manager
            .add(AddTaskInput {
                url: args.source,
                save_dir: Some(download_dir),
                file_name: None,
                connections: None,
                only_files: None,
            })
            .await
            .map_err(|err| err.to_string())?
    };
    let id = task.id;
    println!("task:   {id} ({})", task.name);

    let started = Instant::now();
    let mut paused = false;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let elapsed = started.elapsed();
        let view = manager
            .list()
            .map_err(|err| err.to_string())?
            .into_iter()
            .find(|task| task.id == id)
            .ok_or_else(|| "任务不存在".to_owned())?;

        let total = view.total.unwrap_or(0);
        let percent = if total > 0 {
            view.downloaded as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        println!(
            "[{:>3}s] {:<18} {:>5.1}% {:>10} / {:>10} down {:>10}/s up {:>8}/s peers={}",
            elapsed.as_secs(),
            view.state.as_str(),
            percent,
            format_bytes(view.downloaded),
            if total > 0 {
                format_bytes(total)
            } else {
                "?".to_owned()
            },
            format_bytes(view.speed_bps),
            format_bytes(view.upload_speed_bps),
            view.peers
        );

        if let Some(after) = args.pause_after
            && !paused
            && elapsed.as_secs() >= after
        {
            paused = true;
            println!("暂停…");
            manager.pause(id).await.map_err(|err| err.to_string())?;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let paused_view = manager
                .list()
                .map_err(|err| err.to_string())?
                .into_iter()
                .find(|task| task.id == id)
                .ok_or_else(|| "任务不存在".to_owned())?;
            println!("暂停后状态: {}", paused_view.state.as_str());
            println!("继续…");
            manager.resume(id).await.map_err(|err| err.to_string())?;
        }

        match view.state {
            TaskState::Completed => {
                println!(
                    "完成: {}（{}，上传 {}）",
                    view.path,
                    format_bytes(view.downloaded),
                    format_bytes(view.uploaded)
                );
                return Ok(());
            }
            TaskState::Failed => {
                return Err(view.error.unwrap_or_else(|| "BT 任务失败".to_owned()));
            }
            _ => {}
        }

        if elapsed.as_secs() >= args.seconds {
            println!("达到最长运行时间，暂停任务");
            manager.pause(id).await.map_err(|err| err.to_string())?;
            return Err(format!("{} 秒内未完成", args.seconds));
        }
    }
}

fn parse_headers(values: &[String]) -> Result<Vec<(String, String)>, String> {
    values
        .iter()
        .map(|value| {
            let (name, content) = value
                .split_once(':')
                .ok_or_else(|| format!("请求头格式应为 \"Name: Value\": {value}"))?;
            Ok((name.trim().to_owned(), content.trim().to_owned()))
        })
        .collect()
}

fn print_progress(progress: &Progress) {
    let downloaded = format_bytes(progress.downloaded);
    let total = progress
        .total
        .map(format_bytes)
        .unwrap_or_else(|| "?".to_owned());
    let speed = format!("{}/s", format_bytes(progress.speed_bps));
    let eta = progress
        .eta_secs
        .map(format_duration)
        .unwrap_or_else(|| "--:--".to_owned());

    let prefix = match progress.total {
        Some(total) if total > 0 => {
            let percent = progress.downloaded as f64 / total as f64 * 100.0;
            format!("{percent:5.1}%")
        }
        _ => "  ?.?%".to_owned(),
    };

    print!(
        "\r{prefix}  {downloaded}/{total}  {speed}  ETA {eta}  [{} conn, {}/{} seg]\x1b[K",
        progress.active_connections, progress.completed_segments, progress.total_segments
    );
    let _ = io::stdout().flush();
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_duration(seconds: u64) -> String {
    if seconds >= 3600 {
        format!(
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            (seconds % 3600) / 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}
