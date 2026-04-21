<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { NSpin, useMessage } from "naive-ui";
import Login from "./Login.vue";
import GuestMain from "./GuestMain.vue";
import { fetchInfo } from "../../api/guest";
import { AUTH_EVENT, authedFetch, clearToken, getToken } from "../../api/auth";

const message = useMessage();

type View = "loading" | "login" | "main";
const view = ref<View>("loading");

async function decide() {
  try {
    const info = await fetchInfo();
    if (!info.passwordRequired) {
      view.value = "main";
      return;
    }
    // 需要密码：验证本地 token 是否仍然有效
    if (getToken()) {
      try {
        const r = await authedFetch("/api/list", { cache: "no-store" });
        if (r.ok) {
          view.value = "main";
          return;
        }
      } catch (e: any) {
        // 401 会被 authedFetch 自动清 token
        if (e?.code !== 401) {
          console.warn("auth probe failed:", e);
        }
      }
      clearToken();
    }
    view.value = "login";
  } catch (e) {
    console.error("fetchInfo failed:", e);
    // 服务异常时直接显示 login 让用户看到明确提示
    view.value = "login";
  }
}

function onLoginSuccess() {
  view.value = "main";
}

function onAuthExpired() {
  // 密码被 Host 改了 / token 过期：回登录页让访客重新登录
  if (view.value === "main") {
    message.warning("登录已失效，请重新登录");
  }
  clearToken();
  view.value = "login";
}

onMounted(() => {
  window.addEventListener(AUTH_EVENT, onAuthExpired);
  decide();
});

onUnmounted(() => {
  window.removeEventListener(AUTH_EVENT, onAuthExpired);
});
</script>

<template>
  <div v-if="view === 'loading'" class="guest-boot">
    <NSpin size="large" />
  </div>
  <Login v-else-if="view === 'login'" @success="onLoginSuccess" />
  <GuestMain v-else />
</template>

<style scoped>
.guest-boot {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(180deg, #ecfeff 0%, #f0fdfa 50%, #f5fbff 100%);
}
</style>
