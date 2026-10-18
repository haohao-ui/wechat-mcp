#!/usr/bin/env bash
# install.sh 的隔离回归测试：使用临时 HOME，不触碰真实 Agent 配置，不访问网络。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_HOME="$(mktemp -d)"
cleanup() {
  local status=$?
  rm -rf "$TMP_HOME"
  exit "$status"
}
trap cleanup EXIT

export HOME="$TMP_HOME"
export WECHAT_MCP_HOME="$HOME/.wechat-mcp"
INSTALLER="$ROOT/scripts/install.sh"
AGENTS="claude-code,codex,gemini,cursor,windsurf,cline,opencode,claude-desktop"
BIN="$WECHAT_MCP_HOME/bin/wechat-mcp"
CONFIG="$WECHAT_MCP_HOME/gateway.json"
OUT="$TMP_HOME/out.json"

pass() { printf 'PASS %s\n' "$*"; }
fail() { printf 'FAIL %s\n' "$*" >&2; exit 1; }

json_check() { # $1=file, $2=python 表达式，变量 data 为已解析 JSON
  python3 - "$1" "$2" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    data = json.load(handle)
result = eval(sys.argv[2], {"data": data})
if not result:
    raise SystemExit("assertion failed: " + sys.argv[2])
PY
}

mkdir -p "$WECHAT_MCP_HOME/bin"
cat > "$BIN" <<'STUB'
#!/bin/sh
cat >/dev/null
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}}'
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"fetch_article"}]}}'
STUB
chmod +x "$BIN"
cp "$ROOT/gateway.example.json" "$CONFIG"
mkdir -p "$WECHAT_MCP_HOME/data"
printf 'db\n' > "$WECHAT_MCP_HOME/data/service.sqlite3"

run() { # $1=输出文件，其余为参数
  local output="$1"
  shift
  bash "$INSTALLER" "$@" > "$output" 2> "$output.err"
}

echo '== 1. 配置全部客户端 =='
run "$OUT" --configure-only --agent "$AGENTS" --json
json_check "$OUT" 'data["ok"] is True and data["check"] == "passed" and len(data["agents"]) == 8'
pass 'JSON 摘要'

json_file_entry() { # $1=json 文件，$2=/mcpServers/wechat 或 /mcp/wechat
  python3 - "$1" "$2" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    data = json.load(handle)
node = data
for part in sys.argv[2].strip("/").split("/"):
    node = node[part]
assert node
PY
}
for pair in \
  "$HOME/.claude.json:/mcpServers/wechat" \
  "$HOME/.gemini/settings.json:/mcpServers/wechat" \
  "$HOME/.cursor/mcp.json:/mcpServers/wechat" \
  "$HOME/.codeium/windsurf/mcp_config.json:/mcpServers/wechat" \
  "$HOME/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json:/mcpServers/wechat" \
  "$HOME/.config/opencode/opencode.json:/mcp/wechat" \
  "$HOME/Library/Application Support/Claude/claude_desktop_config.json:/mcpServers/wechat"; do
  json_file_entry "${pair%:*}" "${pair#*:}" || fail "缺少 ${pair%:*}"
done
grep -q '^\[mcp_servers\.wechat\]' "$HOME/.codex/config.toml" || fail 'Codex config.toml 缺少 wechat'
pass '7 个 JSON 客户端 + Codex TOML'

[[ -f "$HOME/.claude/skills/wechat-article/SKILL.md" ]] || fail 'Claude Code skill'
[[ -f "$HOME/.codex/skills/wechat-article/SKILL.md" ]] || fail 'Codex skill'
pass 'skill 目录'

echo '== 2. 幂等 =='
run "$OUT.2" --configure-only --agent "$AGENTS" --json
json_check "$OUT.2" 'data["ok"] is True'
[[ "$(grep -c '^\[mcp_servers\.wechat\]' "$HOME/.codex/config.toml")" == 1 ]] || fail 'Codex TOML 重复'
for marker in 'wechat-mcp:begin' 'wechat-mcp:end'; do
  [[ "$(grep -c "$marker" "$HOME/.codex/AGENTS.md")" == 1 ]] || fail "指令块 $marker 重复"
done
pass '重复执行不产生重复条目'

echo '== 3. 自检 =='
run "$OUT.3" --check --json
json_check "$OUT.3" 'data["ok"] is True and data["check"] == "passed"'
pass 'MCP 自检'

echo '== 4. 卸载 =='
run "$OUT.4" --uninstall --json
json_check "$OUT.4" 'data["ok"] is True and data["action"] == "uninstall"'
for pair in \
  "$HOME/.claude.json:/mcpServers" \
  "$HOME/.gemini/settings.json:/mcpServers" \
  "$HOME/.cursor/mcp.json:/mcpServers" \
  "$HOME/.codeium/windsurf/mcp_config.json:/mcpServers" \
  "$HOME/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json:/mcpServers" \
  "$HOME/.config/opencode/opencode.json:/mcp" \
  "$HOME/Library/Application Support/Claude/claude_desktop_config.json:/mcpServers"; do
  file="${pair%:*}"
  key="${pair#*:}"
  python3 - "$file" "$key" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    data = json.load(handle)
node = data
for part in sys.argv[2].strip("/").split("/"):
    node = node.get(part, {})
assert "wechat" not in node
PY
done
[[ "$(grep -c 'mcp_servers\.wechat' "$HOME/.codex/config.toml" || true)" == 0 ]] || fail 'Codex TOML 残留'
[[ ! -d "$WECHAT_MCP_HOME/bin" && ! -d "$WECHAT_MCP_HOME/skills" ]] || fail '二进制或 skill 残留'
[[ -f "$CONFIG" && -f "$WECHAT_MCP_HOME/data/service.sqlite3" ]] || fail '配置或数据库被误删'
[[ ! -f "$HOME/.claude/skills/wechat-article/SKILL.md" ]] || fail 'Agent skill 残留'
pass '配置/数据库保留，条目与 skill 清理'

echo '== 5. 重复卸载 =='
run "$OUT.5" --uninstall --json
json_check "$OUT.5" 'data["ok"] is True'
pass '卸载幂等'

echo '== 6. 参数与失败路径 =='
bash "$INSTALLER" --help > /dev/null 2>&1 || fail '--help 应返回 0'
if bash "$INSTALLER" --agent nope > /dev/null 2>&1; then fail '未知 Agent 应失败'; fi
rm -rf "${WECHAT_MCP_HOME:?}/bin"
if bash "$INSTALLER" --configure-only --agent cursor > /dev/null 2>&1; then fail '缺少二进制时 --configure-only 应失败'; fi
pass '帮助与失败退出码'

echo '全部安装脚本测试通过'
