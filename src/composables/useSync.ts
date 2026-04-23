import { onUnmounted, ref } from "vue";
import type { ShareFile, ShareText } from "../types";
import { appendTokenToUrl, authedFetch, clearToken, dispatchAuthExpired } from "../api/auth";

export type SyncEvent =
  | { type: "hello"; startedAt: number }
  | { type: "resync" }
  | { type: "fileAdded"; file: ShareFile }
  | { type: "fileRemoved"; id: string }
  | { type: "textAdded"; text: ShareText }
  | { type: "textRemoved"; id: string }
  | { type: "cleared" }
  | { type: "authInvalid" };

export interface UseSyncOptions {
  /** WebSocket URL；默认基于当前页面 (ws(s)://host/api/sync) */
  url?: string;
  /** 收到事件的回调 */
  onEvent: (ev: SyncEvent) => void;
  /** 首次失败后的重连间隔 ms，默认 1500；指数退避上限 10s */
  reconnectDelay?: number;
  /** 是否在 setup 时立即连接，默认 true */
  autoConnect?: boolean;
}

/**
 * 建立与 /api/sync 的 WebSocket 长连接，自动重连；组件卸载时自动关闭。
 */
export function useSync(options: UseSyncOptions) {
  const {
    onEvent,
    url: customUrl,
    reconnectDelay = 1500,
    autoConnect = true,
  } = options;

  const connected = ref(false);
  const startedAt = ref(0);

  let ws: WebSocket | null = null;
  let reconnectTimer: number | null = null;
  let backoff = reconnectDelay;
  let manualClosed = false;

  function buildUrl(): string {
    if (customUrl) return appendTokenToUrl(customUrl);
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    return appendTokenToUrl(`${proto}//${location.host}/api/sync`);
  }

  function connect() {
    if (manualClosed) return;
    if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) return;

    const target = buildUrl();
    let socket: WebSocket;
    try {
      socket = new WebSocket(target);
    } catch (e) {
      console.warn("[sync] construct failed:", e);
      scheduleReconnect();
      return;
    }
    ws = socket;

    socket.onopen = () => {
      connected.value = true;
      backoff = reconnectDelay;
    };
    socket.onmessage = (msg) => {
      try {
        const ev = JSON.parse(msg.data as string) as SyncEvent;
        if (ev.type === "hello") startedAt.value = ev.startedAt;
        // 服务端 JWT 已轮换：立刻清 token、回登录页（配合后端 P0-4）
        if (ev.type === "authInvalid") {
          manualClosed = true;
          clearToken();
          dispatchAuthExpired();
          try { socket.close(); } catch {}
          return;
        }
        onEvent(ev);
      } catch (e) {
        console.warn("[sync] parse error:", e);
      }
    };
    socket.onerror = () => {
      /* 具体处理留给 onclose */
    };
    socket.onclose = () => {
      connected.value = false;
      if (ws === socket) ws = null;
      if (manualClosed) return;
      // 断开兜底：发一次 /api/list 探测 401（若服务端重启丢了旧 token 也能感知）
      authedFetch("/api/list", { cache: "no-store" }).catch(() => {});
      scheduleReconnect();
    };
  }

  function scheduleReconnect() {
    if (manualClosed) return;
    if (reconnectTimer) return;
    reconnectTimer = window.setTimeout(() => {
      reconnectTimer = null;
      backoff = Math.min(backoff * 1.5, 10000);
      connect();
    }, backoff);
  }

  function close() {
    manualClosed = true;
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    if (ws) {
      try {
        ws.close();
      } catch {}
      ws = null;
    }
    connected.value = false;
  }

  /** 手动开启（如果曾被 close 过） */
  function start() {
    manualClosed = false;
    backoff = reconnectDelay;
    connect();
  }

  if (autoConnect) {
    connect();
  }

  onUnmounted(close);

  return { connected, startedAt, close, start };
}
