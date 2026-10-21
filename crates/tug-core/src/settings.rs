use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Result, TugError};
use crate::paths::AppPaths;

/// 界面主题。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// 跟随系统。
    #[default]
    System,
    /// 浅色。
    Light,
    /// 深色。
    Dark,
}

/// 用户设置。字段保持向后兼容：新增字段必须有默认值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 默认下载目录。
    pub download_dir: PathBuf,
    /// 单任务 HTTP 连接数（1–32）。
    pub connections_per_task: u8,
    /// 同时下载的任务数（1–16）。
    pub max_concurrent_tasks: u8,
    /// 全局下载限速（KiB/s），`None` 表示不限速。
    pub global_download_limit_kib: Option<u32>,
    /// 旧版手动 HTTP 代理；仅保留兼容，界面已改为系统代理开关。
    pub proxy: Option<String>,
    /// 是否使用 Windows 系统代理。
    #[serde(default = "default_use_system_proxy")]
    pub use_system_proxy: bool,
    /// 是否监视剪贴板中的下载链接。
    pub clipboard_watch: bool,
    /// 关闭窗口时是否隐藏到托盘。
    pub close_to_tray: bool,
    /// 界面主题。
    pub theme: Theme,
    /// 界面语言；当前仅 `zh-CN`，预留 `en-US`。
    pub locale: String,
    /// 是否启用 DHT。
    pub bt_dht: bool,
    /// BT 监听端口；`None` 表示随机端口。
    pub bt_listen_port: Option<u16>,
    /// 是否启用 UPnP 端口映射。
    pub bt_upnp: bool,
    /// BT 上传限速（KiB/s）；`None` 表示不限速。
    pub bt_upload_limit_kib: Option<u32>,
    /// 下载完成后是否继续共享（上传）。
    pub bt_seed_after_complete: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: PathBuf::new(),
            connections_per_task: 8,
            max_concurrent_tasks: 3,
            global_download_limit_kib: None,
            proxy: None,
            use_system_proxy: true,
            clipboard_watch: false,
            close_to_tray: true,
            theme: Theme::System,
            locale: "zh-CN".to_owned(),
            bt_dht: true,
            bt_listen_port: None,
            bt_upnp: true,
            bt_upload_limit_kib: None,
            bt_seed_after_complete: true,
        }
    }
}

fn default_use_system_proxy() -> bool {
    true
}

impl Settings {
    /// 首次启动时的默认设置。
    pub fn initial(paths: &AppPaths) -> Self {
        Self {
            download_dir: paths.default_download_dir.clone(),
            ..Self::default()
        }
    }

    /// 从磁盘加载；文件不存在时写入默认设置。
    pub fn load(paths: &AppPaths) -> Result<Self> {
        let file = paths.settings_file();
        if !file.exists() {
            let settings = Self::initial(paths);
            settings.save(paths)?;
            return Ok(settings);
        }

        let text = std::fs::read_to_string(&file)?;
        let mut settings: Self = serde_json::from_str(&text)
            .map_err(|err| TugError::Config(format!("解析 {} 失败: {err}", file.display())))?;

        if settings.download_dir.as_os_str().is_empty() {
            settings.download_dir = paths.default_download_dir.clone();
        }
        settings.normalize();
        Ok(settings)
    }

    /// 保存到磁盘（写入前会规范化取值）。
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        std::fs::create_dir_all(&paths.config_dir)?;
        let mut settings = self.clone();
        settings.normalize();
        let text = serde_json::to_string_pretty(&settings)
            .map_err(|err| TugError::Config(format!("序列化设置失败: {err}")))?;
        std::fs::write(paths.settings_file(), text)?;
        Ok(())
    }

    /// 把越界值收敛到允许范围，避免旧配置或手改配置导致异常。
    pub fn normalize(&mut self) {
        self.connections_per_task = self.connections_per_task.clamp(1, 32);
        self.max_concurrent_tasks = self.max_concurrent_tasks.clamp(1, 16);
        if self.global_download_limit_kib == Some(0) {
            self.global_download_limit_kib = None;
        }
        if let Some(proxy) = self.proxy.as_mut() {
            *proxy = proxy.trim().to_owned();
            if proxy.is_empty() {
                self.proxy = None;
            }
        }
        if !matches!(self.locale.as_str(), "zh-CN" | "en-US") {
            self.locale = "zh-CN".to_owned();
        }
        if self.bt_listen_port == Some(0) {
            self.bt_listen_port = None;
        }
        if self.bt_upload_limit_kib == Some(0) {
            self.bt_upload_limit_kib = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(name: &str) -> AppPaths {
        let root = std::env::temp_dir().join(format!(
            "tug-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let paths = AppPaths::under_root(root);
        paths.ensure().expect("ensure dirs");
        paths
    }

    #[test]
    fn first_load_writes_defaults() {
        let paths = temp_paths("settings-default");
        let settings = Settings::load(&paths).expect("load settings");

        assert_eq!(settings.connections_per_task, 8);
        assert_eq!(settings.locale, "zh-CN");
        assert!(settings.bt_dht, "DHT should default to enabled");
        assert!(settings.bt_upnp, "UPnP should default to enabled");
        assert!(
            settings.use_system_proxy,
            "system proxy should default to enabled"
        );
        assert_eq!(settings.download_dir, paths.default_download_dir);
        assert!(paths.settings_file().is_file());

        std::fs::remove_dir_all(&paths.config_dir).ok();
        std::fs::remove_dir_all(&paths.data_dir).ok();
        std::fs::remove_dir_all(&paths.default_download_dir).ok();
    }

    #[test]
    fn roundtrip_preserves_values() {
        let paths = temp_paths("settings-roundtrip");
        let mut settings = Settings::initial(&paths);
        settings.connections_per_task = 16;
        settings.max_concurrent_tasks = 5;
        settings.theme = Theme::Dark;
        settings.global_download_limit_kib = Some(4096);
        settings.proxy = Some("http://127.0.0.1:7890".into());
        settings.clipboard_watch = true;
        settings.close_to_tray = false;
        settings.save(&paths).expect("save");

        let loaded = Settings::load(&paths).expect("load");
        assert_eq!(loaded, settings);

        std::fs::remove_dir_all(&paths.config_dir).ok();
        std::fs::remove_dir_all(&paths.data_dir).ok();
        std::fs::remove_dir_all(&paths.default_download_dir).ok();
    }

    #[test]
    fn missing_use_system_proxy_defaults_to_true() {
        let paths = temp_paths("settings-proxy-default");
        let mut value = serde_json::to_value(Settings::initial(&paths)).expect("serialize");
        value
            .as_object_mut()
            .expect("settings object")
            .remove("useSystemProxy");

        let loaded: Settings = serde_json::from_value(value).expect("deserialize legacy");
        assert!(loaded.use_system_proxy);

        std::fs::remove_dir_all(&paths.config_dir).ok();
        std::fs::remove_dir_all(&paths.data_dir).ok();
        std::fs::remove_dir_all(&paths.default_download_dir).ok();
    }

    #[test]
    fn normalize_clamps_out_of_range_values() {
        let mut settings = Settings {
            connections_per_task: 0,
            max_concurrent_tasks: 99,
            locale: "fr-FR".to_owned(),
            ..Settings::default()
        };
        settings.normalize();

        assert_eq!(settings.connections_per_task, 1);
        assert_eq!(settings.max_concurrent_tasks, 16);
        assert_eq!(settings.locale, "zh-CN");
    }
}
