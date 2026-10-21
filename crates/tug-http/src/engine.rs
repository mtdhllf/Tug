use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_LENGTH, IF_RANGE, RANGE};
use reqwest::{Client, StatusCode};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tokio::task::{JoinHandle, JoinSet};

use crate::error::{HttpError, Result};
use crate::filename::{sanitize_file_name, sidecar_path, unique_path};
use crate::meta::ResumeMeta;
use crate::options::{DownloadOptions, DownloadSummary, Progress, ProgressCallback};
use crate::probe::{DownloadPlan, build_client, parse_content_range, parse_retry_after, probe};
use crate::rate::RateLimiter;
use crate::segments::{Segment, build_segments, effective_segment_size};
use crate::writer::{FileWriter, available_space, io_path};

const MAX_ATTEMPTS: u32 = 5;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(1);
const DISK_MARGIN: u64 = 8 * 1024 * 1024;

/// 协作式取消令牌；worker 在每个 chunk 边界检查。
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    /// 新建未取消的令牌。
    pub fn new() -> Self {
        Self::default()
    }

    /// 请求取消。
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// 是否已取消。
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// 等待取消；25ms 轮询，避免下载流阻塞时暂停迟迟不生效。
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

/// 下载入口：探测、续传判断、分段调度、写盘、校验、落盘。
pub async fn download(
    options: DownloadOptions,
    progress: Option<ProgressCallback>,
    cancel: CancelToken,
) -> Result<DownloadSummary> {
    let started = Instant::now();
    options.validate()?;

    let client = Arc::new(build_client(&options)?);
    let plan = probe(&client, &options).await?;
    let total = plan.total.unwrap_or(0);

    let requested_name = options
        .file_name
        .as_deref()
        .map(sanitize_file_name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| sanitize_file_name(&plan.file_name));

    let mut final_path = options.output_dir.join(&requested_name);
    let mut part_path = sidecar_path(&final_path, ".part");
    let mut meta_path = sidecar_path(&final_path, ".tug.json");
    let mut existing_meta = ResumeMeta::load(&meta_path)?;
    let mut resumable = false;

    if options.restart {
        remove_if_exists(&part_path);
        remove_if_exists(&meta_path);
        existing_meta = None;
    } else if let Some(meta) = existing_meta.as_ref() {
        if !part_path.exists() {
            remove_if_exists(&meta_path);
            existing_meta = None;
        } else if meta.supports_range && meta.matches(&plan) {
            resumable = true;
        } else if meta.supports_range {
            return Err(HttpError::RemoteChanged);
        } else {
            // 上次是不支持 Range 的残留，删掉重来。
            remove_if_exists(&part_path);
            remove_if_exists(&meta_path);
            existing_meta = None;
        }
    }

    if !resumable && !options.restart && existing_meta.is_none() {
        // 没有元数据的 .part 无法信任，直接清掉，避免留下“file (1).part”。
        if part_path.exists() && !meta_path.exists() {
            remove_if_exists(&part_path);
        }
        if final_path.exists() {
            final_path = unique_path(&final_path);
            part_path = sidecar_path(&final_path, ".part");
            meta_path = sidecar_path(&final_path, ".tug.json");
        }
    }

    let file_name = final_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("download")
        .to_owned();

    if resumable && final_path.exists() {
        return Err(HttpError::InvalidOptions(format!(
            "目标文件已存在，无法续传: {}",
            final_path.display()
        )));
    }

    let resumed_from = if resumable {
        existing_meta.as_ref().map_or(0, ResumeMeta::resumed_bytes)
    } else {
        0
    };

    if resumable {
        let length = std::fs::metadata(io_path(&part_path))?.len();
        if let Some(meta) = &existing_meta
            && length < meta.max_written_end()
        {
            return Err(HttpError::RemoteChanged);
        }
    }

    let remaining = total.saturating_sub(resumed_from);
    if let Some(free) = available_space(&options.output_dir)
        && free < remaining.saturating_add(DISK_MARGIN)
    {
        return Err(HttpError::DiskFull {
            needed: remaining,
            available: free,
        });
    }

    let segment_size = if resumable {
        existing_meta.as_ref().map_or(0, |meta| meta.segment_size)
    } else {
        effective_segment_size(total, options.connections, options.segment_size)
    };
    if segment_size == 0 && total > 0 {
        return Err(HttpError::Meta("分片大小为 0".into()));
    }

    let mut meta = if resumable {
        existing_meta.take().expect("resumable 时元数据必须存在")
    } else {
        ResumeMeta::new(&plan, &file_name, segment_size)
    };
    meta.url = plan.url.clone();
    meta.final_url = plan.final_url.clone();
    if !resumable {
        meta.save(&meta_path)?;
    }

    let writer = Arc::new(if resumable {
        FileWriter::open(&part_path, total)?
    } else {
        FileWriter::create(&part_path, total)?
    });

    let total_segments = if plan.supports_range && total > 0 {
        build_segments(total, segment_size).len()
    } else {
        1
    };
    let state = ProgressState::new(resumed_from, plan.total, total_segments);

    let result = if plan.supports_range && total > 0 {
        run_segmented(
            &client,
            &options,
            &plan,
            &writer,
            &meta_path,
            meta,
            segment_size,
            &state,
            &cancel,
            progress.as_ref(),
        )
        .await
    } else {
        run_single(
            &client,
            &options,
            &plan,
            writer.as_ref(),
            &state,
            &cancel,
            progress.as_ref(),
        )
        .await
    };

    if let Err(err) = result {
        // 单流模式无法续传，失败后清理半成品，避免留下无法复用的文件。
        if !plan.supports_range {
            remove_if_exists(&part_path);
            remove_if_exists(&meta_path);
        }
        return Err(err);
    }

    finalize(
        writer.as_ref(),
        &part_path,
        &final_path,
        &meta_path,
        options.checksum_sha256.as_deref(),
    )
    .await?;

    let final_size = std::fs::metadata(io_path(&final_path))?.len();
    if let Some(callback) = &progress {
        callback(Progress {
            downloaded: final_size,
            total: Some(final_size),
            speed_bps: 0,
            eta_secs: Some(0),
            active_connections: 0,
            completed_segments: state.total_segments,
            total_segments: state.total_segments,
        });
    }

    Ok(DownloadSummary {
        path: final_path,
        total: final_size,
        resumed_from,
        elapsed: started.elapsed(),
    })
}

