// 防止 release 构建弹出控制台窗口，请勿删除。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tug_desktop_lib::run()
}
