use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tug_http::meta::META_VERSION;
use tug_http::{CancelToken, DownloadOptions, HttpError, ResumeMeta, download, sidecar_path};

#[derive(Clone)]
struct TestState {
    data: Arc<Vec<u8>>,
    etag: &'static str,
    no_range: bool,
    requests: Arc<AtomicUsize>,
    ranges: Arc<Mutex<Vec<String>>>,
}

async fn file_handler(State(state): State<TestState>, headers: HeaderMap) -> Response {
    state.requests.fetch_add(1, Ordering::SeqCst);
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    state
        .ranges
        .lock()
        .expect("ranges lock")
        .push(range.clone().unwrap_or_else(|| "full".to_owned()));

    if state.no_range {
        return full_response(&state);
    }

    match range
        .as_deref()
        .and_then(|value| parse_range(value, state.data.len() as u64))
    {
        Some((start, end)) => partial_response(&state, start, end),
        None => full_response(&state),
    }
}

async fn redirect_handler() -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, "/file")
        .body(Body::empty())
        .expect("redirect response")
}

fn full_response(state: &TestState) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, state.data.len().to_string())
        .header(header::ETAG, state.etag)
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"server.bin\"",
        )
        .body(Body::from(state.data.as_ref().clone()))
        .expect("full response")
}

fn partial_response(state: &TestState, start: u64, end: u64) -> Response {
    let body = state.data[start as usize..=end as usize].to_vec();
    Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", state.data.len()),
        )
        .header(header::CONTENT_LENGTH, body.len().to_string())
        .header(header::ETAG, state.etag)
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"server.bin\"",
        )
        .body(Body::from(body))
        .expect("partial response")
}

fn parse_range(value: &str, total: u64) -> Option<(u64, u64)> {
    let spec = value.strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    let start: u64 = start.parse().ok()?;
    if start >= total {
        return None;
    }
    let end = if end.is_empty() {
        total - 1
    } else {
        end.parse::<u64>().ok()?.min(total - 1)
    };
    if end < start {
        return None;
    }
    Some((start, end))
}

async fn start_server(
    no_range: bool,
    size: usize,
) -> (
    String,
    Arc<Vec<u8>>,
    Arc<AtomicUsize>,
    Arc<Mutex<Vec<String>>>,
) {
    let data: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
    let data = Arc::new(data);
    let requests = Arc::new(AtomicUsize::new(0));
    let ranges = Arc::new(Mutex::new(Vec::new()));

    let state = TestState {
        data: Arc::clone(&data),
        etag: "\"test-etag\"",
        no_range,
        requests: Arc::clone(&requests),
        ranges: Arc::clone(&ranges),
    };

    let app = Router::new()
        .route("/file", get(file_handler))
        .route("/redirect", get(redirect_handler))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    (format!("http://{address}"), data, requests, ranges)
}

fn options_for(base: &str, dir: &std::path::Path, name: &str) -> DownloadOptions {
    let mut options = DownloadOptions::new(format!("{base}/file"), dir);
    options.file_name = Some(name.to_owned());
    options.connections = 4;
    options.segment_size = 256 * 1024;
    options
}

#[tokio::test]
async fn downloads_with_multiple_connections() {
    let (base, data, requests, _ranges) = start_server(false, 1024 * 1024).await;
    let directory = tempfile::tempdir().expect("tempdir");
    let options = options_for(&base, directory.path(), "out.bin");

    let summary = download(options, None, CancelToken::new())
        .await
        .expect("download");

    assert_eq!(std::fs::read(&summary.path).expect("read"), *data);
    assert_eq!(summary.total, data.len() as u64);
    assert_eq!(summary.resumed_from, 0);
    assert!(
        requests.load(Ordering::SeqCst) >= 4,
        "expected multiple requests, got {}",
        requests.load(Ordering::SeqCst)
    );
    assert!(!sidecar_path(&summary.path, ".part").exists());
    assert!(!sidecar_path(&summary.path, ".tug.json").exists());
}

