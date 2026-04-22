import { invoke } from "@tauri-apps/api/core";
import type { NetworkInterface, ServerStatus, AppConfig } from "../types";

export const isTauri = typeof (window as any).__TAURI_INTERNALS__ !== "undefined";

export async function getNetworkInterfaces(): Promise<NetworkInterface[]> {
  return invoke<NetworkInterface[]>("get_network_interfaces");
}

export async function startServer(config: AppConfig): Promise<ServerStatus> {
  return invoke<ServerStatus>("start_server", { config });
}

export async function stopServer(): Promise<void> {
  return invoke("stop_server");
}

export async function getServerStatus(): Promise<ServerStatus> {
  return invoke<ServerStatus>("get_server_status");
}

export async function setAutoStart(enabled: boolean): Promise<void> {
  return invoke("set_auto_start", { enabled });
}

export async function updateTrayStatus(
  running: boolean,
  shareUrl: string,
): Promise<void> {
  return invoke("update_tray_status", { running, shareUrl });
}

export async function showMainWindow(): Promise<void> {
  return invoke("show_main_window");
}

export async function quitApp(): Promise<void> {
  return invoke("quit_app");
}

export interface AuthRefreshResult {
  rotated: boolean;
  passwordRequired: boolean;
}

export interface ShareClipboardResult {
  kind: "file" | "text";
  id: string;
  name: string;
  size: number;
}

/**
 * 读取 Host 剪贴板，优先图片再文本，注入到分享列表。
 * 成功后服务端会通过 WS 广播 fileAdded / textAdded，本地 ShareList 会自动刷新。
 */
export async function shareClipboard(): Promise<ShareClipboardResult> {
  return invoke<ShareClipboardResult>("share_clipboard");
}

export interface ShareLocalItem {
  id: string;
  name: string;
  size: number;
}

export interface ShareLocalSkipped {
  path: string;
  reason: string;
}

export interface ShareLocalResult {
  added: ShareLocalItem[];
  skipped: ShareLocalSkipped[];
}

/**
 * 把一批本机文件的绝对路径登记到分享列表（零拷贝，原文件移走后记录失效）。
 * 成功后服务端会通过 WS 广播 fileAdded，本地 ShareList 会自动刷新。
 */
export async function shareLocalFiles(
  paths: string[],
): Promise<ShareLocalResult> {
  return invoke<ShareLocalResult>("share_local_files", { paths });
}

/**
 * 在系统文件管理器中定位分享列表里某条文件（按记录中的真实路径）。
 * 对访客上传、剪贴板图片、本机引用文件一视同仁。
 */
export async function revealSharedFile(id: string): Promise<void> {
  return invoke("reveal_shared_file", { id });
}

/**
 * 保存配置后调用，把最新 AppConfig 热应用到运行中的服务。
 * 返回 rotated=true 表示密码相关变化已触发 JWT 轮换，在线访客需重新登录。
 */
export async function refreshServerAuth(
  config: AppConfig,
): Promise<AuthRefreshResult> {
  return invoke<AuthRefreshResult>("refresh_server_auth", { config });
}

/**
 * 根据网卡列表选择一个"最可能是局域网"的 IPv4 地址
 * 优先级：非回环 IPv4 > 私有网段（10/172.16-31/192.168）> 其他
 */
export function pickDefaultIp(list: NetworkInterface[]): string {
  const candidates = list.filter((i) => !i.isLoopback && !i.isIpv6);
  const privatePrefixes = ["10.", "172.", "192.168."];
  const privateOnes = candidates.filter((i) =>
    privatePrefixes.some((p) => i.ip.startsWith(p)),
  );
  if (privateOnes.length > 0) return privateOnes[0].ip;
  if (candidates.length > 0) return candidates[0].ip;
  // 实在没有 IPv4，拿 IPv6 兜底
  const v6 = list.filter((i) => !i.isLoopback && i.isIpv6);
  if (v6.length > 0) return v6[0].ip;
  return "127.0.0.1";
}
