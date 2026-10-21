use crate::options::MIN_SEGMENT_SIZE;

/// 一个字节区间 `[start, end)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// 从 0 开始的序号。
    pub index: usize,
    /// 起始偏移（含）。
    pub start: u64,
    /// 结束偏移（不含）。
    pub end: u64,
}

impl Segment {
    /// 分片长度。
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// 是否为空分片。
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// 按固定大小切分 `[0, total)`。
pub fn build_segments(total: u64, segment_size: u64) -> Vec<Segment> {
    if total == 0 || segment_size == 0 {
        return Vec::new();
    }

    let count = total.div_ceil(segment_size) as usize;
    let mut segments = Vec::with_capacity(count);
    let mut start = 0;
    let mut index = 0;
    while start < total {
        let end = (start + segment_size).min(total);
        segments.push(Segment { index, start, end });
        start = end;
        index += 1;
    }
    segments
}

/// 小文件时缩小分片，让连接数真正用起来；大文件使用期望值。
pub fn effective_segment_size(total: u64, connections: usize, preferred: u64) -> u64 {
    if total == 0 {
        return preferred.max(MIN_SEGMENT_SIZE);
    }

    let connections = connections.max(1) as u64;
    let by_connections = total.div_ceil(connections);
    preferred
        .min(by_connections.max(MIN_SEGMENT_SIZE))
        .max(MIN_SEGMENT_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_evenly() {
        let segments = build_segments(1024, 256);
        assert_eq!(segments.len(), 4);
        assert_eq!(
            segments[0],
            Segment {
                index: 0,
                start: 0,
                end: 256
            }
        );
        assert_eq!(
            segments[3],
            Segment {
                index: 3,
                start: 768,
                end: 1024
            }
        );
    }

    #[test]
    fn keeps_remainder() {
        let segments = build_segments(1000, 256);
        assert_eq!(segments.len(), 4);
        assert_eq!(segments[3].start, 768);
        assert_eq!(segments[3].end, 1000);
    }

    #[test]
    fn empty_for_zero_total() {
        assert!(build_segments(0, 1024).is_empty());
    }

    #[test]
    fn shrinks_for_small_files() {
        let size = effective_segment_size(1024 * 1024, 4, 8 * 1024 * 1024);
        assert_eq!(size, 256 * 1024);
    }

    #[test]
    fn keeps_preferred_for_large_files() {
        let size = effective_segment_size(100 * 1024 * 1024, 8, 8 * 1024 * 1024);
        assert_eq!(size, 8 * 1024 * 1024);
    }

    #[test]
    fn never_below_minimum() {
        let size = effective_segment_size(1024, 4, 8 * 1024 * 1024);
        assert_eq!(size, MIN_SEGMENT_SIZE);
    }
}
