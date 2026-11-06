#!/usr/bin/env bash
# Deploy to the dedicated WeChat Mac.
# Usage: scripts/deploy_host.sh <ssh-host> [--app] [--worker] [--mcp] [--bot] [--yes]
#   No part flags = all parts. Without --yes nothing runs: every step is only printed (dry-run).
#   --app     build 微信客服助手.app on the target, ad-hoc sign it, install the LaunchAgent, and retire WeChat RPA Test.app
#   --worker  build wechat_bot_worker and wechat_click on the target
#   --mcp     cross-build wechat-mcp here (x86_64, macOS 12) and copy it
#   --bot     copy scripts/ to ~/.wechat-rpa/bot/scripts (the bot's own launchd is set up by the migration step)
# Every replaced file is backed up as <name>.before-<timestamp>. Processes are never killed:
# helpers start per connection and pick up the new binaries on the next connect; the app restarts via launchctl.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() { echo "Usage: $0 <ssh-host> [--app] [--worker] [--mcp] [--bot] [--yes]" >&2; exit 2; }
[ $# -ge 1 ] || usage
HOST=$1; shift
APP=0; WORKER=0; MCP=0; BOT=0; YES=0
for arg in "$@"; do
  case "$arg" in
    --app) APP=1 ;; --worker) WORKER=1 ;; --mcp) MCP=1 ;; --bot) BOT=1 ;; --yes) YES=1 ;;
    *) usage ;;
  esac
done
if [ $((APP+WORKER+MCP+BOT)) -eq 0 ]; then APP=1; WORKER=1; MCP=1; BOT=1; fi
case "$HOST" in -*|*" "*) usage ;; esac
TS=$(date +%s)

# remote <description> <script>: the script runs in the remote login shell (remote $HOME, remote $UID).
remote() {
  echo "--- $1"
  if [ $YES -eq 1 ]; then ssh -o BatchMode=yes -o LogLevel=ERROR "$HOST" "bash -s" <<<"set -euo pipefail; TS=$TS; $2"
  else printf '[dry-run] ssh %s <<EOF\n%s\nEOF\n' "$HOST" "$2"; fi
}
# local_step <description> <command...>
local_step() {
  echo "--- $1"
  if [ $YES -eq 1 ]; then "${@:2}"; else echo "[dry-run] ${*:2}"; fi
}

echo "=== deploy to $HOST ($([ $YES -eq 1 ] && echo live || echo dry-run)) app=$APP worker=$WORKER mcp=$MCP bot=$BOT ==="

if [ $APP -eq 1 ] || [ $WORKER -eq 1 ]; then
  # A fresh source tree on the target; the calibrated layout maps in ~/.wechat-rpa/layout are never touched.
  SRC_TAR=$(mktemp -t wechat-src)
  local_step "pack native sources" tar czf "$SRC_TAR" native/bootstrap native/common native/eyes native/action native/bot native/click native/wechat_bubbles.swift
  echo "--- upload sources"
  if [ $YES -eq 1 ]; then
    ssh -o BatchMode=yes -o LogLevel=ERROR "$HOST" 'rm -rf "$HOME/.wechat-rpa/src.new" && mkdir -p "$HOME/.wechat-rpa/src.new" && tar xzf - -C "$HOME/.wechat-rpa/src.new"' < "$SRC_TAR"
    rm -f "$SRC_TAR"
  else echo "[dry-run] ssh $HOST 'tar xzf - -C ~/.wechat-rpa/src.new' < $SRC_TAR"; rm -f "$SRC_TAR"; fi
  remote "swap source tree" '
    cd "$HOME/.wechat-rpa"
    [ -d src ] && mv src "src.before-$TS"
    mv src.new src'
fi

