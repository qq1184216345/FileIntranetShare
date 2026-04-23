/**
 * 文本分享的共享工具：超长文本转 .txt 文件上传。
 */

/** 超过此字符数时提示用户转成 .txt 文件上传（中英文统一按 String.length 计数） */
export const TEXT_SHARE_MAX_LEN = 2000;

/** 根据文本首行生成一个合规的 txt 文件名（不含路径）。 */
export function deriveTxtFilename(content: string, maxChars = 20): string {
  const firstLine = content.split(/\r?\n/).find((l) => l.trim().length > 0) ?? "";
  // 去掉 Windows/类 Unix 上文件名的非法字符 + 控制字符，再 trim
  const cleaned = firstLine
    // eslint-disable-next-line no-control-regex
    .replace(/[\\/:*?"<>|\x00-\x1f]/g, "")
    .trim();
  const truncated = cleaned.slice(0, maxChars).trim();
  const base = truncated.length > 0 ? truncated : "文本分享";
  return `${base}.txt`;
}

/** 把一段文本封装成 File（text/plain; charset=utf-8），供走分片上传流程。 */
export function textToTxtFile(content: string): File {
  const filename = deriveTxtFilename(content);
  const blob = new Blob([content], { type: "text/plain;charset=utf-8" });
  return new File([blob], filename, { type: "text/plain;charset=utf-8" });
}
