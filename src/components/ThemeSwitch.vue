<script setup lang="ts">
import { computed, h } from "vue";
import { NButton, NIcon, NDropdown } from "naive-ui";
import {
  SunnyOutline,
  MoonOutline,
  DesktopOutline,
  CheckmarkOutline,
} from "@vicons/ionicons5";
import { useTheme, type ThemeMode } from "../composables/useTheme";

const { themeMode, isDark, setThemeMode } = useTheme();

const triggerIcon = computed(() => (isDark.value ? MoonOutline : SunnyOutline));

interface Opt {
  key: ThemeMode;
  label: string;
  icon: any;
}
const opts: Opt[] = [
  { key: "light", label: "浅色", icon: SunnyOutline },
  { key: "dark", label: "深色", icon: MoonOutline },
  { key: "system", label: "跟随系统", icon: DesktopOutline },
];

const options = computed(() =>
  opts.map((o) => ({
    key: o.key,
    label: () =>
      h(
        "div",
        { style: "display:flex;align-items:center;gap:8px;min-width:132px;" },
        [
          h(NIcon, { size: 16 }, { default: () => h(o.icon) }),
          h("span", { style: "flex:1" }, o.label),
          themeMode.value === o.key
            ? h(
                NIcon,
                { size: 14, color: "var(--fs-text-link)" },
                { default: () => h(CheckmarkOutline) },
              )
            : null,
        ],
      ),
  })),
);

function onSelect(key: ThemeMode) {
  setThemeMode(key);
}
</script>

<template>
  <NDropdown
    trigger="click"
    :options="(options as any)"
    placement="bottom-end"
    @select="onSelect"
  >
    <NButton size="small" quaternary circle :title="'主题：' + themeMode">
      <template #icon>
        <NIcon>
          <component :is="triggerIcon" />
        </NIcon>
      </template>
    </NButton>
  </NDropdown>
</template>