if [ $WORKER -eq 1 ]; then
  remote "build and install wechat_bot_worker + wechat_click" '
    cd "$HOME/.wechat-rpa/src"
    swiftc -O -target x86_64-apple-macos12 -parse-as-library -D BOT_WORKER native/wechat_bubbles.swift native/common/*.swift native/eyes/*.swift native/action/*.swift native/bot/*.swift -o /tmp/wechat_bot_worker.$TS
    swiftc -O -target x86_64-apple-macos12 native/click/main.swift native/common/*.swift native/action/*.swift -o /tmp/wechat_click.$TS
    for name in wechat_bot_worker wechat_click; do
      dst="$HOME/.wechat-rpa/bin/$name"
      [ -f "$dst" ] && cp -p "$dst" "$dst.before-$TS"
      install -m 0755 "/tmp/$name.$TS" "$dst"; rm -f "/tmp/$name.$TS"
    done
    echo "worker and click installed; a running worker keeps its old binary until its connection ends"'
fi

if [ $MCP -eq 1 ]; then
  local_step "cross-build wechat-mcp for x86_64 macOS 12" env MACOSX_DEPLOYMENT_TARGET=12.0 cargo build --release --locked --target x86_64-apple-darwin
  remote "back up wechat-mcp" '
    dst="$HOME/.wechat-rpa/bin/wechat-mcp"; [ -f "$dst" ] && cp -p "$dst" "$dst.before-$TS"; true'
  echo "--- upload wechat-mcp"
  if [ $YES -eq 1 ]; then
    ssh -o BatchMode=yes -o LogLevel=ERROR "$HOST" 'cat > "$HOME/.wechat-rpa/bin/wechat-mcp.new" && chmod 0755 "$HOME/.wechat-rpa/bin/wechat-mcp.new" && mv "$HOME/.wechat-rpa/bin/wechat-mcp.new" "$HOME/.wechat-rpa/bin/wechat-mcp"' < target/x86_64-apple-darwin/release/wechat-mcp
  else echo "[dry-run] ssh $HOST 'cat > ~/.wechat-rpa/bin/wechat-mcp' < target/x86_64-apple-darwin/release/wechat-mcp"; fi
fi

if [ $APP -eq 1 ]; then
  remote "build, sign and install 微信客服助手.app" '
    cd "$HOME/.wechat-rpa/src"
    swiftc -O -target x86_64-apple-macos12 native/bootstrap/*.swift native/eyes/Screens.swift -o /tmp/wechat-cs-host.$TS
    APP="$HOME/Applications/微信客服助手.app"
    mkdir -p "$HOME/.wechat-rpa/backups"; [ -d "$APP" ] && cp -R "$APP" "$HOME/.wechat-rpa/backups/微信客服助手.app.before-$TS.bak"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
    install -m 0755 "/tmp/wechat-cs-host.$TS" "$APP/Contents/MacOS/wechat-cs-host"; rm -f "/tmp/wechat-cs-host.$TS"
    cp native/bootstrap/Info.plist "$APP/Contents/Info.plist"
    cp native/bootstrap/gateway.test.json "$APP/Contents/Resources/gateway.test.json"
    cp native/bootstrap/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
    codesign --force --sign - "$APP"'
  remote "retire WeChat RPA Test.app (quit it, keep a copy in ~/Applications/.old)" '
    OLD="$HOME/Applications/WeChat RPA Test.app"
    if [ -d "$OLD" ]; then
      osascript -e "tell application id \"local.wechat-mcp.desktop-probe\" to quit" || true
      for i in $(seq 1 20); do pgrep -f "WeChat RPA Test.app/Contents/MacOS" >/dev/null || break; sleep 0.5; done
      if pgrep -f "WeChat RPA Test.app/Contents/MacOS" >/dev/null; then echo "WeChat RPA Test is still running; quit it by hand and re-run" >&2; exit 1; fi
      mkdir -p "$HOME/Applications/.old"; mv "$OLD" "$HOME/Applications/.old/WeChat RPA Test.app.$TS"
    fi'
  remote "install and start the LaunchAgent" '
    PLIST="$HOME/Library/LaunchAgents/local.wechat-mcp.host.plist"
    mkdir -p "$HOME/Library/LaunchAgents" "$HOME/.wechat-rpa/logs"; chmod 0700 "$HOME/.wechat-rpa/logs"
    [ -f "$PLIST" ] && cp -p "$PLIST" "$PLIST.before-$TS"
    sed "s|__HOME__|$HOME|g" "$HOME/.wechat-rpa/src/native/bootstrap/local.wechat-mcp.host.plist" > "$PLIST"; chmod 0600 "$PLIST"
    plutil -lint "$PLIST"
    launchctl bootout "gui/$UID/local.wechat-mcp.host" 2>/dev/null || true
    launchctl bootstrap "gui/$UID" "$PLIST"
    echo "now grant Screen Recording and Accessibility to 微信客服助手 in System Settings, then press 刷新检查"'
fi

if [ $BOT -eq 1 ]; then
  BOT_TAR=$(mktemp -t wechat-bot)
  local_step "pack bot scripts" tar czf "$BOT_TAR" --exclude __pycache__ scripts
  remote "back up bot scripts" '
    BOT="$HOME/.wechat-rpa/bot"; mkdir -p "$BOT"; chmod 0700 "$BOT"
    [ -d "$BOT/scripts" ] && mv "$BOT/scripts" "$BOT/scripts.before-$TS"; true'
  echo "--- upload bot scripts"
  if [ $YES -eq 1 ]; then
    ssh -o BatchMode=yes -o LogLevel=ERROR "$HOST" 'tar xzf - -C "$HOME/.wechat-rpa/bot"' < "$BOT_TAR"; rm -f "$BOT_TAR"
  else echo "[dry-run] ssh $HOST 'tar xzf - -C ~/.wechat-rpa/bot' < $BOT_TAR"; rm -f "$BOT_TAR"; fi
fi

echo "=== $([ $YES -eq 1 ] && echo done || echo 'dry-run finished; add --yes to run') ==="
