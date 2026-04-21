<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import {
  NButton,
  NCard,
  NSpace,
  NIcon,
  NDropdown,
  NModal,
  useMessage,
} from "naive-ui";
import { SettingsOutline, QrCodeOutline } from "@vicons/ionicons5";
import Settings from "./Settings.vue";
import ShareList from "./ShareList.vue";
import QrCode from "../../components/QrCode.vue";
import { useConfigStore } from "../../stores/config";
import { useServerStore } from "../../stores/server";
import {
  getNetworkInterfaces,
  getServerStatus,
  startServer,
  stopServer,
  pickDefaultIp,
  updateTrayStatus,
  refreshServerAuth,
  isTauri,
} from "../../api/host";

const configStore = useConfigStore();
const serverStore = useServerStore();
const message = useMessage();

const showSettings = ref(false);
const showQr = ref(false);
const busy = ref(false);

const running = computed(() => serverStore.running);
const shareUrl = computed(() => serverStore.url);

onMounted(async () => {
  if (!isTauri) return;
  try {
    const list = await getNetworkInterfaces();
    serverStore.setInterfaces(list);

    const status = await getServerStatus();
    if (status.running) {
      serverStore.applyStatus(status);
      if (!serverStore.currentIp) {
        serverStore.setCurrentIp(pickDefaultIp(list));
      }
    }
  } catch (e) {
    console.error("init server status failed:", e);
  }
});

// 托盘状态与前端运行状态保持同步：running / shareUrl 任一变化都推送
watch(
  () => [running.value, shareUrl.value] as const,
  ([r, url]) => {
    if (!isTauri) return;
    updateTrayStatus(r, url).catch((e) => console.warn("tray update:", e));
  },
  { immediate: true },
);

async function toggleServer() {
  if (busy.value) return;
  busy.value = true;
  try {
    if (running.value) {
      await stopServer();
      serverStore.reset();
      message.info("服务已停止");
    } else {
      if (!configStore.config.uploadDir) {
        message.warning("请先在设置中选择上传保存路径");
        showSettings.value = true;
        return;
      }
      const status = await startServer(configStore.config);
      serverStore.applyStatus(status);

      const list = await getNetworkInterfaces();
      serverStore.setInterfaces(list);
      serverStore.setCurrentIp(pickDefaultIp(list));

      message.success(`服务已启动 :${status.port}`);
    }
  } catch (e: any) {
    message.error(String(e?.message ?? e));
  } finally {
    busy.value = false;
  }
}

async function copyLink() {
  if (!shareUrl.value) return;
  try {
    await navigator.clipboard.writeText(shareUrl.value);
    message.success("已复制");
  } catch {
    message.error("复制失败");
  }
}

/* 网卡切换（IPv4 / IPv6 按需筛选） */
const ipv4Options = computed(() =>
  serverStore.interfaces
    .filter((i) => !i.isIpv6 && !i.isLoopback)
    .map((i) => ({
      key: i.ip,
      label: `${i.ip}  (${i.name})`,
    })),
);

const ipv6Options = computed(() =>
  serverStore.interfaces
    .filter((i) => i.isIpv6 && !i.isLoopback)
    .map((i) => ({
      key: i.ip,
      label: `${i.ip}  (${i.name})`,
    })),
);

function handlePickIp(ip: string) {
  serverStore.setCurrentIp(ip);
}

async function refreshInterfaces() {
  const list = await getNetworkInterfaces();
  serverStore.setInterfaces(list);
}

async function onConfigUpdate(cfg: typeof configStore.config) {
  await configStore.save(cfg);
  // 服务运行中：尝试把 password/upload_dir 等运行时字段热应用
  // port 改动不会立即生效（listener 已绑定），仅在重启服务后生效
  if (!serverStore.running) {
    message.success("设置已更新");
    return;
  }
  try {
    const result = await refreshServerAuth(cfg);
    if (result.rotated) {
      message.success("设置已更新；密码已变更，在线访客需要重新登录");
    } else {
      message.success("设置已更新");
    }
  } catch (e) {
    console.warn("refresh auth failed:", e);
    message.warning("设置已保存，但热应用失败，请重启服务");
  }
}
</script>

