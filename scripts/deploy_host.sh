#!/usr/bin/env bash
set -euo pipefail

# Deploy 微信客服助手 host app, worker binaries, and bot to target Mac.
# Usage: scripts/deploy_host.sh <host> [--app] [--worker] [--bot] [--yes]

if [ $# -lt 1 ] || [[ "$1" == --* ]]; then
  echo "Usage: $0 <host> [--app] [--worker] [--bot] [--yes]" >&2
  exit 1
fi

HOST="$1"
shift

DEPLOY_APP=0
DEPLOY_WORKER=0
DEPLOY_BOT=0
DRY_RUN=1

while [ $# -gt 0 ]; do
  case "$1" in
    --app)
      DEPLOY_APP=1
      ;;
    --worker)
      DEPLOY_WORKER=1
      ;;
    --bot)
      DEPLOY_BOT=1
      ;;
    --yes)
      DRY_RUN=0
      ;;
    *)
      echo "Unknown option: $1" >&2
      echo "Usage: $0 <host> [--app] [--worker] [--bot] [--yes]" >&2
      exit 1
      ;;
  esac
  shift
done

# If none of the component flags were specified, deploy all three by default.
if [ "$DEPLOY_APP" -eq 0 ] && [ "$DEPLOY_WORKER" -eq 0 ] && [ "$DEPLOY_BOT" -eq 0 ]; then
  DEPLOY_APP=1
  DEPLOY_WORKER=1
  DEPLOY_BOT=1
fi

TIMESTAMP=$(date +%Y%m%d%H%M%S)
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

run_cmd() {
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "[dry-run] $*"
  else
    "$@"
  fi
}

remote_sh() {
  local cmd="$1"
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "[dry-run] ssh -o BatchMode=yes $HOST \"$cmd\""
  else
    ssh -o BatchMode=yes "$HOST" "$cmd"
  fi
}

remote_copy() {
  local src="$1"
  local dst="$2"
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "[dry-run] scp -r -q $src $HOST:$dst"
  else
    scp -r -q "$src" "$HOST:$dst"
  fi
}

echo "=== Deploying to host: $HOST (dry-run: $([ "$DRY_RUN" -eq 1 ] && echo "YES" || echo "NO")) ==="

if [ "$DEPLOY_APP" -eq 1 ]; then
  echo "--- Step: Deploy Host App (微信客服助手.app) ---"
  # 1. 首次部署时把旧 App 移至 ~/Applications/.old/
  remote_sh '
    mkdir -p "$HOME/Applications/.old"
    if [ -d "$HOME/Applications/WeChat RPA Test.app" ]; then
      echo "Archiving legacy WeChat RPA Test.app..."
      mv "$HOME/Applications/WeChat RPA Test.app" "$HOME/Applications/.old/WeChat RPA Test.app.'$TIMESTAMP'"
    fi
  '

  # 2. 上传源代码并在远端编译
  remote_sh 'mkdir -p "$HOME/.wechat-rpa/src/native/bootstrap"'
  remote_copy "$ROOT_DIR/native/bootstrap/"* "$HOME/.wechat-rpa/src/native/bootstrap/"

  # 远端编译与组装 App bundle
  remote_sh '
    cd "$HOME/.wechat-rpa/src"
    echo "Compiling host binary with swiftc on target Mac..."
    swiftc -O -target x86_64-apple-macos12 \
      native/bootstrap/Supervisor.swift \
      native/bootstrap/Probe.swift \
      native/bootstrap/SocketWorker.swift \
      native/bootstrap/main.swift \
      -o wechat-cs-host

    APP_DIR="$HOME/Applications/微信客服助手.app"
    if [ -d "$APP_DIR" ]; then
      echo "Backing up existing 微信客服助手.app..."
      cp -R "$APP_DIR" "$APP_DIR.before-'$TIMESTAMP'"
    fi

    mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
    mv wechat-cs-host "$APP_DIR/Contents/MacOS/wechat-cs-host"
    chmod 0755 "$APP_DIR/Contents/MacOS/wechat-cs-host"
    cp native/bootstrap/Info.plist "$APP_DIR/Contents/Info.plist"
    cp native/bootstrap/gateway.test.json "$APP_DIR/Contents/Resources/gateway.test.json"
  '

  # 3. 安装与配置 LaunchAgent
  remote_sh '
    PLIST_DST="$HOME/Library/LaunchAgents/local.wechat-mcp.host.plist"
    if [ -f "$PLIST_DST" ]; then
      cp "$PLIST_DST" "$PLIST_DST.before-'$TIMESTAMP'"
    fi
    mkdir -p "$HOME/Library/LaunchAgents" "$HOME/.wechat-rpa/logs"
    chmod 0700 "$HOME/.wechat-rpa/logs"

    sed "s|__HOME__|$HOME|g" "$HOME/.wechat-rpa/src/native/bootstrap/local.wechat-mcp.host.plist" > "$PLIST_DST"
    chmod 0600 "$PLIST_DST"

    # 重新加载服务
    launchctl bootout "gui/$UID/local.wechat-mcp.host" 2>/dev/null || true
    launchctl bootstrap "gui/$UID" "$PLIST_DST"
    launchctl kickstart -k "gui/$UID/local.wechat-mcp.host"
  '
fi

if [ "$DEPLOY_WORKER" -eq 1 ]; then
  echo "--- Step: Deploy Worker Binaries ---"
  remote_sh '
    mkdir -p "$HOME/.wechat-rpa/bin"
    chmod 0700 "$HOME/.wechat-rpa"
  '
  # 如果本地有 bin 文件则同步，并先备份旧文件
  if [ -d "$ROOT_DIR/bin" ]; then
    remote_sh '
      for f in "$HOME/.wechat-rpa/bin/"*; do
        if [ -f "$f" ]; then
          cp "$f" "$f.before-'$TIMESTAMP'"
        fi
      done
    '
    remote_copy "$ROOT_DIR/bin/"* "$HOME/.wechat-rpa/bin/"
    remote_sh 'chmod 0755 "$HOME/.wechat-rpa/bin/"*'
  fi
  # 重启桌面服务宿主以加载最新 worker
  remote_sh '
    launchctl kickstart -k "gui/$UID/local.wechat-mcp.host" 2>/dev/null || true
  '
fi

if [ "$DEPLOY_BOT" -eq 1 ]; then
  echo "--- Step: Deploy Bot Scripts & Launchd ---"
  remote_sh '
    BOT_DIR="$HOME/.wechat-rpa/bot"
    if [ -d "$BOT_DIR/scripts" ]; then
      echo "Backing up existing bot scripts..."
      cp -R "$BOT_DIR/scripts" "$BOT_DIR/scripts.before-'$TIMESTAMP'"
    fi
    mkdir -p "$BOT_DIR"
  '
  remote_copy "$ROOT_DIR/scripts" "$HOME/.wechat-rpa/bot/"

  remote_sh '
    # 重启机器人 launchd
    launchctl kickstart -k "gui/$UID/local.wechat-rpa.bot" 2>/dev/null || true
  '
fi

echo "=== Deployment $([ "$DRY_RUN" -eq 1 ] && echo "dry-run completed" || echo "succeeded") ==="
