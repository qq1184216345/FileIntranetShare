<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";
import {
  NButton,
  NEmpty,
  NIcon,
  NInput,
  NProgress,
  NSpin,
  useMessage,
} from "naive-ui";
import {
  CloudUploadOutline,
  CopyOutline,
  DocumentOutline,
  DownloadOutline,
  ImageOutline,
  FilmOutline,
  MusicalNotesOutline,
  ArchiveOutline,
  DocumentTextOutline,
  CodeSlashOutline,
  RefreshOutline,
  CloseOutline,
  PauseOutline,
  PlayOutline,
} from "@vicons/ionicons5";
import {
  downloadUrl,
  fetchList,
  fetchInfo,
  postText,
} from "../../api/guest";
import { ChunkedUploader, type ChunkedStatus } from "../../api/chunk-upload";
import { formatSize, formatTime, fileKind } from "../../utils/format";
import { copyToClipboard } from "../../utils/clipboard";
import { useSync } from "../../composables/useSync";
import type { ShareFile, ShareText } from "../../types";

interface UploadTask {
  id: string;
  file: File;
  progress: number;
  loaded: number;
  total: number;
  status: ChunkedStatus;
  error?: string;
  resumed: boolean;
}

const message = useMessage();

const loading = ref(true);
const files = ref<ShareFile[]>([]);
const texts = ref<ShareText[]>([]);
const maxUploadSize = ref(100 * 1024 * 1024);

const textDraft = ref("");
const textSubmitting = ref(false);
const tasks = reactive<UploadTask[]>([]);
const uploaders = new Map<string, ChunkedUploader>();

async function refresh() {
  try {
    const { files: f, texts: t } = await fetchList();
    files.value = f;
    texts.value = t;
  } catch (e) {
    console.warn("list failed:", e);
  }
}

onMounted(async () => {
  try {
    const info = await fetchInfo();
    maxUploadSize.value = info.maxUploadSize;
  } catch {}
  await refresh();
  loading.value = false;
});

