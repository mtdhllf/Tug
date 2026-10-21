//! librqbit 适配层：会话、添加、暂停/继续、文件选择与统计。
//!
//! 任务调度、持久化与 UI 不在这里，避免 BT 引擎和 Tauri 耦合。

use std::collections::HashSet;
use std::net::{Ipv6Addr, SocketAddr};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, ListenerOptions,
    ManagedTorrent, Session, SessionOptions, SessionPersistenceConfig,
};
use serde::Serialize;

/// 磁力缺少 `tr` 时追加的公共 tracker（预编码，避免额外依赖）。
pub const DEFAULT_TRACKERS: &[&str] = &[
    "udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce",
    "udp%3A%2F%2Fopen.stealth.si%3A80%2Fannounce",
    "udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce",
    "udp%3A%2F%2Ftracker.openbittorrent.com%3A6969%2Fannounce",
];

/// DHT 启动节点。librqbit 默认只带两个，部分网络环境下全部不可达时
/// 重启后会出现“任务在下载但 Peers 为 0”。多给几个常见节点提高成功率。
const DHT_BOOTSTRAP: &[&str] = &[
    "dht.transmissionbt.com:6881",
    "dht.libtorrent.org:25401",
    "router.bittorrent.com:6881",
    "router.utorrent.com:6881",
    "dht.aelitis.com:6881",
];

/// BT 会话配置。
#[derive(Debug, Clone)]
pub struct TorrentConfig {
    /// 是否启用 DHT。
    pub dht: bool,
    /// 监听端口；`None` 随机。
    pub listen_port: Option<u16>,
    /// 是否启用 UPnP 端口映射。
    pub upnp: bool,
    /// 上传限速（字节/秒）。
    pub upload_limit_bps: Option<u64>,
}

impl Default for TorrentConfig {
    fn default() -> Self {
        Self {
            dht: true,
            listen_port: None,
            upnp: true,
            upload_limit_bps: None,
        }
    }
}

/// 添加参数。
#[derive(Debug, Clone, Default)]
pub struct TorrentAddOptions {
    /// 只下载这些文件序号；`None` 表示全部。
    pub only_files: Option<Vec<usize>>,
    /// 覆盖会话默认输出目录。
    pub output_folder: Option<PathBuf>,
}

/// 一个已加入 session 的种子句柄。
#[derive(Clone)]
pub struct TorrentHandle {
    inner: Arc<ManagedTorrent>,
}

impl TorrentHandle {
    /// session 内数字 id，可用于删除。
    pub fn id(&self) -> usize {
        self.inner.id()
    }

    /// v1 info hash（hex）。
    pub fn info_hash(&self) -> String {
        self.inner.info_hash().as_string()
    }

