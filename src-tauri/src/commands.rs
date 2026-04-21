use crate::config::AppConfig;
use crate::net::{self, NetworkInterface};
use crate::server;
use crate::server::files::{FileItem, TextItem};
use crate::server::state::{now_secs, SyncEvent};
use crate::tray;
use crate::ServerSlot;
use nanoid::nanoid;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub running: bool,
    pub port: u16,
    pub bind_ipv6: bool,
    pub owner_token: String,
    pub started_at: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthRefreshResult {
    /// JWT 签名密钥是否已轮换；true 表示所有在线访客需重新登录
    pub rotated: bool,
    /// 当前密码是否生效
    pub password_required: bool,
}

#[tauri::command]
pub fn get_network_interfaces() -> Vec<NetworkInterface> {
    net::list_interfaces()
}

#[tauri::command]
pub async fn start_server(
    app: AppHandle,
    slot: State<'_, ServerSlot>,
    config: AppConfig,
) -> Result<ServerStatus, String> {
    {
        let guard = slot.lock();
        if guard.is_some() {
            return Err("服务已在运行".into());
        }
    }

    // 数据库放在 app_data_dir 下，确保用户换 uploadDir 时记录连续
    let db_path = app
        .path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join("fileshare.db"));

    let handle = server::start(config.clone(), db_path)
        .await
        .map_err(|e| format!("启动失败: {e:#}"))?;

    let status = ServerStatus {
        running: true,
        port: handle.addr.port(),
        bind_ipv6: handle.addr.is_ipv6(),
        owner_token: handle.owner_token.clone(),
        started_at: handle.state.started_at,
    };

    slot.lock().replace(handle);
    Ok(status)
}

#[tauri::command]
pub async fn stop_server(slot: State<'_, ServerSlot>) -> Result<(), String> {
    let handle = slot.lock().take();
    if let Some(h) = handle {
        h.shutdown().await;
    }
    Ok(())
}

#[tauri::command]
pub fn get_server_status(slot: State<'_, ServerSlot>) -> ServerStatus {
    let guard = slot.lock();
    match guard.as_ref() {
        Some(h) => ServerStatus {
            running: true,
            port: h.addr.port(),
            bind_ipv6: h.addr.is_ipv6(),
            owner_token: h.owner_token.clone(),
            started_at: h.state.started_at,
        },
        None => ServerStatus::default(),
    }
}

#[tauri::command]
pub fn set_auto_start(app: AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())?;
    } else {
        manager.disable().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 前端在服务启停/URL 变化时调用，驱动托盘刷新
#[tauri::command]
pub fn update_tray_status(app: AppHandle, running: bool, share_url: String) {
    tray::update_status(&app, running, share_url);
}

/// 显示并聚焦主窗口（可被托盘菜单或前端调用）
#[tauri::command]
pub fn show_main_window(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("main") {
        w.show().map_err(|e| e.to_string())?;
        let _ = w.unminimize();
        w.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 真正退出应用（绕过"关闭即隐藏"的窗口事件拦截）
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// 将最新 AppConfig 热应用到运行中的服务，并重新计算 password hash。
/// 密码或启用状态变化时会轮换 jwt_secret，所有在线访客被踢下线。
/// 注意：port / bind_ipv6 属于启动时字段，热更新不会影响绑定，需要重启服务才生效。
#[tauri::command]
pub fn refresh_server_auth(
    slot: State<'_, ServerSlot>,
    config: AppConfig,
) -> Result<AuthRefreshResult, String> {
    let guard = slot.lock();
    let handle = guard.as_ref().ok_or_else(|| "服务未运行".to_string())?;
    handle.state.update_config(config);
    let rotated = handle.state.reload_auth();
    Ok(AuthRefreshResult {
        rotated,
        password_required: handle.state.password_required(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareClipboardResult {
    /// "file" | "text"
    pub kind: String,
    pub id: String,
    /// file：保存后的文件名；text：内容摘要（最多 80 字符）
    pub name: String,
    /// file：字节数；text：原始长度
    pub size: u64,
}

/// 读取 Host 剪贴板，把图片/文本注入到分享列表。
/// 优先级：图片 > 文本；两者都没有时返回错误。
#[tauri::command]
pub fn share_clipboard(
    app: AppHandle,
    slot: State<'_, ServerSlot>,
) -> Result<ShareClipboardResult, String> {
    let state = {
        let guard = slot.lock();
        guard
            .as_ref()
            .map(|h| h.state.clone())
            .ok_or_else(|| "服务未运行".to_string())?
    };

    let clipboard = app.clipboard();

    // 1) 先尝试图片
    if let Ok(img) = clipboard.read_image() {
        let width = img.width();
        let height = img.height();
        let rgba = img.rgba().to_vec();
        if width > 0 && height > 0 && !rgba.is_empty() {
            return save_clipboard_image(&state, width, height, rgba);
        }
    }

    // 2) 再尝试文本
    if let Ok(text) = clipboard.read_text() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return save_clipboard_text(&state, text);
        }
    }

    Err("剪贴板为空或格式不支持".to_string())
}

fn save_clipboard_image(
    state: &std::sync::Arc<server::state::AppState>,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
) -> Result<ShareClipboardResult, String> {
    let upload_dir: PathBuf = state.config.read().upload_dir.clone();
    if upload_dir.as_os_str().is_empty() {
        return Err("uploadDir 未配置".into());
    }

    let id = nanoid!(16);
    let file_dir = upload_dir.join(&id);
    std::fs::create_dir_all(&file_dir).map_err(|e| format!("mkdir: {e}"))?;

    // 文件名用毫秒时间戳，保证唯一且可读；格式化成日期字符串需要 chrono，这里先用毫秒。
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = format!("clipboard-{ms}.png");
    let file_path = file_dir.join(&name);

    let buffer = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "剪贴板图片尺寸与数据不匹配".to_string())?;
    buffer
        .save_with_format(&file_path, image::ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败: {e}"))?;

    let size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

    let item = FileItem {
        id: id.clone(),
        name: name.clone(),
        size,
        mime: "image/png".into(),
        uploader_ip: "host".into(),
        created_at: now_secs(),
        path: file_path,
    };
    state.registry.add_file(item.clone());
    state.broadcast(SyncEvent::FileAdded { file: item });

    Ok(ShareClipboardResult {
        kind: "file".into(),
        id,
        name,
        size,
    })
}

fn save_clipboard_text(
    state: &std::sync::Arc<server::state::AppState>,
    content: String,
) -> Result<ShareClipboardResult, String> {
    const MAX_TEXT: usize = 1024 * 1024;
    if content.len() > MAX_TEXT {
        return Err("剪贴板文本过大（>1MB）".into());
    }
    let id = nanoid!(12);
    let preview: String = content.chars().take(80).collect();
    let size = content.len() as u64;

    let item = TextItem {
        id: id.clone(),
        content,
        uploader_ip: "host".into(),
        created_at: now_secs(),
    };
    state.registry.add_text(item.clone());
    state.broadcast(SyncEvent::TextAdded { text: item });

    Ok(ShareClipboardResult {
        kind: "text".into(),
        id,
        name: preview,
        size,
    })
}
