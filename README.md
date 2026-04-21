# FileShare · 局域网文件共享

> 基于 **Tauri 2 + Vue 3 + Rust (axum)** 的桌面端局域网文件 / 文本共享工具。
> 同一工程内既是桌面壳（主机端），也内嵌 HTTP 服务并分发 H5（访客端）。

## 功能

- [x] 内嵌 HTTP / WebSocket 服务，局域网浏览器免安装访问
- [x] 上传文件、分享文本；Owner 凭 `owner_token` 删除，访客只读可复制 / 下载
- [x] 服务自启 / 端口 / 上传目录 / 密码配置持久化（`tauri-plugin-store`）
- [x] 多网卡枚举 + IPv4 / IPv6 切换 + 二维码快速连接
- [x] WebSocket 实时同步：任意端增删文件 / 文本，所有在线设备即时刷新
- [x] 访客端大文件分片上传，支持断点续传（client 侧 localStorage 记 uploadId）
- [x] 密码保护：Argon2 哈希 + JWT（header / query 双通道），设置页改密码 **热生效**、在线访客被踢
- [x] 系统托盘常驻：左键弹主窗、右键菜单启停服务 / 打开分享链接 / 退出
- [x] 剪贴板一键分享：Host 点按钮把剪贴板图片（自动编码 PNG）或文本加到列表
- [x] 记录持久化：SQLite（`rusqlite` bundled）存储 files / texts，重启后下载链接继续可用，物理文件缺失自动 reconcile
- [ ] 打包发布（进行中）
- [ ] 暗色主题 / i18n

## 技术栈

| 层 | 选型 |
|---|---|
| 桌面壳 | Tauri 2 (Windows / macOS / Linux) |
| 前端 | Vue 3 + Vite + TypeScript + Naive UI + Pinia + Vue Router |
| HTTP / WS | Rust axum 0.7 + tokio + tower-http |
| 鉴权 | argon2 + jsonwebtoken (HS256) |
| 持久化 | tauri-plugin-store (JSON 配置) + rusqlite (分享记录) |
| 图片编码 | image 0.25（仅 png feature）|

## 开发环境要求

- Node.js ≥ 20
- pnpm ≥ 9
- Rust stable ≥ 1.77（Tauri 2 要求）
- Windows：**WebView2 Runtime**（Win11 已内置）+ **Build Tools for Visual Studio**（MSVC + Windows SDK）
- macOS：Xcode Command Line Tools
- Linux：`libwebkit2gtk-4.1-dev` 等（见 [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)）

### 安装 Rust

```powershell
# Windows (winget)
winget install --id Rustlang.Rustup -e
# 或
Invoke-WebRequest -Uri https://win.rustup.rs/x86_64 -OutFile rustup-init.exe; .\rustup-init.exe -y
```

验证：`cargo --version` 与 `rustc --version`

## 开发运行

```bash
pnpm install
pnpm tauri:dev      # 开发模式（首次会编译 Rust，较慢）
```

仅前端调试（无 Rust，适合改 UI）：

```bash
pnpm dev
# 打开 http://localhost:1420   →  自动跳转访客端 /s
# 打开 http://localhost:1420/#/ → 也可以在浏览器里预览 Host UI（功能受限）
```

## 打包分发

### 出 Windows NSIS 安装包

```powershell
pnpm tauri:build
```

产物位置：

```
src-tauri/target/release/bundle/nsis/FileShare_0.1.0_x64-setup.exe
```

