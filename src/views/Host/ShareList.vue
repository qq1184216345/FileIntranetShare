<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  NButton,
  NCard,
  NSpace,
  NEmpty,
  NIcon,
  NModal,
  NInput,
  useDialog,
  useMessage,
} from "naive-ui";
import {
  MailOutline,
  TrashOutline,
  LinkOutline,
  FolderOpenOutline,
  CopyOutline,
  ClipboardOutline,
} from "@vicons/ionicons5";
import { useConfigStore } from "../../stores/config";
import { useServerStore } from "../../stores/server";
import { formatSize, formatTime } from "../../utils/format";
import {
  fetchList,
  deleteFile,
  deleteText,
  postText,
} from "../../api/guest";
import { shareClipboard } from "../../api/host";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { appendTokenToUrl } from "../../api/auth";
import type { ShareFile, ShareText } from "../../types";

const configStore = useConfigStore();
const serverStore = useServerStore();
const message = useMessage();
const dialog = useDialog();

const files = ref<ShareFile[]>([]);
const texts = ref<ShareText[]>([]);

function localBase() {
  // Host 端直接访问本地回环，避免 IP 切换时链接失效
  return `http://127.0.0.1:${serverStore.port}`;
}

function wsUrl() {
  return appendTokenToUrl(`ws://127.0.0.1:${serverStore.port}/api/sync`);
}

async function refresh() {
  if (!serverStore.running) return;
  try {
    const data = await fetchList(localBase());
    files.value = data.files;
    texts.value = data.texts;
  } catch (e) {
    console.warn("list failed:", e);
  }
}

// ========== WebSocket 实时同步（原生实现，URL 依赖动态端口） ==========

let ws: WebSocket | null = null;
let reconnectTimer: number | null = null;
let manualClosed = false;

function handleSyncEvent(ev: any) {
  switch (ev?.type) {
    case "hello":
      refresh();
      break;
    case "fileAdded":
      if (!files.value.some((f) => f.id === ev.file.id)) {
        files.value = [ev.file, ...files.value];
      }
      break;
    case "fileRemoved":
      files.value = files.value.filter((f) => f.id !== ev.id);
      break;
    case "textAdded":
      if (!texts.value.some((t) => t.id === ev.text.id)) {
        texts.value = [ev.text, ...texts.value];
      }
      break;
    case "textRemoved":
      texts.value = texts.value.filter((t) => t.id !== ev.id);
      break;
    case "cleared":
    case "resync":
      refresh();
      break;
  }
}

function connectWs() {
  if (ws || manualClosed || !serverStore.running) return;
  try {
    ws = new WebSocket(wsUrl());
  } catch (e) {
    console.warn("host ws construct failed:", e);
    scheduleReconnect();
    return;
  }
  ws.onopen = () => refresh();
  ws.onmessage = (msg) => {
    try {
      handleSyncEvent(JSON.parse(msg.data));
    } catch {}
  };
  ws.onclose = () => {
    ws = null;
    scheduleReconnect();
  };
  ws.onerror = () => {};
}

function scheduleReconnect() {
  if (manualClosed || !serverStore.running || reconnectTimer) return;
  reconnectTimer = window.setTimeout(() => {
    reconnectTimer = null;
    connectWs();
  }, 2000);
}

function startSync() {
  manualClosed = false;
  connectWs();
}

function stopSync() {
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
}

onMounted(() => {
  if (serverStore.running) startSync();
});

onUnmounted(stopSync);

watch(
  () => serverStore.running,
  (v) => {
    if (v) {
      startSync();
    } else {
      stopSync();
      files.value = [];
      texts.value = [];
    }
  },
);

// ========== 清空 / 删除 ==========

function confirmClear() {
  dialog.warning({
    title: "确认清空",
    content: "将删除所有已分享的文件和文本，文件将从磁盘移除，不可恢复。",
    positiveText: "清空",
    negativeText: "取消",
    onPositiveClick: async () => {
      for (const f of files.value) {
        await deleteFile(f.id, serverStore.ownerToken, localBase()).catch(() => {});
      }
      for (const t of texts.value) {
        await deleteText(t.id, serverStore.ownerToken, localBase()).catch(() => {});
      }
      await refresh();
      message.success("已清空");
    },
  });
}