<template>
  <div class="host-bg">
    <div class="host-container">
      <div class="top-bar">
        <NButton size="small" @click="showSettings = true">
          <template #icon>
            <NIcon><SettingsOutline /></NIcon>
          </template>
          设置
        </NButton>
      </div>

      <div v-if="!running" class="start-wrap">
        <div class="ripple-ring">
          <button class="start-btn" :disabled="busy" @click="toggleServer">
            {{ busy ? "启动中" : "开启服务" }}
          </button>
        </div>
      </div>

      <template v-else>
        <NCard size="small" class="status-card">
          <div class="status-row">
            <div class="status-label">
              <span class="dot dot-running" />
              正在分享...
            </div>
            <NButton
              size="small"
              type="error"
              ghost
              :loading="busy"
              @click="toggleServer"
            >
              取消分享
            </NButton>
          </div>
          <div class="divider" />
          <div class="share-block">
            <div
              class="qr-thumb"
              title="点击放大二维码"
              @click="showQr = true"
            >
              <QrCode v-if="shareUrl" :value="shareUrl" :size="88" :margin="1" />
            </div>
            <div class="share-right">
              <div class="share-url">
                分享链接：<a :href="shareUrl" target="_blank">{{ shareUrl || "(无可用地址)" }}</a>
              </div>
              <NSpace :size="8" class="share-actions">
                <NButton size="small" @click="copyLink" :disabled="!shareUrl">
                  复制链接
                </NButton>
                <NButton size="small" @click="showQr = true" :disabled="!shareUrl">
                  <template #icon>
                    <NIcon><QrCodeOutline /></NIcon>
                  </template>
                  二维码
                </NButton>
                <NDropdown
                  trigger="click"
                  :options="ipv4Options"
                  @select="handlePickIp"
                  @update:show="(v) => v && refreshInterfaces()"
                >
                  <NButton size="small">切换网卡</NButton>
                </NDropdown>
                <NDropdown
                  trigger="click"
                  :options="ipv6Options"
                  @select="handlePickIp"
                  @update:show="(v) => v && refreshInterfaces()"
                >
                  <NButton size="small" :disabled="ipv6Options.length === 0">
                    切换ipv6
                  </NButton>
                </NDropdown>
              </NSpace>
            </div>
          </div>
        </NCard>

        <ShareList />
      </template>
    </div>

    <Settings
      v-model:show="showSettings"
      :config="configStore.config"
      @update="onConfigUpdate"
    />

    <NModal
      v-model:show="showQr"
      preset="card"
      title="扫码访问分享页"
      style="width: 380px"
    >
      <div class="qr-modal">
        <QrCode
          v-if="shareUrl"
          :value="shareUrl"
          :size="280"
          :margin="2"
          error-correction-level="M"
        />
        <div class="qr-modal-url">{{ shareUrl }}</div>
        <div class="qr-modal-tip">
          同一局域网的设备用浏览器或微信扫一扫即可访问
        </div>
      </div>
      <template #footer>
        <NSpace justify="end">
          <NButton @click="copyLink" :disabled="!shareUrl">复制链接</NButton>
          <NButton type="primary" @click="showQr = false">关闭</NButton>
        </NSpace>
      </template>
    </NModal>
  </div>
</template>

<style scoped>
.host-container {
  max-width: 860px;
  margin: 0 auto;
  padding: 20px 24px 32px;
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.top-bar {
  display: flex;
  justify-content: flex-end;
}

.start-wrap {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 60vh;
}

.ripple-ring {
  padding: 24px;
  border-radius: 50%;
  background: radial-gradient(
    circle,
    rgba(255, 255, 255, 0.55) 0%,
    rgba(255, 255, 255, 0) 70%
  );
}

.start-btn {
  width: 140px;
  height: 140px;
  border-radius: 50%;
  border: none;
  cursor: pointer;
  color: white;
  font-size: 18px;
  font-weight: 500;
  background: linear-gradient(180deg, #5fa7ff 0%, #3d87f2 100%);
  box-shadow: 0 10px 28px rgba(61, 135, 242, 0.4),
    inset 0 0 0 6px rgba(255, 255, 255, 0.35);
  transition: transform 0.15s ease, box-shadow 0.2s ease;
}
.start-btn:hover:not(:disabled) {
  transform: scale(1.03);
}
.start-btn:active:not(:disabled) {
  transform: scale(0.98);
}
.start-btn:disabled {
  opacity: 0.7;
  cursor: not-allowed;
}

.status-card {
  border-radius: 12px;
}

.status-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 4px 0;
}

.status-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 500;
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #a0a0a0;
}
.dot-running {
  background: #52c41a;
  box-shadow: 0 0 0 3px rgba(82, 196, 26, 0.2);
}

.share-url a {
  color: #1e90ff;
  text-decoration: none;
  word-break: break-all;
}
.share-url a:hover {
  text-decoration: underline;
}

.divider {
  height: 1px;
  background: #eee;
  margin: 10px 0;
}

.share-block {
  display: flex;
  gap: 16px;
  align-items: center;
}
.qr-thumb {
  flex-shrink: 0;
  padding: 4px;
  border-radius: 10px;
  background: white;
  border: 1px solid #e5e7eb;
  cursor: pointer;
  transition: box-shadow 0.15s ease, transform 0.15s ease;
}
.qr-thumb:hover {
  box-shadow: 0 6px 20px rgba(30, 144, 255, 0.2);
  transform: translateY(-1px);
}
.share-right {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.share-actions {
  flex-wrap: wrap;
}

.qr-modal {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 8px 0 4px;
}
.qr-modal-url {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 13px;
  color: #334155;
  word-break: break-all;
  text-align: center;
  max-width: 320px;
}
.qr-modal-tip {
  font-size: 12px;
  color: #94a3b8;
}
</style>
