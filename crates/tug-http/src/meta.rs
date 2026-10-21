use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{HttpError, Result};
use crate::probe::DownloadPlan;
use crate::segments::Segment;

/// 续传元数据版本。
pub const META_VERSION: u32 = 1;

/// 与数据文件同目录的 sidecar 元数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeMeta {
    /// 格式版本。
    pub version: u32,
    /// 初始 URL。
    pub url: String,
    /// 重定向后的最终 URL。
    pub final_url: String,
    /// 最终文件名。
    pub file_name: String,
    /// 文件总大小。
    pub total: u64,
    /// 分片大小。
    pub segment_size: u64,
    /// 服务器是否支持 Range。
    pub supports_range: bool,
    /// 强 ETag（用于 `If-Range`）。
    pub etag: Option<String>,
    /// Last-Modified。
    pub last_modified: Option<String>,
    /// 已完成区间，按起点排序且已合并。
    pub completed: Vec<(u64, u64)>,
    /// 在途分片已落盘的区间；旧版元数据没有该字段。
    #[serde(default)]
    pub partial: Vec<(u64, u64)>,
}

impl ResumeMeta {
    /// 为一次新下载创建元数据。
    pub fn new(plan: &DownloadPlan, file_name: &str, segment_size: u64) -> Self {
        Self {
            version: META_VERSION,
            url: plan.url.clone(),
            final_url: plan.final_url.clone(),
            file_name: file_name.to_owned(),
            total: plan.total.unwrap_or(0),
            segment_size,
            supports_range: plan.supports_range,
            etag: plan.etag.clone(),
            last_modified: plan.last_modified.clone(),
            completed: Vec::new(),
            partial: Vec::new(),
        }
    }

