import type { ShareFile, ShareText } from "../types";
import { appendTokenToUrl, authedFetch, authHeaders } from "./auth";

/** 获取当前页面所在服务器地址（浏览器访客） */
export function baseUrl(): string {
  return `${window.location.protocol}//${window.location.host}`;
}

export interface ServerInfoResp {
  name: string;
  version: string;
  passwordRequired: boolean;
  httpsEnabled: boolean;
  startedAt: number;
  maxUploadSize: number;
}

export interface ListResp {
  files: ShareFile[];
  texts: ShareText[];
}

export async function fetchInfo(base = ""): Promise<ServerInfoResp> {
  // 公开接口，不需要 token
  const res = await fetch(`${base || baseUrl()}/api/info`);
  if (!res.ok) throw new Error(`info ${res.status}`);
  return res.json();
}

export async function fetchList(base = ""): Promise<ListResp> {
  const res = await authedFetch(`${base || baseUrl()}/api/list`, { cache: "no-store" });
  if (!res.ok) throw new Error(`list ${res.status}`);
  return res.json();
}

export async function postText(content: string, base = ""): Promise<ShareText> {
  const res = await authedFetch(`${base || baseUrl()}/api/text`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ content }),
  });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}

export async function deleteFile(id: string, ownerToken: string, base = ""): Promise<void> {
  // Host 删除：传 owner_token（覆盖 localStorage 里的访客 token）
  const res = await fetch(`${base || baseUrl()}/api/file/${encodeURIComponent(id)}`, {
    method: "DELETE",
    headers: { Authorization: `Bearer ${ownerToken}` },
  });
  if (!res.ok && res.status !== 204) throw new Error(`delete ${res.status}`);
}

export async function deleteText(id: string, ownerToken: string, base = ""): Promise<void> {
  const res = await fetch(`${base || baseUrl()}/api/text/${encodeURIComponent(id)}`, {
    method: "DELETE",
    headers: { Authorization: `Bearer ${ownerToken}` },
  });
  if (!res.ok && res.status !== 204) throw new Error(`delete ${res.status}`);
}

/** 下载链接：`?token=` 注入（浏览器 `<a>` / location 无法带 header） */
export function downloadUrl(id: string, base = ""): string {
  return appendTokenToUrl(`${base || baseUrl()}/api/file/${encodeURIComponent(id)}`);
}

// 给 upload XHR 用的 header 辅助
export { authHeaders };

export interface UploadTask {
  file: File;
  progress: number; // 0-100
  loaded: number;
  total: number;
  status: "pending" | "uploading" | "success" | "error";
  error?: string;
  abort?: () => void;
}

/**
 * 使用 XHR 上传单个文件（fetch 无法精确报告上传进度）
 */
export function uploadFileXhr(
  file: File,
  opts: {
    base?: string;
    onProgress?: (loaded: number, total: number) => void;
  } = {},
): { promise: Promise<ShareFile>; abort: () => void } {
  const xhr = new XMLHttpRequest();
  const form = new FormData();
  form.append("file", file, file.name);

  const url = `${opts.base || baseUrl()}/api/upload`;
  const promise = new Promise<ShareFile>((resolve, reject) => {
    xhr.open("POST", url, true);
    const headers = authHeaders();
    for (const [k, v] of Object.entries(headers)) xhr.setRequestHeader(k, v);
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && opts.onProgress) {
        opts.onProgress(e.loaded, e.total);
      }
    };
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          const data = JSON.parse(xhr.responseText) as { uploaded: ShareFile[] };
          if (data.uploaded && data.uploaded.length > 0) {
            resolve(data.uploaded[0]);
          } else {
            reject(new Error("服务端未返回上传结果"));
          }
        } catch (e) {
          reject(new Error("解析响应失败"));
        }
      } else {
        reject(new Error(xhr.responseText || `HTTP ${xhr.status}`));
      }
    };
    xhr.onerror = () => reject(new Error("网络错误"));
    xhr.onabort = () => reject(new Error("已取消"));
    xhr.send(form);
  });

  return { promise, abort: () => xhr.abort() };
}