#[derive(Debug, Clone, Copy)]
struct WorkItem {
    segment: Segment,
    offset: u64,
}

#[allow(clippy::too_many_arguments)]
async fn run_segmented(
    client: &Arc<Client>,
    options: &DownloadOptions,
    plan: &DownloadPlan,
    writer: &Arc<FileWriter>,
    meta_path: &Path,
    meta: ResumeMeta,
    segment_size: u64,
    state: &ProgressState,
    cancel: &CancelToken,
    progress: Option<&ProgressCallback>,
) -> Result<()> {
    let segments = build_segments(meta.total, segment_size);
    let mut meta = meta;
    meta.normalize_partials(&segments);

    let completed = segments
        .iter()
        .filter(|segment| meta.is_segment_complete(segment))
        .count();
    state.done.store(completed, Ordering::SeqCst);

    let pending: Vec<WorkItem> = segments
        .iter()
        .filter(|segment| !meta.is_segment_complete(segment))
        .map(|segment| WorkItem {
            segment: *segment,
            offset: meta.partial_offset(segment).unwrap_or(segment.start),
        })
        .collect();

    if pending.is_empty() {
        return Ok(());
    }

    let reporter = spawn_reporter(state, progress);
    if let Some(callback) = progress {
        callback(state.snapshot());
    }

    let meta = Arc::new(Mutex::new(meta));
    let pending = Arc::new(pending);
    let segments = Arc::new(segments);
    let offsets = Arc::new(StdMutex::new(HashMap::<usize, u64>::new()));
    let next = Arc::new(AtomicUsize::new(0));
    let stop_checkpoint = Arc::new(AtomicBool::new(false));
    let checkpoint = spawn_checkpoint(
        Arc::clone(&meta),
        meta_path.to_path_buf(),
        Arc::clone(writer),
        Arc::clone(&segments),
        Arc::clone(&offsets),
        Arc::clone(&stop_checkpoint),
    );

    let context = WorkerContext {
        client: Arc::clone(client),
        url: plan.url.clone(),
        etag: plan.etag.clone(),
        headers: options.headers.clone(),
        writer: Arc::clone(writer),
        pending: Arc::clone(&pending),
        next: Arc::clone(&next),
        meta: Arc::clone(&meta),
        meta_path: meta_path.to_path_buf(),
        downloaded: Arc::clone(&state.downloaded),
        active: Arc::clone(&state.active),
        done: Arc::clone(&state.done),
        offsets: Arc::clone(&offsets),
        rate: RateLimiter::new(options.rate_limit_bps),
        cancel: cancel.clone(),
    };

    let worker_count = options.connections.min(pending.len()).max(1);
    let mut workers = JoinSet::new();
    for _ in 0..worker_count {
        workers.spawn(worker_loop(context.clone()));
    }

    let mut failure = None;
    while let Some(joined) = workers.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                failure = Some(err);
                workers.abort_all();
                break;
            }
            Err(err) => {
                failure = Some(HttpError::Internal(format!("worker 任务失败: {err}")));
                workers.abort_all();
                break;
            }
        }
    }

    // 退出前把在途进度落一次盘，供下次续传。
    if let Err(err) = checkpoint_once(&meta, meta_path, writer, segments.as_slice(), &offsets).await
    {
        tracing::warn!(error = %err, "保存最终 checkpoint 失败");
    }

    stop_checkpoint.store(true, Ordering::SeqCst);
    checkpoint.abort();
    let _ = checkpoint.await;

    state.stop();
    stop_reporter(reporter).await;

    match failure {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

fn spawn_checkpoint(
    meta: Arc<Mutex<ResumeMeta>>,
    meta_path: PathBuf,
    writer: Arc<FileWriter>,
    segments: Arc<Vec<Segment>>,
    offsets: Arc<StdMutex<HashMap<usize, u64>>>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(CHECKPOINT_INTERVAL).await;
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if let Err(err) =
                checkpoint_once(&meta, &meta_path, &writer, segments.as_slice(), &offsets).await
            {
                tracing::warn!(error = %err, "保存下载 checkpoint 失败");
            }
        }
    })
}

