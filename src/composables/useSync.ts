import { onUnmounted, ref } from "vue";
import type { ShareFile, ShareText } from "../types";
import { appendTokenToUrl, authedFetch } from "../api/auth";

export type SyncEvent =
  | { type: "hello"; startedAt: number }
  | { type: "resync" }
  | { type: "fileAdded"; file: ShareFile }
  | { type: "fileRemoved"; id: string }
  | { type: "textAdded"; text: ShareText }
  | { type: "textRemoved"; id: string }
  | { type: "cleared" };

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
      // 探测是否因鉴权失效导致断开：authedFetch 遇 401 会自动清 token 并发 AUTH_EVENT
      authedFetch("/api/list", { cache: "no-store" }).catch(() => {
        // 忽略网络错误；401 已被 authedFetch 处理
      });
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
