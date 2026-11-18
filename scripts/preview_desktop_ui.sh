#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
preview_dir="$PWD/target/desktop-ui-preview"
mkdir -p "$preview_dir/home"
cat > "$preview_dir/control-stub" <<'STUB'
#!/bin/sh
printf '%s\n' '{"ok":false,"error":{"code":"preview_only","message":"离屏预览：未连接业务服务"}}'
STUB
chmod 700 "$preview_dir/control-stub"
sources=()
for source in native/bootstrap/*.swift; do
  [[ "$source" == native/bootstrap/main.swift ]] || sources+=("$source")
done
swiftc -target "$(uname -m)-apple-macos12" "${sources[@]}" native/eyes/Screens.swift tests/desktop_ui_preview/main.swift -o "$preview_dir/render"
CFFIXED_USER_HOME="$preview_dir/home" WECHAT_MCP_BIN="$preview_dir/control-stub" "$preview_dir/render" "$preview_dir"
