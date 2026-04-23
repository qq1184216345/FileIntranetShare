<script setup lang="ts">
import { computed, h, ref, watch } from "vue";
import {
  NModal,
  NCard,
  NSpace,
  NButton,
  NDataTable,
  NTag,
  NEmpty,
  useMessage,
  useDialog,
  type DataTableColumns,
} from "naive-ui";
import {
  listAuditLogs,
  clearAuditLogs,
  type AuditLog,
} from "../../api/host";

const props = defineProps<{
  show: boolean;
}>();
const emit = defineEmits<{
  "update:show": [value: boolean];
}>();

const message = useMessage();
const dialog = useDialog();
const loading = ref(false);
const rows = ref<AuditLog[]>([]);

function fmtTs(ts: number) {
  const d = new Date(ts * 1000);
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(
    d.getHours(),
  )}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

const kindMap: Record<string, { label: string; type: "default" | "success" | "info" | "warning" | "error" }> = {
  login: { label: "登录", type: "success" },
  login_fail: { label: "登录失败", type: "error" },
  upload: { label: "上传", type: "info" },
  download: { label: "下载", type: "default" },
  text_add: { label: "发布文本", type: "info" },
  text_delete: { label: "删除文本", type: "warning" },
  file_delete: { label: "删除文件", type: "warning" },
};

const columns = computed<DataTableColumns<AuditLog>>(() => [
  {
    title: "时间",
    key: "ts",
    width: 170,
    render(row) {
      return fmtTs(row.ts);
    },
  },
  {
    title: "类型",
    key: "kind",
    width: 100,
    render(row) {
      const m = kindMap[row.kind] ?? { label: row.kind, type: "default" as const };
      return h(
        NTag,
        { size: "small", type: m.type, bordered: false },
        { default: () => m.label },
      );
    },
  },
  { title: "来源", key: "ip", width: 140 },
  { title: "详情", key: "detail" },
]);

async function refresh() {
  loading.value = true;
  try {
    rows.value = await listAuditLogs(200, 0);
  } catch (e: any) {
    message.error(`加载失败: ${e?.message || e}`);
  } finally {
    loading.value = false;
  }
}

function handleClear() {
  dialog.warning({
    title: "清空审计日志",
    content: "将永久删除所有审计记录，确定继续？",
    positiveText: "清空",
    negativeText: "取消",
    onPositiveClick: async () => {
      try {
        await clearAuditLogs();
        rows.value = [];
        message.success("已清空");
      } catch (e: any) {
        message.error(`清空失败: ${e?.message || e}`);
      }
    },
  });
}

watch(
  () => props.show,
  (v) => {
    if (v) refresh();
  },
);
</script>

<template>
  <NModal :show="show" @update:show="(v) => emit('update:show', v)">
    <NCard
      style="width: 780px; max-height: 78vh"
      title="访问日志"
      :bordered="false"
      size="medium"
      role="dialog"
      aria-modal="true"
      closable
      @close="emit('update:show', false)"
    >
      <template #header-extra>
        <NSpace :size="8">
          <NButton size="small" :loading="loading" @click="refresh">刷新</NButton>
          <NButton size="small" type="warning" ghost @click="handleClear">清空</NButton>
        </NSpace>
      </template>

      <NEmpty
        v-if="!loading && rows.length === 0"
        description="暂无访问记录"
        style="margin: 40px 0"
      />
      <NDataTable
        v-else
        :columns="columns"
        :data="rows"
        :loading="loading"
        :max-height="520"
        :pagination="{ pageSize: 20 }"
        size="small"
        flex-height
      />

      <div class="hint">
        保留最近 2000 条；下载仅记录全量请求，避免 Range/分片产生噪声。
      </div>
    </NCard>
  </NModal>
</template>

<style scoped>
.hint {
  margin-top: 10px;
  font-size: 12px;
  color: var(--fs-text-muted);
}
</style>
