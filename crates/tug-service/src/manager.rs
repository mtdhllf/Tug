use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use tug_core::{AppPaths, NewTask, Settings, TaskRecord, TaskState, TaskStore, TaskView};
use tug_http::{CancelToken, DownloadOptions, HttpError, Progress, ProgressCallback};
use tug_torrent::{
    TorrentAddOptions, TorrentConfig, TorrentFileInfo, TorrentHandle, TorrentRuntimeDetails,
    TorrentSession, TorrentSnapshot,
};

/// 服务层错误。
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// 核心库错误。
    #[error(transparent)]
    Core(#[from] tug_core::TugError),

    /// 输入非法。
    #[error("{0}")]
    Invalid(String),

    /// 任务不存在。
    #[error("任务不存在: {0}")]
    NotFound(i64),

    /// 任务仍在运行。
    #[error("任务正在运行中，请先暂停")]
    Busy,

    /// BT 引擎错误。
    #[error("BT 引擎错误: {0}")]
    Torrent(String),
}

/// 推给前端的事件。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TaskEvent {
    /// 任务更新（进度、状态）。
    Updated {
        /// 最新视图。
        task: Box<TaskView>,
    },
    /// 任务已移除。
    Removed {
        /// 任务主键。
        id: i64,
    },
}

/// 新建任务的输入（与前端 JSON 对应）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTaskInput {
    /// HTTP/HTTPS 链接、磁力或本地 `.torrent` 路径。
    pub url: String,
    /// 输出目录；为空时使用设置中的默认目录。
    pub save_dir: Option<PathBuf>,
    /// 覆盖文件名。
    pub file_name: Option<String>,
    /// 连接数。
    pub connections: Option<u8>,
    /// BT 文件选择；`None` 表示全部。
    pub only_files: Option<Vec<usize>>,
}

struct Runtime {
    cancel: CancelToken,
    handle: Option<JoinHandle<()>>,
    view: TaskView,
    last_persist: Instant,
}

enum TorrentFinish {
    Paused,
    Failed(String),
    Completed {
        total: u64,
        name: String,
        file_name: Option<String>,
        path: PathBuf,
    },
}

/// 任务管理器。
pub struct TaskManager {
    store: Arc<TaskStore>,
    settings: Arc<RwLock<Settings>>,
    runtimes: Mutex<HashMap<i64, Runtime>>,
    scheduler: tokio::sync::Mutex<()>,
    events: broadcast::Sender<TaskEvent>,
    torrent: tokio::sync::OnceCell<Arc<TorrentSession>>,
    bt_state_dir: PathBuf,
}

impl TaskManager {
    /// 打开任务库、恢复中断任务，并创建事件通道。
    pub fn new(
        paths: &AppPaths,
        settings: Arc<RwLock<Settings>>,
    ) -> Result<Arc<Self>, ServiceError> {
        let store = TaskStore::open(&paths.data_dir.join("tasks.db"))?;
        let interrupted = store.mark_interrupted()?;
        if interrupted > 0 {
            tracing::info!(
                count = interrupted,
                "上次任务已恢复：未完成转暂停，已完成保持完成"
            );
        }

        let (events, _) = broadcast::channel(256);
        Ok(Arc::new(Self {
            store: Arc::new(store),
            settings,
            runtimes: Mutex::new(HashMap::new()),
            scheduler: tokio::sync::Mutex::new(()),
            events,
            torrent: tokio::sync::OnceCell::new(),
            bt_state_dir: paths.data_dir.join("bt-session"),
        }))
    }