async function removeFile(id: string) {
  try {
    await deleteFile(id, serverStore.ownerToken, localBase());
    message.success("已删除");
  } catch (e: any) {
    message.error(String(e?.message ?? e));
  }
}

async function removeText(id: string) {
  try {
    await deleteText(id, serverStore.ownerToken, localBase());
    message.success("已删除");
  } catch (e: any) {
    message.error(String(e?.message ?? e));
  }
}

// ========== 辅助操作 ==========

async function copyDownloadLink(id: string) {
  try {
    await navigator.clipboard.writeText(`${serverStore.url}/api/file/${encodeURIComponent(id)}`);
    message.success("下载链接已复制");
  } catch {
    message.error("复制失败");
  }
}

async function copyText(content: string) {
  try {
    await navigator.clipboard.writeText(content);
    message.success("已复制");
  } catch {
    message.error("复制失败");
  }
}

async function openFolder(f: ShareFile) {
  try {
    const uploadDir = configStore.config.uploadDir;
    const path = `${uploadDir}/${f.id}/${f.name}`.replace(/\\/g, "/");
    await revealItemInDir(path);
  } catch (e) {
    console.warn(e);
    message.warning("打开文件夹失败");
  }
}

// ========== 分享文本 ==========

const showTextModal = ref(false);
const textDraft = ref("");
const textSubmitting = ref(false);

async function submitText() {
  if (!textDraft.value.trim()) return;
  textSubmitting.value = true;
  try {
    await postText(textDraft.value, localBase());
    textDraft.value = "";
    showTextModal.value = false;
    message.success("已分享");
  } catch (e: any) {
    message.error(String(e?.message ?? e));
  } finally {
    textSubmitting.value = false;
  }
}

// ========== 从剪贴板分享 ==========

const clipboardSharing = ref(false);

async function handleShareClipboard() {
  if (clipboardSharing.value) return;
  if (!serverStore.running) {
    message.warning("请先启动服务");
    return;
  }
  clipboardSharing.value = true;
  try {
    const res = await shareClipboard();
    if (res.kind === "file") {
      message.success(`已分享剪贴板图片：${res.name}（${formatSize(res.size)}）`);
    } else {
      const preview = res.name.length > 30 ? res.name.slice(0, 30) + "..." : res.name;
      message.success(`已分享剪贴板文本：${preview}`);
    }
  } catch (e: any) {
    const msg = String(e?.message ?? e);
    message.warning(msg || "剪贴板为空");
  } finally {
    clipboardSharing.value = false;
  }
}

// ========== 派生：统一的列表项 ==========

interface RowBase {
  kind: "file" | "text";
  id: string;
  ip: string;
  primary: string;
  secondary: string;
  createdAt: number;
}

const rows = computed<RowBase[]>(() => {
  const fileRows: RowBase[] = files.value.map((f) => ({
    kind: "file",
    id: f.id,
    ip: f.uploaderIp,
    primary: f.name,
    secondary: `${formatSize(f.size)} · ${formatTime(f.createdAt)}`,
    createdAt: f.createdAt,
  }));
  const textRows: RowBase[] = texts.value.map((t) => ({
    kind: "text",
    id: t.id,
    ip: t.uploaderIp,
    primary: t.content.slice(0, 60) + (t.content.length > 60 ? "..." : ""),
    secondary: `文本 · ${formatTime(t.createdAt)}`,
    createdAt: t.createdAt,
  }));
  return [...fileRows, ...textRows].sort((a, b) => b.createdAt - a.createdAt);
});

function textContent(id: string) {
  return texts.value.find((t) => t.id === id)?.content ?? "";
}
function fileItem(id: string) {
  return files.value.find((f) => f.id === id);
}
</script>

