export interface AppConfig {
  autoStart: boolean;
  uploadDir: string;
  port: number;
  passwordEnabled: boolean;
  password: string;
  httpsEnabled: boolean;
  bindIpv6: boolean;
  /** 磁盘最小保留空间（MB）。上传时若剩余 < (文件大小 + 此值)，拒绝。0 为禁用。 */
  diskMinFreeMb: number;
}

export const DEFAULT_CONFIG: AppConfig = {
  autoStart: false,
  uploadDir: "",
  port: 18888,
  passwordEnabled: false,
  password: "",
  httpsEnabled: false,
  bindIpv6: false,
  diskMinFreeMb: 500,
};

export interface NetworkInterface {
  name: string;
  ip: string;
  isIpv6: boolean;
  isLoopback: boolean;
}

export interface ShareFile {
  id: string;
  name: string;
  size: number;
  mime: string;
  uploaderIp: string;
  createdAt: number;
  hash?: string;
}

export interface ShareText {
  id: string;
  content: string;
  uploaderIp: string;
  createdAt: number;
}

export type ShareItem =
  | ({ kind: "file" } & ShareFile)
  | ({ kind: "text" } & ShareText);

export interface ServerStatus {
  running: boolean;
  port: number;
  bindIpv6: boolean;
  ownerToken: string;
  startedAt: number;
}