    /// 订阅任务事件。
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.events.subscribe()
    }

    /// 列出所有任务（运行中的任务使用实时视图）。
    pub fn list(&self) -> Result<Vec<TaskView>, ServiceError> {
        let records = self.store.list()?;
        let runtimes = self.runtimes.lock().expect("runtimes poisoned");
        Ok(records
            .into_iter()
            .map(|record| {
                runtimes
                    .get(&record.id)
                    .map(|runtime| runtime.view.clone())
                    .unwrap_or_else(|| record.view())
            })
            .collect())
    }

    /// 新建任务并入队；有空闲槽位时立即开始。
    pub async fn add(self: &Arc<Self>, input: AddTaskInput) -> Result<TaskView, ServiceError> {
        let mut source = input.url.trim().to_owned();
        let is_torrent = is_torrent_source(&source);
        if !is_torrent && !(source.starts_with("http://") || source.starts_with("https://")) {
            return Err(ServiceError::Invalid(
                "支持 HTTP/HTTPS、磁力链接或 .torrent 文件".into(),
            ));
        }
        if is_torrent && Path::new(&source).is_file() {
            source = std::fs::canonicalize(&source)
                .map_err(|err| ServiceError::Invalid(err.to_string()))?
                .display()
                .to_string();
        }

        let settings = self.settings.read().expect("settings poisoned").clone();
        let save_dir = input.save_dir.unwrap_or(settings.download_dir);
        let connections = input
            .connections
            .unwrap_or(settings.connections_per_task)
            .clamp(1, 32);

        if is_torrent {
            let record = self.store.insert(&NewTask {
                kind: "torrent".into(),
                name: torrent_display_name(&source),
                url: source,
                save_dir,
                file_name: None,
                connections,
                rate_limit_bps: None,
                headers: Vec::new(),
                proxy: None,
                checksum: None,
                extra: serde_json::json!({
                    "only_files": input.only_files.unwrap_or_default(),
                }),
            })?;
            let _ = self.events.send(TaskEvent::Updated {
                task: Box::new(record.view()),
            });
            self.pump().await;
            return self.task_view(record.id);
        }

        let requested_file_name = input
            .file_name
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(|value| tug_http::sanitize_file_name(&value));

        // 同一目录下显式指定同名文件时，先做任务级去重，避免并发任务互相覆盖。
        let used_paths: HashSet<PathBuf> = self
            .list()?
            .into_iter()
            .map(|task| PathBuf::from(task.path))
            .collect();
        let file_name = match requested_file_name {
            Some(name) => {
                let candidate = save_dir.join(&name);
                if used_paths.contains(&candidate) || candidate.exists() {
                    Some(unique_task_name(&save_dir, &name, &used_paths))
                } else {
                    Some(name)
                }
            }
            None => None,
        };

        let name = file_name
            .clone()
            .or_else(|| tug_http::guess_file_name(&source))
            .unwrap_or_else(|| "download".to_owned());
        let name = tug_http::sanitize_file_name(&name);

        let record = self.store.insert(&NewTask {
            kind: "http".into(),
            name,
            url: source,
            save_dir,
            file_name,
            connections,
            rate_limit_bps: None,
            headers: Vec::new(),
            proxy: None,
            checksum: None,
            extra: serde_json::json!({}),
        })?;

        let _ = self.events.send(TaskEvent::Updated {
            task: Box::new(record.view()),
        });
        self.pump().await;
        self.task_view(record.id)
    }

    /// 懒加载 BT 会话；HTTP-only 启动不会初始化 DHT/监听端口。
    async fn torrent_session(&self) -> Result<Arc<TorrentSession>, ServiceError> {
        let settings = self.settings.read().expect("settings poisoned").clone();
        let output = settings.download_dir.clone();
        let config = TorrentConfig {
            dht: settings.bt_dht,
            listen_port: settings.bt_listen_port,
            upnp: settings.bt_upnp,
            upload_limit_bps: settings
                .bt_upload_limit_kib
                .map(|kib| u64::from(kib).saturating_mul(1024)),
        };
        self.torrent
            .get_or_try_init(|| async {
                TorrentSession::open(output, self.bt_state_dir.clone(), config)
                    .await
                    .map_err(|err| ServiceError::Torrent(err.to_string()))
            })
            .await
            .cloned()
    }

    /// 列出 BT 任务文件。元数据未就绪时返回可读错误。
    pub async fn torrent_files(&self, id: i64) -> Result<Vec<TorrentFileInfo>, ServiceError> {
        let record = self.store.get(id)?.ok_or(ServiceError::NotFound(id))?;
        if record.kind != "torrent" {
            return Err(ServiceError::Invalid("不是 BT 任务".into()));
        }
        let torrent_id = record
            .extra
            .get("librqbit_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ServiceError::Invalid("元数据尚未就绪".into()))?;
        let session = self.torrent_session().await?;
        let handle = session
            .get(torrent_id as usize)
            .ok_or_else(|| ServiceError::Invalid("BT 会话中找不到任务".into()))?;
        Ok(handle.files())
    }

    /// 列出 BT 任务的 live peer 与 tracker。
    pub async fn torrent_runtime(&self, id: i64) -> Result<TorrentRuntimeDetails, ServiceError> {
        let record = self.store.get(id)?.ok_or(ServiceError::NotFound(id))?;
        if record.kind != "torrent" {
            return Err(ServiceError::Invalid("不是 BT 任务".into()));
        }
        let torrent_id = record
            .extra
            .get("librqbit_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ServiceError::Invalid("元数据尚未就绪".into()))?;
        let session = self.torrent_session().await?;
        let handle = session
            .get(torrent_id as usize)
            .ok_or_else(|| ServiceError::Invalid("BT 会话中找不到任务".into()))?;
        Ok(handle.runtime_details())
    }

    /// 更新 BT 文件选择。运行中立即生效；排队任务在启动时生效。
    pub async fn set_torrent_files(
        &self,
        id: i64,
        indices: Vec<usize>,
    ) -> Result<(), ServiceError> {
        let record = self.store.get(id)?.ok_or(ServiceError::NotFound(id))?;
        if record.kind != "torrent" {
            return Err(ServiceError::Invalid("不是 BT 任务".into()));
        }
        if record.state == TaskState::Completed {
            return Err(ServiceError::Invalid("任务已完成，不能修改文件选择".into()));
        }

        let mut indices = indices;
        indices.sort_unstable();
        indices.dedup();
        if indices.is_empty() {
            return Err(ServiceError::Invalid("至少选择一个文件".into()));
        }

        let session = self.torrent_session().await?;
        if let Some(torrent_id) = record
            .extra
            .get("librqbit_id")
            .and_then(serde_json::Value::as_u64)
            && let Some(handle) = session.get(torrent_id as usize)
        {
            let files = handle.files();
            if indices.iter().any(|index| *index >= files.len()) {
                return Err(ServiceError::Invalid("文件序号越界".into()));
            }
            session
                .set_files(&handle, indices.clone())
                .await
                .map_err(|err| ServiceError::Torrent(err.to_string()))?;
        }

        let mut extra = record.extra;
        extra["only_files"] = serde_json::json!(indices);
        self.store.set_extra(id, &extra)?;
        Ok(())
    }

    /// 按并发上限启动排队任务；由 add/resume/finish/设置变更触发。
    pub async fn pump(self: &Arc<Self>) {
        let _guard = self.scheduler.lock().await;

        loop {
            let limit = self
                .settings
                .read()
                .expect("settings poisoned")
                .max_concurrent_tasks as usize;
            let active = self
                .runtimes
                .lock()
                .expect("runtimes poisoned")
                .values()
                .filter(|runtime| runtime.view.state != TaskState::Seeding)
                .count();
            if active >= limit {
                break;
            }

            let next = match self.store.next_queued() {
                Ok(Some(record)) => record,
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!(%err, "读取排队任务失败");
                    break;
                }
            };
            let id = next.id;

            if let Err(err) = self.start(next) {
                let message = err.to_string();
                tracing::warn!(task_id = id, %message, "启动排队任务失败");
                let _ = self.store.set_state(id, TaskState::Failed, Some(&message));
                if let Ok(Some(record)) = self.store.get(id) {
                    let _ = self.events.send(TaskEvent::Updated {
                        task: Box::new(record.view()),
                    });
                }
            }
        }
    }

    /// 设置变更后重新调度（例如提高并发上限）。
    pub async fn settings_changed(self: &Arc<Self>) {
        if let Some(session) = self.torrent.get() {
            let limit = self
                .settings
                .read()
                .expect("settings poisoned")
                .bt_upload_limit_kib
                .map(|kib| u64::from(kib).saturating_mul(1024));
            session.set_upload_limit(limit);
        }
        self.pump().await;
    }

    /// 暂停任务并等待下载任务退出（checkpoint 已落盘）。
    pub async fn pause(self: &Arc<Self>, id: i64) -> Result<(), ServiceError> {
        let running = {
            let mut runtimes = self.runtimes.lock().expect("runtimes poisoned");
            runtimes
                .get_mut(&id)
                .map(|runtime| (runtime.cancel.clone(), runtime.handle.take()))
        };

        if let Some((cancel, handle)) = running {
            cancel.cancel();
            if let Some(handle) = handle {
                let _ = handle.await;
            }
        } else if let Some(record) = self.store.get(id)?
            && matches!(
                record.state,
                TaskState::Queued
                    | TaskState::FetchingMetadata
                    | TaskState::Downloading
                    | TaskState::Seeding
            )
        {
            self.store.set_state(id, TaskState::Paused, None)?;
            let mut view = record.view();
            view.state = TaskState::Paused;
            view.speed_bps = 0;
            view.eta_secs = None;
            view.error = None;
            let _ = self.events.send(TaskEvent::Updated {
                task: Box::new(view),
            });
        }

        self.pump().await;
        Ok(())
    }

    /// 继续已暂停/失败的任务；没有空闲槽位时进入排队。
    pub async fn resume(self: &Arc<Self>, id: i64) -> Result<TaskView, ServiceError> {
        if self
            .runtimes
            .lock()
            .expect("runtimes poisoned")
            .contains_key(&id)
        {
            return Err(ServiceError::Busy);
        }

        let record = self.store.get(id)?.ok_or(ServiceError::NotFound(id))?;
        if record.state == TaskState::Completed {
            return Err(ServiceError::Invalid("任务已完成".into()));
        }

        self.store.set_state(id, TaskState::Queued, None)?;
        let mut view = record.view();
        view.state = TaskState::Queued;
        view.error = None;
        view.speed_bps = 0;
        view.eta_secs = None;
        let _ = self.events.send(TaskEvent::Updated {
            task: Box::new(view),
        });

        self.pump().await;
        self.task_view(id)
    }

    /// 移除任务；可选删除已下载文件与续传 sidecar。
    pub async fn remove(self: &Arc<Self>, id: i64, delete_files: bool) -> Result<(), ServiceError> {
        self.pause(id).await?;
        let record = self.store.get(id)?.ok_or(ServiceError::NotFound(id))?;

        self.runtimes.lock().expect("runtimes poisoned").remove(&id);
        self.store.delete(id)?;

        if record.kind == "torrent" {
            let expected = record
                .extra
                .get("info_hash")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let torrent_id = record
                .extra
                .get("librqbit_id")
                .and_then(serde_json::Value::as_u64);
            let mut deleted = false;

            if let Some(torrent_id) = torrent_id
                && let Ok(session) = self.torrent_session().await
            {
                let handle = session.get(torrent_id as usize);
                let matches = handle.as_ref().is_none_or(|handle| {
                    expected
                        .as_deref()
                        .is_none_or(|hash| handle.info_hash() == hash)
                });
                if matches {
                    match session.delete(torrent_id as usize, delete_files).await {
                        Ok(()) => deleted = true,
                        Err(err) => tracing::warn!(%err, "从 BT session 删除任务失败"),
                    }
                }
            }

            if delete_files && !deleted {
                delete_files_for(&record);
            }
        } else if delete_files {
            delete_files_for(&record);
        }

        let _ = self.events.send(TaskEvent::Removed { id });
        self.pump().await;
        Ok(())
    }

    /// 暂停所有活动/排队任务。
    pub async fn pause_all(self: &Arc<Self>) -> Result<(), ServiceError> {
        let ids: Vec<i64> = self
            .list()?
            .into_iter()
            .filter(|task| {
                matches!(
                    task.state,
                    TaskState::Queued
                        | TaskState::FetchingMetadata
                        | TaskState::Downloading
                        | TaskState::Seeding
                )
            })
            .map(|task| task.id)
            .collect();
        for id in ids {
            let _ = self.pause(id).await;
        }
        Ok(())
    }

    /// 继续所有已暂停任务。
    pub async fn resume_all(self: &Arc<Self>) -> Result<(), ServiceError> {
        let ids: Vec<i64> = self
            .list()?
            .into_iter()
            .filter(|task| task.state == TaskState::Paused)
            .map(|task| task.id)
            .collect();
        for id in ids {
            let _ = self.resume(id).await;
        }
        Ok(())
    }

    fn task_view(&self, id: i64) -> Result<TaskView, ServiceError> {
        if let Some(runtime) = self.runtimes.lock().expect("runtimes poisoned").get(&id) {
            return Ok(runtime.view.clone());
        }
        let record = self.store.get(id)?.ok_or(ServiceError::NotFound(id))?;
        Ok(record.view())
    }

    fn start(self: &Arc<Self>, record: TaskRecord) -> Result<TaskView, ServiceError> {
        if record.kind == "torrent" {
            self.start_torrent(record)
        } else {
            self.start_http(record)
        }
    }

    fn start_http(self: &Arc<Self>, record: TaskRecord) -> Result<TaskView, ServiceError> {
        if self
            .runtimes
            .lock()
            .expect("runtimes poisoned")
            .contains_key(&record.id)
        {
            return Err(ServiceError::Busy);
        }

        let settings = self.settings.read().expect("settings poisoned").clone();
        let max_concurrent = u64::from(settings.max_concurrent_tasks.max(1));
        let global_limit = settings
            .global_download_limit_kib
            .map(|kib| u64::from(kib).saturating_mul(1024));
        let rate_limit = record
            .rate_limit_bps
            .or_else(|| global_limit.map(|global| (global / max_concurrent).max(1024)));

        let mut options = DownloadOptions::new(record.url.clone(), record.save_dir.clone());
        options.file_name = record.file_name.clone();
        options.connections = record.connections as usize;
        options.rate_limit_bps = rate_limit;
        options.headers = record.headers.clone();
        // 显式任务代理（CLI）优先；界面只暴露系统代理开关。
        options.proxy = record.proxy.clone();
        options.use_system_proxy = settings.use_system_proxy;
        options.checksum_sha256 = record.checksum.clone();
        options
            .validate()
            .map_err(|err| ServiceError::Invalid(err.to_string()))?;

        self.store
            .set_state(record.id, TaskState::Downloading, None)?;

        let mut view = record.view();
        view.state = TaskState::Downloading;
        view.error = None;
        view.speed_bps = 0;
        view.eta_secs = None;

        let cancel = CancelToken::new();
        self.runtimes.lock().expect("runtimes poisoned").insert(
            record.id,
            Runtime {
                cancel: cancel.clone(),
                handle: None,
                view: view.clone(),
                last_persist: Instant::now(),
            },
        );
        let _ = self.events.send(TaskEvent::Updated {
            task: Box::new(view.clone()),
        });

        let manager = Arc::clone(self);
        let progress_manager = Arc::clone(self);
        let task_id = record.id;
        let callback: ProgressCallback = Arc::new(move |progress: Progress| {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let manager = Arc::clone(&progress_manager);
                handle.spawn(async move {
                    manager.on_progress(task_id, progress).await;
                });
            }
        });

        let handle = tokio::spawn(async move {
            let result = tug_http::download(options, Some(callback), cancel).await;
            manager.finish(task_id, result).await;
        });

        if let Some(runtime) = self
            .runtimes
            .lock()
            .expect("runtimes poisoned")
            .get_mut(&record.id)
        {
            runtime.handle = Some(handle);
        }

        Ok(view)
    }

    fn start_torrent(self: &Arc<Self>, record: TaskRecord) -> Result<TaskView, ServiceError> {
        if self
            .runtimes
            .lock()
            .expect("runtimes poisoned")
            .contains_key(&record.id)
        {
            return Err(ServiceError::Busy);
        }

        let initial_state = if record.url.starts_with("magnet:") {
            TaskState::FetchingMetadata
        } else {
            TaskState::Downloading
        };
        self.store.set_state(record.id, initial_state, None)?;

        let mut view = record.view();
        view.state = initial_state;
        view.speed_bps = 0;
        view.upload_speed_bps = 0;
        view.peers = 0;
        view.eta_secs = None;
        view.error = None;

        let cancel = CancelToken::new();
        self.runtimes.lock().expect("runtimes poisoned").insert(
            record.id,
            Runtime {
                cancel: cancel.clone(),
                handle: None,
                view: view.clone(),
                last_persist: Instant::now(),
            },
        );
        let _ = self.events.send(TaskEvent::Updated {
            task: Box::new(view.clone()),
        });

        let manager = Arc::clone(self);
        let task_id = record.id;
        let source = record.url.clone();
        let save_dir = record.save_dir.clone();
        let only_files = only_files_from_extra(&record.extra);
        let handle = tokio::spawn(async move {
            manager
                .run_torrent(task_id, source, save_dir, only_files, cancel)
                .await;
        });

        if let Some(runtime) = self
            .runtimes
            .lock()
            .expect("runtimes poisoned")
            .get_mut(&record.id)
        {
            runtime.handle = Some(handle);
        }

        Ok(view)
    }

    async fn run_torrent(
        self: Arc<Self>,
        id: i64,
        source: String,
        save_dir: PathBuf,
        only_files: Option<Vec<usize>>,
        cancel: CancelToken,
    ) {
        let session = match self.torrent_session().await {
            Ok(session) => session,
            Err(err) => {
                self.finish_torrent(id, TorrentFinish::Failed(err.to_string()))
                    .await;
                return;
            }
        };

        // 暂停后继续：session 里已有句柄，直接 unpause，避免再次 add 时阻塞。
        // 必须校验 info hash，避免应用重启后 session id 被新任务复用时删错/恢复错种子。
        let existing = self.store.get(id).ok().flatten().and_then(|record| {
            let torrent_id = record
                .extra
                .get("librqbit_id")
                .and_then(serde_json::Value::as_u64)?;
            let expected = record
                .extra
                .get("info_hash")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            session.get(torrent_id as usize).filter(|handle| {
                expected
                    .as_deref()
                    .is_none_or(|hash| handle.info_hash() == hash)
            })
        });

        let add_options = TorrentAddOptions {
            only_files: only_files.clone(),
            output_folder: Some(save_dir.clone()),
        };

        let handle = if let Some(handle) = existing {
            // 恢复的种子可能停在 initializing 的暂停态，不能只看 state == "paused"。
            match session.resume(&handle).await {
                Ok(()) => {
                    tracing::info!(id, torrent_id = handle.id(), "BT 任务已恢复");
                    handle
                }
                Err(err) => {
                    // 仅记录后继续会让任务看起来在下载、实际停在 paused，
                    // 表现为进度不动、Peers 为 0。这里释放旧句柄后重新加入，
                    // 保留已下载文件，让 librqbit 重新校验并连接 tracker/DHT。
                    tracing::warn!(id, torrent_id = handle.id(), %err, "恢复 BT 任务失败，尝试重新加入");
                    if let Err(delete_err) = session.delete(handle.id(), false).await {
                        tracing::warn!(id, %delete_err, "重新加入前删除旧 BT 句柄失败");
                    }
                    match session.add(&source, add_options.clone()).await {
                        Ok(handle) => handle,
                        Err(add_err) => {
                            self.finish_torrent(id, TorrentFinish::Failed(add_err.to_string()))
                                .await;
                            return;
                        }
                    }
                }
            }
        } else {
            let add = session.add(&source, add_options.clone());
            tokio::pin!(add);
            let deadline = tokio::time::sleep(Duration::from_secs(60));
            tokio::pin!(deadline);
            let mut ticker = tokio::time::interval(Duration::from_millis(250));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            let result = loop {
                tokio::select! {
                    result = &mut add => break Some(result),
                    _ = &mut deadline => break None,
                    _ = ticker.tick() => {
                        if cancel.is_cancelled() {
                            self.finish_torrent(id, TorrentFinish::Paused).await;
                            return;
                        }
                    }
                }
            };

            let Some(result) = result else {
                self.finish_torrent(id, TorrentFinish::Failed("获取元数据超时（60 秒）".into()))
                    .await;
                return;
            };
            match result {
                Ok(handle) => handle,
                Err(err) => {
                    self.finish_torrent(id, TorrentFinish::Failed(err.to_string()))
                        .await;
                    return;
                }
            }
        };

        if cancel.is_cancelled() {
            let _ = session.pause(&handle).await;
            self.finish_torrent(id, TorrentFinish::Paused).await;
            return;
        }

        let mut snapshot = handle.snapshot();
        let files = handle.files();
        let fallback_name = self
            .runtimes
            .lock()
            .expect("runtimes poisoned")
            .get(&id)
            .map(|runtime| runtime.view.name.clone())
            .unwrap_or_else(|| "torrent".to_owned());
        let name = if snapshot.name.is_empty() {
            fallback_name
        } else {
            snapshot.name.clone()
        };
        let (file_name, path) = if files.len() == 1 {
            let relative = files[0].path.clone();
            (Some(relative.clone()), save_dir.join(relative))
        } else {
            (None, save_dir.join(&name))
        };

        let extra = serde_json::json!({
            "librqbit_id": handle.id(),
            "info_hash": handle.info_hash(),
            "only_files": only_files.unwrap_or_default(),
        });
        if let Err(err) = self.store.set_extra(id, &extra) {
            tracing::warn!(%err, "保存 BT 任务扩展失败");
        }
        let _ = self.store.set_name(id, &name, file_name.as_deref());
        let _ = self.store.set_state(id, TaskState::Downloading, None);
        let _ = self.store.set_progress(
            id,
            snapshot.downloaded,
            (snapshot.total > 0).then_some(snapshot.total),
        );

        let updated = {
            let mut runtimes = self.runtimes.lock().expect("runtimes poisoned");
            runtimes.get_mut(&id).map(|runtime| {
                runtime.view.name = name;
                runtime.view.file_name = file_name;
                runtime.view.path = path.display().to_string();
                runtime.view.total = (snapshot.total > 0).then_some(snapshot.total);
                runtime.view.downloaded = snapshot.downloaded;
                runtime.view.state = TaskState::Downloading;
                runtime.view.error = None;
                runtime.view.clone()
            })
        };
        if let Some(view) = updated {
            let _ = self.events.send(TaskEvent::Updated {
                task: Box::new(view),
            });
        }

        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                _ = cancel.cancelled() => {
                    let _ = session.pause(&handle).await;
                    self.finish_torrent(id, TorrentFinish::Paused).await;
                    return;
                }
            }

            snapshot = handle.snapshot();
            if let Some(message) = snapshot.error.clone() {
                self.finish_torrent(id, TorrentFinish::Failed(message))
                    .await;
                return;
            }

            if let Some(view) = self.update_torrent_progress(id, &snapshot) {
                let _ = self.events.send(TaskEvent::Updated {
                    task: Box::new(view),
                });
            }

            if snapshot.finished {
                let total = snapshot.total.max(snapshot.downloaded);
                let name = if snapshot.name.is_empty() {
                    self.runtimes
                        .lock()
                        .expect("runtimes poisoned")
                        .get(&id)
                        .map(|runtime| runtime.view.name.clone())
                        .unwrap_or_else(|| "torrent".to_owned())
                } else {
                    snapshot.name
                };
                let files = handle.files();
                let (file_name, path) = if files.len() == 1 {
                    let relative = files[0].path.clone();
                    (Some(relative.clone()), save_dir.join(relative))
                } else {
                    (None, save_dir.join(&name))
                };

                let settings = self.settings.read().expect("settings poisoned").clone();
                if !settings.bt_seed_after_complete {
                    let _ = session.pause(&handle).await;
                    self.finish_torrent(
                        id,
                        TorrentFinish::Completed {
                            total,
                            name,
                            file_name,
                            path,
                        },
                    )
                    .await;
                    return;
                }

                self.store.set_state(id, TaskState::Seeding, None).ok();
                self.store.set_progress(id, total, Some(total)).ok();
                let updated = {
                    let mut runtimes = self.runtimes.lock().expect("runtimes poisoned");
                    runtimes.get_mut(&id).map(|runtime| {
                        runtime.view.state = TaskState::Seeding;
                        runtime.view.name = name.clone();
                        runtime.view.file_name = file_name.clone();
                        runtime.view.path = path.display().to_string();
                        runtime.view.total = Some(total);
                        runtime.view.downloaded = total;
                        runtime.view.speed_bps = 0;
                        runtime.view.error = None;
                        runtime.view.clone()
                    })
                };
                if let Some(view) = updated {
                    let _ = self.events.send(TaskEvent::Updated {
                        task: Box::new(view),
                    });
                }

                self.seed_torrent(id, session, handle, cancel).await;
                return;
            }
        }
    }

    async fn seed_torrent(
        self: &Arc<Self>,
        id: i64,
        session: Arc<TorrentSession>,
        handle: TorrentHandle,
        cancel: CancelToken,
    ) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                _ = cancel.cancelled() => {
                    let _ = session.pause(&handle).await;
                    self.finish_torrent(id, TorrentFinish::Paused).await;
                    return;
                }
            }

            let snapshot = handle.snapshot();
            if let Some(message) = snapshot.error.clone() {
                self.finish_torrent(id, TorrentFinish::Failed(message))
                    .await;
                return;
            }

            if let Some(view) = self.update_seeding_progress(id, &snapshot) {
                let _ = self.events.send(TaskEvent::Updated {
                    task: Box::new(view),
                });
            }
        }
    }

    fn update_seeding_progress(&self, id: i64, snapshot: &TorrentSnapshot) -> Option<TaskView> {
        let view = {
            let mut runtimes = self.runtimes.lock().expect("runtimes poisoned");
            let runtime = runtimes.get_mut(&id)?;
            runtime.view.state = TaskState::Seeding;
            runtime.view.downloaded = snapshot.downloaded.max(runtime.view.downloaded);
            runtime.view.uploaded = snapshot.uploaded;
            runtime.view.upload_speed_bps = snapshot.upload_bps;
            runtime.view.speed_bps = 0;
            runtime.view.peers = snapshot.peers;
            runtime.view.eta_secs = None;
            runtime.view.error = None;
            if !snapshot.name.is_empty() {
                runtime.view.name = snapshot.name.clone();
            }
            runtime.view.clone()
        };

        let _ = self.store.set_progress(id, view.downloaded, view.total);
        let _ = self.store.set_uploaded(id, view.uploaded);
        Some(view)
    }

    fn update_torrent_progress(&self, id: i64, snapshot: &TorrentSnapshot) -> Option<TaskView> {
        let (view, persist) = {
            let mut runtimes = self.runtimes.lock().expect("runtimes poisoned");
            let runtime = runtimes.get_mut(&id)?;

            runtime.view.downloaded = snapshot.downloaded;
            if snapshot.total > 0 {
                runtime.view.total = Some(snapshot.total);
            }
            runtime.view.uploaded = snapshot.uploaded;
            runtime.view.speed_bps = snapshot.download_bps;
            runtime.view.upload_speed_bps = snapshot.upload_bps;
            runtime.view.peers = snapshot.peers;
            runtime.view.eta_secs = snapshot.eta_secs;
            runtime.view.state = TaskState::Downloading;
            runtime.view.error = None;
            if !snapshot.name.is_empty() {
                runtime.view.name = snapshot.name.clone();
            }

            let now = Instant::now();
            let persist = now.duration_since(runtime.last_persist) >= Duration::from_secs(1);
            if persist {
                runtime.last_persist = now;
            }
            (runtime.view.clone(), persist)
        };

        if persist {
            let total = (snapshot.total > 0).then_some(snapshot.total);
            let _ = self.store.set_progress(id, snapshot.downloaded, total);
            let _ = self.store.set_uploaded(id, snapshot.uploaded);
        }
        Some(view)
    }

    async fn finish_torrent(self: &Arc<Self>, id: i64, finish: TorrentFinish) {
        let Some(mut view) = self
            .runtimes
            .lock()
            .expect("runtimes poisoned")
            .remove(&id)
            .map(|runtime| runtime.view)
        else {
            return;
        };

        match finish {
            TorrentFinish::Paused => {
                let _ = self.store.set_progress(id, view.downloaded, view.total);
                let _ = self.store.set_state(id, TaskState::Paused, None);
                view.state = TaskState::Paused;
                view.speed_bps = 0;
                view.upload_speed_bps = 0;
                view.peers = 0;
                view.eta_secs = None;
                view.error = None;
            }
            TorrentFinish::Failed(message) => {
                let _ = self.store.set_progress(id, view.downloaded, view.total);
                let _ = self.store.set_state(id, TaskState::Failed, Some(&message));
                view.state = TaskState::Failed;
                view.speed_bps = 0;
                view.upload_speed_bps = 0;
                view.peers = 0;
                view.eta_secs = None;
                view.error = Some(message);
            }
            TorrentFinish::Completed {
                total,
                name,
                file_name,
                path,
            } => {
                let total = total.max(view.downloaded);
                let name = if name.is_empty() {
                    view.name.clone()
                } else {
                    name
                };
                let _ = self.store.set_progress(id, total, Some(total));
                let _ = self.store.set_uploaded(id, view.uploaded);
                let _ = self.store.set_name(id, &name, file_name.as_deref());
                let _ = self.store.set_state(id, TaskState::Completed, None);
                view.state = TaskState::Completed;
                view.name = name;
                view.file_name = file_name;
                view.path = path.display().to_string();
                view.total = Some(total);
                view.downloaded = total;
                view.speed_bps = 0;
                view.upload_speed_bps = 0;
                view.peers = 0;
                view.eta_secs = Some(0);
                view.error = None;
            }
        }

        let _ = self.events.send(TaskEvent::Updated {
            task: Box::new(view),
        });
        self.pump().await;
    }

    async fn on_progress(&self, id: i64, progress: Progress) {
        let (view, persist) = {
            let mut runtimes = self.runtimes.lock().expect("runtimes poisoned");
            let Some(runtime) = runtimes.get_mut(&id) else {
                return;
            };

            runtime.view.downloaded = progress.downloaded;
            runtime.view.total = progress.total.or(runtime.view.total);
            runtime.view.speed_bps = progress.speed_bps;
            runtime.view.eta_secs = progress.eta_secs;
            runtime.view.state = TaskState::Downloading;
            runtime.view.error = None;

            let now = Instant::now();
            let persist = now.duration_since(runtime.last_persist) >= Duration::from_secs(1);
            if persist {
                runtime.last_persist = now;
            }
            (runtime.view.clone(), persist)
        };

        if persist && let Err(err) = self.store.set_progress(id, view.downloaded, view.total) {
            tracing::warn!(%err, "保存任务进度失败");
        }
        let _ = self.events.send(TaskEvent::Updated {
            task: Box::new(view),
        });
    }

    async fn finish(
        self: &Arc<Self>,
        id: i64,
        result: Result<tug_http::DownloadSummary, HttpError>,
    ) {
        let Some(mut view) = self
            .runtimes
            .lock()
            .expect("runtimes poisoned")
            .remove(&id)
            .map(|runtime| runtime.view)
        else {
            return;
        };

        match result {
            Ok(summary) => {
                let name = summary
                    .path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| view.name.clone());

                let _ = self
                    .store
                    .set_progress(id, summary.total, Some(summary.total));
                let _ = self.store.set_name(id, &name, Some(&name));
                let _ = self.store.set_state(id, TaskState::Completed, None);

                view.state = TaskState::Completed;
                view.name = name.clone();
                view.file_name = Some(name);
                view.path = summary.path.display().to_string();
                view.total = Some(summary.total);
                view.downloaded = summary.total;
                view.speed_bps = 0;
                view.eta_secs = Some(0);
                view.error = None;
            }
            Err(HttpError::Cancelled) => {
                let _ = self.store.set_progress(id, view.downloaded, view.total);
                let _ = self.store.set_state(id, TaskState::Paused, None);
                view.state = TaskState::Paused;
                view.speed_bps = 0;
                view.eta_secs = None;
                view.error = None;
            }
            Err(err) => {
                let message = err.to_string();
                let _ = self.store.set_progress(id, view.downloaded, view.total);
                let _ = self.store.set_state(id, TaskState::Failed, Some(&message));
                view.state = TaskState::Failed;
                view.speed_bps = 0;
                view.eta_secs = None;
                view.error = Some(message);
            }
        }

        let _ = self.events.send(TaskEvent::Updated {
            task: Box::new(view),
        });

        self.pump().await;
    }
}

