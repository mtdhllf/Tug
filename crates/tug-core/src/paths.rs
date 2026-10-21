use std::env;
use std::path::{Path, PathBuf};

use crate::error::{Result, TugError};

/// 应用运行所需的目录集合。
///
/// Windows 下默认布局：
/// - 配置：`%APPDATA%\Tug`
/// - 数据/日志：`%LOCALAPPDATA%\Tug`、`%LOCALAPPDATA%\Tug\logs`
/// - 默认下载目录：`%USERPROFILE%\Downloads\Tug`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    /// 配置文件目录。
    pub config_dir: PathBuf,
    /// 本地数据目录（续传状态、BT 会话等）。
    pub data_dir: PathBuf,
    /// 日志目录。
    pub log_dir: PathBuf,
    /// 默认下载目录。
    pub default_download_dir: PathBuf,
}

impl AppPaths {
    /// 根据当前系统环境解析目录，并确保目录存在。
    pub fn discover() -> Result<Self> {
        let config_root = env_dir("APPDATA")
            .or_else(|| env_dir("XDG_CONFIG_HOME"))
            .ok_or(TugError::MissingDir("APPDATA"))?;
        let data_root = env_dir("LOCALAPPDATA")
            .or_else(|| env_dir("XDG_DATA_HOME"))
            .unwrap_or_else(|| config_root.clone());
        let home = env_dir("USERPROFILE")
            .or_else(|| env_dir("HOME"))
            .ok_or(TugError::MissingDir("USERPROFILE"))?;

        let paths = Self::from_roots(config_root, data_root, home);
        paths.ensure()?;
        Ok(paths)
    }

    /// 以给定根目录构造一套测试/沙盒目录，不读取系统环境。
    pub fn under_root(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            log_dir: root.join("data").join("logs"),
            default_download_dir: root.join("downloads"),
        }
    }

    /// 创建所有必需目录（幂等）。
    pub fn ensure(&self) -> Result<()> {
        for dir in [
            &self.config_dir,
            &self.data_dir,
            &self.log_dir,
            &self.default_download_dir,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// 设置文件路径。
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    fn from_roots(config_root: PathBuf, data_root: PathBuf, home: PathBuf) -> Self {
        Self {
            config_dir: config_root.join(crate::APP_NAME),
            data_dir: data_root.join(crate::APP_NAME),
            log_dir: data_root.join(crate::APP_NAME).join("logs"),
            default_download_dir: home.join("Downloads").join(crate::APP_NAME),
        }
    }
}

fn env_dir(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = env::temp_dir().join(format!(
            "tug-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        root
    }

    #[test]
    fn under_root_creates_expected_layout() {
        let root = temp_root("paths");
        let paths = AppPaths::under_root(&root);
        paths.ensure().expect("ensure dirs");

        assert!(paths.config_dir.is_dir());
        assert!(paths.data_dir.is_dir());
        assert!(paths.log_dir.is_dir());
        assert!(paths.default_download_dir.is_dir());
        assert_eq!(
            paths.settings_file(),
            paths.config_dir.join("settings.json")
        );

        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn discover_provides_non_empty_download_dir() {
        // Windows 上 APPDATA/LOCALAPPDATA/USERPROFILE 一定存在。
        let paths = AppPaths::discover().expect("discover paths");
        assert!(!paths.default_download_dir.as_os_str().is_empty());
        assert!(paths.default_download_dir.ends_with(crate::APP_NAME));
    }
}
