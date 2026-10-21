use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    /// 排队等待。
    Queued,
    /// 正在获取磁力元数据。
    #[serde(rename = "fetching_metadata")]
    FetchingMetadata,
    /// 正在下载。
    Downloading,
    /// 用户暂停或上次异常退出。
    Paused,
    /// 下载完成后做种中。
    Seeding,
    /// 已完成。
    Completed,
    /// 失败。
    Failed,
}

impl TaskState {
    /// 数据库/事件使用的字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::FetchingMetadata => "fetching_metadata",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Seeding => "seeding",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    /// 从数据库字符串解析；未知值按 `Failed` 处理。
    pub fn parse(value: &str) -> Self {
        match value {
            "queued" => Self::Queued,
            "fetching_metadata" => Self::FetchingMetadata,
            "downloading" => Self::Downloading,
            "paused" => Self::Paused,
            "seeding" => Self::Seeding,
            "completed" => Self::Completed,
            _ => Self::Failed,
        }
    }

    /// 是否是“活动”状态（UI 侧栏“下载中”）。
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::FetchingMetadata | Self::Downloading | Self::Seeding
        )
    }
}

/// 持久化的任务记录。
#[derive(Debug, Clone)]
pub struct TaskRecord {
    /// 自增主键。
    pub id: i64,
    /// 任务类型，M2a 固定为 `http`。
    pub kind: String,
    /// 展示名。
    pub name: String,
    /// 原始 URL。
    pub url: String,
    /// 输出目录。
    pub save_dir: PathBuf,
    /// 最终文件名；下载完成前可能为空。
    pub file_name: Option<String>,
    /// 当前状态。
    pub state: TaskState,
    /// 总大小。
    pub total: Option<u64>,
    /// 已下载字节。
    pub downloaded: u64,
    /// 已上传字节。
    pub uploaded: u64,
    /// 最近一次错误。
    pub error: Option<String>,
    /// 类型相关扩展（BT 的 info hash、文件选择等）。
    pub extra: serde_json::Value,
    /// 连接数。
    pub connections: u8,
    /// 限速（字节/秒）。
    pub rate_limit_bps: Option<u64>,
    /// 额外请求头。
    pub headers: Vec<(String, String)>,
    /// 代理。
    pub proxy: Option<String>,
    /// 期望 SHA-256。
    pub checksum: Option<String>,
    /// 创建时间（Unix 秒）。
    pub created_at: i64,
    /// 更新时间（Unix 秒）。
    pub updated_at: i64,
}

impl TaskRecord {
    /// 最终/预期的文件路径。
    pub fn path(&self) -> PathBuf {
        let name = self.file_name.clone().unwrap_or_else(|| self.name.clone());
        self.save_dir.join(name)
    }

    /// 转成前端使用的视图（实时速度由运行时覆盖）。
    pub fn view(&self) -> TaskView {
        let path = self.path();
        TaskView {
            id: self.id,
            kind: self.kind.clone(),
            name: self.name.clone(),
            state: self.state,
            total: self.total,
            downloaded: self.downloaded,
            uploaded: self.uploaded,
            speed_bps: 0,
            upload_speed_bps: 0,
            peers: 0,
            eta_secs: None,
            error: self.error.clone(),
            save_dir: self.save_dir.display().to_string(),
            file_name: self.file_name.clone(),
            path: path.display().to_string(),
            created_at: self.created_at,
        }
    }
}

/// 前端渲染用的任务视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    /// 主键。
    pub id: i64,
    /// 类型。
    pub kind: String,
    /// 展示名。
    pub name: String,
    /// 状态。
    pub state: TaskState,
    /// 总大小。
    pub total: Option<u64>,
    /// 已下载字节。
    pub downloaded: u64,
    /// 已上传字节。
    pub uploaded: u64,
    /// 实时下载速度。
    pub speed_bps: u64,
    /// 实时上传速度。
    pub upload_speed_bps: u64,
    /// 当前活跃 peer 数。
    pub peers: u32,
    /// 预计剩余秒数。
    pub eta_secs: Option<u64>,
    /// 错误信息。
    pub error: Option<String>,
    /// 输出目录。
    pub save_dir: String,
    /// 文件名。
    pub file_name: Option<String>,
    /// 完整路径。
    pub path: String,
    /// 创建时间。
    pub created_at: i64,
}

/// 新建任务的输入。
#[derive(Debug, Clone)]
pub struct NewTask {
    /// 类型：`http` 或 `torrent`。
    pub kind: String,
    /// 展示名。
    pub name: String,
    /// URL / 磁力 / 种子路径。
    pub url: String,
    /// 输出目录。
    pub save_dir: PathBuf,
    /// 指定文件名。
    pub file_name: Option<String>,
    /// 连接数。
    pub connections: u8,
    /// 限速。
    pub rate_limit_bps: Option<u64>,
    /// 请求头。
    pub headers: Vec<(String, String)>,
    /// 代理。
    pub proxy: Option<String>,
    /// 校验。
    pub checksum: Option<String>,
    /// 类型相关扩展。
    pub extra: serde_json::Value,
}

/// 当前 Unix 时间戳（秒）。
pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
}

/// 从路径中取文件名用于展示。
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| "download".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_states_round_trip_through_strings() {
        for state in [
            TaskState::Queued,
            TaskState::FetchingMetadata,
            TaskState::Downloading,
            TaskState::Paused,
            TaskState::Seeding,
            TaskState::Completed,
            TaskState::Failed,
        ] {
            assert_eq!(TaskState::parse(state.as_str()), state);
        }
    }

    #[test]
    fn fetching_metadata_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&TaskState::FetchingMetadata).expect("serialize"),
            "\"fetching_metadata\""
        );
    }
}
