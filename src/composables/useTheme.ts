import { computed, ref, watch } from "vue";
import { darkTheme, type GlobalTheme } from "naive-ui";

export type ThemeMode = "light" | "dark" | "system";

const STORAGE_KEY = "fileshare:theme-mode";

function loadInitial(): ThemeMode {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "light" || v === "dark" || v === "system") return v;
  } catch {}
  return "system";
}

// 模块级单例 —— 所有组件共享同一份主题状态
const themeMode = ref<ThemeMode>(loadInitial());

const mq =
  typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;
const systemDark = ref<boolean>(mq?.matches ?? false);

if (mq) {
  const handler = (e: MediaQueryListEvent) => {
    systemDark.value = e.matches;
  };
  // 老版 Safari 没有 addEventListener
  if (typeof mq.addEventListener === "function") {
    mq.addEventListener("change", handler);
  } else {
    (mq as any).addListener?.(handler);
  }
}

const isDark = computed(
  () =>
    themeMode.value === "dark" ||
    (themeMode.value === "system" && systemDark.value),
);

const naiveTheme = computed<GlobalTheme | null>(() =>
  isDark.value ? darkTheme : null,
);

// 同步到 <html> 上，便于全局 CSS 通过 `html.dark` / `[data-theme=dark]` 进行样式切换
if (typeof document !== "undefined") {
  watch(
    isDark,
    (v) => {
      document.documentElement.classList.toggle("dark", v);
      document.documentElement.dataset.theme = v ? "dark" : "light";
    },
    { immediate: true },
  );
}

function setThemeMode(m: ThemeMode) {
  themeMode.value = m;
  try {
    localStorage.setItem(STORAGE_KEY, m);
  } catch {}
}

/**
 * 主题 composable（单例）。
 *
 * - `themeMode`: "light" | "dark" | "system"（用户显式选择）
 * - `isDark`: 当前实际是否暗色（system 模式下由系统决定）
 * - `naiveTheme`: 传给 `<NConfigProvider :theme>`
 * - `setThemeMode(m)`: 切换并写入 localStorage
 */
export function useTheme() {
  return {
    themeMode,
    isDark,
    naiveTheme,
    setThemeMode,
  };
}
