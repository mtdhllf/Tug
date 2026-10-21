use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::{Result, TugError};
use crate::task::{NewTask, TaskRecord, TaskState, now_ts};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS tasks (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    kind           TEXT    NOT NULL DEFAULT 'http',
    name           TEXT    NOT NULL,
    url            TEXT    NOT NULL,
    save_dir       TEXT    NOT NULL,
    file_name      TEXT,
    state          TEXT    NOT NULL,
    total          INTEGER,
    downloaded     INTEGER NOT NULL DEFAULT 0,
    uploaded       INTEGER NOT NULL DEFAULT 0,
    error          TEXT,
    connections    INTEGER NOT NULL DEFAULT 8,
    rate_limit_bps INTEGER,
    headers_json   TEXT    NOT NULL DEFAULT '[]',
    proxy          TEXT,
    checksum       TEXT,
    extra_json     TEXT    NOT NULL DEFAULT '{}',
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tasks_created ON tasks(created_at DESC, id DESC);
"#;

/// SQLite 任务存储。连接由互斥锁保护，操作都是毫秒级。
pub struct TaskStore {
    connection: Mutex<Connection>,
}

impl TaskStore {
    /// 打开/创建数据库并初始化表结构。
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        connection.execute_batch(SCHEMA)?;
        ensure_columns(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// 按创建时间倒序列出所有任务。
    pub fn list(&self) -> Result<Vec<TaskRecord>> {
        let connection = self.connection.lock().expect("task store poisoned");
        let mut statement = connection.prepare(
            "SELECT id, kind, name, url, save_dir, file_name, state, total, downloaded, uploaded,
                    error, connections, rate_limit_bps, headers_json, proxy, checksum, extra_json,
                    created_at, updated_at
             FROM tasks ORDER BY created_at DESC, id DESC",
        )?;
        let records = statement
            .query_map([], row_to_record)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(records)
    }

    /// 读取单个任务。
    pub fn get(&self, id: i64) -> Result<Option<TaskRecord>> {
        let connection = self.connection.lock().expect("task store poisoned");
        let record = connection
            .query_row(
                "SELECT id, kind, name, url, save_dir, file_name, state, total, downloaded, uploaded,
                        error, connections, rate_limit_bps, headers_json, proxy, checksum, extra_json,
                        created_at, updated_at
                 FROM tasks WHERE id = ?1",
                params![id],
                row_to_record,
            )
            .optional()?;
        Ok(record)
    }

    /// 取下一条排队中的任务（最早创建优先）。
    pub fn next_queued(&self) -> Result<Option<TaskRecord>> {
        let connection = self.connection.lock().expect("task store poisoned");
        let record = connection
            .query_row(
                "SELECT id, kind, name, url, save_dir, file_name, state, total, downloaded, uploaded,
                        error, connections, rate_limit_bps, headers_json, proxy, checksum, extra_json,
                        created_at, updated_at
                 FROM tasks WHERE state = 'queued' ORDER BY created_at ASC, id ASC LIMIT 1",
                [],
                row_to_record,
            )
            .optional()?;
        Ok(record)
    }

    /// 插入任务，返回带主键的记录。
    pub fn insert(&self, task: &NewTask) -> Result<TaskRecord> {
        let headers = serde_json::to_string(&task.headers)
            .map_err(|err| TugError::Config(format!("序列化请求头失败: {err}")))?;
        let extra = serde_json::to_string(&task.extra)
            .map_err(|err| TugError::Config(format!("序列化任务扩展失败: {err}")))?;
        let now = now_ts();

        let id = {
            let connection = self.connection.lock().expect("task store poisoned");
            connection.execute(
                "INSERT INTO tasks
                    (kind, name, url, save_dir, file_name, state, downloaded, uploaded,
                     connections, rate_limit_bps, headers_json, proxy, checksum, extra_json,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'queued', 0, 0, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                params![
                    task.kind,
                    task.name,
                    task.url,
                    task.save_dir.display().to_string(),
                    task.file_name,
                    task.connections,
                    task.rate_limit_bps,
                    headers,
                    task.proxy,
                    task.checksum,
                    extra,
                    now,
                ],
            )?;
            connection.last_insert_rowid()
        };

        self.get(id)?
            .ok_or_else(|| TugError::Config("插入任务后读取失败".into()))
    }

    /// 更新进度。
    pub fn set_progress(&self, id: i64, downloaded: u64, total: Option<u64>) -> Result<()> {
        let connection = self.connection.lock().expect("task store poisoned");
        connection.execute(
            "UPDATE tasks SET downloaded = ?1, total = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                downloaded as i64,
                total.map(|value| value as i64),
                now_ts(),
                id
            ],
        )?;
        Ok(())
    }

    /// 更新上传字节数。
    pub fn set_uploaded(&self, id: i64, uploaded: u64) -> Result<()> {
        let connection = self.connection.lock().expect("task store poisoned");
        connection.execute(
            "UPDATE tasks SET uploaded = ?1, updated_at = ?2 WHERE id = ?3",
            params![uploaded as i64, now_ts(), id],
        )?;
        Ok(())
    }

    /// 更新任务扩展字段。
    pub fn set_extra(&self, id: i64, extra: &serde_json::Value) -> Result<()> {
        let extra = serde_json::to_string(extra)
            .map_err(|err| TugError::Config(format!("序列化任务扩展失败: {err}")))?;
        let connection = self.connection.lock().expect("task store poisoned");
        connection.execute(
            "UPDATE tasks SET extra_json = ?1, updated_at = ?2 WHERE id = ?3",
            params![extra, now_ts(), id],
        )?;
        Ok(())
    }

    /// 更新状态与错误信息。
    pub fn set_state(&self, id: i64, state: TaskState, error: Option<&str>) -> Result<()> {
        let connection = self.connection.lock().expect("task store poisoned");
        connection.execute(
            "UPDATE tasks SET state = ?1, error = ?2, updated_at = ?3 WHERE id = ?4",
            params![state.as_str(), error, now_ts(), id],
        )?;
        Ok(())
    }

    /// 更新展示名与文件名。
    pub fn set_name(&self, id: i64, name: &str, file_name: Option<&str>) -> Result<()> {
        let connection = self.connection.lock().expect("task store poisoned");
        connection.execute(
            "UPDATE tasks SET name = ?1, file_name = ?2, updated_at = ?3 WHERE id = ?4",
            params![name, file_name, now_ts(), id],
        )?;
        Ok(())
    }

    /// 删除任务。
    pub fn delete(&self, id: i64) -> Result<()> {
        let connection = self.connection.lock().expect("task store poisoned");
        connection.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 应用启动时恢复中断任务，返回受影响数量。
    ///
    /// `seeding` 表示下载已经完成、只是上次退出时还在共享；
    /// 重启后应回到“已完成”，不能提示“上次退出未完成”。
    pub fn mark_interrupted(&self) -> Result<usize> {
        let connection = self.connection.lock().expect("task store poisoned");
        let now = now_ts();
        let completed = connection.execute(
            "UPDATE tasks SET state = 'completed', error = NULL, updated_at = ?1
             WHERE state = 'seeding'",
            params![now],
        )?;
        // 关机、任务管理器结束进程都属于正常中断，只恢复为暂停，
        // 不显示“上次退出未完成”这类提示；只有真正的网络/校验错误才保留 error。
        let paused = connection.execute(
            "UPDATE tasks SET state = 'paused', error = NULL, updated_at = ?1
             WHERE state IN ('queued', 'fetching_metadata', 'downloading')",
            params![now],
        )?;
        // 兼容旧版本：旧版会把 seeding 直接写成带“未完成”提示的 paused。
        // 已经下满但仍处于 paused 的任务，升级后应恢复为已完成。
        let repaired = connection.execute(
            "UPDATE tasks SET state = 'completed', error = NULL, updated_at = ?1
             WHERE state = 'paused' AND error = ?2
               AND total IS NOT NULL AND downloaded >= total",
            params![now, "上次退出时未完成，点击继续下载"],
        )?;
        // 清理旧版本遗留在普通暂停任务上的“上次退出未完成”提示。
        connection.execute(
            "UPDATE tasks SET error = NULL
             WHERE state = 'paused' AND error = ?1",
            params!["上次退出时未完成，点击继续下载"],
        )?;
        Ok(completed + paused + repaired)
    }
}

fn ensure_columns(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare("PRAGMA table_info(tasks)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !columns.iter().any(|column| column == "uploaded") {
        connection.execute(
            "ALTER TABLE tasks ADD COLUMN uploaded INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !columns.iter().any(|column| column == "extra_json") {
        connection.execute(
            "ALTER TABLE tasks ADD COLUMN extra_json TEXT NOT NULL DEFAULT '{}'",
            [],
        )?;
    }
    Ok(())
}

fn row_to_record(row: &Row<'_>) -> rusqlite::Result<TaskRecord> {
    let headers_json: String = row.get("headers_json")?;
    let headers = serde_json::from_str(&headers_json).unwrap_or_default();

    Ok(TaskRecord {
        id: row.get("id")?,
        kind: row.get("kind")?,
        name: row.get("name")?,
        url: row.get("url")?,
        save_dir: PathBuf::from(row.get::<_, String>("save_dir")?),
        file_name: row.get("file_name")?,
        state: TaskState::parse(&row.get::<_, String>("state")?),
        total: row
            .get::<_, Option<i64>>("total")?
            .and_then(|value| u64::try_from(value).ok()),
        downloaded: row
            .get::<_, i64>("downloaded")
            .map(|value| u64::try_from(value).unwrap_or(0))?,
        uploaded: row
            .get::<_, i64>("uploaded")
            .map(|value| u64::try_from(value).unwrap_or(0))?,
        error: row.get("error")?,
        connections: row
            .get::<_, i64>("connections")
            .map(|value| value.clamp(1, 32) as u8)?,
        rate_limit_bps: row
            .get::<_, Option<i64>>("rate_limit_bps")?
            .and_then(|value| u64::try_from(value).ok()),
        headers,
        proxy: row.get("proxy")?,
        checksum: row.get("checksum")?,
        extra: {
            let extra_json: String = row.get("extra_json")?;
            serde_json::from_str(&extra_json).unwrap_or_else(|_| serde_json::json!({}))
        },
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> (TaskStore, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "tug-store-{}-{}-{name}",
            std::process::id(),
            now_ts()
        ));
        std::fs::create_dir_all(&root).expect("tmp");
        let store = TaskStore::open(&root.join("tasks.db")).expect("open store");
        (store, root)
    }

    fn new_task(dir: &Path) -> NewTask {
        NewTask {
            kind: "http".into(),
            name: "file.bin".into(),
            url: "https://example.com/file.bin".into(),
            save_dir: dir.to_path_buf(),
            file_name: None,
            connections: 8,
            rate_limit_bps: None,
            headers: vec![("Referer".into(), "https://example.com".into())],
            proxy: None,
            checksum: None,
            extra: serde_json::json!({}),
        }
    }

    #[test]
    fn inserts_lists_and_updates() {
        let (store, root) = temp_store("crud");
        let record = store.insert(&new_task(&root)).expect("insert");
        assert_eq!(record.state, TaskState::Queued);

        store
            .set_progress(record.id, 1024, Some(4096))
            .expect("progress");
        store
            .set_state(record.id, TaskState::Downloading, None)
            .expect("state");
        store
            .set_name(record.id, "renamed.bin", Some("renamed.bin"))
            .expect("name");
        store.set_uploaded(record.id, 2048).expect("uploaded");
        store
            .set_extra(record.id, &serde_json::json!({"librqbit_id": 7}))
            .expect("extra");

        let renamed = store.get(record.id).expect("get").expect("exists");
        assert_eq!(renamed.name, "renamed.bin");
        assert_eq!(renamed.uploaded, 2048);
        assert_eq!(renamed.extra["librqbit_id"], 7);

        let loaded = store.get(record.id).expect("get").expect("exists");
        assert_eq!(loaded.downloaded, 1024);
        assert_eq!(loaded.total, Some(4096));
        assert_eq!(loaded.state, TaskState::Downloading);
        assert_eq!(loaded.name, "renamed.bin");

        let list = store.list().expect("list");
        assert_eq!(list.len(), 1);

        store.delete(record.id).expect("delete");
        assert!(store.get(record.id).expect("get").is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn marks_interrupted_as_paused() {
        let (store, root) = temp_store("interrupted");
        let record = store.insert(&new_task(&root)).expect("insert");
        store
            .set_state(record.id, TaskState::Downloading, None)
            .expect("state");

        assert_eq!(store.mark_interrupted().expect("mark"), 1);
        let loaded = store.get(record.id).expect("get").expect("exists");
        assert_eq!(loaded.state, TaskState::Paused);
        assert!(loaded.error.is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn completed_seeding_stays_completed_after_restart() {
        let (store, root) = temp_store("seeding-completed");
        let record = store.insert(&new_task(&root)).expect("insert");
        store
            .set_state(record.id, TaskState::Seeding, None)
            .expect("state");

        assert_eq!(store.mark_interrupted().expect("mark"), 1);
        let loaded = store.get(record.id).expect("get").expect("exists");
        assert_eq!(loaded.state, TaskState::Completed);
        assert!(loaded.error.is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn clears_old_interrupted_prompt_on_paused_task() {
        let (store, root) = temp_store("paused-prompt");
        let record = store.insert(&new_task(&root)).expect("insert");
        store.set_progress(record.id, 1, Some(2)).expect("progress");
        store
            .set_state(
                record.id,
                TaskState::Paused,
                Some("上次退出时未完成，点击继续下载"),
            )
            .expect("state");

        store.mark_interrupted().expect("mark");
        let loaded = store.get(record.id).expect("get").expect("exists");
        assert_eq!(loaded.state, TaskState::Paused);
        assert!(loaded.error.is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn repairs_old_paused_completed_task() {
        let (store, root) = temp_store("paused-completed");
        let record = store.insert(&new_task(&root)).expect("insert");
        store
            .set_progress(record.id, 1024, Some(1024))
            .expect("progress");
        store
            .set_state(
                record.id,
                TaskState::Paused,
                Some("上次退出时未完成，点击继续下载"),
            )
            .expect("state");

        assert_eq!(store.mark_interrupted().expect("mark"), 1);
        let loaded = store.get(record.id).expect("get").expect("exists");
        assert_eq!(loaded.state, TaskState::Completed);
        assert!(loaded.error.is_none());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn migrates_legacy_schema() {
        let root = std::env::temp_dir().join(format!(
            "tug-store-legacy-{}-{}",
            std::process::id(),
            now_ts()
        ));
        std::fs::create_dir_all(&root).expect("tmp");
        let path = root.join("tasks.db");
        {
            let connection = Connection::open(&path).expect("open");
            connection
                .execute_batch(
                    "CREATE TABLE tasks (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        kind TEXT NOT NULL DEFAULT 'http',
                        name TEXT NOT NULL,
                        url TEXT NOT NULL,
                        save_dir TEXT NOT NULL,
                        file_name TEXT,
                        state TEXT NOT NULL,
                        total INTEGER,
                        downloaded INTEGER NOT NULL DEFAULT 0,
                        error TEXT,
                        connections INTEGER NOT NULL DEFAULT 8,
                        rate_limit_bps INTEGER,
                        headers_json TEXT NOT NULL DEFAULT '[]',
                        proxy TEXT,
                        checksum TEXT,
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );",
                )
                .expect("schema");
            connection
                .execute(
                    "INSERT INTO tasks
                        (kind, name, url, save_dir, state, created_at, updated_at)
                     VALUES ('http', 'old.bin', 'https://example.com/old.bin', 'C:\\tmp', 'paused', 1, 1)",
                    [],
                )
                .expect("insert");
        }

        let store = TaskStore::open(&path).expect("open migrated");
        let records = store.list().expect("list");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].uploaded, 0);
        assert_eq!(records[0].extra, serde_json::json!({}));
        std::fs::remove_dir_all(root).ok();
    }
}
