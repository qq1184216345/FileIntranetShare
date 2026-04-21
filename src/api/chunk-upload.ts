import type { ShareFile } from "../types";
import { authHeaders, authedFetch } from "./auth";
import { baseUrl } from "./guest";

/** 默认分片 4MB，与后端 DEFAULT_CHUNK_SIZE 对齐 */
const DEFAULT_CHUNK_SIZE = 4 * 1024 * 1024;

interface InitResp {
  uploadId: string;
  chunkSize: number;
  chunkCount: number;
  uploaded: number[];
  resumed: boolean;
}

interface ChunkResp {
  uploaded: number[];
  received: number;
  complete: boolean;
}

export type ChunkedStatus =
  | "pending"
  | "uploading"
  | "paused"
  | "success"
  | "error"
  | "cancelled";

export interface ChunkedUploaderOptions {
  base?: string;
  chunkSize?: number;
  onProgress?: (loaded: number, total: number) => void;
  onStatus?: (status: ChunkedStatus) => void;
  onInit?: (uploadId: string, resumed: boolean) => void;
}

/**
 * 分片上传控制器：支持暂停 / 继续 / 取消 / 服务端续传
 */
export class ChunkedUploader {
  readonly file: File;
  private opts: ChunkedUploaderOptions;
  private base: string;

  uploadId = "";
  chunkSize = DEFAULT_CHUNK_SIZE;
  chunkCount = 0;
  uploaded = new Set<number>();
  status: ChunkedStatus = "pending";
  error?: string;

  private aborter?: AbortController;
  private currentXhr?: XMLHttpRequest;
  /** 最终合并后的服务端文件元数据 */
  private finalItem?: ShareFile;

  constructor(file: File, opts: ChunkedUploaderOptions = {}) {
    this.file = file;
    this.opts = opts;
    this.base = opts.base || baseUrl();
    this.chunkSize = opts.chunkSize || DEFAULT_CHUNK_SIZE;
  }

  /** 当前已完成字节数（用于进度显示） */
  loaded(): number {
    let bytes = 0;
    for (const i of this.uploaded) {
      if (i + 1 === this.chunkCount) {
        bytes += this.file.size - this.chunkSize * i;
      } else {
        bytes += this.chunkSize;
      }
    }
    return bytes;
  }

  private setStatus(s: ChunkedStatus) {
    this.status = s;
    this.opts.onStatus?.(s);
  }

  private reportProgress() {
    this.opts.onProgress?.(this.loaded(), this.file.size);
  }

  /** 启动上传；若已暂停过则等同于恢复 */
  async start(): Promise<ShareFile> {
    if (this.status === "success" && this.finalItem) return this.finalItem;
    this.error = undefined;
    this.setStatus("uploading");

    try {
      if (!this.uploadId) {
        await this.init();
      }
      this.reportProgress();
      await this.uploadLoop();
      if (this.status === "paused" || this.status === "cancelled") {
        throw new Error(this.status === "cancelled" ? "已取消" : "已暂停");
      }
      const item = await this.complete();
      this.finalItem = item;
      this.setStatus("success");
      return item;
    } catch (e: any) {
      if (this.status !== "paused" && this.status !== "cancelled") {
        this.error = e?.message || String(e);
        this.setStatus("error");
      }
      throw e;
    }
  }

  /** 暂停：中断当前块上传；保留 uploadId 以便续传 */
  pause() {
    if (this.status === "uploading") {
      this.setStatus("paused");
      this.currentXhr?.abort();
      this.aborter?.abort();
    }
  }

  /** 取消：中断并通知服务端清理 */
  async cancel() {
    this.setStatus("cancelled");
    this.currentXhr?.abort();
    this.aborter?.abort();
    if (this.uploadId) {
      try {
        await authedFetch(`${this.base}/api/upload/${encodeURIComponent(this.uploadId)}`, {
          method: "DELETE",
        });
      } catch {
        // 忽略：服务端可能已清理
      }
    }
  }

  // ========== 内部 ==========

