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
  useMessage,
} from "naive-ui";
import { open } from "@tauri-apps/plugin-dialog";
import type { AppConfig } from "../../types";
import { DEFAULT_CONFIG } from "../../types";

const props = defineProps<{
  show: boolean;
  config: AppConfig;
}>();

const emit = defineEmits<{
  "update:show": [value: boolean];
  update: [config: AppConfig];
}>();

const message = useMessage();
const form = ref<AppConfig>({ ...DEFAULT_CONFIG });

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
        <NFormItem label="服务自启">
          <NSwitch v-model:value="form.autoStart" />
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
</style>
