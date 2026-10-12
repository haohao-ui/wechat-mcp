#!/usr/bin/env bash
# wechat-mcp 一键安装（macOS）
#
#   公开仓库：
#   curl -fsSL https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash
#   私有仓库（token 需 repo / contents:read 权限）：
#   export GITHUB_TOKEN=ghp_xxx
#   curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash
#
# 可选环境变量：
#   WECHAT_MCP_VERSION=v0.1.0     指定版本（默认最新 Release）
#   WECHAT_MCP_HOME=~/.wechat-mcp 安装目录
#   WECHAT_MCP_FROM_SOURCE=1      跳过 Release，直接从源码构建（需要 Rust 与 Xcode 命令行工具）
#   WECHAT_MCP_NO_REGISTER=1      不自动注册到 Claude Code
#   GITHUB_TOKEN=...              私有仓库下载用
# 卸载：curl -fsSL .../install.sh | bash -s -- --uninstall
set -euo pipefail

REPO="haohao-ui/wechat-mcp"
INSTALL_HOME="${WECHAT_MCP_HOME:-$HOME/.wechat-mcp}"
BIN_DIR="$INSTALL_HOME/bin"
CONFIG="$INSTALL_HOME/gateway.json"
TOOLS=(wechat-mcp wechat_ocr wechat_click wechat_window)

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m警告:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31m错误:\033[0m %s\n' "$*" >&2; exit 1; }

[[ "$(uname -s)" == "Darwin" ]] || die "wechat-mcp 仅支持 macOS"

if [[ "${1:-}" == "--uninstall" ]]; then
  if command -v claude >/dev/null 2>&1; then
    claude mcp remove wechat --scope user >/dev/null 2>&1 || true
  fi
  rm -rf "$BIN_DIR"
  info "已删除 ${BIN_DIR}，并移除 Claude Code 中的 wechat 服务"
  info "配置 $CONFIG 与客服数据库已保留，如需彻底清除请手动删除"
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

curl_gh() {
  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" "$@"
  else
    curl -fsSL "$@"
  fi
}

# 从 release JSON 中取出指定资产的 API 地址（私有仓库也可用 token 下载）
asset_api_url() {
  /usr/bin/osascript -l JavaScript - "$1" "$2" <<'JS'
function run(argv) {
  const release = JSON.parse($.NSString.stringWithContentsOfFileEncodingError(argv[0], $.NSUTF8StringEncoding, null).js);
  const asset = (release.assets || []).find(a => a.name === argv[1]);
  return asset ? asset.url : "";
}
JS
}

download_asset() {
  local url
  url="$(asset_api_url "$TMP/release.json" "$1")"
  [[ -n "$url" ]] || return 1
  curl_gh -H "Accept: application/octet-stream" -o "$TMP/$1" "$url"
}

install_from_release() {
  local endpoint="releases/latest"
  [[ -n "${WECHAT_MCP_VERSION:-}" ]] && endpoint="releases/tags/$WECHAT_MCP_VERSION"
  curl_gh -o "$TMP/release.json" "https://api.github.com/repos/$REPO/$endpoint" 2>/dev/null || return 1
  local tag
  tag="$(sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' "$TMP/release.json" | head -1)"
  [[ -n "$tag" ]] || return 1

  local name="wechat-mcp-${tag}-macos-universal"
  info "下载 $tag 预编译版本"
  download_asset "$name.tar.gz" || return 1
  download_asset "$name.tar.gz.sha256" || return 1

  info "校验 SHA-256"
  local expected actual
  expected="$(awk '{print $1}' "$TMP/$name.tar.gz.sha256")"
  actual="$(shasum -a 256 "$TMP/$name.tar.gz" | awk '{print $1}')"
  [[ "$expected" == "$actual" ]] || die "校验失败，文件可能被篡改（期望 ${expected}，实际 ${actual}）"

  tar -C "$TMP" -xzf "$TMP/$name.tar.gz"
  SRC_DIR="$TMP/$name"
  EXAMPLE_CONFIG="$SRC_DIR/gateway.example.json"
}

install_from_source() {
  command -v git >/dev/null || die "需要 git"
  command -v cargo >/dev/null || die "需要 Rust（https://rustup.rs）"
  command -v swiftc >/dev/null || die "需要 Swift 编译器：xcode-select --install"
  info "从源码构建（首次约需几分钟）"
  local url="https://github.com/$REPO.git"
  [[ -n "${GITHUB_TOKEN:-}" ]] && url="https://x-access-token:${GITHUB_TOKEN}@github.com/$REPO.git"
  git clone --depth 1 ${WECHAT_MCP_VERSION:+--branch "$WECHAT_MCP_VERSION"} "$url" "$TMP/src" >/dev/null 2>&1 \
    || die "克隆仓库失败（私有仓库请设置 GITHUB_TOKEN）"
  (cd "$TMP/src" && bash scripts/build.sh)
  SRC_DIR="$TMP/src/target/release"
  EXAMPLE_CONFIG="$TMP/src/gateway.example.json"
}

if [[ "${WECHAT_MCP_FROM_SOURCE:-}" == "1" ]] || ! install_from_release; then
  [[ "${WECHAT_MCP_FROM_SOURCE:-}" == "1" ]] || warn "未找到可用的 Release，改为从源码构建"
  install_from_source
fi

info "安装到 $BIN_DIR"
mkdir -p "$BIN_DIR"
for t in "${TOOLS[@]}"; do
  [[ -f "$SRC_DIR/$t" ]] || die "安装包缺少 $t"
  install -m 755 "$SRC_DIR/$t" "$BIN_DIR/$t"
done
xattr -dr com.apple.quarantine "$BIN_DIR" 2>/dev/null || true

if [[ -f "$CONFIG" ]]; then
  info "保留已有配置 $CONFIG"
else
  install -m 600 "$EXAMPLE_CONFIG" "$CONFIG"
  info "已生成默认配置 $CONFIG"
fi

if [[ "${WECHAT_MCP_NO_REGISTER:-}" != "1" ]] && command -v claude >/dev/null 2>&1; then
  claude mcp remove wechat --scope user >/dev/null 2>&1 || true
  if claude mcp add wechat --scope user -e "MESSAGE_GATEWAY_CONFIG=$CONFIG" -- "$BIN_DIR/wechat-mcp" >/dev/null; then
    info "已注册到 Claude Code（用户级 MCP 服务：wechat）"
  else
    warn "注册到 Claude Code 失败，请参考下方配置手动添加"
  fi
fi

cat <<EOF

✅ 安装完成：$BIN_DIR/wechat-mcp

其他 MCP 客户端（Claude Desktop / Cursor 等）配置：
{
  "mcpServers": {
    "wechat": {
      "command": "$BIN_DIR/wechat-mcp",
      "env": { "MESSAGE_GATEWAY_CONFIG": "$CONFIG" }
    }
  }
}

首次使用前请在「系统设置 → 隐私与安全性」中为运行 MCP 的宿主程序（终端 / Claude 等）授权：
  • 辅助功能   • 屏幕录制   • 自动化（控制 WeChat / System Events）
并确保微信已启动并登录。
EOF
