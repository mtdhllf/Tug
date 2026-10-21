//! Windows 通知身份注册。
//!
//! Windows 的 toast 归属由 AppUserModelID（AUMID）决定。未安装开始菜单快捷方式时，
//! 系统会把通知显示成发起进程（通常是 Windows PowerShell）。这里在启动时补齐：
//! - `HKCU\Software\Classes\AppUserModelId\<id>` 的显示名与图标；
//! - 带 `System.AppUserModel.ID` 的开始菜单快捷方式；
//! - 当前进程的显式 AUMID。

#[cfg(windows)]
pub fn ensure_identity(data_dir: &std::path::Path, app_id: &str) {
    if let Err(err) = ensure_identity_inner(data_dir, app_id) {
        tracing::warn!(%err, "注册 Windows 通知身份失败");
    }
}

#[cfg(not(windows))]
pub fn ensure_identity(_data_dir: &std::path::Path, _app_id: &str) {}

#[cfg(windows)]
fn ensure_identity_inner(data_dir: &std::path::Path, app_id: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0_0};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemAlloc, IPersistFile,
    };
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{
        IShellLinkW, SetCurrentProcessExplicitAppUserModelID, ShellLink,
    };
    use windows::core::{Interface, PCWSTR, PWSTR};

    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    std::fs::create_dir_all(data_dir).map_err(|err| err.to_string())?;
    let icon_path = data_dir.join("notification-icon.png");
    std::fs::write(&icon_path, include_bytes!("../icons/128x128.png"))
        .map_err(|err| err.to_string())?;
    let icon_uri = icon_path.to_string_lossy().to_string();

    let key = windows_registry::CURRENT_USER
        .create(format!(r"SOFTWARE\Classes\AppUserModelId\{app_id}"))
        .map_err(|err| err.to_string())?;
    key.set_string("DisplayName", "Tug")
        .map_err(|err| err.to_string())?;
    key.set_string("IconUri", &icon_uri)
        .map_err(|err| err.to_string())?;
    key.set_string("IconBackgroundColor", "0")
        .map_err(|err| err.to_string())?;

    let executable = std::env::current_exe().map_err(|err| err.to_string())?;
    let executable_wide = wide(executable.as_os_str());

    let appdata = std::env::var_os("APPDATA").ok_or_else(|| "缺少 APPDATA 环境变量".to_owned())?;
    let programs = PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs");
    std::fs::create_dir_all(&programs).map_err(|err| err.to_string())?;
    let shortcut = programs.join("Tug.lnk");
    let shortcut_wide = wide(shortcut.as_os_str());

    let app_id_wide: Vec<u16> = app_id.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|err| err.to_string())?;
        link.SetPath(PCWSTR(executable_wide.as_ptr()))
            .map_err(|err| err.to_string())?;
        link.SetIconLocation(PCWSTR(executable_wide.as_ptr()), 0)
            .map_err(|err| err.to_string())?;

        let store: IPropertyStore = link.cast().map_err(|err| err.to_string())?;

        // Shell 属性存储会接管这个字符串的所有权；不能用 Rust Vec 的指针。
        let utf16: Vec<u16> = app_id.encode_utf16().chain(std::iter::once(0)).collect();
        let memory = CoTaskMemAlloc(utf16.len() * 2);
        if memory.is_null() {
            return Err("分配 AUMID 字符串失败".to_owned());
        }
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), memory as *mut u16, utf16.len());

        let mut property = PROPVARIANT::default();
        let mut value = PROPVARIANT_0_0 {
            vt: VT_LPWSTR,
            ..Default::default()
        };
        value.Anonymous.pwszVal = PWSTR(memory as *mut u16);
        property.Anonymous.Anonymous = std::mem::ManuallyDrop::new(value);
        store
            .SetValue(&PKEY_AppUserModel_ID, &property)
            .map_err(|err| err.to_string())?;
        store.Commit().map_err(|err| err.to_string())?;

        let file: IPersistFile = link.cast().map_err(|err| err.to_string())?;
        file.Save(PCWSTR(shortcut_wide.as_ptr()), true)
            .map_err(|err| err.to_string())?;

        let _ = SetCurrentProcessExplicitAppUserModelID(PCWSTR(app_id_wide.as_ptr()));
    }

    Ok(())
}
