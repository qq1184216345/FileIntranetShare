<script setup lang="ts">
import { onMounted } from "vue";
import {
  NConfigProvider,
  NDialogProvider,
  NMessageProvider,
  NNotificationProvider,
  zhCN,
  dateZhCN,
} from "naive-ui";
import { useConfigStore } from "./stores/config";
import { useTheme } from "./composables/useTheme";

const configStore = useConfigStore();
const { naiveTheme } = useTheme();

onMounted(async () => {
  try {
    await configStore.init();
  } catch (e) {
    console.warn("config init failed:", e);
  }
});
</script>

<template>
  <NConfigProvider
    :locale="zhCN"
    :date-locale="dateZhCN"
    :theme="naiveTheme"
    :theme-overrides="themeOverrides"
  >
    <NDialogProvider>
      <NNotificationProvider>
        <NMessageProvider>
          <RouterView />
        </NMessageProvider>
      </NNotificationProvider>
    </NDialogProvider>
  </NConfigProvider>
</template>

<script lang="ts">
export const themeOverrides = {
  common: {
    primaryColor: "#4f8cff",
    primaryColorHover: "#669dff",
    primaryColorPressed: "#3874e0",
    primaryColorSuppl: "#4f8cff",
    borderRadius: "8px",
  },
  Button: {
    borderRadiusMedium: "8px",
  },
  Card: {
    borderRadius: "12px",
  },
};
</script>