// WebSocket 实时同步：增量 patch 本地列表
useSync({
  onEvent: (ev) => {
    switch (ev.type) {
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
  },
});

// ========== 文件上传 ==========

const fileInput = ref<HTMLInputElement | null>(null);

function openPicker() {
  fileInput.value?.click();
}

function onFileInputChange(e: Event) {
  const input = e.target as HTMLInputElement;
  if (input.files) {
    handleFiles(Array.from(input.files));
    input.value = "";
  }
}

function onDrop(e: DragEvent) {
  e.preventDefault();
  dragOver.value = false;
  const list = e.dataTransfer?.files;
  if (list) handleFiles(Array.from(list));
}

const dragOver = ref(false);

let taskCounter = 0;

function handleFiles(list: File[]) {
  for (const file of list) {
    if (file.size === 0) {
      message.error(`${file.name} 是空文件，已跳过`);
      continue;
    }
    if (file.size > maxUploadSize.value) {
      message.error(
        `${file.name} 超过单文件 ${formatSize(maxUploadSize.value)} 上限`,
      );
      continue;
    }
    const id = `${++taskCounter}`;
    const task: UploadTask = reactive({
      id,
      file,
      progress: 0,
      loaded: 0,
      total: file.size,
      status: "pending",
      resumed: false,
    });
    const uploader = new ChunkedUploader(file, {
      onProgress: (loaded, total) => {
        task.loaded = loaded;
        task.total = total;
        task.progress = Math.min(99, Math.floor((loaded / total) * 100));
      },
      onStatus: (s) => {
        task.status = s;
      },
      onInit: (_uploadId, resumed) => {
        task.resumed = resumed;
        if (resumed) {
          message.info(`${file.name} 已从中断处继续`);
        }
      },
    });
    uploaders.set(id, uploader);
    tasks.push(task);
    runTask(task);
  }
}

function runTask(task: UploadTask) {
  const uploader = uploaders.get(task.id);
  if (!uploader) return;
  task.error = undefined;
  uploader
    .start()
    .then(() => {
      task.progress = 100;
      task.loaded = task.total;
      message.success(`${task.file.name} 上传成功`);
      setTimeout(() => {
        removeTask(task);
      }, 1500);
    })
    .catch((err) => {
      if (task.status === "paused" || task.status === "cancelled") return;
      task.error = String(err?.message ?? err);
      message.error(`${task.file.name}: ${task.error}`);
    });
}

function pauseTask(task: UploadTask) {
  uploaders.get(task.id)?.pause();
}

function resumeTask(task: UploadTask) {
  runTask(task);
}

async function cancelTask(task: UploadTask) {
  await uploaders.get(task.id)?.cancel();
  removeTask(task);
}

function removeTask(task: UploadTask) {
  uploaders.delete(task.id);
  const i = tasks.indexOf(task);
  if (i >= 0) tasks.splice(i, 1);
}

// ========== 文本分享 ==========

async function submitText() {
  if (!textDraft.value.trim()) return;
  textSubmitting.value = true;
  try {
    await postText(textDraft.value);
    textDraft.value = "";
    message.success("文本已分享");
    await refresh();
  } catch (e: any) {
    message.error(String(e?.message ?? e));
  } finally {
    textSubmitting.value = false;
  }
}

async function copyText(content: string) {
  const ok = await copyToClipboard(content);
  if (ok) {
    message.success("已复制");
  } else {
    message.error("复制失败，请长按文本手动选择复制");
  }
}

function iconFor(name: string, mime: string) {
  switch (fileKind(name, mime)) {
    case "image": return ImageOutline;
    case "video": return FilmOutline;
    case "audio": return MusicalNotesOutline;
    case "archive": return ArchiveOutline;
    case "text": return DocumentTextOutline;
    case "code": return CodeSlashOutline;
    default: return DocumentOutline;
  }
}

function download(file: ShareFile) {
  window.location.href = downloadUrl(file.id);
}
</script>

<template>
  <div class="guest-root">
    <header class="header">
      <div class="brand">
        <div class="logo">FS</div>
        <div class="brand-text">
          <div class="brand-title">FileShare</div>
          <div class="brand-sub">局域网文件共享</div>
        </div>
      </div>
      <NButton quaternary circle @click="refresh">
        <template #icon>
          <NIcon><RefreshOutline /></NIcon>
        </template>
      </NButton>
    </header>

    <main class="main">
      <!-- 上传区 -->
      <section
        class="dropzone"
        :class="{ 'is-drag': dragOver }"
        @dragover.prevent="dragOver = true"
        @dragleave.prevent="dragOver = false"
        @drop="onDrop"
        @click="openPicker"
      >
        <NIcon size="40" class="dropzone-icon">
          <CloudUploadOutline />
        </NIcon>
        <div class="dropzone-title">拖拽文件到此或点击选择</div>
        <div class="dropzone-sub">
          单文件最大 {{ formatSize(maxUploadSize) }}，支持多文件
        </div>
        <input
          ref="fileInput"
          type="file"
          multiple
          hidden
          @change="onFileInputChange"
        />
      </section>

      <!-- 上传任务列表 -->
      <section v-if="tasks.length > 0" class="section">
        <div class="section-title">上传任务 ({{ tasks.length }})</div>
        <div class="task-list">
          <div v-for="t in tasks" :key="t.id" class="task">
            <div class="task-head">
              <span class="task-name" :title="t.file.name">{{ t.file.name }}</span>
              <div class="task-actions">
                <NButton
                  v-if="t.status === 'uploading'"
                  text
                  size="tiny"
                  title="暂停"
                  @click="pauseTask(t)"
                >
                  <template #icon>
                    <NIcon><PauseOutline /></NIcon>
                  </template>
                </NButton>
                <NButton
                  v-if="t.status === 'paused' || t.status === 'error'"
                  text
                  size="tiny"
                  title="继续"
                  @click="resumeTask(t)"
                >
                  <template #icon>
                    <NIcon><PlayOutline /></NIcon>
                  </template>
                </NButton>
                <NButton
                  text
                  size="tiny"
                  :title="t.status === 'success' ? '移除' : '取消'"
                  @click="cancelTask(t)"
                >
                  <template #icon>
                    <NIcon><CloseOutline /></NIcon>
                  </template>
                </NButton>
              </div>
            </div>
            <NProgress
              :percentage="t.progress"
              :status="
                t.status === 'error'
                  ? 'error'
                  : t.status === 'success'
                  ? 'success'
                  : t.status === 'paused'
                  ? 'warning'
                  : 'default'
              "
              :show-indicator="false"
              :height="6"
            />
            <div class="task-meta">
              <span>{{ formatSize(t.loaded) }} / {{ formatSize(t.total) }}</span>
              <span class="task-status">
                <template v-if="t.status === 'uploading'">
                  {{ t.progress }}%<span v-if="t.resumed" class="resumed-tag">续传</span>
                </template>
                <template v-else-if="t.status === 'paused'">已暂停</template>
                <template v-else-if="t.status === 'success'">完成</template>
                <template v-else-if="t.status === 'error'">失败：{{ t.error }}</template>
                <template v-else-if="t.status === 'cancelled'">已取消</template>
                <template v-else>等待中</template>
              </span>
            </div>
          </div>
        </div>
      </section>

      <!-- 分享文本 -->
      <section class="section">
        <div class="section-title">分享文本</div>
        <div class="text-compose">
          <NInput
            v-model:value="textDraft"
            type="textarea"
            :autosize="{ minRows: 2, maxRows: 5 }"
            placeholder="输入要分享的文本..."
          />
          <NButton
            type="primary"
            :loading="textSubmitting"
            :disabled="!textDraft.trim()"
            @click="submitText"
          >
            发送
          </NButton>
        </div>
      </section>

      <!-- 文本列表 -->
      <section v-if="texts.length > 0" class="section">
        <div class="section-title">文本 ({{ texts.length }})</div>
        <div class="text-list">
          <div v-for="t in texts" :key="t.id" class="text-card">
            <div class="text-content">{{ t.content }}</div>
            <div class="text-meta">
              <span>{{ t.uploaderIp }}</span>
              <span>·</span>
              <span>{{ formatTime(t.createdAt) }}</span>
              <NButton size="tiny" quaternary @click="copyText(t.content)">
                <template #icon>
                  <NIcon><CopyOutline /></NIcon>
                </template>
                复制
              </NButton>
            </div>
          </div>
        </div>
      </section>

      <!-- 文件列表 -->
      <section class="section">
        <div class="section-title">
          文件 ({{ files.length }})
          <NSpin v-if="loading" size="small" />
        </div>
        <div v-if="!loading && files.length === 0" class="empty">
          <NEmpty description="还没有文件，来上传第一个吧" />
        </div>
        <div v-else class="file-list">
          <div v-for="f in files" :key="f.id" class="file-item" @click="download(f)">
            <NIcon size="28" class="file-icon">
              <component :is="iconFor(f.name, f.mime)" />
            </NIcon>
            <div class="file-info">
              <div class="file-name" :title="f.name">{{ f.name }}</div>
              <div class="file-meta">
                <span>{{ formatSize(f.size) }}</span>
                <span>·</span>
                <span>{{ f.uploaderIp }}</span>
                <span>·</span>
                <span>{{ formatTime(f.createdAt) }}</span>
              </div>
            </div>
            <NButton
              circle
              quaternary
              size="small"
              @click.stop="download(f)"
              title="下载"
            >
              <template #icon>
                <NIcon><DownloadOutline /></NIcon>
              </template>
            </NButton>
          </div>
        </div>
      </section>
    </main>

    <footer class="footer">
      <span>FileShare · 仅限局域网使用</span>
    </footer>
  </div>
</template>

<style scoped>
.guest-root {
  min-height: 100vh;
  background: var(--fs-guest-bg);
  color: var(--fs-card-title);
  display: flex;
  flex-direction: column;
  transition: background 0.25s ease;
}

.header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 20px;
  max-width: 760px;
  margin: 0 auto;
  width: 100%;
  box-sizing: border-box;
}

