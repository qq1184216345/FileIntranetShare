<script setup lang="ts">
import { ref, watch } from "vue";
import QRCode from "qrcode";

const props = withDefaults(
  defineProps<{
    value: string;
    size?: number;
    /** 纠错等级：L/M/Q/H，默认 M */
    errorCorrectionLevel?: "L" | "M" | "Q" | "H";
    margin?: number;
    foreground?: string;
    background?: string;
  }>(),
  {
    size: 160,
    errorCorrectionLevel: "M",
    margin: 1,
    foreground: "#0f172a",
    background: "#ffffff",
  },
);

const dataUrl = ref("");
const error = ref("");

async function render() {
  if (!props.value) {
    dataUrl.value = "";
    return;
  }
  try {
    const url = await QRCode.toDataURL(props.value, {
      width: props.size,
      margin: props.margin,
      errorCorrectionLevel: props.errorCorrectionLevel,
      color: {
        dark: props.foreground,
        light: props.background,
      },
    });
    dataUrl.value = url;
    error.value = "";
  } catch (e: any) {
    error.value = String(e?.message ?? e);
    dataUrl.value = "";
  }
}

watch(
  () => [props.value, props.size, props.errorCorrectionLevel, props.margin, props.foreground, props.background],
  render,
  { immediate: true },
);
</script>

<template>
  <div class="qr-wrap" :style="{ width: `${size}px`, height: `${size}px` }">
    <img
      v-if="dataUrl"
      :src="dataUrl"
      :width="size"
      :height="size"
      alt="二维码"
      draggable="false"
    />
    <div v-else-if="error" class="qr-error">{{ error }}</div>
    <div v-else class="qr-loading">...</div>
  </div>
</template>

<style scoped>
.qr-wrap {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: white;
  border-radius: 8px;
  overflow: hidden;
}
.qr-wrap img {
  display: block;
  image-rendering: pixelated;
}
.qr-error {
  color: #ef4444;
  font-size: 12px;
  padding: 8px;
  text-align: center;
}
.qr-loading {
  color: #94a3b8;
  font-size: 12px;
}
</style>
