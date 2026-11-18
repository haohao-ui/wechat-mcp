#!/bin/bash
# Double-clickable host recovery launcher for the dedicated Mac.
#
# The host app is signed with the local "WeChat CS Local Signing" identity and is
# not notarized. Finder/double-click launches are rejected by Gatekeeper and can
# leave a stopped process at dyld. LaunchAgent starts the same signed app
# directly, which is the supported path.
set -euo pipefail

LABEL="local.wechat-mcp.host"
DOMAIN="gui/$(id -u)"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
APP_PROCESS="微信客服助手.app/Contents/MacOS/wechat-cs-host"

pause_before_exit() {
    printf '\n按回车关闭此窗口...'
    read -r _ || true
}

if ! launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
    if [[ ! -f "$PLIST" ]]; then
        echo "找不到 LaunchAgent: $PLIST" >&2
        echo "请先在部署机上运行 scripts/deploy_host.sh <host> --app --yes 安装宿主。" >&2
        pause_before_exit
        exit 1
    fi
    launchctl bootstrap "$DOMAIN" "$PLIST"
fi

# -k makes this safe to use when a previous instance is stuck or half-started.
launchctl kickstart -k "$DOMAIN/$LABEL"

for _ in $(seq 1 20); do
    if pgrep -f "$APP_PROCESS" >/dev/null 2>&1; then
        break
    fi
    sleep 0.5
done

JOB_PID=$(launchctl print "$DOMAIN/$LABEL" 2>/dev/null | awk '/pid =/{print $3; exit}')
if [[ -n "$JOB_PID" ]]; then
    # A Finder double-click can leave a second, Gatekeeper-stopped process.
    # Keep only the LaunchAgent-owned instance so duplicate schedulers cannot run.
    while IFS= read -r pid; do
        [[ -n "$pid" && "$pid" != "$JOB_PID" ]] || continue
        echo "清理未由 LaunchAgent 管理的宿主进程: $pid"
        kill -9 "$pid" 2>/dev/null || true
    done < <(pgrep -f "$APP_PROCESS" 2>/dev/null || true)
fi

if pgrep -f "$APP_PROCESS" >/dev/null 2>&1; then
    echo "微信客服助手已通过 LaunchAgent 启动。"
    echo "如果窗口没有出现，请检查屏幕共享或 Mission Control；请勿双击 App 本体。"
    echo "微信未登录时 core 会保持 paused/wechat_not_logged_in，登录后会自动恢复。"
else
    echo "启动失败。请查看 ~/.wechat-rpa/logs/host.stderr.log 与 host.log。" >&2
    pause_before_exit
    exit 1
fi

pause_before_exit
