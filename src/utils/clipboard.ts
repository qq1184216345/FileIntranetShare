/**
 * 通用文本复制，兼容手机浏览器在非安全上下文（http 局域网）下的复制需求。
 *
 * 策略：
 * 1. 优先用 `navigator.clipboard.writeText`（需要 secure context：https 或 localhost）；
 * 2. 不可用或被拒时，回退到 `document.execCommand('copy')` + 隐藏 textarea。
 *    - 这是老式 API，仍被全部主流浏览器支持，**不要求 https**。
 *    - iOS Safari 对元素可见性有要求，这里用极小尺寸 + readonly 规避虚拟键盘。
 *
 * 注意：必须在用户手势回调（click/touch）内调用，否则被浏览器拦截。
 */
export async function copyToClipboard(text: string): Promise<boolean> {
  if (text == null) return false;

  if (
    typeof navigator !== "undefined" &&
    navigator.clipboard &&
    typeof window !== "undefined" &&
    window.isSecureContext
  ) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // 某些场景（权限被拒、iframe 跨域、focus 丢失）会 reject，继续走 fallback
    }
  }

  return execCommandCopy(String(text));
}

function execCommandCopy(text: string): boolean {
  if (typeof document === "undefined") return false;

  const ta = document.createElement("textarea");
  ta.value = text;
  ta.setAttribute("readonly", "");
  // iOS Safari 要求元素真实在 DOM 且可见；用极小尺寸 + 透明 + 不可交互避免视觉跳动
  ta.style.position = "fixed";
  ta.style.top = "0";
  ta.style.left = "0";
  ta.style.width = "1px";
  ta.style.height = "1px";
  ta.style.padding = "0";
  ta.style.border = "none";
  ta.style.outline = "none";
  ta.style.boxShadow = "none";
  ta.style.background = "transparent";
  ta.style.opacity = "0";
  ta.style.pointerEvents = "none";
  document.body.appendChild(ta);

  // 保存用户原选区，复制完复原
  const sel = document.getSelection();
  const saved =
    sel && sel.rangeCount > 0 ? sel.getRangeAt(0).cloneRange() : null;

  let ok = false;
  try {
    ta.focus({ preventScroll: true });
    ta.select();
    ta.setSelectionRange(0, ta.value.length);
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  } finally {
    document.body.removeChild(ta);
    if (saved && sel) {
      sel.removeAllRanges();
      sel.addRange(saved);
    }
  }
  return ok;
}
