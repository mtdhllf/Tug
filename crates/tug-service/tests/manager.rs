use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use tokio::net::TcpListener;
use tug_core::{AppPaths, NewTask, Settings, TaskState, TaskStore, TaskView};
use tug_service::{AddTaskInput, TaskManager};

async fn start_server(delay: Duration) -> String {
    let data: Arc<Vec<u8>> = Arc::new((0..256 * 1024).map(|index| (index % 251) as u8).collect());
    let app = Router::new().route(
        "/file",
        get(move |headers: HeaderMap| {
            let data = Arc::clone(&data);
            async move {
                tokio::time::sleep(delay).await;
                file_response(&headers, &data)
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{address}/file")
}

fn file_response(headers: &HeaderMap, data: &[u8]) -> Response {
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_range(value, data.len() as u64));

    match range {
        Some((start, end)) => {
            let body = data[start as usize..=end as usize].to_vec();
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {start}-{end}/{}", data.len()),
                )
                .header(header::CONTENT_LENGTH, body.len().to_string())
                .header(header::ETAG, "\"svc-etag\"")
                .body(Body::from(body))
                .expect("partial")
        }
        None => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_LENGTH, data.len().to_string())
            .header(header::ETAG, "\"svc-etag\"")
            .body(Body::from(data.to_vec()))
            .expect("full"),
    }
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

fn setup() -> (tempfile::TempDir, Arc<TaskManager>) {
    let root = tempfile::tempdir().expect("tempdir");
    let paths = AppPaths::under_root(root.path());
    paths.ensure().expect("paths");
    let settings = Arc::new(RwLock::new(Settings::initial(&paths)));
    let manager = TaskManager::new(&paths, settings).expect("manager");
    (root, manager)
}

async fn wait_for(manager: &TaskManager, id: i64, expected: TaskState) -> TaskView {
    for _ in 0..200 {
        let task = manager
            .list()
            .expect("list")
            .into_iter()
            .find(|task| task.id == id)
            .expect("task exists");
        if task.state == expected {
            return task;
        }
        assert_ne!(
            task.state,
            TaskState::Failed,
            "task failed: {:?}",
            task.error
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timeout waiting for {expected:?}");
}

fn input(url: String, directory: &std::path::Path) -> AddTaskInput {
    AddTaskInput {
        url,
        save_dir: Some(directory.to_path_buf()),
        file_name: Some("service.bin".into()),
        connections: Some(4),
        only_files: None,
    }
}

#[tokio::test]
async fn add_download_complete_and_remove() {
    let (root, manager) = setup();
    let url = start_server(Duration::ZERO).await;

    let created = manager
        .add(input(url, root.path()))
        .await
        .expect("add task");
    let completed = wait_for(&manager, created.id, TaskState::Completed).await;
    assert_eq!(
        std::fs::read(&completed.path).expect("read"),
        (0..256 * 1024)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>()
    );

    manager.remove(completed.id, true).await.expect("remove");
    assert!(manager.list().expect("list").is_empty());
    assert!(!std::path::Path::new(&completed.path).exists());
}

#[tokio::test]
async fn pause_then_resume() {
    let (root, manager) = setup();
    let url = start_server(Duration::from_millis(200)).await;

    let created = manager
        .add(input(url, root.path()))
        .await
        .expect("add task");
    tokio::time::sleep(Duration::from_millis(50)).await;
    manager.pause(created.id).await.expect("pause");

    let paused = wait_for(&manager, created.id, TaskState::Paused).await;
    assert_eq!(paused.state, TaskState::Paused);

    manager.resume(created.id).await.expect("resume");
    let completed = wait_for(&manager, created.id, TaskState::Completed).await;
    assert!(std::path::Path::new(&completed.path).is_file());
}

#[tokio::test]
async fn queues_when_concurrency_reached() {
    let root = tempfile::tempdir().expect("tempdir");
    let paths = AppPaths::under_root(root.path());
    paths.ensure().expect("paths");
    let mut settings = Settings::initial(&paths);
    settings.max_concurrent_tasks = 1;
    let manager = TaskManager::new(&paths, Arc::new(RwLock::new(settings))).expect("manager");
    let url = start_server(Duration::from_millis(100)).await;

    let first = manager
        .add(input(url.clone(), root.path()))
        .await
        .expect("first task");
    let second = manager
        .add(input(url, root.path()))
        .await
        .expect("second task");

    let tasks = manager.list().expect("list");
    let first_state = tasks
        .iter()
        .find(|task| task.id == first.id)
        .expect("first")
        .state;
    let second_state = tasks
        .iter()
        .find(|task| task.id == second.id)
        .expect("second")
        .state;
    assert!(matches!(
        first_state,
        TaskState::Downloading | TaskState::Completed
    ));
    assert_eq!(second_state, TaskState::Queued);

    manager.pause(first.id).await.expect("pause first");
    let completed = wait_for(&manager, second.id, TaskState::Completed).await;
    assert!(std::path::Path::new(&completed.path).is_file());

    manager.resume(first.id).await.expect("resume first");
    let first_done = wait_for(&manager, first.id, TaskState::Completed).await;
    assert!(std::path::Path::new(&first_done.path).is_file());
}

#[tokio::test]
async fn pauses_and_resumes_all_tasks() {
    let root = tempfile::tempdir().expect("tempdir");
    let paths = AppPaths::under_root(root.path());
    paths.ensure().expect("paths");
    let mut settings = Settings::initial(&paths);
    settings.max_concurrent_tasks = 2;
    let manager = TaskManager::new(&paths, Arc::new(RwLock::new(settings))).expect("manager");
    let url = start_server(Duration::from_millis(200)).await;

    let first = manager
        .add(input(url.clone(), root.path()))
        .await
        .expect("first");
    let second = manager.add(input(url, root.path())).await.expect("second");

    manager.pause_all().await.expect("pause all");
    for task in manager.list().expect("list") {
        assert_ne!(task.state, TaskState::Downloading);
    }

    manager.resume_all().await.expect("resume all");
    wait_for(&manager, first.id, TaskState::Completed).await;
    wait_for(&manager, second.id, TaskState::Completed).await;
}

#[tokio::test]
async fn marks_previous_session_as_paused() {
    let root = tempfile::tempdir().expect("tempdir");
    let paths = AppPaths::under_root(root.path());
    paths.ensure().expect("paths");

    {
        let store = TaskStore::open(&paths.data_dir.join("tasks.db")).expect("store");
        let record = store
            .insert(&NewTask {
                kind: "http".into(),
                name: "old.bin".into(),
                url: "https://example.com/old.bin".into(),
                save_dir: root.path().to_path_buf(),
                file_name: None,
                connections: 8,
                rate_limit_bps: None,
                headers: Vec::new(),
                proxy: None,
                checksum: None,
                extra: serde_json::json!({}),
            })
            .expect("insert");
        store
            .set_state(record.id, TaskState::Downloading, None)
            .expect("state");
    }

    let settings = Arc::new(RwLock::new(Settings::initial(&paths)));
    let manager = TaskManager::new(&paths, settings).expect("manager");
    let tasks = manager.list().expect("list");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].state, TaskState::Paused);
    // 正常退出/关机导致的中断不显示错误提示。
    assert!(tasks[0].error.is_none());
}
