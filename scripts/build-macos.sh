#!/usr/bin/env bash
# FileShare · macOS Universal 打包脚本
# 产物: src-tauri/target/universal-apple-darwin/release/bundle/
#        ├─ macos/FileShare.app
#        └─ dmg/FileShare_<version>_universal.dmg
#
# 用法（在 macOS 机器上仓库根目录执行）：
#   bash scripts/build-macos.sh
#
# 前置：
#   1. Xcode Command Line Tools:  xcode-select --install
#   2. rustup + Node.js ≥ 20 + pnpm ≥ 9
#   3. 第一次会自动给 rustup 加两个目标（x86_64-apple-darwin / aarch64-apple-darwin）
#
# 已知坑：
#   - 沙箱环境（某些 CI/代理）下 hdiutil 创建 DMG 会被拦截。如在本地终端直接执行则无此问题。
#   - 若 TAURI_BUNDLER_TOOLS_GITHUB_MIRROR 未设且国内网络慢，首次下载 dmg-license 等工具可能超时。

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "==> 工作目录: $REPO_ROOT"

# --- 平台校验 ---
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "❌ 此脚本仅能在 macOS 上运行（当前: $(uname -s)）"
  exit 1
fi

# --- 工具链校验 ---
command -v cargo  >/dev/null 2>&1 || { echo "❌ 未找到 cargo，请先装 Rust (https://rustup.rs)"; exit 1; }
command -v pnpm   >/dev/null 2>&1 || { echo "❌ 未找到 pnpm，请执行 'npm i -g pnpm' 或 'corepack enable'"; exit 1; }
command -v node   >/dev/null 2>&1 || { echo "❌ 未找到 node"; exit 1; }

# --- Rust targets ---
echo "==> 确认 rustup targets..."
rustup target add aarch64-apple-darwin  >/dev/null
rustup target add x86_64-apple-darwin   >/dev/null

# --- 前端依赖 ---
if [[ ! -d node_modules ]]; then
  echo "==> pnpm install..."
  pnpm install --frozen-lockfile
fi

# --- 打包 ---
# 本地/CI 默认关闭签名；若后续需要签名，改为直接调用 tauri build 并提供证书环境。
echo "==> pnpm tauri build --target universal-apple-darwin --no-sign"
pnpm tauri build --target universal-apple-darwin --no-sign

BUNDLE_DIR="$REPO_ROOT/src-tauri/target/universal-apple-darwin/release/bundle"
if [[ -d "$BUNDLE_DIR" ]]; then
  echo
  echo "✅ 打包完成，产物路径："
  find "$BUNDLE_DIR" -maxdepth 3 -type f \( -name "*.dmg" -o -name "*.app.tar.gz" \) -print
  find "$BUNDLE_DIR" -maxdepth 2 -type d -name "*.app" -print
else
  echo "⚠️ 未找到 $BUNDLE_DIR，请检查上方 tauri build 日志"
  exit 1
fi
