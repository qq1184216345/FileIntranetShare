<script setup lang="ts">
import { ref } from "vue";
import { NButton, NIcon, NInput, useMessage } from "naive-ui";
import { LockClosedOutline } from "@vicons/ionicons5";
import { login } from "../../api/auth";

const emit = defineEmits<{ (e: "success"): void }>();

const message = useMessage();
const password = ref("");
const submitting = ref(false);

async function submit() {
  if (!password.value) return;
  submitting.value = true;
  try {
    await login(password.value);
    message.success("登录成功");
    emit("success");
  } catch (e: any) {
    message.error(String(e?.message ?? e));
    password.value = "";
  } finally {
    submitting.value = false;
  }
}
</script>

<template>
  <div class="login-root">
    <div class="login-card">
      <div class="login-logo">
        <NIcon size="28"><LockClosedOutline /></NIcon>
      </div>
      <div class="login-title">需要密码</div>
      <div class="login-sub">向分享者询问访问密码</div>
      <NInput
        v-model:value="password"
        type="password"
        placeholder="请输入密码"
        size="large"
        show-password-on="click"
        :maxlength="128"
        @keyup.enter="submit"
      />
      <NButton
        type="primary"
        size="large"
        block
        :loading="submitting"
        :disabled="!password"
        @click="submit"
      >
        进入
      </NButton>
    </div>
    <div class="login-footer">FileShare · 局域网文件共享</div>
  </div>
</template>

<style scoped>
.login-root {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  background: var(--fs-guest-bg);
  padding: 24px;
  transition: background 0.25s ease;
}
.login-card {
  width: 100%;
  max-width: 360px;
  background: var(--fs-card-bg);
  border-radius: 16px;
  padding: 32px 24px 24px;
  box-shadow: 0 12px 32px rgba(14, 165, 233, 0.12);
  display: flex;
  flex-direction: column;
  gap: 14px;
  text-align: center;
}
.login-logo {
  width: 56px;
  height: 56px;
  border-radius: 14px;
  margin: 0 auto;
  background: linear-gradient(135deg, #06b6d4 0%, #0ea5e9 100%);
  color: #ffffff;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 8px 20px rgba(14, 165, 233, 0.3);
}
.login-title {
  font-size: 18px;
  font-weight: 600;
  color: var(--fs-card-title);
}
.login-sub {
  font-size: 13px;
  color: var(--fs-card-text);
  margin-bottom: 4px;
}
.login-footer {
  margin-top: 24px;
  font-size: 12px;
  color: var(--fs-card-muted);
}
</style>