#[tokio::test]
async fn falls_back_to_single_stream_without_range() {
    let (base, data, requests, _ranges) = start_server(true, 300 * 1024).await;
    let directory = tempfile::tempdir().expect("tempdir");
    let options = options_for(&base, directory.path(), "single.bin");

    let summary = download(options, None, CancelToken::new())
        .await
        .expect("download");

    assert_eq!(std::fs::read(&summary.path).expect("read"), *data);
    assert_eq!(summary.total, data.len() as u64);
    assert!(requests.load(Ordering::SeqCst) >= 2);
}

#[tokio::test]
async fn follows_redirects() {
    let (base, data, _requests, _ranges) = start_server(false, 128 * 1024).await;
    let directory = tempfile::tempdir().expect("tempdir");
    let mut options = options_for(&base, directory.path(), "redirect.bin");
    options.url = format!("{base}/redirect");

    let summary = download(options, None, CancelToken::new())
        .await
        .expect("download");

    assert_eq!(std::fs::read(&summary.path).expect("read"), *data);
}

#[tokio::test]
async fn resumes_from_completed_segment() {
    let (base, data, _requests, ranges) = start_server(false, 1024 * 1024).await;
    let directory = tempfile::tempdir().expect("tempdir");
    let options = options_for(&base, directory.path(), "resume.bin");

    let final_path = directory.path().join("resume.bin");
    let part_path = sidecar_path(&final_path, ".part");
    let meta_path = sidecar_path(&final_path, ".tug.json");

    let first_segment = 256 * 1024;
    std::fs::write(&part_path, &data[..first_segment]).expect("seed part");
    let meta = ResumeMeta {
        version: META_VERSION,
        url: format!("{base}/file"),
        final_url: format!("{base}/file"),
        file_name: "resume.bin".into(),
        total: data.len() as u64,
        segment_size: first_segment as u64,
        supports_range: true,
        etag: Some("\"test-etag\"".into()),
        last_modified: None,
        completed: vec![(0, first_segment as u64)],
        partial: Vec::new(),
    };
    meta.save(&meta_path).expect("save meta");

    let summary = download(options, None, CancelToken::new())
        .await
        .expect("resume download");

    assert_eq!(std::fs::read(&summary.path).expect("read"), *data);
    assert_eq!(summary.resumed_from, first_segment as u64);

    let ranges = ranges.lock().expect("ranges");
    assert!(
        !ranges.iter().any(|value| value == "bytes=0-262143"),
        "first segment should not be re-requested: {ranges:?}"
    );
}

#[tokio::test]
async fn resumes_from_partial_segment_checkpoint() {
    let (base, data, _requests, ranges) = start_server(false, 1024 * 1024).await;
    let directory = tempfile::tempdir().expect("tempdir");
    let options = options_for(&base, directory.path(), "partial.bin");

    let final_path = directory.path().join("partial.bin");
    let part_path = sidecar_path(&final_path, ".part");
    let meta_path = sidecar_path(&final_path, ".tug.json");

    let segment = 256 * 1024u64;
    std::fs::write(&part_path, &data[..(2 * segment) as usize]).expect("seed part");
    let meta = ResumeMeta {
        version: META_VERSION,
        url: format!("{base}/file"),
        final_url: format!("{base}/file"),
        file_name: "partial.bin".into(),
        total: data.len() as u64,
        segment_size: segment,
        supports_range: true,
        etag: Some("\"test-etag\"".into()),
        last_modified: None,
        completed: vec![(segment, 2 * segment)],
        partial: vec![(0, 100 * 1024)],
    };
    meta.save(&meta_path).expect("save meta");

    let summary = download(options, None, CancelToken::new())
        .await
        .expect("partial resume");

    assert_eq!(std::fs::read(&summary.path).expect("read"), *data);
    assert_eq!(summary.resumed_from, 100 * 1024 + segment);

    let ranges = ranges.lock().expect("ranges");
    assert!(
        ranges.iter().any(|value| value == "bytes=102400-262143"),
        "partial segment should resume at checkpoint: {ranges:?}"
    );
    assert!(
        !ranges.iter().any(|value| value == "bytes=0-262143"),
        "partial segment must not restart from zero: {ranges:?}"
    );
}

