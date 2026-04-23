<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
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
  CloudUploadOutline,
} from "@vicons/ionicons5";
import { useServerStore } from "../../stores/server";
import { formatSize, formatTime } from "../../utils/format";
import {
  fetchList,
  deleteFile,
  deleteText,
  postText,
} from "../../api/guest";
import {
  shareClipboard,
  shareLocalFiles,
  revealSharedFile,
} from "../../api/host";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { copyToClipboard } from "../../utils/clipboard";
import { useSync } from "../../composables/useSync";
import { useShareList } from "../../composables/useShareList";
import {
  TEXT_SHARE_MAX_LEN,
  deriveTxtFilename,
  textToTxtFile,
} from "../../utils/text-share";
import { ChunkedUploader } from "../../api/chunk-upload";
import type { ShareFile } from "../../types";

const serverStore = useServerStore();
const message = useMessage();
const dialog = useDialog();

function localBase() {
  // Host 端直接访问本地回环，避免 IP 切换时链接失效
  return `http://127.0.0.1:${serverStore.port}`;
}

// 列表状态 + 事件合并（unshift/splice 就地修改，O(1) id 查表）
const { files, texts, refresh, applyEvent } = useShareList(() =>
  fetchList(localBase()),
);

// WebSocket 实时同步（使用统一的 useSync composable：自动重连、心跳、authInvalid 处理）
const sync = useSync({
  url: `ws://127.0.0.1:${serverStore.port}/api/sync`,
  autoConnect: false,
  onEvent: applyEvent,
});

onMounted(() => {
  if (serverStore.running) {
    sync.start();
    refresh();
  }
});

watch(
  () => serverStore.running,
  (v) => {
    if (v) {
      sync.start();
      refresh();
    } else {
      sync.close();
      files.value = [];
      texts.value = [];
    }
  },
);

// ========== 清空 / 删除 ==========