async fn checkpoint_once(
    meta: &Mutex<ResumeMeta>,
    meta_path: &Path,
    writer: &FileWriter,
    segments: &[Segment],
    offsets: &StdMutex<HashMap<usize, u64>>,
) -> Result<()> {
    let snapshot: Vec<(usize, u64)> = {
        let offsets = offsets.lock().expect("offsets lock poisoned");
        offsets.iter().map(|(index, end)| (*index, *end)).collect()
    };
    if snapshot.is_empty() {
        return Ok(());
    }

    // 先确保数据落盘，再让元数据声明这段数据可复用。
    writer.sync_data()?;

    let mut meta = meta.lock().await;
    for (index, end) in snapshot {
        let Some(segment) = segments.get(index) else {
            continue;
        };
        if meta.is_segment_complete(segment) {
            continue;
        }
        if end > segment.start && end < segment.end {
            meta.set_partial(segment.start, end);
        }
    }
    meta.save(meta_path)?;
    Ok(())
}

async fn run_single(
    client: &Client,
    options: &DownloadOptions,
    plan: &DownloadPlan,
    writer: &FileWriter,
    state: &ProgressState,
    cancel: &CancelToken,
    progress: Option<&ProgressCallback>,
) -> Result<()> {
    let mut request = client.get(&plan.url).header(ACCEPT_ENCODING, "identity");
    for (name, value) in &options.headers {
        request = request.header(name, value);
    }

    let reporter = spawn_reporter(state, progress);
    if let Some(callback) = progress {
        callback(state.snapshot());
    }

    let result = async {
        let response = tokio::select! {
            response = request.send() => response?,
            _ = cancel.cancelled() => return Err(HttpError::Cancelled),
        };
        let status = response.status();
        if !status.is_success() {
            return Err(HttpError::Status {
                status,
                url: plan.url.clone(),
            });
        }

        if let Some(expected) = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            && let Some(total) = plan.total
            && expected != total
        {
            return Err(HttpError::RemoteChanged);
        }

        let rate = RateLimiter::new(options.rate_limit_bps);
        let mut stream = response.bytes_stream();
        let mut written = 0u64;
        loop {
            let next = tokio::select! {
                item = stream.next() => item,
                _ = cancel.cancelled() => return Err(HttpError::Cancelled),
            };
            let Some(item) = next else {
                break;
            };
            let chunk = item?;
            if chunk.is_empty() {
                continue;
            }
            writer.write_at(written, &chunk)?;
            written += chunk.len() as u64;
            state
                .downloaded
                .fetch_add(chunk.len() as u64, Ordering::SeqCst);
            tokio::select! {
                _ = rate.acquire(chunk.len() as u64) => {}
                _ = cancel.cancelled() => return Err(HttpError::Cancelled),
            }
        }

        if let Some(total) = plan.total
            && written != total
        {
            return Err(HttpError::ShortRead {
                expected: total,
                actual: written,
            });
        }

        writer.sync_all()?;
        Ok(())
    }
    .await;

    state.stop();
    stop_reporter(reporter).await;
    result
}