    /// 读取元数据；文件不存在返回 `None`。
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path)?;
        let meta: Self = serde_json::from_str(&text)
            .map_err(|err| HttpError::Meta(format!("解析失败: {err}")))?;
        if meta.version != META_VERSION {
            return Err(HttpError::Meta(format!(
                "不支持的元数据版本 {}",
                meta.version
            )));
        }
        Ok(Some(meta))
    }

    /// 原子写入元数据：先写同目录临时文件，再替换目标文件。
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| HttpError::Meta("元数据路径缺少父目录".into()))?;
        std::fs::create_dir_all(parent)?;

        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temporary, self)
            .map_err(|err| HttpError::Meta(format!("序列化失败: {err}")))?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(path)
            .map_err(|err| HttpError::Io(err.error))?;
        Ok(())
    }

    /// 已完成字节总数。
    pub fn completed_bytes(&self) -> u64 {
        self.completed
            .iter()
            .map(|(start, end)| end.saturating_sub(*start))
            .sum()
    }

    /// 在途分片已落盘的字节数。
    pub fn partial_bytes(&self) -> u64 {
        self.partial
            .iter()
            .map(|(start, end)| end.saturating_sub(*start))
            .sum()
    }

    /// 可复用的总字节数（已完成 + 在途 checkpoint）。
    pub fn resumed_bytes(&self) -> u64 {
        self.completed_bytes() + self.partial_bytes()
    }

    /// 已完成区间与在途区间中的最大结束偏移。
    pub fn max_written_end(&self) -> u64 {
        self.completed
            .iter()
            .chain(self.partial.iter())
            .map(|(_, end)| *end)
            .max()
            .unwrap_or(0)
    }

    /// 记录某个分片的当前写入进度。
    pub fn set_partial(&mut self, segment_start: u64, end: u64) {
        self.partial.retain(|(start, _)| *start != segment_start);
        if end > segment_start {
            self.partial.push((segment_start, end));
            self.partial.sort_unstable_by_key(|(start, _)| *start);
        }
    }

    /// 清除某个分片的在途进度。
    pub fn clear_partial(&mut self, segment_start: u64) {
        self.partial.retain(|(start, _)| *start != segment_start);
    }

    /// 返回分片内可跳过的偏移；没有有效 checkpoint 时返回 `None`。
    pub fn partial_offset(&self, segment: &Segment) -> Option<u64> {
        self.partial
            .iter()
            .find(|(start, _)| *start == segment.start)
            .map(|(_, end)| *end)
            .filter(|end| *end > segment.start && *end < segment.end)
    }

    /// 把已经覆盖完整分片的 checkpoint 提升为 completed，避免重复下载。
    pub fn normalize_partials(&mut self, segments: &[Segment]) {
        let mut promoted = Vec::new();
        self.partial.retain(|(start, end)| {
            let Some(segment) = segments.iter().find(|segment| segment.start == *start) else {
                return false;
            };
            if *end >= segment.end {
                promoted.push((segment.start, segment.end));
                false
            } else {
                true
            }
        });
        for (start, end) in promoted {
            self.mark_complete(start, end);
        }
    }

    /// 判断分片是否已被完整覆盖。
    pub fn is_segment_complete(&self, segment: &Segment) -> bool {
        self.completed
            .iter()
            .any(|(start, end)| *start <= segment.start && *end >= segment.end)
    }

    /// 标记区间完成，并合并相邻/重叠区间。
    pub fn mark_complete(&mut self, start: u64, end: u64) {
        if end <= start {
            return;
        }
        self.completed.push((start, end));
        self.completed.sort_unstable_by_key(|(start, _)| *start);

        let mut merged: Vec<(u64, u64)> = Vec::with_capacity(self.completed.len());
        for (start, end) in self.completed.drain(..) {
            if let Some(last) = merged.last_mut()
                && start <= last.1
            {
                last.1 = last.1.max(end);
                continue;
            }
            merged.push((start, end));
        }
        self.completed = merged;
    }

    /// 判断元数据是否仍与远端一致。ETag 优先，其次 Last-Modified，最后只比大小。
    pub fn matches(&self, plan: &DownloadPlan) -> bool {
        if !self.supports_range || !plan.supports_range {
            return false;
        }
        if self.total != plan.total.unwrap_or(0) {
            return false;
        }
        match (&self.etag, &plan.etag) {
            (Some(ours), Some(theirs)) => ours == theirs,
            _ => match (&self.last_modified, &plan.last_modified) {
                (Some(ours), Some(theirs)) => ours == theirs,
                _ => true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(total: u64, etag: &str) -> DownloadPlan {
        DownloadPlan {
            url: "https://example.com/file".into(),
            final_url: "https://cdn.example.com/file".into(),
            file_name: "file.bin".into(),
            total: Some(total),
            supports_range: true,
            etag: Some(etag.into()),
            last_modified: None,
            content_type: None,
        }
    }

    fn meta() -> ResumeMeta {
        ResumeMeta::new(&plan(1000, "\"abc\""), "file.bin", 256)
    }

    #[test]
    fn merges_adjacent_ranges() {
        let mut meta = meta();
        meta.mark_complete(0, 256);
        meta.mark_complete(256, 512);
        meta.mark_complete(700, 800);
        assert_eq!(meta.completed, vec![(0, 512), (700, 800)]);
        assert_eq!(meta.completed_bytes(), 612);
        assert_eq!(meta.max_written_end(), 800);
    }

    #[test]
    fn detects_complete_segments() {
        let mut meta = meta();
        meta.mark_complete(0, 512);
        assert!(meta.is_segment_complete(&Segment {
            index: 0,
            start: 0,
            end: 256
        }));
        assert!(meta.is_segment_complete(&Segment {
            index: 1,
            start: 256,
            end: 512
        }));
        assert!(!meta.is_segment_complete(&Segment {
            index: 2,
            start: 512,
            end: 768
        }));
    }

    #[test]
    fn tracks_and_normalizes_partial() {
        let mut meta = meta();
        meta.set_partial(0, 100);
        meta.set_partial(0, 150);
        assert_eq!(meta.partial, vec![(0, 150)]);
        assert_eq!(meta.partial_bytes(), 150);
        assert_eq!(meta.resumed_bytes(), 150);

        let segment = Segment {
            index: 0,
            start: 0,
            end: 256,
        };
        assert_eq!(meta.partial_offset(&segment), Some(150));

        meta.set_partial(0, 300);
        meta.normalize_partials(&[segment]);
        assert!(meta.partial.is_empty());
        assert!(meta.is_segment_complete(&segment));
        assert_eq!(meta.resumed_bytes(), 256);
    }

    #[test]
    fn matches_same_etag_and_total() {
        let meta = meta();
        assert!(meta.matches(&plan(1000, "\"abc\"")));
        assert!(!meta.matches(&plan(1000, "\"changed\"")));
        assert!(!meta.matches(&plan(999, "\"abc\"")));
    }

    #[test]
    fn roundtrip_atomic_save() {
        let root = std::env::temp_dir().join(format!("tug-meta-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("tmp");
        let path = root.join("file.tug.json");

        let mut original = meta();
        original.mark_complete(0, 256);
        original.save(&path).expect("save");

        let loaded = ResumeMeta::load(&path).expect("load").expect("exists");
        assert_eq!(loaded.completed, vec![(0, 256)]);
        assert_eq!(loaded.total, 1000);

        std::fs::remove_dir_all(root).ok();
    }
}
