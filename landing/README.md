# FileShare Landing Page

零依赖单文件静态页，拖到任何 Web 服务器的静态目录即可访问。

## 文件

- `index.html` · 唯一的入口文件，所有 CSS 均已内联
- 无外部 JS，无 CDN 依赖，纯净 HTML + CSS
- 自带暗色主题（跟随系统 `prefers-color-scheme`）
- 内置 favicon（data URI SVG，无外部请求）

## 部署方式任选一种

### 1) nginx

```nginx
server {
    listen 80;
    server_name fileshare.example.com;

    root /var/www/fileshare-landing;
    index index.html;

    location / {
        try_files $uri $uri/ /index.html;
    }
}
```

把 `index.html` 上传到 `/var/www/fileshare-landing/` 即可。建议再套一层 HTTPS（Let's Encrypt 或 Caddy 自动 HTTPS）。

### 2) Caddy（推荐，自动 HTTPS）

```
fileshare.example.com {
    root * /var/www/fileshare-landing
    file_server
}
```

一条命令 `caddy run` 就跑起来了，证书自动续。

### 3) 本地预览（零服务端）

```bash
cd landing
python -m http.server 8080
# 打开 http://localhost:8080
```

或用 `npx serve .` / `npx http-server` 之类。

## 版本号更新

页面里出现版本号的位置有三处（全部在 `index.html`）：

1. `<span class="tag">` 里的「当前稳定版 v0.1.2」
2. Windows 下载按钮的 `href`（指向具体 release 附件 URL）
3. "安装包约 2.79 MB" 的体积说明

发新版时 `Ctrl+F` 搜 `0.1.2` 全部替换即可。

## 备注

- **Linux 按钮当前是占位**：`data-soon` 样式 + 「即将支持」badge，`href` 兜底跳到 GitHub Releases 列表。等真的打出 `.AppImage` / `.deb` 之后，把它从 ghost 改成 primary、换上真实下载链接即可。
- 如果自己加截图：建议放到 `landing/screenshots/` 目录，在 `index.html` 里加一段 `<section>`，保持单文件哲学可以继续内联 `<img>` base64，但通常 PNG 建议外链以便 CDN 缓存。