struct WorkerContext {
    client: Arc<Client>,
    url: String,
    etag: Option<String>,
    headers: Vec<(String, String)>,
    writer: Arc<FileWriter>,
    pending: Arc<Vec<WorkItem>>,
    next: Arc<AtomicUsize>,
    meta: Arc<Mutex<ResumeMeta>>,
    meta_path: PathBuf,
    downloaded: Arc<AtomicU64>,
    active: Arc<AtomicUsize>,
    done: Arc<AtomicUsize>,
    offsets: Arc<StdMutex<HashMap<usize, u64>>>,
    rate: RateLimiter,
    cancel: CancelToken,
}

impl Clone for WorkerContext {
    fn clone(&self) -> Self {
        Self {
            client: Arc::clone(&self.client),
            url: self.url.clone(),
            etag: self.etag.clone(),
            headers: self.headers.clone(),
            writer: Arc::clone(&self.writer),
            pending: Arc::clone(&self.pending),
            next: Arc::clone(&self.next),
            meta: Arc::clone(&self.meta),
            meta_path: self.meta_path.clone(),
            downloaded: Arc::clone(&self.downloaded),
            active: Arc::clone(&self.active),
            done: Arc::clone(&self.done),
            offsets: Arc::clone(&self.offsets),
            rate: self.rate.clone(),
            cancel: self.cancel.clone(),
        }
    }
}

async fn worker_loop(context: WorkerContext) -> Result<()> {
    loop {
        if context.cancel.is_cancelled() {
            return Err(HttpError::Cancelled);
        }

        let index = context.next.fetch_add(1, Ordering::SeqCst);
        let Some(item) = context.pending.get(index).copied() else {
            break;
        };

        context
            .offsets
            .lock()
            .expect("offsets lock poisoned")
            .insert(item.segment.index, item.offset);

        context.active.fetch_add(1, Ordering::SeqCst);
        let result = fetch_segment(&context, item).await;
        context.active.fetch_sub(1, Ordering::SeqCst);
        result?;

        // 先落盘再标记完成，避免元数据说“已完成”但数据还在缓存里。
        context.writer.sync_data()?;
        let mut meta = context.meta.lock().await;
        meta.clear_partial(item.segment.start);
        meta.mark_complete(item.segment.start, item.segment.end);
        meta.save(&context.meta_path)?;
        drop(meta);

        context
            .offsets
            .lock()
            .expect("offsets lock poisoned")
            .remove(&item.segment.index);

        context.done.fetch_add(1, Ordering::SeqCst);
    }

    Ok(())
}

