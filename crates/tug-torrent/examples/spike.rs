//! M3 BT Spike：用 librqbit 跑磁力/种子，输出元数据耗时、peer 数、速度与 DHT-only 结果。
//!
//! 用法：
//! `cargo run --release -p tug-torrent --example spike -- <magnet|torrent|url> [输出目录] [--seconds 60] [--dht-only] [--pause-after 10] [--upnp]`

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, ListenerOptions, Session,
    SessionOptions, TorrentMetadata,
};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("librqbit=info,tug_torrent=info")
        .init();

    let mut source: Option<String> = None;
    let mut output = PathBuf::from("D:\\Tug\\.cache\\bt-spike");
    let mut max_seconds = 60u64;
    let mut dht_only = false;
    let mut pause_after = None;
    let mut upnp = false;
    let mut ipv4_only = false;
    let mut bootstrap: Option<Vec<String>> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => {
                max_seconds = args
                    .next()
                    .context("--seconds 需要数字")?
                    .parse()
                    .context("--seconds 不是数字")?;
            }
            "--dht-only" => dht_only = true,
            "--pause-after" => {
                pause_after = Some(
                    args.next()
                        .context("--pause-after 需要数字")?
                        .parse::<u64>()
                        .context("--pause-after 不是数字")?,
                );
            }
            "--upnp" => upnp = true,
            "--ipv4" => ipv4_only = true,
            "--bootstrap" => {
                let value = args.next().context("--bootstrap 需要 host:port")?;
                bootstrap = Some(value.split(',').map(str::to_owned).collect());
            }
            _ if source.is_none() => source = Some(arg),
            _ => output = PathBuf::from(arg),
        }
    }

    let source = source.context(
        "用法: spike <magnet|torrent|url> [输出目录] [--seconds N] [--dht-only] [--pause-after N] [--upnp]",
    )?;
    std::fs::create_dir_all(&output)?;

    let mut options = SessionOptions {
        fastresume: true,
        disable_trackers: dht_only,
        peer_limit: Some(100),
        ipv4_only,
        client_name_and_version: Some(format!("Tug/{}", env!("CARGO_PKG_VERSION"))),
        listen: Some(ListenerOptions {
            enable_upnp_port_forwarding: upnp,
            ..Default::default()
        }),
        ..Default::default()
    };
    options.persistence = None;
    if let Some(addrs) = bootstrap {
        options.dht = Some(DhtSessionConfig {
            bootstrap_addrs: Some(addrs),
            ..Default::default()
        });
    }

    let session = Session::new_with_opts(output.clone(), options)
        .await
        .context("创建 BT session 失败")?;

    println!("source={source}");
    println!("output={}", output.display());
    println!("dht_only={dht_only} upnp={upnp} max_seconds={max_seconds}");

    let started = Instant::now();
    let add_result = tokio::time::timeout(
        Duration::from_secs(30),
        session.add_torrent(
            AddTorrent::from_cli_argument(&source)?,
            Some(AddTorrentOptions {
                overwrite: true,
                ..Default::default()
            }),
        ),
    )
    .await;

    let response = match add_result {
        Ok(result) => result.context("添加种子失败")?,
        Err(_) => {
            println!(
                "SPIKE_SUMMARY metadata=false error=metadata_timeout elapsed_s=30 dht_only={dht_only}"
            );
            return Ok(());
        }
    };

    let handle = match response {
        AddTorrentResponse::Added(_, handle) | AddTorrentResponse::AlreadyManaged(_, handle) => {
            handle
        }
        AddTorrentResponse::ListOnly(_) => return Err(anyhow!("list-only 模式没有句柄")),
    };

    // 磁力链接的元数据可能稍后到达。
    let mut metadata = None;
    for _ in 0..120 {
        if let Ok(info) = handle.with_metadata(|meta| extract_metadata(meta)) {
            metadata = Some(info);
            break;
        }
        sleep(Duration::from_millis(250)).await;
    }

    let metadata = metadata.ok_or_else(|| anyhow!("30 秒内没有拿到元数据"))?;
    let metadata_seconds = started.elapsed().as_secs_f64();
    println!(
        "metadata_seconds={metadata_seconds:.2} name={} total_bytes={} files={} pieces={}",
        metadata.name, metadata.total, metadata.files, metadata.pieces
    );

    let mut max_live_peers = 0u32;
    let mut max_seen_peers = 0u32;
    let mut max_speed = 0u64;
    let mut paused = false;

    loop {
        sleep(Duration::from_secs(1)).await;
        let elapsed = started.elapsed();
        let stats = handle.stats();

        let (live, seen, speed, eta) = if let Some(live) = &stats.live {
            (
                live.snapshot.peer_stats.live,
                live.snapshot.peer_stats.seen,
                live.download_speed.as_bytes(),
                live.time_remaining
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "--".to_owned()),
            )
        } else {
            (0, 0, 0, "--".to_owned())
        };

        max_live_peers = max_live_peers.max(live);
        max_seen_peers = max_seen_peers.max(seen);
        max_speed = max_speed.max(speed);

        let percent = if metadata.total > 0 {
            stats.progress_bytes as f64 / metadata.total as f64 * 100.0
        } else {
            0.0
        };
        println!(
            "[{:>3}s] state={} {:>5.1}% {:>10} / {:>10} {:>12} peers live={} seen={} connecting={} eta={}",
            elapsed.as_secs(),
            stats.state,
            percent,
            human_bytes(stats.progress_bytes),
            human_bytes(metadata.total),
            human_speed(speed),
            live,
            seen,
            stats
                .live
                .as_ref()
                .map(|live| live.snapshot.peer_stats.connecting)
                .unwrap_or(0),
            eta
        );

        if let Some(after) = pause_after
            && !paused
            && elapsed.as_secs() >= after
        {
            paused = true;
            println!("pause_after={after}s -> pause");
            session.pause(&handle).await.context("暂停失败")?;
            sleep(Duration::from_secs(2)).await;
            let paused_stats = handle.stats();
            println!(
                "paused state={} progress_bytes={}",
                paused_stats.state, paused_stats.progress_bytes
            );
            session.unpause(&handle).await.context("继续失败")?;
            println!("resumed");
        }

        if stats.finished || elapsed.as_secs() >= max_seconds {
            let avg = stats.progress_bytes as f64 / elapsed.as_secs_f64().max(0.001);
            println!(
                "SPIKE_SUMMARY elapsed_s={:.1} metadata_s={:.2} total_bytes={} downloaded_bytes={} avg_bps={:.0} max_live_peers={} max_seen_peers={} max_speed_bps={} finished={} dht_only={}",
                elapsed.as_secs_f64(),
                metadata_seconds,
                metadata.total,
                stats.progress_bytes,
                avg,
                max_live_peers,
                max_seen_peers,
                max_speed,
                stats.finished,
                dht_only
            );
            break;
        }
    }

    session.pause(&handle).await.ok();
    Ok(())
}

struct MetadataInfo {
    name: String,
    total: u64,
    files: usize,
    pieces: usize,
}

fn extract_metadata(metadata: &TorrentMetadata) -> MetadataInfo {
    let name = metadata
        .info
        .name()
        .map(|value| value.into_owned())
        .unwrap_or_else(|| "-".to_owned());
    let total = metadata.lengths().total_length();
    let files = metadata.file_infos.len();
    let pieces = metadata.lengths().total_pieces() as usize;

    MetadataInfo {
        name,
        total,
        files,
        pieces,
    }
}

fn human_bytes(bytes: u64) -> String {
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

fn human_speed(bytes_per_second: u64) -> String {
    format!("{}/s", human_bytes(bytes_per_second))
}
