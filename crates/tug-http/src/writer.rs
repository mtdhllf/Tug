use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use crate::error::Result;

/// 以定位写方式写入同一个文件，多个 worker 可共享。
#[derive(Debug)]
pub struct FileWriter {
    file: File,
}

impl FileWriter {
    /// 新建/截断数据文件并预分配长度。
    pub fn create(path: &Path, total: u64) -> Result<Self> {
        ensure_parent(path)?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(io_path(path))?;
        if total > 0 {
            file.set_len(total)?;
        }
        Ok(Self { file })
    }

    /// 打开已有数据文件用于续传；文件较短时扩展，较长时截断。
    pub fn open(path: &Path, total: u64) -> Result<Self> {
        ensure_parent(path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(io_path(path))?;
        file.set_len(total)?;
        Ok(Self { file })
    }

    /// 在指定偏移写入全部字节。
    pub fn write_at(&self, offset: u64, buffer: &[u8]) -> Result<()> {
        if buffer.is_empty() {
            return Ok(());
        }

        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;

            let mut written = 0usize;
            while written < buffer.len() {
                let count = self
                    .file
                    .seek_write(&buffer[written..], offset + written as u64)?;
                if count == 0 {
                    return Err(io::Error::new(io::ErrorKind::WriteZero, "写入 0 字节").into());
                }
                written += count;
            }
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;

            let mut written = 0usize;
            while written < buffer.len() {
                let count = self
                    .file
                    .write_at(&buffer[written..], offset + written as u64)?;
                if count == 0 {
                    return Err(io::Error::new(io::ErrorKind::WriteZero, "写入 0 字节").into());
                }
                written += count;
            }
        }

        Ok(())
    }

    /// 落盘，完成前调用一次。
    pub fn sync_all(&self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }

    /// 只同步文件数据（不含元数据），用于分片完成时快速持久化。
    pub fn sync_data(&self) -> Result<()> {
        self.file.sync_data()?;
        Ok(())
    }
}

/// 创建父目录（幂等）。
pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(io_path(parent))?;
    }
    Ok(())
}

/// Windows 下超长路径加 `\\?\` 前缀，避免 260 字符限制。
pub fn io_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.as_os_str().to_string_lossy();
        if path.is_absolute() && text.len() >= 240 && !text.starts_with(r"\\?\") {
            return PathBuf::from(format!(r"\\?\{text}"));
        }
    }
    path.to_path_buf()
}

/// 输出目录所在卷的可用字节数；无法获取时返回 `None`。
#[cfg(windows)]
pub fn available_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::core::PCWSTR;

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free = 0u64;
    let result = unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide.as_ptr()),
            Some(&mut free as *mut u64),
            None,
            None,
        )
    };
    result.ok().map(|()| free)
}

/// 非 Windows 平台暂不检查磁盘空间。
#[cfg(not(windows))]
pub fn available_space(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_at_offsets() {
        let root = std::env::temp_dir().join(format!("tug-writer-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("tmp");
        let path = root.join("data.bin");

        let writer = FileWriter::create(&path, 8).expect("create");
        writer.write_at(4, b"efgh").expect("write tail");
        writer.write_at(0, b"abcd").expect("write head");
        writer.sync_all().expect("sync");

        assert_eq!(std::fs::read(&path).expect("read"), b"abcdefgh");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn extends_existing_file_for_resume() {
        let root = std::env::temp_dir().join(format!("tug-writer-open-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("tmp");
        let path = root.join("data.bin");
        std::fs::write(&path, b"abc").expect("seed");

        let writer = FileWriter::open(&path, 6).expect("open");
        writer.write_at(3, b"def").expect("write");
        writer.sync_all().expect("sync");

        assert_eq!(std::fs::read(&path).expect("read"), b"abcdef");
        std::fs::remove_dir_all(root).ok();
    }

    #[cfg(windows)]
    #[test]
    fn prefixes_long_windows_paths() {
        let path = PathBuf::from(format!("D:\\{}", "a".repeat(260)));
        let prefixed = io_path(&path);
        assert!(prefixed.as_os_str().to_string_lossy().starts_with(r"\\?\"));
    }
}
