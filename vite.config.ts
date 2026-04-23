import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },

  build: {
    // 单 chunk 告警阈值调整到 1MB（我们分包后最大约 700KB）
    chunkSizeWarningLimit: 1000,
    rollupOptions: {
      output: {
        // 拆成稳定可缓存的几个 chunk，避免 Guest 首屏拉单个大 bundle
        // vendor-ui：naive-ui + icons（最大头），单独缓存 1 次够用
        // vendor-vue：vue 运行时 + router + pinia，升级频率低
        // tauri：@tauri-apps/*，只在 Host 端用到；Guest 通过 tree-shaking 应被剔除
        manualChunks: {
          "vendor-ui": ["naive-ui", "@vicons/ionicons5"],
          "vendor-vue": ["vue", "vue-router", "pinia", "@vueuse/core"],
          "vendor-qrcode": ["qrcode"],
        },
      },
    },
  },
}));
