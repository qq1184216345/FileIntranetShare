use parking_lot::Mutex;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};
use tauri_plugin_opener::OpenerExt;

/// 托盘运行时状态：保留菜单项 handle 以便动态更新文案
pub struct TrayState {
    pub status_item: MenuItem<Wry>,
    pub open_browser_item: MenuItem<Wry>,
    pub current_url: Mutex<String>,
}

/// 在 App 启动阶段调用，创建托盘图标与菜单
pub fn build(app: &AppHandle<Wry>) -> tauri::Result<()> {
    let status_item = MenuItem::with_id(app, "status", "○ 服务已停止", false, None::<&str>)?;
    let open_browser_item =
        MenuItem::with_id(app, "open_browser", "打开分享页", false, None::<&str>)?;
    let show_item = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "完全退出", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(
        app,
        &[
            &status_item,
            &open_browser_item,
            &sep1,
            &show_item,
            &sep2,
            &quit_item,
        ],
    )?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::WindowNotFound)?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("FileShare - 未开启")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quit" => app.exit(0),
            "open_browser" => {
                if let Some(state) = app.try_state::<TrayState>() {
                    let url = state.current_url.lock().clone();
                    if !url.is_empty() {
                        let _ = app.opener().open_url(&url, None::<&str>);
                    }
                }
            }
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
        })
        .build(app)?;

    app.manage(TrayState {
        status_item,
        open_browser_item,
        current_url: Mutex::new(String::new()),
    });

    Ok(())
}

fn show_main(app: &AppHandle<Wry>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 业务层调用：根据服务运行状态刷新菜单文案 / tooltip
pub fn update_status(app: &AppHandle<Wry>, running: bool, share_url: String) {
    if let Some(state) = app.try_state::<TrayState>() {
        if running {
            let _ = state.status_item.set_text("● 正在分享");
            let label = if share_url.is_empty() {
                "打开分享页".to_string()
            } else {
                format!("打开分享页 ({})", truncate_mid(&share_url, 40))
            };
            let _ = state.open_browser_item.set_text(&label);
            let _ = state.open_browser_item.set_enabled(!share_url.is_empty());
        } else {
            let _ = state.status_item.set_text("○ 服务已停止");
            let _ = state.open_browser_item.set_text("打开分享页");
            let _ = state.open_browser_item.set_enabled(false);
        }
        *state.current_url.lock() = share_url;

        if let Some(tray) = app.tray_by_id("main") {
            let tip = if running {
                "FileShare - 正在分享"
            } else {
                "FileShare - 未开启"
            };
            let _ = tray.set_tooltip(Some(tip));
        }
    }
}

fn truncate_mid(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let half = max / 2;
    let head: String = s.chars().take(half).collect();
    let tail_rev: Vec<char> = s.chars().rev().take(half).collect();
    let tail: String = tail_rev.into_iter().rev().collect();
    format!("{head}...{tail}")
}