    /// 种子名；元数据未就绪时为空。
    pub fn name(&self) -> String {
        self.inner
            .with_metadata(|meta| meta.info.name().map(|name| name.into_owned()))
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// 文件列表。
    pub fn files(&self) -> Vec<TorrentFileInfo> {
        let selected = self.inner.only_files();
        self.inner
            .with_metadata(|meta| {
                meta.file_infos
                    .iter()
                    .enumerate()
                    .map(|(index, file)| TorrentFileInfo {
                        index,
                        path: file.relative_filename.display().to_string(),
                        length: file.len,
                        selected: selected.as_ref().is_none_or(|files| files.contains(&index)),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 当前统计快照。
    pub fn snapshot(&self) -> TorrentSnapshot {
        let stats = self.inner.stats();
        let name = self.name();
        let (download_bps, upload_bps, peers) = stats.live.as_ref().map_or((0, 0, 0), |live| {
            (
                live.download_speed.as_bytes(),
                live.upload_speed.as_bytes(),
                live.snapshot.peer_stats.live,
            )
        });
        let eta_secs = if download_bps > 0 && stats.total_bytes > stats.progress_bytes {
            Some((stats.total_bytes - stats.progress_bytes) / download_bps)
        } else {
            None
        };

        TorrentSnapshot {
            name,
            total: stats.total_bytes,
            downloaded: stats.progress_bytes,
            uploaded: stats.uploaded_bytes,
            download_bps,
            upload_bps,
            peers,
            state: stats.state.to_string(),
            finished: stats.finished,
            error: stats.error,
            eta_secs,
        }
    }

    /// 当前 live peer 与 tracker 列表。
    pub fn runtime_details(&self) -> TorrentRuntimeDetails {
        let mut peers = self
            .inner
            .live()
            .map(|live| {
                let snapshot = live.per_peer_stats_snapshot(Default::default());
                snapshot
                    .peers
                    .into_iter()
                    .map(|(address, peer)| TorrentPeerInfo {
                        address,
                        state: peer.state.to_owned(),
                        client: peer.client_name,
                        downloaded: peer.counters.fetched_bytes,
                        uploaded: peer.counters.uploaded_bytes,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        peers.sort_by(|left, right| left.address.cmp(&right.address));

        let mut trackers = self
            .inner
            .shared()
            .trackers
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        trackers.sort();

        TorrentRuntimeDetails { peers, trackers }
    }
}

/// 统计快照，供 service 转成 `TaskView`。
#[derive(Debug, Clone)]
pub struct TorrentSnapshot {
    pub name: String,
    pub total: u64,
    pub downloaded: u64,
    pub uploaded: u64,
    pub download_bps: u64,
    pub upload_bps: u64,
    pub peers: u32,
    pub state: String,
    pub finished: bool,
    pub error: Option<String>,
    pub eta_secs: Option<u64>,
}

/// 文件信息。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFileInfo {
    pub index: usize,
    pub path: String,
    pub length: u64,
    pub selected: bool,
}

/// 当前连接/见过的 peer。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentPeerInfo {
    pub address: String,
    pub state: String,
    pub client: Option<String>,
    pub downloaded: u64,
    pub uploaded: u64,
}

/// 运行时详情（peers + trackers）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentRuntimeDetails {
    pub peers: Vec<TorrentPeerInfo>,
    pub trackers: Vec<String>,
}

/// BT 会话。内部 `Arc<Session>`，可在线程间共享。
pub struct TorrentSession {
    session: Arc<Session>,
}

impl TorrentSession {
    /// 创建会话；默认输出目录只作为未指定 `output_folder` 时的回退。
    /// `state_dir` 保存 librqbit 的 session 状态与 piece 状态，用于重启 fastresume。
    pub async fn open(
        default_output: PathBuf,
        state_dir: PathBuf,
        config: TorrentConfig,
    ) -> anyhow::Result<Arc<Self>> {
        let listen_addr = match config.listen_port {
            Some(port) => SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port),
            None => SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0),
        };
        let upload_limit = config
            .upload_limit_bps
            .and_then(|bps| u32::try_from(bps).ok())
            .and_then(NonZeroU32::new);
        let options = SessionOptions {
            fastresume: true,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(state_dir),
            }),
            dht: config.dht.then(|| DhtSessionConfig {
                bootstrap_addrs: Some(
                    DHT_BOOTSTRAP
                        .iter()
                        .map(|addr| (*addr).to_owned())
                        .collect(),
                ),
                ..Default::default()
            }),
            listen: Some(ListenerOptions {
                listen_addr,
                enable_upnp_port_forwarding: config.upnp,
                ..Default::default()
            }),
            peer_limit: Some(100),
            ratelimits: LimitsConfig {
                upload_bps: upload_limit,
                download_bps: None,
            },
            client_name_and_version: Some(format!("Tug/{}", env!("CARGO_PKG_VERSION"))),
            ..Default::default()
        };
        let session = Session::new_with_opts(default_output, options)
            .await
            .context("创建 librqbit session 失败")?;

        // 持久化恢复的种子可能自动开始；统一暂停，等用户点“继续”。
        let restored: Vec<usize> =
            session.with_torrents(|torrents| torrents.map(|(id, _)| id).collect());
        for id in restored {
            if let Some(handle) = session.get(librqbit::api::TorrentIdOrHash::Id(id))
                && let Err(err) = session.pause(&handle).await
            {
                tracing::warn!(id, %err, "暂停恢复的 BT 任务失败");
            }
        }

        Ok(Arc::new(Self { session }))
    }

    /// 添加磁力、种子 URL 或本地 `.torrent` 路径。
    pub async fn add(
        &self,
        source: &str,
        options: TorrentAddOptions,
    ) -> anyhow::Result<TorrentHandle> {
        let prepared = with_default_trackers(source);
        let output = options
            .output_folder
            .map(|path| path.to_string_lossy().into_owned());
        let response = self
            .session
            .add_torrent(
                AddTorrent::from_cli_argument(&prepared)?,
                Some(AddTorrentOptions {
                    overwrite: true,
                    only_files: options.only_files,
                    output_folder: output,
                    ..Default::default()
                }),
            )
            .await
            .context("添加 BT 任务失败")?;

        match response {
            AddTorrentResponse::Added(_, handle)
            | AddTorrentResponse::AlreadyManaged(_, handle) => Ok(TorrentHandle { inner: handle }),
            AddTorrentResponse::ListOnly(_) => Err(anyhow!("list-only 模式没有下载句柄")),
        }
    }

    /// 暂停。
    pub async fn pause(&self, handle: &TorrentHandle) -> anyhow::Result<()> {
        self.session.pause(&handle.inner).await
    }

    /// 继续。
    pub async fn resume(&self, handle: &TorrentHandle) -> anyhow::Result<()> {
        self.session.unpause(&handle.inner).await
    }

    /// 更新选中文件。
    pub async fn set_files(
        &self,
        handle: &TorrentHandle,
        indices: Vec<usize>,
    ) -> anyhow::Result<()> {
        let files: HashSet<usize> = indices.into_iter().collect();
        self.session.update_only_files(&handle.inner, &files).await
    }

    /// 从 session 删除；`delete_files` 为真时同时删除已下载文件。
    pub async fn delete(&self, id: usize, delete_files: bool) -> anyhow::Result<()> {
        self.session
            .delete(librqbit::api::TorrentIdOrHash::Id(id), delete_files)
            .await
    }

    /// 按 session id 取句柄。
    pub fn get(&self, id: usize) -> Option<TorrentHandle> {
        self.session
            .get(librqbit::api::TorrentIdOrHash::Id(id))
            .map(|inner| TorrentHandle { inner })
    }

    /// 运行时更新上传限速。
    pub fn set_upload_limit(&self, bps: Option<u64>) {
        let limit = bps
            .and_then(|value| u32::try_from(value).ok())
            .and_then(NonZeroU32::new);
        self.session.ratelimits.set_upload_bps(limit);
    }
}

/// 磁力无 `tr` 时追加默认 tracker。
pub fn with_default_trackers(source: &str) -> String {
    if source.starts_with("magnet:") && !source.contains("tr=") {
        let mut result = source.to_owned();
        for tracker in DEFAULT_TRACKERS {
            result.push_str("&tr=");
            result.push_str(tracker);
        }
        result
    } else {
        source.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_trackers_only_when_missing() {
        let bare = "magnet:?xt=urn:btih:abc";
        let with_tracker = "magnet:?xt=urn:btih:abc&tr=udp%3A%2F%2Fexample.org%3A80";
        assert!(with_default_trackers(bare).contains("tr=udp%3A%2F%2Ftracker.opentrackr.org"));
        assert_eq!(with_default_trackers(with_tracker), with_tracker);
        assert_eq!(
            with_default_trackers("https://example.com/a.torrent"),
            "https://example.com/a.torrent"
        );
    }
}