.brand {
  display: flex;
  align-items: center;
  gap: 12px;
}

.logo {
  width: 40px;
  height: 40px;
  border-radius: 10px;
  background: linear-gradient(135deg, #06b6d4 0%, #0ea5e9 100%);
  color: #ffffff;
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: 700;
  letter-spacing: -0.5px;
  box-shadow: 0 6px 16px rgba(14, 165, 233, 0.25);
}

.brand-title {
  font-size: 16px;
  font-weight: 600;
}
.brand-sub {
  font-size: 12px;
  color: var(--fs-card-text);
}

.main {
  flex: 1;
  max-width: 760px;
  width: 100%;
  margin: 0 auto;
  padding: 8px 20px 40px;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  gap: 20px;
}

/* 拖拽区 */
.dropzone {
  border: 2px dashed #a5f3fc;
  border-radius: 16px;
  padding: 36px 20px;
  background: var(--fs-card-bg-translucent);
  text-align: center;
  cursor: pointer;
  transition: all 0.2s ease;
}
.dropzone:hover {
  border-color: var(--fs-accent-cyan);
  background: var(--fs-card-bg-elevated);
}
.dropzone.is-drag {
  border-color: var(--fs-accent-cyan);
  background: var(--fs-accent-cyan-bg);
  transform: scale(1.01);
}
.dropzone-icon {
  color: var(--fs-accent-cyan-text);
}
.dropzone-title {
  font-size: 15px;
  margin-top: 8px;
  color: var(--fs-card-title);
  font-weight: 500;
}
.dropzone-sub {
  font-size: 12px;
  color: var(--fs-card-text);
  margin-top: 4px;
}

.section-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--fs-card-label);
  margin-bottom: 10px;
  display: flex;
  align-items: center;
  gap: 8px;
}