<template>
  <NCard size="small" class="list-card">
    <div class="list-header">
      <span class="title">分享列表</span>
      <NSpace :size="8">
        <NButton
          size="small"
          :loading="clipboardSharing"
          :disabled="!serverStore.running"
          title="把本地剪贴板里的图片或文本一键加入列表"
          @click="handleShareClipboard"
        >
          <template #icon>
            <NIcon><ClipboardOutline /></NIcon>
          </template>
          从剪贴板
        </NButton>
        <NButton size="small" @click="showTextModal = true">
          <template #icon>
            <NIcon><MailOutline /></NIcon>
          </template>
          分享文本
        </NButton>
        <NButton
          size="small"
          :disabled="rows.length === 0"
          @click="confirmClear"
        >
          <template #icon>
            <NIcon><TrashOutline /></NIcon>
          </template>
          清空列表
        </NButton>
      </NSpace>
    </div>

    <div v-if="rows.length === 0" class="empty-wrap">
      <NEmpty description="暂无分享内容 — 等待局域网用户上传" />
    </div>

    <div v-else class="items">
      <div v-for="r in rows" :key="r.kind + '-' + r.id" class="row">
        <div class="col-ip" :title="r.ip">{{ r.ip }}</div>
        <div class="col-name" :title="r.primary">{{ r.primary }}</div>
        <div class="col-meta">{{ r.secondary }}</div>
        <div class="col-actions">
          <NButton
            v-if="r.kind === 'file'"
            size="small"
            quaternary
            circle
            title="复制下载链接"
            @click="copyDownloadLink(r.id)"
          >
            <template #icon>
              <NIcon><LinkOutline /></NIcon>
            </template>
          </NButton>
          <NButton
            v-if="r.kind === 'text'"
            size="small"
            quaternary
            circle
            title="复制文本"
            @click="copyText(textContent(r.id))"
          >
            <template #icon>
              <NIcon><CopyOutline /></NIcon>
            </template>
          </NButton>
          <NButton
            v-if="r.kind === 'file' && fileItem(r.id)"
            size="small"
            quaternary
            circle
            title="打开所在文件夹"
            @click="openFolder(fileItem(r.id)!)"
          >
            <template #icon>
              <NIcon><FolderOpenOutline /></NIcon>
            </template>
          </NButton>
          <NButton
            size="small"
            quaternary
            circle
            title="删除"
            @click="r.kind === 'file' ? removeFile(r.id) : removeText(r.id)"
          >
            <template #icon>
              <NIcon><TrashOutline /></NIcon>
            </template>
          </NButton>
        </div>
      </div>
    </div>

    <!-- 分享文本弹窗 -->
    <NModal
      v-model:show="showTextModal"
      preset="card"
      title="分享文本"
      style="width: 520px"
    >
      <NInput
        v-model:value="textDraft"
        type="textarea"
        :autosize="{ minRows: 4, maxRows: 10 }"
        placeholder="输入要分享到局域网的文本..."
      />
      <template #footer>
        <NSpace justify="end">
          <NButton @click="showTextModal = false">取消</NButton>
          <NButton
            type="primary"
            :loading="textSubmitting"
            :disabled="!textDraft.trim()"
            @click="submitText"
          >
            发送
          </NButton>
        </NSpace>
      </template>
    </NModal>
  </NCard>
</template>

<style scoped>
.list-card {
  border-radius: 12px;
}
.list-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-bottom: 12px;
}
.title {
  font-weight: 500;
}
.empty-wrap {
  padding: 24px 0;
}
.items {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.row {
  display: grid;
  grid-template-columns: 110px 1fr 140px auto;
  align-items: center;
  gap: 12px;
  padding: 8px 6px;
  border-radius: 6px;
}
.row:hover {
  background: var(--fs-row-hover);
}
.col-ip {
  color: var(--fs-text-secondary);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.col-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.col-meta {
  color: var(--fs-text-tertiary);
  font-size: 12px;
  text-align: right;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.col-actions {
  display: flex;
  gap: 4px;
}
</style>
