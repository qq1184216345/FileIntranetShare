import { defineStore } from "pinia";
import { ref } from "vue";
import { load, Store } from "@tauri-apps/plugin-store";
import { downloadDir } from "@tauri-apps/api/path";
import { DEFAULT_CONFIG, type AppConfig } from "../types";

const STORE_FILE = "settings.json";
const CONFIG_KEY = "config";

export const useConfigStore = defineStore("config", () => {
  const config = ref<AppConfig>({ ...DEFAULT_CONFIG });
  const loaded = ref(false);
  let store: Store | null = null;

  async function init() {
    store = await load(STORE_FILE, { autoSave: false, defaults: {} });
    const saved = await store.get<AppConfig>(CONFIG_KEY);
    if (saved) {
      config.value = { ...DEFAULT_CONFIG, ...saved };
    }
    if (!config.value.uploadDir) {
      try {
        config.value.uploadDir = await downloadDir();
      } catch {
        config.value.uploadDir = "";
      }
    }
    loaded.value = true;
  }

  async function save(partial: Partial<AppConfig>) {
    config.value = { ...config.value, ...partial };
    if (!store) return;
    await store.set(CONFIG_KEY, config.value);
    await store.save();
  }

  async function persist() {
    if (!store) return;
    await store.set(CONFIG_KEY, config.value);
    await store.save();
  }

  return { config, loaded, init, save, persist };
});