| 项 | 说明 |
|---|---|
| **格式** | NSIS（`tauri.conf.json` 里 `bundle.targets = ["nsis"]`）|
| **安装模式** | `currentUser`：不需要管理员，装到用户目录下的 `%LocalAppData%\Programs\FileShare\` |
| **体积** | 首次打包约 8–12 MB（不含 WebView2） |
| **WebView2** | 使用 `downloadBootstrapper`，安装时若系统未装会自动拉；Win11 已内置可忽略 |
| **中文** | NSIS 语言默认简中，不弹语言选择框 |

首次启动服务时 Windows 防火墙会弹一次 UAC（用 `netsh` 写入规则 `FileShare-HTTP`），之后不会再问。

### 出 MSI（可选）

```
# 临时切 targets
pnpm tauri build --bundles msi
```

需要额外下载 WiX Toolset（Tauri CLI 首次会自动处理）。

### 代码签名（可选，未启用）

Windows 用户首次运行未签名 exe 时 SmartScreen 会提示"未识别的发布者"，点「更多信息 → 仍要运行」即可。
若要消除该提示需购买 EV / OV 代码签名证书并在 `tauri.conf.json` 的 `bundle.windows` 下配置 `certificateThumbprint` / `digestAlgorithm` / `timestampUrl`。

### 替换应用图标

Tauri CLI 自带批量生成工具，一条命令搞定所有尺寸（ico / icns / png 全套）：

```bash
# 准备一张 1024×1024 的 PNG（最好透明背景）
pnpm tauri icon path/to/source.png
```

会覆盖写入 `src-tauri/icons/` 下所有图标。若想换产品名，直接改 `tauri.conf.json` 的 `productName` 即可。

### 版本号

`package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` 三处版本号需同步更新。

## 目录结构

```
.
├─ src/                        # Vue 前端
│  ├─ api/                     # 调用 Tauri 命令 / HTTP
│  │  ├─ auth.ts               # JWT token 注入 + AUTH_EVENT 事件
│  │  ├─ chunk-upload.ts       # 分片上传 + 续传
│  │  ├─ guest.ts              # 访客 HTTP API
│  │  └─ host.ts               # Tauri 命令包装（startServer / shareClipboard ...）
│  ├─ composables/useSync.ts   # WebSocket 实时同步
│  ├─ router/                  # / → Host，/s → Guest
│  ├─ stores/                  # Pinia：config + server
│  └─ views/
│     ├─ Host/                 # 桌面端 UI
│     └─ Guest/                # 浏览器访客端 UI
├─ src-tauri/
│  ├─ src/
│  │  ├─ lib.rs                # 入口：托盘 / 插件 / 命令注册
│  │  ├─ commands.rs           # Tauri 命令（startServer / refreshServerAuth / shareClipboard ...）
│  │  ├─ config.rs             # AppConfig
│  │  ├─ net.rs                # 网卡枚举
│  │  ├─ tray.rs               # 系统托盘
│  │  └─ server/
│  │     ├─ mod.rs             # axum 启停
│  │     ├─ auth.rs            # argon2 + JWT
│  │     ├─ files.rs           # Registry（内存 + SQLite）
│  │     ├─ state.rs           # AppState + SyncEvent
│  │     ├─ upload.rs          # 分片上传会话管理
│  │     ├─ firewall.rs        # Windows 防火墙规则
│  │     └─ routes.rs          # 所有 HTTP / WS 端点
│  ├─ capabilities/default.json
│  ├─ icons/                   # 应用图标（可用 `pnpm tauri icon` 批量替换）
│  └─ tauri.conf.json
└─ README.md
```

## HTTP / WS 接口

| Method | Path | 权限 | 说明 |
|---|---|---|---|
| GET  | `/api/info`          | 公开      | 返回是否需要密码、限额等 |
| POST | `/api/login`         | 公开      | 密码换 JWT（7 天有效）|
| GET  | `/api/list`          | visitor   | 文件 + 文本列表 |
| GET  | `/api/file/:id`      | visitor   | 下载（支持 Range 断点续传）|
| POST | `/api/upload`        | visitor   | multipart 小文件（legacy）|
| POST | `/api/upload/init`   | visitor   | 初始化分片上传会话 |
| GET  | `/api/upload/:id`    | visitor   | 查询 / 恢复会话 |
| PUT  | `/api/upload/:id/chunk/:index` | visitor | 上传分片（支持并发、乱序、重传）|
| POST | `/api/upload/:id/complete`     | visitor | 合并分片 |
| DELETE | `/api/upload/:id`  | visitor   | 取消并清理临时分片 |
| POST | `/api/text`          | visitor   | 分享文本 |
| DELETE | `/api/file/:id`    | owner     | 删除文件 |
| DELETE | `/api/text/:id`    | owner     | 删除文本 |
| GET  | `/api/sync`          | visitor   | WebSocket：fileAdded / textAdded / fileRemoved / textRemoved / cleared |
| GET  | `/`                  | 公开      | 访客端 SPA |

鉴权两种方式共存：`Authorization: Bearer <token>` header，或 `?token=xxx` query（用于 `<a download>` 与 WebSocket）。
owner 用固定 `owner_token`（服务启动时 nanoid(32) 随机生成），visitor 用 JWT。密码改动会轮换 JWT 签名密钥，所有在线访客被踢下线。

## 持久化数据位置

- **配置**（`AppConfig`）：`%AppData%\com.fileshare.app\store.json`
- **记录数据库**（文件 + 文本）：`%AppData%\com.fileshare.app\fileshare.db`
- **上传实体文件**：用户在设置中指定的 `uploadDir`

> 切换 `uploadDir` 不会迁移旧文件；旧记录的下载链接仍指向原目录文件，新上传落到新目录。

## 常见问题

### `Port 1420 is already in use`

上一次 `pnpm dev` / `pnpm tauri:dev` 非正常退出，遗留 node / Vite 进程占用端口：

```powershell
Get-NetTCPConnection -LocalPort 1420 -ErrorAction SilentlyContinue |
  Select-Object -ExpandProperty OwningProcess -Unique |
  ForEach-Object { Stop-Process -Id $_ -Force }
```

### 首次 `pnpm tauri:dev` 很慢

首次会编译 600+ 个 Rust crate，其中 `rusqlite`（bundled）单独要编 sqlite3.c 约 1–2 分钟，首编合计 **5–10 分钟**。之后增量编译 5–10 秒即可。`target/` 编译产物约 2 GB，已在 `.gitignore`。

### 同网段设备打不开 `http://<本机IP>:端口`

- 首次启动服务时 Windows 会弹 UAC 写入防火墙规则；若当时拒绝了，可在「控制面板 → Windows Defender 防火墙 → 允许应用」里找到 `FileShare-HTTP` 勾选公共 / 专用
- 或重启服务重新触发 UAC 弹窗
- 检查对端是否与本机在同一子网、路由器开启了"AP 隔离"的话也会互相不通

### SmartScreen 警告

未签名 exe 正常现象，点「更多信息 → 仍要运行」。见上文「代码签名」。

## License

MIT