  private async init() {
    // 优先尝试复用 localStorage 记录的 uploadId（GET 状态）
    const saved = readSavedId(this.file);
    if (saved) {
      try {
        const resp = await authedFetch(
          `${this.base}/api/upload/${encodeURIComponent(saved)}`,
          { cache: "no-store" },
        );
        if (resp.ok) {
          const data: InitResp = await resp.json();
          if (data.uploadId === saved) {
            this.applyInit(data);
            this.opts.onInit?.(data.uploadId, true);
            return;
          }
        }
      } catch {
        // 忽略：继续走 init 流程
      }
      clearSavedId(this.file);
    }

    const res = await authedFetch(`${this.base}/api/upload/init`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        name: this.file.name,
        size: this.file.size,
        mime: this.file.type || "application/octet-stream",
        chunkSize: this.chunkSize,
      }),
    });
    if (!res.ok) {
      throw new Error((await res.text()) || `init ${res.status}`);
    }
    const data: InitResp = await res.json();
    this.applyInit(data);
    saveId(this.file, data.uploadId);
    this.opts.onInit?.(data.uploadId, data.resumed);
  }

  private applyInit(data: InitResp) {
    this.uploadId = data.uploadId;
    this.chunkSize = data.chunkSize;
    this.chunkCount = data.chunkCount;
    this.uploaded = new Set(data.uploaded);
  }

  private async uploadLoop() {
    for (let i = 0; i < this.chunkCount; i++) {
      if (this.uploaded.has(i)) continue;
      if ((this.status as ChunkedStatus) === "paused" || (this.status as ChunkedStatus) === "cancelled") {
        return;
      }
      await this.sendChunk(i);
      this.uploaded.add(i);
      this.reportProgress();
    }
  }

  private sendChunk(index: number): Promise<void> {
    const start = index * this.chunkSize;
    const end = Math.min(start + this.chunkSize, this.file.size);
    const blob = this.file.slice(start, end);
    const baseLoaded = this.loaded();

    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      this.currentXhr = xhr;
      xhr.open(
        "POST",
        `${this.base}/api/upload/${encodeURIComponent(this.uploadId)}/chunk/${index}`,
        true,
      );
      xhr.setRequestHeader("Content-Type", "application/octet-stream");
      const headers = authHeaders();
      for (const [k, v] of Object.entries(headers)) xhr.setRequestHeader(k, v);
      xhr.upload.onprogress = (e) => {
        if (e.lengthComputable) {
          const loaded = Math.min(baseLoaded + e.loaded, this.file.size);
          this.opts.onProgress?.(loaded, this.file.size);
        }
      };
      xhr.onload = () => {
        this.currentXhr = undefined;
        if (xhr.status >= 200 && xhr.status < 300) {
          try {
            const data: ChunkResp = JSON.parse(xhr.responseText);
            for (const i of data.uploaded) this.uploaded.add(i);
            resolve();
          } catch {
            resolve();
          }
        } else {
          reject(new Error(xhr.responseText || `chunk ${xhr.status}`));
        }
      };
      xhr.onerror = () => {
        this.currentXhr = undefined;
        reject(new Error("网络错误"));
      };
      xhr.onabort = () => {
        this.currentXhr = undefined;
        reject(new Error("aborted"));
      };
      xhr.send(blob);
    });
  }

  private async complete(): Promise<ShareFile> {
    const res = await authedFetch(
      `${this.base}/api/upload/${encodeURIComponent(this.uploadId)}/complete`,
      { method: "POST" },
    );
    if (!res.ok) {
      throw new Error((await res.text()) || `complete ${res.status}`);
    }
    const data: { file: ShareFile } = await res.json();
    clearSavedId(this.file);
    return data.file;
  }
}

// ============ localStorage 续传键 ============

const LS_KEY = "fileshare.chunks.v1";
type SavedMap = Record<string, { uploadId: string; ts: number }>;

function sig(file: File): string {
  return `${file.name}::${file.size}::${file.lastModified}`;
}

function readMap(): SavedMap {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (!raw) return {};
    return JSON.parse(raw) as SavedMap;
  } catch {
    return {};
  }
}

function writeMap(m: SavedMap) {
  try {
    // 3 天之前的记录清理掉
    const now = Date.now();
    for (const k of Object.keys(m)) {
      if (now - m[k].ts > 3 * 24 * 3600 * 1000) delete m[k];
    }
    localStorage.setItem(LS_KEY, JSON.stringify(m));
  } catch {
    // 忽略配额错误
  }
}

function saveId(file: File, uploadId: string) {
  const m = readMap();
  m[sig(file)] = { uploadId, ts: Date.now() };
  writeMap(m);
}

function readSavedId(file: File): string | null {
  const m = readMap();
  return m[sig(file)]?.uploadId ?? null;
}

function clearSavedId(file: File) {
  const m = readMap();
  delete m[sig(file)];
  writeMap(m);
}