#[tokio::test]
async fn reports_checksum_mismatch() {
    let (base, _data, _requests, _ranges) = start_server(false, 64 * 1024).await;
    let directory = tempfile::tempdir().expect("tempdir");
    let mut options = options_for(&base, directory.path(), "checksum.bin");
    options.checksum_sha256 = Some("0".repeat(64));

    let error = download(options, None, CancelToken::new())
        .await
        .expect_err("checksum must fail");
    assert!(matches!(error, HttpError::ChecksumMismatch { .. }));
}

#[tokio::test]
async fn avoids_overwriting_and_restarts_when_requested() {
    let (base, data, _requests, _ranges) = start_server(false, 64 * 1024).await;
    let directory = tempfile::tempdir().expect("tempdir");

    let first = download(
        options_for(&base, directory.path(), "dup.bin"),
        None,
        CancelToken::new(),
    )
    .await
    .expect("first download");
    assert_eq!(
        first.path.file_name().and_then(|value| value.to_str()),
        Some("dup.bin")
    );

    let second = download(
        options_for(&base, directory.path(), "dup.bin"),
        None,
        CancelToken::new(),
    )
    .await
    .expect("second download");
    assert_eq!(
        second.path.file_name().and_then(|value| value.to_str()),
        Some("dup (1).bin")
    );

    let mut restart = options_for(&base, directory.path(), "dup.bin");
    restart.restart = true;
    let third = download(restart, None, CancelToken::new())
        .await
        .expect("restart download");
    assert_eq!(
        third.path.file_name().and_then(|value| value.to_str()),
        Some("dup.bin")
    );
    assert_eq!(std::fs::read(&third.path).expect("read"), *data);
}

#[tokio::test]
async fn refuses_resume_when_remote_changed() {
    let (base, data, _requests, _ranges) = start_server(false, 128 * 1024).await;
    let directory = tempfile::tempdir().expect("tempdir");
    let options = options_for(&base, directory.path(), "changed.bin");

    let final_path = directory.path().join("changed.bin");
    let part_path = sidecar_path(&final_path, ".part");
    let meta_path = sidecar_path(&final_path, ".tug.json");

    std::fs::write(&part_path, &data[..64 * 1024]).expect("seed part");
    let meta = ResumeMeta {
        version: META_VERSION,
        url: format!("{base}/file"),
        final_url: format!("{base}/file"),
        file_name: "changed.bin".into(),
        total: data.len() as u64,
        segment_size: 64 * 1024,
        supports_range: true,
        etag: Some("\"old-etag\"".into()),
        last_modified: None,
        completed: vec![(0, 64 * 1024)],
        partial: Vec::new(),
    };
    meta.save(&meta_path).expect("save meta");

    let error = download(options, None, CancelToken::new())
        .await
        .expect_err("changed remote must fail");
    assert!(matches!(error, HttpError::RemoteChanged));
}

#[tokio::test]
async fn cancel_returns_promptly_while_body_stalls() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stall");
    let address = listener.local_addr().expect("stall addr");
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buffer = [0u8; 4096];
                let _ = socket.read(&mut buffer).await;
                let headers = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 1048576\r\nAccept-Ranges: bytes\r\n\r\n";
                let _ = socket.write_all(headers).await;
                let _ = socket.flush().await;
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            });
        }
    });

    let directory = tempfile::tempdir().expect("tempdir");
    let options = DownloadOptions::new(format!("http://{address}/stall"), directory.path());
    let cancel = CancelToken::new();
    let waiter = cancel.clone();
    let task = tokio::spawn(async move { download(options, None, waiter).await });

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    cancel.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("pause must cancel a stalled body within 2s")
        .expect("join download task");
    assert!(matches!(result, Err(HttpError::Cancelled)));
    server.abort();
}
