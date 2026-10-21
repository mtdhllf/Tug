use std::sync::{Arc, RwLock};

mod notifications;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};
#[cfg(not(windows))]
use tauri_plugin_notification::NotificationExt;
use tug_core::{APP_NAME, APP_VERSION, AppPaths, Settings, TaskState, TaskView, logging};
use tug_service::{AddTaskInput, TaskEvent, TaskManager, TorrentFileInfo, TorrentRuntimeDetails};

struct AppState {
    paths: AppPaths,
    settings: Arc<RwLock<Settings>>,
    manager: Arc<TaskManager>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    name: &'static str,
    version: &'static str,
    platform: &'static str,
    config_dir: String,
    data_dir: String,
    download_dir: String,
}

/// 读取剪贴板中的文件路径（Windows Explorer 复制文件时走 CF_HDROP）。
#[tauri::command]
fn clipboard_files() -> Vec<String> {
    clipboard_file_paths()
}

#[cfg(windows)]
fn clipboard_file_paths() -> Vec<String> {
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    const CF_HDROP: u32 = 15;

    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP).is_err() {
            return Vec::new();
        }
        if OpenClipboard(None).is_err() {
            return Vec::new();
        }

        let mut paths = Vec::new();
        if let Ok(handle) = GetClipboardData(CF_HDROP)
            && !handle.0.is_null()
        {
            let drop = HDROP(handle.0);
            let count = DragQueryFileW(drop, u32::MAX, None);
            for index in 0..count {
                let length = DragQueryFileW(drop, index, None);
                if length == 0 {
                    continue;
                }
                let mut buffer = vec![0u16; length as usize + 1];
                let written = DragQueryFileW(drop, index, Some(&mut buffer));
                if written > 0 {
                    paths.push(String::from_utf16_lossy(&buffer[..written as usize]));
                }
            }
        }
        let _ = CloseClipboard();
        paths
    }
}

#[cfg(not(windows))]
fn clipboard_file_paths() -> Vec<String> {
    Vec::new()
}
/// 前端启动时探活。
#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

/// 让 Windows 原生标题栏/边框颜色跟随应用主题，避免顶部与底部色差。
#[tauri::command]
fn set_window_appearance(window: tauri::Window, dark: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::mem::size_of;
        use windows::Win32::Graphics::Dwm::{
            DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
            DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
        };

        let hwnd = window.hwnd().map_err(|err| err.to_string())?;
        // COLORREF 是 0x00BBGGRR。
        let (caption, border, text) = if dark {
            (0x0021_1a17u32, 0x0036_2b26u32, 0x00f4_ede9u32)
        } else {
            (0x00ff_ffffu32, 0x00ee_eae8u32, 0x001a_1817u32)
        };
        let immersive = u32::from(dark);
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&immersive as *const u32).cast(),
                size_of::<u32>() as u32,
            );
            for (attribute, color) in [
                (DWMWA_CAPTION_COLOR, caption),
                (DWMWA_BORDER_COLOR, border),
                (DWMWA_TEXT_COLOR, text),
            ] {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    attribute,
                    (&color as *const u32).cast(),
                    size_of::<u32>() as u32,
                );
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (window, dark);
    }
    Ok(())
}

/// 返回版本、平台与目录信息。
#[tauri::command]
fn app_info(state: State<'_, AppState>) -> AppInfo {
    let settings = state.settings.read().expect("settings poisoned");
    AppInfo {
        name: APP_NAME,
        version: APP_VERSION,
        platform: std::env::consts::OS,
        config_dir: state.paths.config_dir.display().to_string(),
        data_dir: state.paths.data_dir.display().to_string(),
        download_dir: settings.download_dir.display().to_string(),
    }
}

/// 返回当前设置。
#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.read().expect("settings poisoned").clone()
}

/// 保存设置并触发一次调度（并发上限可能变化）。
#[tauri::command]
async fn update_settings(
    state: State<'_, AppState>,
    mut settings: Settings,
) -> Result<Settings, String> {
    settings.normalize();
    settings.save(&state.paths).map_err(|err| err.to_string())?;
    *state.settings.write().expect("settings poisoned") = settings.clone();
    state.manager.settings_changed().await;
    Ok(settings)
}

/// 列出全部任务。
#[tauri::command]
fn list_tasks(state: State<'_, AppState>) -> Result<Vec<TaskView>, String> {
    state.manager.list().map_err(|err| err.to_string())
}

/// 新建并启动任务。
#[tauri::command]
async fn add_task(state: State<'_, AppState>, input: AddTaskInput) -> Result<TaskView, String> {
    state
        .manager
        .add(input)
        .await
        .map_err(|err| err.to_string())
}

/// 暂停任务。
#[tauri::command]
async fn pause_task(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.manager.pause(id).await.map_err(|err| err.to_string())
}

/// 继续任务。
#[tauri::command]
async fn resume_task(state: State<'_, AppState>, id: i64) -> Result<TaskView, String> {
    state
        .manager
        .resume(id)
        .await
        .map_err(|err| err.to_string())
}

/// 移除任务，可选删除文件。
#[tauri::command]
async fn remove_task(
    state: State<'_, AppState>,
    id: i64,
    delete_files: bool,
) -> Result<(), String> {
    state
        .manager
        .remove(id, delete_files)
        .await
        .map_err(|err| err.to_string())
}

