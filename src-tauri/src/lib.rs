mod commands;
mod config;
mod net;
mod server;
mod tray;

use parking_lot::Mutex;
use std::sync::Arc;
use tauri::WindowEvent;
use tauri_plugin_autostart::MacosLauncher;

/// 全局托管的服务器状态（None 表示未启动）
pub type ServerSlot = Arc<Mutex<Option<server::ServerHandle>>>;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,fileshare=debug,fileshare_lib=debug".into()),
        )
        .init();

    let server_slot: ServerSlot = Arc::new(Mutex::new(None));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(server_slot)
        .setup(|app| {
            // 创建系统托盘（仅桌面端，移动端 tray-icon 不可用；Tauri 会跳过）
            #[cfg(all(desktop, not(target_os = "android"), not(target_os = "ios")))]
            {
                if let Err(e) = tray::build(app.handle()) {
                    tracing::warn!("build tray failed: {e}");
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关闭主窗口时，改为隐藏到托盘；真正退出需走托盘菜单 "完全退出" 或 quit_app
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_network_interfaces,
            commands::start_server,
            commands::stop_server,
            commands::get_server_status,
            commands::set_auto_start,
            commands::update_tray_status,
            commands::show_main_window,
            commands::quit_app,
            commands::refresh_server_auth,
            commands::share_clipboard,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