async fn fetch_segment(context: &WorkerContext, item: WorkItem) -> Result<()> {
    let mut attempt = 0u32;

    loop {
        if context.cancel.is_cancelled() {
            return Err(HttpError::Cancelled);
        }

        context
            .offsets
            .lock()
            .expect("offsets lock poisoned")
            .insert(item.segment.index, item.offset);

        let mut written = 0u64;
        match stream_segment(context, item, &mut written).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                if written > 0 {
                    context.downloaded.fetch_sub(written, Ordering::SeqCst);
                }
                context
                    .offsets
                    .lock()
                    .expect("offsets lock poisoned")
                    .insert(item.segment.index, item.offset);

                if !is_retryable(&err) || attempt >= MAX_ATTEMPTS {
                    return Err(err);
                }
                let delay = retry_delay(&err, attempt);
                tracing::warn!(
                    segment = item.segment.index,
                    attempt,
                    error = %err,
                    "分片下载失败，准备重试"
                );
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = context.cancel.cancelled() => return Err(HttpError::Cancelled),
                }
                attempt += 1;
            }
        }
    }
}

async fn stream_segment(context: &WorkerContext, item: WorkItem, written: &mut u64) -> Result<()> {
    let segment = item.segment;
    let request_start = item.offset + *written;
    let mut request = context
        .client
        .get(&context.url)
        .header(RANGE, format!("bytes={request_start}-{}", segment.end - 1))
        .header(ACCEPT_ENCODING, "identity");

    if let Some(etag) = &context.etag
        && !etag.starts_with("W/")
    {
        request = request.header(IF_RANGE, etag);
    }
    for (name, value) in &context.headers {
        request = request.header(name, value);
    }

    let response = tokio::select! {
        response = request.send() => response?,
        _ = context.cancel.cancelled() => return Err(HttpError::Cancelled),
    };
    let status = response.status();

    if status == StatusCode::PARTIAL_CONTENT {
        if let Some((start, _, _)) = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_content_range)
            && start != request_start
        {
            return Err(HttpError::RemoteChanged);
        }
    } else if status == StatusCode::OK {
        // 带 If-Range 仍返回 200，说明远端已变化；否则是服务器不支持 Range。
        if context.etag.is_some() {
            return Err(HttpError::RemoteChanged);
        }
        return Err(HttpError::RangeUnsupported(context.url.clone()));
    } else if status == StatusCode::RANGE_NOT_SATISFIABLE {
        return Err(HttpError::RemoteChanged);
    } else if status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
    {
        return Err(HttpError::Transient {
            status,
            retry_after: parse_retry_after(response.headers()),
        });
    } else {
        return Err(HttpError::Status {
            status,
            url: context.url.clone(),
        });
    }

    let expected = segment.end - item.offset;
    let mut stream = response.bytes_stream();
    loop {
        let next = tokio::select! {
            chunk = stream.next() => chunk,
            _ = context.cancel.cancelled() => return Err(HttpError::Cancelled),
        };
        let Some(next) = next else {
            break;
        };
        let chunk = next?;
        if chunk.is_empty() {
            continue;
        }
        if *written + chunk.len() as u64 > expected {
            return Err(HttpError::Internal("服务器返回的数据超出分片范围".into()));
        }
        context.writer.write_at(request_start + *written, &chunk)?;
        *written += chunk.len() as u64;
        context
            .downloaded
            .fetch_add(chunk.len() as u64, Ordering::SeqCst);
        context
            .offsets
            .lock()
            .expect("offsets lock poisoned")
            .insert(segment.index, item.offset + *written);
        tokio::select! {
            _ = context.rate.acquire(chunk.len() as u64) => {}
            _ = context.cancel.cancelled() => return Err(HttpError::Cancelled),
        }
    }

    if *written != expected {
        return Err(HttpError::ShortRead {
            expected,
            actual: *written,
        });
    }

    Ok(())
}

fn is_retryable(error: &HttpError) -> bool {
    matches!(
        error,
        HttpError::Request(_) | HttpError::Transient { .. } | HttpError::ShortRead { .. }
    )
}