/// 列出 BT 任务文件。
#[tauri::command]
async fn list_torrent_files(
    state: State<'_, AppState>,
    id: i64,
) -> Result<Vec<TorrentFileInfo>, String> {
    state
        .manager
        .torrent_files(id)
        .await
        .map_err(|err| err.to_string())
}

/// 更新 BT 文件选择。
#[tauri::command]
async fn set_torrent_files(
    state: State<'_, AppState>,
    id: i64,
    files: Vec<usize>,
) -> Result<(), String> {
    state
        .manager
        .set_torrent_files(id, files)
        .await
        .map_err(|err| err.to_string())
}

/// BT peer/tracker 详情。
#[tauri::command]
async fn torrent_runtime(
    state: State<'_, AppState>,
    id: i64,
) -> Result<TorrentRuntimeDetails, String> {
    state
        .manager
        .torrent_runtime(id)
        .await
        .map_err(|err| err.to_string())
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn send_notification(handle: &AppHandle, app_id: &str, title: &str, body: &str) {
    #[cfg(windows)]
    {
        let _ = handle;
        let mut notification = notify_rust::Notification::new();
        notification.summary(title).body(body).app_id(app_id);
        if let Err(err) = notification.show() {
            tracing::warn!(%err, "发送系统通知失败");
        }
    }

    #[cfg(not(windows))]
    {
        let _ = app_id;
        if let Err(err) = handle
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            tracing::warn!(%err, "发送系统通知失败");
        }
    }
}

struct TrayLabels {
    show: &'static str,
    pause: &'static str,
    resume: &'static str,
    quit: &'static str,
}

fn tray_labels(locale: &str) -> TrayLabels {
    if locale == "en-US" {
        TrayLabels {
            show: "Show window",
            pause: "Pause all",
            resume: "Resume all",
            quit: "Quit",
        }
    } else {
        TrayLabels {
            show: "显示主窗口",
            pause: "暂停全部",
            resume: "继续全部",
            quit: "退出",
        }
    }
}

fn setup_tray(app: &mut tauri::App, locale: &str) -> tauri::Result<()> {
    let labels = tray_labels(locale);
    let show = MenuItem::with_id(app, "show", labels.show, true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause_all", labels.pause, true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume_all", labels.resume, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", labels.quit, true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &pause, &resume, &separator, &quit])?;

    let mut builder = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "pause_all" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(state) = app.try_state::<AppState>()
                        && let Err(err) = state.manager.pause_all().await
                    {
                        tracing::warn!(%err, "暂停全部任务失败");
                    }
                });
            }
            "resume_all" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(state) = app.try_state::<AppState>()
                        && let Err(err) = state.manager.resume_all().await
                    {
                        tracing::warn!(%err, "继续全部任务失败");
                    }
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let paths = AppPaths::discover().expect("failed to resolve app directories");
    let _log_guard = logging::init(&paths).expect("failed to initialize logging");
    let context = tauri::generate_context!();
    notifications::ensure_identity(&paths.data_dir, context.config().identifier.as_str());
    let notification_app_id = context.config().identifier.clone();

    let settings = Settings::load(&paths).unwrap_or_else(|err| {
        tracing::warn!(%err, "加载设置失败，使用默认值");
        Settings::initial(&paths)
    });
    let settings = Arc::new(RwLock::new(settings));

    let manager = TaskManager::new(&paths, Arc::clone(&settings)).expect("failed to init tasks");
    let manager_for_events = Arc::clone(&manager);

    tracing::info!(
        version = APP_VERSION,
        config_dir = %paths.config_dir.display(),
        "Tug starting"
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState {
            paths,
            settings,
            manager,
        })
        .setup(move |app| {
            let locale = app
                .state::<AppState>()
                .settings
                .read()
                .map(|settings| settings.locale.clone())
                .unwrap_or_else(|_| "zh-CN".to_owned());
            setup_tray(app, &locale)?;

            let handle = app.handle().clone();
            let notification_app_id = notification_app_id.clone();
            let mut receiver = manager_for_events.subscribe();
            tauri::async_runtime::spawn(async move {
                while let Ok(event) = receiver.recv().await {
                    if let TaskEvent::Updated { task } = &event
                        && matches!(task.state, TaskState::Completed | TaskState::Failed)
                    {
                        let focused = handle
                            .get_webview_window("main")
                            .and_then(|window| window.is_focused().ok())
                            .unwrap_or(false);
                        if !focused {
                            let title = if task.state == TaskState::Completed {
                                "下载完成"
                            } else {
                                "下载失败"
                            };
                            tracing::info!(task = %task.name, title, "发送系统通知");
                            send_notification(&handle, &notification_app_id, title, &task.name);
                        }
                    }

                    if let Err(err) = handle.emit("task-event", &event) {
                        tracing::warn!(%err, "发送任务事件失败");
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let close_to_tray = window
                    .app_handle()
                    .try_state::<AppState>()
                    .map(|state| {
                        state
                            .settings
                            .read()
                            .map(|settings| settings.close_to_tray)
                            .unwrap_or(true)
                    })
                    .unwrap_or(true);

                if close_to_tray {
                    api.prevent_close();
                    if let Err(err) = window.hide() {
                        tracing::warn!(%err, "隐藏主窗口失败");
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            set_window_appearance,
            clipboard_files,
            app_info,
            get_settings,
            update_settings,
            list_tasks,
            add_task,
            pause_task,
            resume_task,
            remove_task,
            list_torrent_files,
            set_torrent_files,
            torrent_runtime
        ])
        .run(context)
        .expect("error while running Tauri application");
}
