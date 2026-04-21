import { createRouter, createWebHashHistory, createWebHistory, type RouterHistory } from "vue-router";
import HostIndex from "../views/Host/Index.vue";
import GuestIndex from "../views/Guest/Index.vue";

// Tauri 窗口使用 hash 路由，浏览器访客端使用 history 路由
// 我们统一使用 hash，简单可靠；访客端默认路径 "/" 当未在 Tauri 环境时走 Guest
const isTauri = typeof (window as any).__TAURI_INTERNALS__ !== "undefined";

const history: RouterHistory = isTauri ? createWebHashHistory() : createWebHistory();

export const router = createRouter({
  history,
  routes: [
    {
      path: "/",
      name: "host",
      component: HostIndex,
      meta: { requiresTauri: true },
    },
    {
      path: "/s",
      name: "guest",
      component: GuestIndex,
    },
    {
      path: "/:pathMatch(.*)*",
      redirect: isTauri ? "/" : "/s",
    },
  ],
});

router.beforeEach((to) => {
  if (!isTauri && to.meta.requiresTauri) {
    return { name: "guest" };
  }
});