fn is_torrent_source(source: &str) -> bool {
    if source.starts_with("magnet:") {
        return true;
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        return source
            .split('?')
            .next()
            .is_some_and(|path| path.ends_with(".torrent"));
    }
    source.ends_with(".torrent") && Path::new(source).is_file()
}

fn torrent_display_name(source: &str) -> String {
    if let Some(rest) = source.strip_prefix("magnet:") {
        if let Some(value) = rest.split("dn=").nth(1) {
            let raw = value.split('&').next().unwrap_or_default();
            let decoded = percent_decode(raw);
            if !decoded.trim().is_empty() {
                return tug_http::sanitize_file_name(decoded.trim());
            }
        }
        return "magnet".to_owned();
    }

    let name = if source.starts_with("http://") || source.starts_with("https://") {
        tug_http::guess_file_name(source)
    } else {
        Path::new(source)
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned)
    }
    .unwrap_or_else(|| "torrent".to_owned());

    tug_http::sanitize_file_name(name.trim_end_matches(".torrent"))
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                if let (Some(high), Some(low)) =
                    (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
                {
                    output.push(high * 16 + low);
                    index += 3;
                    continue;
                }
                output.push(bytes[index]);
            }
            b'+' => output.push(b' '),
            byte => output.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn only_files_from_extra(extra: &serde_json::Value) -> Option<Vec<usize>> {
    let files = extra
        .get("only_files")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .filter_map(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .collect::<Vec<_>>();
    (!files.is_empty()).then_some(files)
}

fn unique_task_name(directory: &Path, name: &str, used: &HashSet<PathBuf>) -> String {
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let extension = path.extension().and_then(|value| value.to_str());

    for index in 1..=999 {
        let candidate = match extension {
            Some(ext) => format!("{stem} ({index}).{ext}"),
            None => format!("{stem} ({index})"),
        };
        let full = directory.join(&candidate);
        if !used.contains(&full) && !full.exists() {
            return candidate;
        }
    }

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    match extension {
        Some(ext) => format!("{stem}-{stamp}.{ext}"),
        None => format!("{stem}-{stamp}"),
    }
}

fn delete_files_for(record: &TaskRecord) {
    let path = record.path();
    if path.is_dir() {
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!(
                path = %path.display(),
                error = %err,
                "删除任务目录失败"
            ),
        }
        return;
    }

    let part = tug_http::sidecar_path(&path, ".part");
    let meta = tug_http::sidecar_path(&path, ".tug.json");

    for target in [path, part, meta] {
        match std::fs::remove_file(&target) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!(
                path = %target.display(),
                error = %err,
                "删除任务文件失败"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_torrent_sources() {
        assert!(is_torrent_source("magnet:?xt=urn:btih:abc"));
        assert!(is_torrent_source(
            "https://example.com/debian.iso.torrent?token=1"
        ));
        assert!(!is_torrent_source("https://example.com/debian.iso"));
        assert!(!is_torrent_source("missing.torrent"));
    }

    #[test]
    fn decodes_magnet_display_name() {
        let source = "magnet:?xt=urn:btih:abc&dn=Debian+13.7+netinst&tr=udp%3A%2F%2Ftracker";
        assert_eq!(torrent_display_name(source), "Debian 13.7 netinst");
        assert_eq!(
            percent_decode("%E4%B8%AD%E6%96%87+file"),
            "中文 file".to_owned()
        );
    }

    #[test]
    fn parses_only_files() {
        assert_eq!(
            only_files_from_extra(&serde_json::json!({"only_files": [1, 3]})),
            Some(vec![1, 3])
        );
        assert_eq!(
            only_files_from_extra(&serde_json::json!({"only_files": []})),
            None
        );
        assert_eq!(only_files_from_extra(&serde_json::json!({})), None);
    }
}