function confirmClear() {
  dialog.warning({
    title: "确认清空",
    content: "将清空所有已分享的文件和文本记录，源文件保留在磁盘上不会删除。",
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
  const url = `${serverStore.url}/api/file/${encodeURIComponent(id)}`;
  const ok = await copyToClipboard(url);
  if (ok) {
    message.success("下载链接已复制");
  } else {
    message.error("复制失败");
  }
}

async function copyText(content: string) {
  const ok = await copyToClipboard(content);
  if (ok) {
    message.success("已复制");
  } else {
    message.error("复制失败");
  }
}

async function openFolder(f: ShareFile) {
  try {
    await revealSharedFile(f.id);
  } catch (e: any) {
    const msg = String(e?.message ?? e);
    console.warn(e);
    message.warning(msg || "打开文件夹失败");
  }
}

// ========== 分享文本 ==========

const showTextModal = ref(false);
const textDraft = ref("");
const textSubmitting = ref(false);

async function submitText() {
  const content = textDraft.value;
  if (!content.trim()) return;

  // 超长文本转为 .txt 文件上传，避免列表项难看 + 方便保存
  if (content.length > TEXT_SHARE_MAX_LEN) {
    const filename = deriveTxtFilename(content);
    dialog.info({
      title: "文本较长",
      content: `当前文本 ${content.length} 字，超过 ${TEXT_SHARE_MAX_LEN} 字阈值。将自动转换为 “${filename}” 上传。是否继续？`,
      positiveText: "转 TXT 上传",
      negativeText: "取消",
      onPositiveClick: async () => {
        await uploadTextAsFile(content);
      },
    });
    return;
  }

  textSubmitting.value = true;
  try {
    await postText(content, localBase());
    textDraft.value = "";
    showTextModal.value = false;
    message.success("已分享");
  } catch (e: any) {
    message.error(String(e?.message ?? e));
  } finally {
    textSubmitting.value = false;
  }
}

/** 把文本包装成 File，走分片上传 */
async function uploadTextAsFile(content: string) {
  textSubmitting.value = true;
  try {
    const file = textToTxtFile(content);
    const uploader = new ChunkedUploader(file, { base: localBase() });
    await uploader.start();
    textDraft.value = "";
    showTextModal.value = false;
    message.success(`已转为 ${file.name} 上传`);
  } catch (e: any) {
    message.error(String(e?.message ?? e));
  } finally {
    textSubmitting.value = false;
  }
}

// ========== 从本机选择文件分享 ==========

const pickingLocal = ref(false);

function shortenPath(p: string) {
  if (p.length <= 44) return p;
  return p.slice(0, 18) + "..." + p.slice(-22);
}

async function handlePickLocal() {
  if (pickingLocal.value) return;
  if (!serverStore.running) {
    message.warning("请先启动服务");
    return;
  }
  try {
    const picked = await openFileDialog({
      multiple: true,
      directory: false,
      title: "选择要分享的本机文件（零拷贝）",
    });
    if (!picked) return;
    const paths = Array.isArray(picked) ? picked : [picked];
    if (paths.length === 0) return;
    pickingLocal.value = true;
    const res = await shareLocalFiles(paths);
    if (res.added.length > 0) {
      message.success(`已添加 ${res.added.length} 个文件到分享列表`);
    }
    for (const s of res.skipped) {
      message.warning(`${shortenPath(s.path)}：${s.reason}`);
    }
    if (res.added.length === 0 && res.skipped.length === 0) {
      message.info("未选中文件");
    }
  } catch (e: any) {
    message.error(String(e?.message ?? e));
  } finally {
    pickingLocal.value = false;
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

// ========== 派生：分组列表 ==========

const sortedFiles = computed(() =>
  [...files.value].sort((a, b) => b.createdAt - a.createdAt),
);
const sortedTexts = computed(() =>
  [...texts.value].sort((a, b) => b.createdAt - a.createdAt),
);
const totalCount = computed(() => files.value.length + texts.value.length);
</script>

<template>
  <NCard size="small" class="list-card">
    <div class="list-header">
      <span class="title">分享列表</span>
      <NSpace :size="8">
        <NButton
          size="small"
          :loading="pickingLocal"
          :disabled="!serverStore.running"
          title="从本机选择文件加入分享列表（零拷贝，也支持直接拖拽到窗口）"
          @click="handlePickLocal"
        >
          <template #icon>
            <NIcon><CloudUploadOutline /></NIcon>
          </template>
          本机文件
        </NButton>
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
          :disabled="totalCount === 0"
          @click="confirmClear"
        >
          <template #icon>
            <NIcon><TrashOutline /></NIcon>
          </template>
          清空列表
        </NButton>
      </NSpace>
    </div>

    <!-- 显式拖拽/点选区：居中大块，视觉与 Guest 端保持一致 -->
    <div
      class="local-dropzone"
      :class="{ disabled: !serverStore.running || pickingLocal }"
      :title="serverStore.running ? '点击从本机选择文件（或拖拽到窗口任意位置）' : '请先启动服务'"
      @click="handlePickLocal"
    >
      <NIcon :size="40" class="ld-icon"><CloudUploadOutline /></NIcon>
      <div class="ld-title">拖拽文件到此或点击选择</div>
      <div class="ld-sub">支持多文件，单文件不限大小</div>
    </div>

    <div v-if="totalCount === 0" class="empty-wrap">
      <NEmpty>
        <template #default>
          <div class="empty-title">暂无分享内容</div>
          <div class="empty-sub">
            点击上方区域选择文件，或等待局域网用户上传
          </div>
        </template>
      </NEmpty>
    </div>

    <template v-else>
      <!-- 文件 -->
      <section class="section">
        <div class="section-title">文件 ({{ sortedFiles.length }})</div>
        <div v-if="sortedFiles.length === 0" class="sub-empty">暂无文件</div>
        <div v-else class="items">
          <div v-for="f in sortedFiles" :key="'f-' + f.id" class="row">
            <div class="col-ip" :title="f.uploaderIp">{{ f.uploaderIp }}</div>
            <div class="col-name" :title="f.name">{{ f.name }}</div>
            <div class="col-meta">
              {{ formatSize(f.size) }} · {{ formatTime(f.createdAt) }}
            </div>
            <div class="col-actions">
              <NButton
                size="small"
                quaternary
                circle
                title="复制下载链接"
                @click="copyDownloadLink(f.id)"
              >
                <template #icon>
                  <NIcon><LinkOutline /></NIcon>
                </template>
              </NButton>
              <NButton
                size="small"
                quaternary
                circle
                title="打开所在文件夹"
                @click="openFolder(f)"
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
                @click="removeFile(f.id)"
              >
                <template #icon>
                  <NIcon><TrashOutline /></NIcon>
                </template>
              </NButton>
            </div>
          </div>
        </div>
      </section>

      <!-- 文本 -->
      <section class="section">
        <div class="section-title">文本 ({{ sortedTexts.length }})</div>
        <div v-if="sortedTexts.length === 0" class="sub-empty">暂无文本</div>
        <div v-else class="items">
          <div v-for="t in sortedTexts" :key="'t-' + t.id" class="row row-text">
            <div class="col-ip" :title="t.uploaderIp">{{ t.uploaderIp }}</div>
            <div class="col-name col-name-text" :title="t.content">{{ t.content }}</div>
            <div class="col-meta">{{ formatTime(t.createdAt) }}</div>
            <div class="col-actions">
              <NButton
                size="small"
                quaternary
                circle
                title="复制文本"
                @click="copyText(t.content)"
              >
                <template #icon>
                  <NIcon><CopyOutline /></NIcon>
                </template>
              </NButton>
              <NButton
                size="small"
                quaternary
                circle
                title="删除"
                @click="removeText(t.id)"
              >
                <template #icon>
                  <NIcon><TrashOutline /></NIcon>
                </template>
              </NButton>
            </div>
          </div>
        </div>
      </section>
    </template>

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
      <div
        class="text-count"
        :class="{ 'is-over': textDraft.length > TEXT_SHARE_MAX_LEN }"
      >
        {{ textDraft.length }} / {{ TEXT_SHARE_MAX_LEN }} 字
        <span v-if="textDraft.length > TEXT_SHARE_MAX_LEN">
          · 超长将转为 .txt 文件上传
        </span>
      </div>
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
.local-dropzone {
  border: 2px dashed #a5f3fc;
  border-radius: 16px;
  padding: 36px 20px;
  background: var(--fs-card-bg-translucent, rgba(255, 255, 255, 0.6));
  text-align: center;
  cursor: pointer;
  transition: all 0.2s ease;
  margin-bottom: 12px;
  user-select: none;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
}
.local-dropzone:hover {
  border-color: var(--fs-accent-cyan, #22d3ee);
  background: var(--fs-card-bg-elevated, rgba(255, 255, 255, 0.85));
}
.local-dropzone.disabled {
  opacity: 0.55;
  cursor: not-allowed;
  pointer-events: none;
}
.ld-icon {
  color: var(--fs-accent-cyan-text, #0891b2);
}
.ld-title {
  font-size: 15px;
  font-weight: 500;
  color: var(--fs-card-title);
}
.ld-sub {
  font-size: 12px;
  color: var(--fs-card-text);
}

.empty-wrap {
  padding: 24px 0;
}
.section {
  margin-top: 8px;
}
.section + .section {
  margin-top: 16px;
}
.section-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--fs-text-secondary);
  padding: 4px 2px 8px;
  border-bottom: 1px solid var(--fs-border-soft, rgba(0, 0, 0, 0.06));
  margin-bottom: 6px;
}
.sub-empty {
  padding: 12px 6px;
  color: var(--fs-text-tertiary);
  font-size: 12px;
}
.empty-title {
  font-size: 14px;
  color: var(--fs-text-secondary);
}
.empty-sub {
  margin-top: 4px;
  font-size: 12px;
  color: var(--fs-text-tertiary);
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
.row-text {
  align-items: flex-start;
}
.col-name-text {
  /* 文本预览：最多 6 行，保留换行，超出省略 */
  white-space: pre-wrap;
  word-break: break-all;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 6;
  line-clamp: 6;
  line-height: 1.5;
  overflow: hidden;
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
.text-count {
  margin-top: 8px;
  font-size: 12px;
  color: var(--fs-text-tertiary);
  text-align: right;
}
.text-count.is-over {
  color: #e08b28;
}
</style>
