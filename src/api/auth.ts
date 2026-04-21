import { baseUrl } from "./guest";

const LS_KEY = "fileshare.guest_token.v1";

let cachedToken: string | null = null;

/** 读取本地缓存的访客 token（登录态） */
export function getToken(): string {
  if (cachedToken !== null) return cachedToken;
  try {
    cachedToken = localStorage.getItem(LS_KEY) ?? "";
  } catch {
    cachedToken = "";
  }
  return cachedToken;
}

export function setToken(token: string) {
  cachedToken = token;
  try {
    if (token) localStorage.setItem(LS_KEY, token);
    else localStorage.removeItem(LS_KEY);
  } catch {
    // 忽略 localStorage 失败
  }
}

export function clearToken() {
  setToken("");
}

/**
 * 构造带 Authorization 头的 headers 对象
 */
export function authHeaders(extra?: Record<string, string>): Record<string, string> {
  const token = getToken();
  const headers: Record<string, string> = { ...(extra || {}) };
  if (token) headers["Authorization"] = `Bearer ${token}`;
  return headers;
}

/**
 * 追加 `?token=xxx` 到 URL（用于 <a download> 和 WebSocket，这两者无法设置 header）
 */
export function appendTokenToUrl(url: string): string {
  const token = getToken();
  if (!token) return url;
  const sep = url.includes("?") ? "&" : "?";
  return `${url}${sep}token=${encodeURIComponent(token)}`;
}

/** 广播"鉴权失效"事件，入口组件据此切回登录页 */
export const AUTH_EVENT = "fileshare:auth-expired";

export function dispatchAuthExpired() {
  try {
    window.dispatchEvent(new CustomEvent(AUTH_EVENT));
  } catch {
    // SSR / 非浏览器环境忽略
  }
}

/**
 * fetch 包装：自动注入 token；401 时清除 token，广播失效事件，并抛出可识别错误
 */
export async function authedFetch(
  input: string,
  init: RequestInit = {},
): Promise<Response> {
  const headers = authHeaders(init.headers as Record<string, string> | undefined);
  const res = await fetch(input, { ...init, headers });
  if (res.status === 401) {
    clearToken();
    dispatchAuthExpired();
    const err = new Error("UNAUTHORIZED");
    (err as any).code = 401;
    throw err;
  }
  return res;
}

export interface LoginResp {
  token: string;
  expiresIn: number;
}

/** 提交密码换取 token */
export async function login(password: string, base = ""): Promise<LoginResp> {
  const res = await fetch(`${base || baseUrl()}/api/login`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ password }),
  });
  if (res.status === 401) {
    throw new Error("密码错误");
  }
  if (!res.ok) {
    throw new Error((await res.text()) || `login ${res.status}`);
  }
  const data: LoginResp = await res.json();
  setToken(data.token);
  return data;
}