fn retry_delay(error: &HttpError, attempt: u32) -> Duration {
    if let HttpError::Transient {
        retry_after: Some(delay),
        ..
    } = error
    {
        return *delay;
    }

    let base = 500u64.saturating_mul(1u64 << attempt.min(4));
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.subsec_nanos() as u64 % 250)
        .unwrap_or(0);
    Duration::from_millis((base + jitter).min(8_000))
}

async fn finalize(
    writer: &FileWriter,
    part_path: &Path,
    final_path: &Path,
    meta_path: &Path,
    checksum: Option<&str>,
) -> Result<()> {
    writer.sync_all()?;

    if let Some(expected) = checksum {
        let actual = sha256_file(part_path).await?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(HttpError::ChecksumMismatch {
                expected: expected.to_owned(),
                actual,
            });
        }
    }

    if final_path.exists() {
        std::fs::remove_file(io_path(final_path))?;
    }
    std::fs::rename(io_path(part_path), io_path(final_path))?;
    remove_if_exists(meta_path);
    Ok(())
}

async fn sha256_file(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        use std::io::Read;

        let mut file = std::fs::File::open(io_path(&path))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hex::encode(hasher.finalize()))
    })
    .await
    .map_err(|err| HttpError::Internal(format!("校验任务失败: {err}")))??;
    Ok(result)
}

fn remove_if_exists(path: &Path) {
    match std::fs::remove_file(io_path(path)) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => tracing::warn!(path = %path.display(), error = %err, "删除文件失败"),
    }
}

struct ProgressState {
    downloaded: Arc<AtomicU64>,
    done: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    total: Option<u64>,
    total_segments: usize,
}

impl ProgressState {
    fn new(downloaded: u64, total: Option<u64>, total_segments: usize) -> Self {
        Self {
            downloaded: Arc::new(AtomicU64::new(downloaded)),
            done: Arc::new(AtomicUsize::new(0)),
            active: Arc::new(AtomicUsize::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            total,
            total_segments,
        }
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    fn snapshot(&self) -> Progress {
        Progress {
            downloaded: self.downloaded.load(Ordering::SeqCst),
            total: self.total,
            speed_bps: 0,
            eta_secs: None,
            active_connections: self.active.load(Ordering::SeqCst),
            completed_segments: self.done.load(Ordering::SeqCst),
            total_segments: self.total_segments,
        }
    }
}

fn spawn_reporter(
    state: &ProgressState,
    progress: Option<&ProgressCallback>,
) -> Option<JoinHandle<()>> {
    let callback = Arc::clone(progress?);
    let downloaded = Arc::clone(&state.downloaded);
    let done = Arc::clone(&state.done);
    let active = Arc::clone(&state.active);
    let stop = Arc::clone(&state.stop);
    let total = state.total;
    let total_segments = state.total_segments;

    Some(tokio::spawn(async move {
        let mut last_bytes = downloaded.load(Ordering::SeqCst);
        let mut last_at = Instant::now();

        loop {
            tokio::time::sleep(PROGRESS_INTERVAL).await;
            if stop.load(Ordering::SeqCst) {
                break;
            }

            let now = Instant::now();
            let bytes = downloaded.load(Ordering::SeqCst);
            let elapsed = now.duration_since(last_at).as_secs_f64().max(0.001);
            let speed = (bytes.saturating_sub(last_bytes) as f64 / elapsed) as u64;
            let eta = if speed > 0 {
                total.map(|total| total.saturating_sub(bytes) / speed)
            } else {
                None
            };

            callback(Progress {
                downloaded: bytes,
                total,
                speed_bps: speed,
                eta_secs: eta,
                active_connections: active.load(Ordering::SeqCst),
                completed_segments: done.load(Ordering::SeqCst),
                total_segments,
            });

            last_bytes = bytes;
            last_at = now;
        }
    }))
}

async fn stop_reporter(reporter: Option<JoinHandle<()>>) {
    if let Some(handle) = reporter {
        handle.abort();
        let _ = handle.await;
    }
}
