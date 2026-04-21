import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { NetworkInterface, ShareItem, ServerStatus } from "../types";
import { setToken as setAuthToken, clearToken as clearAuthToken } from "../api/auth";

export const useServerStore = defineStore("server", () => {
  const running = ref(false);
  const port = ref(0);
  const bindIpv6 = ref(false);
  const ownerToken = ref("");
  const startedAt = ref(0);

  const interfaces = ref<NetworkInterface[]>([]);
  const currentIp = ref(""); // 当前在 UI 展示的 IP（用户可切换）
  const items = ref<ShareItem[]>([]);

  const url = computed(() => {
    if (!running.value || !currentIp.value) return "";
    const host = currentIp.value.includes(":")
      ? `[${currentIp.value}]`
      : currentIp.value;
    return `http://${host}:${port.value}`;
  });

  function applyStatus(status: ServerStatus) {
    running.value = status.running;
    port.value = status.port;
    bindIpv6.value = status.bindIpv6;
    ownerToken.value = status.ownerToken;
    startedAt.value = status.startedAt;
    // Host 窗口复用 authedFetch 基础设施：把 owner_token 存到 localStorage，
    // 这样所有 authed 请求/WS 自动携带，middleware 识别为管理员。
    // Host 与 Guest 不同源，localStorage 相互隔离，不会冲突。
    if (status.running && status.ownerToken) {
      setAuthToken(status.ownerToken);
    }
  }

  function setInterfaces(list: NetworkInterface[]) {
    interfaces.value = list;
  }

  function setCurrentIp(ip: string) {
    currentIp.value = ip;
  }

  function reset() {
    running.value = false;
    port.value = 0;
    bindIpv6.value = false;
    ownerToken.value = "";
    startedAt.value = 0;
    currentIp.value = "";
    clearAuthToken();
  }

  return {
    running,
    port,
    bindIpv6,
    ownerToken,
    startedAt,
    interfaces,
    currentIp,
    items,
    url,
    applyStatus,
    setInterfaces,
    setCurrentIp,
    reset,
  };
});