/* 上传任务卡 */
.task-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.task {
  background: var(--fs-card-bg);
  border-radius: 10px;
  padding: 12px 14px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12);
}
.task-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}
.task-name {
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
}
.task-meta {
  display: flex;
  justify-content: space-between;
  font-size: 11px;
  color: var(--fs-card-text);
  margin-top: 4px;
}
.task-status {
  font-variant-numeric: tabular-nums;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.task-actions {
  display: flex;
  gap: 6px;
}
.resumed-tag {
  font-size: 10px;
  padding: 1px 5px;
  border-radius: 4px;
  background: var(--fs-accent-cyan-bg);
  color: var(--fs-accent-cyan-deep);
  border: 1px solid #a5f3fc;
  line-height: 1.4;
}

/* 文本 */
.text-compose {
  display: flex;
  gap: 10px;
  align-items: flex-start;
}
.text-compose :deep(.n-input) {
  flex: 1;
}

.text-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.text-card {
  background: var(--fs-card-bg);
  border-radius: 10px;
  padding: 12px 14px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12);
}
.text-content {
  font-size: 14px;
  white-space: pre-wrap;
  word-break: break-all;
  color: var(--fs-card-title);
  line-height: 1.5;
}
.text-meta {
  display: flex;
  gap: 8px;
  align-items: center;
  font-size: 12px;
  color: var(--fs-card-text);
  margin-top: 8px;
}

/* 文件列表 */
.file-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.file-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  background: var(--fs-card-bg);
  border-radius: 10px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.12);
  cursor: pointer;
  transition: box-shadow 0.15s ease, transform 0.1s ease;
}
.file-item:hover {
  box-shadow: 0 6px 18px rgba(14, 165, 233, 0.15);
  transform: translateY(-1px);
}
.file-icon {
  color: var(--fs-accent-cyan-text);
  flex-shrink: 0;
}
.file-info {
  flex: 1;
  min-width: 0;
}
.file-name {
  font-size: 14px;
  font-weight: 500;
  color: var(--fs-card-title);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.file-meta {
  display: flex;
  gap: 6px;
  font-size: 11px;
  color: var(--fs-card-text);
  margin-top: 3px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.empty {
  padding: 24px 0;
}

.footer {
  text-align: center;
  color: var(--fs-card-muted);
  font-size: 12px;
  padding: 16px 0 24px;
}

@media (max-width: 480px) {
  .main {
    padding: 8px 14px 40px;
    gap: 16px;
  }
  .dropzone {
    padding: 24px 16px;
  }
  .text-compose {
    flex-direction: column;
  }
  .text-compose :deep(.n-button) {
    align-self: flex-end;
  }
}
</style>
