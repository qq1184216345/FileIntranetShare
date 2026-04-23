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

/// 当前进程是否由系统"开机自启"拉起（而非用户手动打开）。
/// 对应 lib.rs 中 `tauri_plugin_autostart::init` 注入的专用命令行参数。
#[tauri::command]
pub fn is_launched_by_autostart() -> bool {
    std::env::args().any(|a| a == "--flag-from-autostart")
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
    let arc = state.registry.add_file(item);
    state.broadcast(SyncEvent::FileAdded { file: arc });

    Ok(ShareClipboardResult {
        kind: "file".into(),
        id,
        name,
        size,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareLocalItem {
    pub id: String,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareLocalSkipped {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareLocalResult {
    pub added: Vec<ShareLocalItem>,
    pub skipped: Vec<ShareLocalSkipped>,
}

/// 在系统文件管理器中定位分享列表里某条文件记录的真实路径。
/// 直接根据 FileItem.path 打开——对零拷贝引用的本机文件也能正确定位，
/// 不再依赖 "uploadDir/id/name" 的拼接约定。
#[tauri::command]
pub fn reveal_shared_file(slot: State<'_, ServerSlot>, id: String) -> Result<(), String> {
    let state = {
        let guard = slot.lock();
        guard
            .as_ref()
            .map(|h| h.state.clone())
            .ok_or_else(|| "服务未运行".to_string())?
    };
    let item = state
        .registry
        .get_file(&id)
        .ok_or_else(|| "记录不存在".to_string())?;
    if !item.path.exists() {
        return Err("文件已被移动或删除".into());
    }
    tauri_plugin_opener::reveal_item_in_dir(&item.path).map_err(|e| format!("打开失败: {e}"))
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OrphanItem {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CleanupOrphansResult {
    /// 孤儿文件清单（含子目录下实际物理文件）
    pub items: Vec<OrphanItem>,
    /// 总字节数（便于 UI 显示"可回收 XX MB"）
    pub total_size: u64,
    /// dryRun=false 时本次是否真的执行了删除
    pub removed: bool,
}

/// 用户显式点击"修复防火墙规则"时调用；唯一会拉 UAC 的入口。
/// 默认端口从 ServerSlot 读取（未启动时返回 18888）。
#[tauri::command]
pub fn repair_firewall_rule(slot: State<'_, ServerSlot>) -> Result<(), String> {
    let port: u16 = {
        let guard = slot.lock();
        guard.as_ref().map(|h| h.addr.port()).unwrap_or(18888)
    };
    #[cfg(windows)]
    {
        crate::server::firewall::try_add_rule_elevated(port)
    }
    #[cfg(not(windows))]
    {
        let _ = port;
        Ok(())
    }
}

/// 扫描 uploadDir，找出"物理文件存在但没有对应分享记录"的孤儿文件。
///
/// 产生孤儿的典型来源：
/// - Guest/Host 删除分享记录后（按需求仅删记录不删源文件）
/// - 上传过程中客户端崩溃导致 `.partial` 遗留
/// - 服务重启期间 Registry reconcile 清掉了找不到物理文件的记录（反向不成立，但留作兜底）
///
/// 保守策略：**只扫描"像是本服务创建"的目录**（uploadDir 下 16 字符 id 子目录），
/// 不触碰 uploadDir 根目录下用户自己放的文件。
/// `dryRun = true` 仅预览，`dryRun = false` 执行删除。
#[tauri::command]
pub fn cleanup_orphans(
    slot: State<'_, ServerSlot>,
    dry_run: bool,
) -> Result<CleanupOrphansResult, String> {
    let (upload_dir, known, active_partials) = {
        let guard = slot.lock();
        let h = guard.as_ref().ok_or_else(|| "服务未运行".to_string())?;
        let upload_dir = h.state.config.read().upload_dir.clone();
        let known: std::collections::HashSet<PathBuf> = h
            .state
            .registry
            .known_file_paths()
            .into_iter()
            .filter_map(|p| std::fs::canonicalize(&p).ok().or(Some(p)))
            .collect();
        let active: std::collections::HashSet<PathBuf> = h
            .state
            .uploads
            .snapshot()
            .into_iter()
            .map(|s| s.partial_path)
            .filter_map(|p| std::fs::canonicalize(&p).ok().or(Some(p)))
            .collect();
        (upload_dir, known, active)
    };

    if upload_dir.as_os_str().is_empty() {
        return Err("uploadDir 未配置".into());
    }

    let mut items: Vec<OrphanItem> = Vec::new();
    let mut total: u64 = 0;

    // 遍历 uploadDir 一级子目录
    let read_top = std::fs::read_dir(&upload_dir)
        .map_err(|e| format!("读取目录失败: {e}"))?;
    for entry in read_top.flatten() {
        let sub = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_dir() {
            // 用户自己放在根目录下的文件不动
            continue;
        }
        let dir_name = sub.file_name().and_then(|s| s.to_str()).unwrap_or("");
        // 只处理疑似我们创建的 id 目录（16 个 ascii 字符）；其余用户目录一律跳过
        if dir_name.len() != 16 || !dir_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            continue;
        }

        // 深入这个 id 目录，逐个文件判断
        let files = match std::fs::read_dir(&sub) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for f in files.flatten() {
            let fpath = f.path();
            if !f.metadata().map(|m| m.is_file()).unwrap_or(false) {
                continue;
            }
            let canon = std::fs::canonicalize(&fpath).unwrap_or_else(|_| fpath.clone());
            if known.contains(&canon) || active_partials.contains(&canon) {
                continue;
            }
            let sz = f.metadata().map(|m| m.len()).unwrap_or(0);
            total = total.saturating_add(sz);
            items.push(OrphanItem {
                path: fpath.to_string_lossy().to_string(),
                size: sz,
            });
        }
    }

    if !dry_run {
        for it in &items {
            let p = PathBuf::from(&it.path);
            let _ = std::fs::remove_file(&p);
            // 顺手删空的父目录
            if let Some(parent) = p.parent() {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }

    Ok(CleanupOrphansResult {
        items,
        total_size: total,
        removed: !dry_run,
    })
}

/// 把一批本地文件"登记"到分享列表（零拷贝：记录的是原路径）。
/// 原文件被移走 / 删除后，下载链接会 404；下次启动服务时 Registry 的
/// reconcile 机制会自动把记录清理掉。
/// 传入的路径为目录或不存在的文件都会被 skip，不会中断整体提交。
#[tauri::command]
pub fn share_local_files(
    slot: State<'_, ServerSlot>,
    paths: Vec<String>,
) -> Result<ShareLocalResult, String> {
    let state = {
        let guard = slot.lock();
        guard
            .as_ref()
            .map(|h| h.state.clone())
            .ok_or_else(|| "服务未运行".to_string())?
    };

    let mut added: Vec<ShareLocalItem> = Vec::new();
    let mut skipped: Vec<ShareLocalSkipped> = Vec::new();

    for raw in paths {
        let path = PathBuf::from(&raw);
        let md = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                skipped.push(ShareLocalSkipped {
                    path: raw,
                    reason: "文件不存在或不可访问".into(),
                });
                continue;
            }
        };
        if md.is_dir() {
            skipped.push(ShareLocalSkipped {
                path: raw,
                reason: "暂不支持目录（请先打包为 zip 等单文件）".into(),
            });
            continue;
        }
        if !md.is_file() {
            skipped.push(ShareLocalSkipped {
                path: raw,
                reason: "不是常规文件".into(),
            });
            continue;
        }

        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".into());
        let size = md.len();
        let mime = mime_guess::from_path(&path)
            .first_or_octet_stream()
            .to_string();
        let id = nanoid!(16);

        let item = FileItem {
            id: id.clone(),
            name: name.clone(),
            size,
            mime,
            uploader_ip: "host".into(),
            created_at: now_secs(),
            path: path.clone(),
        };
        let arc = state.registry.add_file(item);
        state.broadcast(SyncEvent::FileAdded { file: arc });

        added.push(ShareLocalItem { id, name, size });
    }

    Ok(ShareLocalResult { added, skipped })
}

/// 查询最近的审计日志（P3-18）。按 ts 倒序，默认 200 条。
#[tauri::command]
pub fn list_audit_logs(
    slot: State<'_, ServerSlot>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<crate::server::files::AuditLog>, String> {
    let guard = slot.lock();
    let h = guard.as_ref().ok_or_else(|| "服务未运行".to_string())?;
    Ok(h.state
        .registry
        .list_audit(limit.unwrap_or(200), offset.unwrap_or(0)))
}

/// 清空审计日志（Host 端按钮触发）
#[tauri::command]
pub fn clear_audit_logs(slot: State<'_, ServerSlot>) -> Result<(), String> {
    let guard = slot.lock();
    let h = guard.as_ref().ok_or_else(|| "服务未运行".to_string())?;
    h.state.registry.clear_audit();
    Ok(())
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
    let arc = state.registry.add_text(item);
    state.broadcast(SyncEvent::TextAdded { text: arc });

    Ok(ShareClipboardResult {
        kind: "text".into(),
        id,
        name: preview,
        size,
    })
}
