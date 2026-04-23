<script setup lang="ts">
import { ref, watch } from "vue";
import {
  NModal,
  NCard,
  NForm,
  NFormItem,
  NInput,
  NInputNumber,
  NSwitch,
  NButton,
  NSpace,
  NInputGroup,
  NDivider,
  useMessage,
  useDialog,
} from "naive-ui";
import { open } from "@tauri-apps/plugin-dialog";
import type { AppConfig } from "../../types";
import { DEFAULT_CONFIG } from "../../types";
import { cleanupOrphans, repairFirewallRule } from "../../api/host";

const props = defineProps<{
  show: boolean;
  config: AppConfig;
}>();

const emit = defineEmits<{
  "update:show": [value: boolean];
  update: [config: AppConfig];
}>();

const message = useMessage();
const dialog = useDialog();
const form = ref<AppConfig>({ ...DEFAULT_CONFIG });
const cleaning = ref(false);

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

async function handleRepairFirewall() {
  try {
    await repairFirewallRule();
    message.success("已请求添加防火墙规则（请在弹出的 UAC 中点击允许）");
  } catch (e: any) {
    message.error(`修复失败: ${e?.message || e}`);
  }
}

async function handleCleanupOrphans() {
  cleaning.value = true;
  try {
    const preview = await cleanupOrphans(true);
    if (preview.items.length === 0) {
      message.success("磁盘很干净，没有孤儿文件");
      return;
    }
    const sizeText = formatBytes(preview.totalSize);
    dialog.warning({
      title: "发现孤儿文件",
      content: `在上传目录下扫描到 ${preview.items.length} 个没有分享记录的物理文件，合计 ${sizeText}。是否立即删除？（删除后不可恢复）`,
      positiveText: "全部删除",
      negativeText: "取消",
      onPositiveClick: async () => {
        try {
          const done = await cleanupOrphans(false);
          message.success(`已清理 ${done.items.length} 个孤儿文件（${formatBytes(done.totalSize)}）`);
        } catch (e: any) {
          message.error(`清理失败: ${e?.message || e}`);
        }
      },
    });
  } catch (e: any) {
    message.error(`扫描失败: ${e?.message || e}`);
  } finally {
    cleaning.value = false;
  }
}

watch(
  () => props.show,
  (v) => {
    if (v) {
      form.value = { ...props.config };
    }
  },
  { immediate: true },
);

async function pickFolder() {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      defaultPath: form.value.uploadDir || undefined,
    });
    if (typeof selected === "string") {
      form.value.uploadDir = selected;
    }
  } catch (e) {
    message.error("选择文件夹失败");
  }
}

function handleUpdate() {
  if (!form.value.uploadDir) {
    message.warning("请选择上传保存路径");
    return;
  }
  if (form.value.port < 1 || form.value.port > 65535) {
    message.warning("端口必须在 1-65535 之间");
    return;
  }
  if (form.value.passwordEnabled && !form.value.password) {
    message.warning("请填写访问密码");
    return;
  }
  emit("update", { ...form.value });
  emit("update:show", false);
}

function handleCancel() {
  emit("update:show", false);
}
</script>

<template>
  <NModal :show="show" @update:show="(v) => emit('update:show', v)">
    <NCard
      style="width: 520px"
      title="设置"
      :bordered="false"
      size="medium"
      role="dialog"
      aria-modal="true"
      closable
      @close="handleCancel"
    >
      <NForm label-placement="left" :label-width="90" size="medium">
        <NFormItem label="开机自启">
          <NSpace align="center" :size="8">
            <NSwitch v-model:value="form.autoStart" />
            <span class="hint">开机后自动启动本软件，并自动开启分享服务</span>
          </NSpace>
        </NFormItem>

        <NFormItem label="上传路径">
          <NInputGroup>
            <NInput v-model:value="form.uploadDir" placeholder="选择文件保存目录" readonly />
            <NButton @click="pickFolder">浏览</NButton>
          </NInputGroup>
        </NFormItem>

        <NFormItem label="服务端口">
          <NInputNumber
            v-model:value="form.port"
            :min="1"
            :max="65535"
            :show-button="false"
            style="width: 100%"
          />
        </NFormItem>

        <NFormItem label="密码认证">
          <NSwitch v-model:value="form.passwordEnabled" />
        </NFormItem>

        <NFormItem v-if="form.passwordEnabled" label="访问密码">
          <NInput
            v-model:value="form.password"
            type="password"
            show-password-on="click"
            placeholder="填写访客访问时需要的密码"
          />
        </NFormItem>

        <NFormItem v-if="form.passwordEnabled" label=" ">
          <div class="settings-hint">
            密码以 argon2 哈希保存；修改后立即生效，在线访客需要重新登录。
          </div>
        </NFormItem>

        <NFormItem label="磁盘软限">
          <NSpace vertical :size="4" style="width: 100%">
            <NInputNumber
              v-model:value="form.diskMinFreeMb"
              :min="0"
              :max="102400"
              :step="100"
              :show-button="false"
              style="width: 100%"
            >
              <template #suffix>MB</template>
            </NInputNumber>
            <span class="hint">
              上传前检查磁盘剩余，若 &lt; (文件大小 + 此值) 则拒绝。0 为不限制，默认 500MB。
            </span>
          </NSpace>
        </NFormItem>

        <NDivider style="margin: 8px 0 12px" />

        <NFormItem label="磁盘清理">
          <NSpace vertical :size="6" style="width: 100%">
            <NButton :loading="cleaning" @click="handleCleanupOrphans">
              扫描并清理孤儿文件
            </NButton>
            <span class="hint">
              删除分享记录时默认保留源文件；使用此功能可回收"只剩物理文件、已无记录"的空间。
            </span>
          </NSpace>
        </NFormItem>

        <NFormItem label="防火墙">
          <NSpace vertical :size="6" style="width: 100%">
            <NButton @click="handleRepairFirewall">修复防火墙规则</NButton>
            <span class="hint">
              安装时已尝试自动添加；若端口被修改或 LAN 同事无法访问，点此补一次（会弹 UAC）。
            </span>
          </NSpace>
        </NFormItem>
      </NForm>

      <template #footer>
        <NSpace justify="end">
          <NButton type="primary" @click="handleUpdate">更新</NButton>
          <NButton @click="handleCancel">取消</NButton>
        </NSpace>
      </template>
    </NCard>
  </NModal>
</template>

<style scoped>
.settings-hint {
  font-size: 12px;
  color: var(--fs-card-text);
  line-height: 1.5;
}
.hint {
  font-size: 12px;
  color: var(--fs-card-text);
}
</style>
