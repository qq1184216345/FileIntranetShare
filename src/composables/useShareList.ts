import { ref } from "vue";
import type { ShareFile, ShareText } from "../types";
import type { SyncEvent } from "./useSync";

/**
 * 分享列表状态 + 事件合并。
 *
 * 把 Host/ShareList 和 Guest/GuestMain 原本各自手写的
 * `files.value = [ev.file, ...files.value]` 改为:
 *  - `files.value.unshift(ev.file)` —— 就地修改，避免 O(n) 拷贝 (P2-12)
 *  - O(1) id 重复检测 —— 用 Map 替代 `.some(...)` 的 O(n) 线性扫描
 *
 * 调用方只需：传入一个 refresh 用的 fetchList 函数，然后把 useSync 的事件
 * 转给 applyEvent 即可。
 */
export function useShareList(fetcher: () => Promise<{ files: ShareFile[]; texts: ShareText[] }>) {
  const files = ref<ShareFile[]>([]);
  const texts = ref<ShareText[]>([]);

  // 快速 id -> index 查表，保持与数组同步
  const fileIdx = new Map<string, number>();
  const textIdx = new Map<string, number>();

  function rebuildIndex() {
    fileIdx.clear();
    textIdx.clear();
    files.value.forEach((f, i) => fileIdx.set(f.id, i));
    texts.value.forEach((t, i) => textIdx.set(t.id, i));
  }

  async function refresh() {
    try {
      const data = await fetcher();
      files.value = data.files;
      texts.value = data.texts;
      rebuildIndex();
    } catch (e) {
      console.warn("[useShareList] refresh failed:", e);
    }
  }

  function applyEvent(ev: SyncEvent) {
    switch (ev.type) {
      case "hello":
        // 握手时主动拉一次；"resync" / "cleared" 同理
        refresh();
        break;
      case "fileAdded":
        if (!fileIdx.has(ev.file.id)) {
          files.value.unshift(ev.file);
          rebuildIndex();
        }
        break;
      case "fileRemoved": {
        const i = fileIdx.get(ev.id);
        if (i !== undefined) {
          files.value.splice(i, 1);
          rebuildIndex();
        }
        break;
      }
      case "textAdded":
        if (!textIdx.has(ev.text.id)) {
          texts.value.unshift(ev.text);
          rebuildIndex();
        }
        break;
      case "textRemoved": {
        const i = textIdx.get(ev.id);
        if (i !== undefined) {
          texts.value.splice(i, 1);
          rebuildIndex();
        }
        break;
      }
      case "resync":
      case "cleared":
        refresh();
        break;
      case "authInvalid":
        // 由 useSync 统一处理（清 token、跳登录），这里无事可做
        break;
    }
  }

  return {
    files,
    texts,
    refresh,
    applyEvent,
  };
}
