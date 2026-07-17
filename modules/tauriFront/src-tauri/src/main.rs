// 防止 Windows 上 release 构建弹出控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    effisuite_app_lib::run()
}
