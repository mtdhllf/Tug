use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::Url;
use reqwest::header::{CONTENT_DISPOSITION, HeaderMap};

/// Windows 保留名（不区分大小写，带扩展名同样保留）。
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 清洗文件名：去掉 Windows 非法字符、保留名、结尾点/空格，并限制长度。
pub fn sanitize_file_name(input: &str) -> String {
    let mut name: String = input
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                '_'
            } else {
                ch
            }
        })
        .collect();

    name = name.trim().trim_end_matches([' ', '.']).to_owned();
    if name.is_empty() || name == "." || name == ".." {
        return "download".to_owned();
    }

    if name.chars().count() > 180 {
        name = name.chars().take(180).collect();
        name = name.trim_end_matches([' ', '.']).to_owned();
    }

    let path = Path::new(&name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(name.as_str())
        .to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        name = format!("_{name}");
    }

    name
}

/// 从 `Content-Disposition` 解析文件名，优先 `filename*`（RFC 5987）。
pub fn parse_content_disposition(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(CONTENT_DISPOSITION)?.to_str().ok()?;

    for part in value.split(';') {
        let part = part.trim();
        let lowered = part.to_ascii_lowercase();
        if lowered.starts_with("filename*=") {
            let raw = part[10..].trim().trim_matches('"');
            let encoded = raw.split_once("''").map(|(_, value)| value).unwrap_or(raw);
            let decoded = percent_decode(encoded);
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }

    for part in value.split(';') {
        let part = part.trim();
        let lowered = part.to_ascii_lowercase();
        if lowered.starts_with("filename=") {
            let decoded = part[9..].trim().trim_matches('"').to_owned();
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }

    None
}

/// 从 URL 猜测文件名（仅用于任务展示，最终以服务器 Content-Disposition 为准）。
pub fn guess_file_name(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    file_name_from_url(&parsed)
}

/// 从 URL 路径取最后一段并做百分号解码。
pub fn file_name_from_url(url: &Url) -> Option<String> {
    let last = url.path_segments()?.next_back()?;
    if last.is_empty() {
        return None;
    }
    let decoded = percent_decode(last);
    if decoded.is_empty() {
        None
    } else {
        Some(decoded)
    }
}

/// 在文件名后追加后缀（不是替换扩展名）：`foo.iso` → `foo.iso.part`。
pub fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// 若路径已存在，生成 `name (1).ext` 形式的新路径。
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let extension = path.extension().and_then(|value| value.to_str());

    for index in 1..=999 {
        let candidate = match extension {
            Some(ext) => parent.join(format!("{stem} ({index}).{ext}")),
            None => parent.join(format!("{stem} ({index})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    match extension {
        Some(ext) => parent.join(format!("{stem}-{stamp}.{ext}")),
        None => parent.join(format!("{stem}-{stamp}")),
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            output.push(high * 16 + low);
            index += 3;
            continue;
        }
        output.push(bytes[index]);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_windows_invalid_characters() {
        assert_eq!(
            sanitize_file_name("a<b>c:d\"e/f\\g|h?i*j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
    }

    #[test]
    fn keeps_chinese_and_spaces() {
        assert_eq!(sanitize_file_name("测试 文件.iso"), "测试 文件.iso");
    }

    #[test]
    fn handles_reserved_names() {
        assert_eq!(sanitize_file_name("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_file_name("nul"), "_nul");
    }

    #[test]
    fn trims_trailing_dots_and_spaces() {
        assert_eq!(sanitize_file_name("name.  "), "name");
    }

    #[test]
    fn falls_back_for_empty_names() {
        assert_eq!(sanitize_file_name("   "), "download");
        assert_eq!(sanitize_file_name(".."), "download");
    }

    #[test]
    fn parses_rfc5987_filename() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_DISPOSITION,
            "attachment; filename=\"fallback.bin\"; filename*=UTF-8''%E4%B8%AD%E6%96%87.iso"
                .parse()
                .expect("header"),
        );
        assert_eq!(
            parse_content_disposition(&headers).as_deref(),
            Some("中文.iso")
        );
    }

    #[test]
    fn parses_plain_filename() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_DISPOSITION,
            "attachment; filename=\"plain.bin\""
                .parse()
                .expect("header"),
        );
        assert_eq!(
            parse_content_disposition(&headers).as_deref(),
            Some("plain.bin")
        );
    }

    #[test]
    fn extracts_name_from_url() {
        let url = Url::parse("https://example.com/files/%E6%B5%8B%E8%AF%95.iso?x=1").expect("url");
        assert_eq!(file_name_from_url(&url).as_deref(), Some("测试.iso"));
    }

    #[test]
    fn unique_path_adds_counter() {
        let root = std::env::temp_dir().join(format!("tug-name-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("tmp");
        let first = root.join("file.bin");
        std::fs::write(&first, b"x").expect("write");
        let second = unique_path(&first);
        assert_eq!(
            second.file_name().and_then(|v| v.to_str()),
            Some("file (1).bin")
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn sidecar_appends_suffix() {
        let path = Path::new("C:/downloads/foo.iso");
        assert_eq!(
            sidecar_path(path, ".part"),
            PathBuf::from("C:/downloads/foo.iso.part")
        );
    }
}
