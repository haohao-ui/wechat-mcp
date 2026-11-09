#!/usr/bin/env bash
# wechat-mcp 一键安装（macOS）
#
# 公开仓库：
#   curl -fsSL https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash
# 私有仓库（token 需 repo / contents:read 权限；已 gh auth login 时可省略）：
#   export GITHUB_TOKEN=ghp_xxx
#   curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash -s -- --agent auto
#
# 用法：install.sh [选项]
#   --agent <id|auto|all|none>  目标 Agent，可逗号分隔；默认自动识别，识别不到时配置 Claude Code
#                               id：claude-code codex gemini cursor windsurf cline opencode claude-desktop
#                               auto = 识别当前运行的 Agent；all = 所有检测到已安装的 Agent
#                               none = 只安装二进制和 skill，不注册 MCP
#   --configure-only            不下载/安装二进制，只写 Agent 配置和 skill（修复配置用）
#   --check                     只做 MCP 自检：initialize + tools/list，确认个人微信核心工具已暴露
#   --uninstall                 卸载二进制、skill 和各 Agent 中的 wechat 配置（保留 gateway.json 与数据库）
#   --json                      输出 JSON 结果摘要（人类可读日志走 stderr）
#   -h, --help                  显示本帮助
#
# 环境变量：
#   WECHAT_MCP_AGENT            同 --agent
#   WECHAT_MCP_VERSION=v0.1.0   指定版本（默认最新 Release）
#   WECHAT_MCP_HOME=~/.wechat-mcp 安装目录
#   WECHAT_MCP_FROM_SOURCE=1    跳过 Release，直接从源码构建（需要 Rust 与 Xcode 命令行工具）
#   WECHAT_MCP_CONFIGURE_ONLY=1 同 --configure-only
#   WECHAT_MCP_JSON=1           同 --json
#   WECHAT_MCP_SKIP_CHECK=1     安装后不做 MCP 自检
#   WECHAT_MCP_NO_REGISTER=1    同 --agent none
#   GITHUB_TOKEN / GH_TOKEN     私有仓库下载用（未设置时尝试 gh auth token）
#
# 安全与环境隔离：
#   脚本采用绝对路径与各 Agent 局部环境变量进行配置，不修改系统全局 PATH，
#   不污染 ~/.zshrc / ~/.bashrc 等 Shell 配置文件。目前仅支持 macOS。
set -euo pipefail

REPO="haohao-ui/wechat-mcp"
INSTALL_HOME="${WECHAT_MCP_HOME:-$HOME/.wechat-mcp}"
BIN_DIR="$INSTALL_HOME/bin"
BIN="$BIN_DIR/wechat-mcp"
CONFIG="$INSTALL_HOME/gateway.json"
VERSION_FILE="$INSTALL_HOME/VERSION"
SKILL_NAME="wechat-operator"
SKILL_NAMES=(wechat-operator wechat-article)
SKILL_HOME="$INSTALL_HOME/skills/$SKILL_NAME"
TOOLS=(wechat-mcp wechat_ocr wechat_click wechat_window)
AGENT_IDS=(claude-code codex gemini cursor windsurf cline opencode claude-desktop)

ACTION="install"
AGENT_SPEC="${WECHAT_MCP_AGENT:-}"
CONFIGURE_ONLY="${WECHAT_MCP_CONFIGURE_ONLY:-0}"
JSON_OUT="${WECHAT_MCP_JSON:-0}"
SKIP_CHECK="${WECHAT_MCP_SKIP_CHECK:-0}"
[[ "${WECHAT_MCP_NO_REGISTER:-}" == "1" ]] && AGENT_SPEC="none"

CHECK_STATUS="skipped"
CHECK_DETAIL=""
VERSION=""
SOURCE_KIND=""
RESULTS=""
FAILED=0
COMPLETED=0
TMP=""
SKILL_SRC=""
EXAMPLE_CONFIG=""
REF=""
SELECTED=()

info() { printf '\033[1;34m==>\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[1;33m警告:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31m错误:\033[0m %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
用法：install.sh [选项]
  --agent <id|auto|all|none>  目标 Agent，可逗号分隔（claude-code codex gemini cursor windsurf cline opencode claude-desktop）
  --configure-only            只写 Agent 配置和 skill，不安装二进制
  --check                     只做 MCP 自检
  --uninstall                 卸载（保留 gateway.json 与数据库）
  --json                      输出 JSON 摘要
  --no-register               同 --agent none
  -h, --help                  显示帮助
EOF
}

cleanup() {
  local status=$?
  # bash 3.2 在 set -e + EXIT trap 下会把致命展开错误的退出码报成 0，
  # 因此只有走到正常完成点才允许退出码为 0。
  if [[ "$COMPLETED" != "1" && "$status" -eq 0 ]]; then
    status=1
  fi
  if [[ -n "$TMP" && -d "$TMP" ]]; then
    rm -rf "$TMP"
  fi
  exit "$status"
}
trap cleanup EXIT

while [[ $# -gt 0 ]]; do
  case "$1" in
    --agent)
      [[ $# -ge 2 ]] || die "--agent 需要参数"
      AGENT_SPEC="$2"
      shift 2
      ;;
    --agent=*) AGENT_SPEC="${1#*=}"; shift ;;
    --configure-only) CONFIGURE_ONLY=1; shift ;;
    --check) ACTION="check"; shift ;;
    --uninstall) ACTION="uninstall"; shift ;;
    --json) JSON_OUT=1; shift ;;
    --no-register) AGENT_SPEC="none"; shift ;;
    -h|--help) usage; COMPLETED=1; exit 0 ;;
    *) die "未知参数：$1（--help 查看用法）" ;;
  esac
done

json_escape() {
  local s="$1"
  s="${s//\\/\\\\}"
  s="${s//\"/\\\"}"
  s="${s//$'\n'/\\n}"
  s="${s//$'\r'/}"
  s="${s//$'\t'/\\t}"
  printf '%s' "$s"
}

toml_escape() {
  local s="$1"
  s="${s//\\/\\\\}"
  s="${s//\"/\\\"}"
  printf '%s' "$s"
}

result() {
  RESULTS="${RESULTS}$1|$2|$3|$4"$'\n'
}

agent_label() {
  case "$1" in
    claude-code) printf 'Claude Code' ;;
    codex) printf 'Codex CLI' ;;
    gemini) printf 'Gemini CLI' ;;
    cursor) printf 'Cursor' ;;
    windsurf) printf 'Windsurf' ;;
    cline) printf 'Cline' ;;
    opencode) printf 'OpenCode' ;;
    claude-desktop) printf 'Claude Desktop' ;;
    none) printf 'None' ;;
    *) printf '%s' "$1" ;;
  esac
}

is_agent_id() {
  local a
  for a in "${AGENT_IDS[@]}"; do
    [[ "$a" == "$1" ]] && return 0
  done
  return 1
}

agent_installed() {
  case "$1" in
    claude-code)
      command -v claude >/dev/null 2>&1 || [[ -f "$HOME/.claude.json" ]]
      ;;
    codex)
      command -v codex >/dev/null 2>&1 || [[ -d "$HOME/.codex" ]]
      ;;
    gemini)
      command -v gemini >/dev/null 2>&1 || [[ -d "$HOME/.gemini" ]]
      ;;
    cursor)
      [[ -d "$HOME/.cursor" ]] || [[ -d "/Applications/Cursor.app" ]]
      ;;
    windsurf)
      [[ -d "$HOME/.codeium/windsurf" ]] || [[ -d "/Applications/Windsurf.app" ]]
      ;;
    cline)
      [[ -d "/Applications/Visual Studio Code.app" ]] || [[ -d "/Applications/Cursor.app" ]] || [[ -d "/Applications/Windsurf.app" ]] || [[ -f "$(cline_settings_path)" ]]
      ;;
    opencode)
      command -v opencode >/dev/null 2>&1 || [[ -d "$HOME/.config/opencode" ]]
      ;;
    claude-desktop)
      [[ -d "/Applications/Claude.app" ]] || [[ -d "$HOME/Library/Application Support/Claude" ]]
      ;;
    *) return 1 ;;
  esac
}

detect_agent() {
  # 先看进程祖先：能正确处理“在 Cursor 终端里运行 Codex”这类嵌套场景。
  local pid="$PPID" cmd i=0
  while [[ -n "$pid" && "$pid" -gt 1 && $i -lt 12 ]]; do
    cmd="$(ps -o command= -p "$pid" 2>/dev/null | tr '[:upper:]' '[:lower:]' || true)"
    case "$cmd" in
      *claude*) printf 'claude-code'; return 0 ;;
      *codex*) printf 'codex'; return 0 ;;
      *gemini*) printf 'gemini'; return 0 ;;
      *windsurf*) printf 'windsurf'; return 0 ;;
      *opencode*) printf 'opencode'; return 0 ;;
      *cursor*) printf 'cursor'; return 0 ;;
    esac
    [[ -z "$cmd" ]] && break
    pid="$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
    i=$((i + 1))
  done
  # 进程信息不可用时，退回稳定的环境变量提示。
  if [[ "${CLAUDECODE:-}" == "1" || -n "${CLAUDE_CODE_ENTRYPOINT:-}" || -n "${CLAUDE_CODE_SSE_PORT:-}" ]]; then
    printf 'claude-code'
    return 0
  fi
  if [[ -n "${CURSOR_TRACE_ID:-}" ]]; then
    printf 'cursor'
    return 0
  fi
  printf ''
}

parse_agent_spec() { # $1 = spec, $2 = install|uninstall
  local spec="$1" mode="${2:-install}" item found detected
  SELECTED=()
  case "$spec" in
    "")
      return 0
      ;;
    auto)
      detected="$(detect_agent)"
      [[ -n "$detected" ]] || die "无法自动识别当前 Agent，请用 --agent <id> 指定。可用：${AGENT_IDS[*]}、none"
      spec="$detected"
      info "检测到当前 Agent：$(agent_label "$detected")"
      ;;
    all)
      if [[ "$mode" == "uninstall" ]]; then
        spec="claude-code,codex,gemini,cursor,windsurf,cline,opencode,claude-desktop"
      else
        found=""
        local a
        for a in "${AGENT_IDS[@]}"; do
          agent_installed "$a" && found="${found}${a},"
        done
        [[ -n "$found" ]] || die "没有检测到已安装的 Agent（可用 --agent <id> 指定，或 --agent none 跳过注册）"
        spec="${found%,}"
        info "检测到已安装的 Agent：$spec"
      fi
      ;;
    none)
      SELECTED+=("none")
      return 0
      ;;
  esac
  while IFS= read -r item; do
    item="$(printf '%s' "$item" | tr -d '[:space:]')"
    [[ -z "$item" ]] && continue
    if [[ "$item" == "none" ]]; then
      SELECTED=("none")
      return 0
    fi
    if [[ "$item" == "auto" || "$item" == "all" ]]; then
      die "--agent 不能把 auto/all 与其他值混用"
    fi
    is_agent_id "$item" || die "未知 Agent：${item}（可用：${AGENT_IDS[*]}、none）"
    SELECTED+=("$item")
  done <<EOF
$(printf '%s\n' "$spec" | tr ',' '\n')
EOF
  [[ ${#SELECTED[@]} -gt 0 ]] || die "没有可配置的 Agent（可用 none 跳过）"
}

resolve_install_agents() {
  if [[ -z "$AGENT_SPEC" ]]; then
    local detected
    detected="$(detect_agent)"
    if [[ -n "$detected" ]]; then
      AGENT_SPEC="$detected"
      info "检测到当前 Agent：$(agent_label "$detected")"
    elif command -v claude >/dev/null 2>&1; then
      AGENT_SPEC="claude-code"
      info "未识别当前 Agent，默认配置 Claude Code"
    else
      AGENT_SPEC="none"
      info "未识别到 Agent，只安装二进制和 skill（--agent 可指定）"
    fi
  fi
  parse_agent_spec "$AGENT_SPEC" install
}

# ---------- 通用文件/配置操作 ----------

backup_file() {
  local file="$1"
  [[ -f "$file" ]] || return 0
  if [[ ! -f "$file.bak" ]]; then
    cp -p "$file" "$file.bak"
    info "已备份 $file → $file.bak"
  fi
}

INSTR_BEGIN='<!-- wechat-mcp:begin -->'
INSTR_END='<!-- wechat-mcp:end -->'

instruction_snippet() {
  cat <<EOF
$INSTR_BEGIN
## 个人微信操作
个人微信私聊、群聊、好友、桌面诊断和监听任务，先阅读 ${SKILL_HOME}/SKILL.md；按实际工具能力执行，不能把界面变化当作已确认入站消息。
EOF
  if skill_complete "wechat-article" "$INSTALL_HOME/skills"; then
    printf '仅处理 mp.weixin.qq.com 公众号文章网页时，阅读 %s/skills/wechat-article/SKILL.md；不需要操作微信客户端。\n' "$INSTALL_HOME"
  fi
  printf '%s\n' "$INSTR_END"
}

remove_block() {
  local file="$1" tmp
  [[ -f "$file" ]] || return 0
  tmp="$(mktemp)"
  awk -v b="$INSTR_BEGIN" -v e="$INSTR_END" '
    index($0, b) { skip = 1; next }
    index($0, e) { skip = 0; next }
    !skip { print }
  ' "$file" > "$tmp"
  cat "$tmp" > "$file"
  rm -f "$tmp"
}

trim_trailing_blank_lines() {
  local file="$1" tmp
  tmp="$(mktemp)"
  awk '{ lines[NR] = $0; if (NF) last = NR } END { for (i = 1; i <= last; i++) print lines[i] }' "$file" > "$tmp"
  cat "$tmp" > "$file"
  rm -f "$tmp"
}

upsert_block() {
  local file="$1"
  mkdir -p "$(dirname "$file")"
  if [[ -f "$file" ]]; then
    backup_file "$file"
    remove_block "$file"
  else
    : > "$file"
  fi
  trim_trailing_blank_lines "$file"
  [[ -s "$file" ]] && printf '\n' >> "$file"
  instruction_snippet >> "$file"
}

remove_codex_toml_table() {
  local file="$HOME/.codex/config.toml" tmp
  [[ -f "$file" ]] || return 0
  tmp="$(mktemp)"
  awk '
    /^\[mcp_servers\.wechat\]/ { skip = 1; next }
    /^\[mcp_servers\.wechat\./ { skip = 1; next }
    skip && /^\[/ { skip = 0 }
    !skip { print }
  ' "$file" > "$tmp"
  cat "$tmp" > "$file"
  rm -f "$tmp"
}

upsert_codex_toml() {
  local file="$HOME/.codex/config.toml"
  mkdir -p "$(dirname "$file")"
  if [[ -f "$file" ]]; then
    backup_file "$file"
    remove_codex_toml_table
  else
    : > "$file"
  fi
  trim_trailing_blank_lines "$file"
  [[ -s "$file" ]] && printf '\n' >> "$file"
  {
    printf '[mcp_servers.wechat]\n'
    printf 'command = "%s"\n\n' "$(toml_escape "$BIN")"
    printf '[mcp_servers.wechat.env]\n'
    printf 'MESSAGE_GATEWAY_CONFIG = "%s"\n' "$(toml_escape "$CONFIG")"
  } >> "$file"
}

# 用 osascript/JXA 合并 JSON 配置，避免依赖 jq 或 python3。
# shape: plain = { command, env }；claude = { type:"stdio", command, env }；opencode = { type:"local", command:[...], environment:{...}, enabled:true }
json_edit() {
  local file="$1" action="$2" shape="$3"
  mkdir -p "$(dirname "$file")"
  backup_file "$file"
  if /usr/bin/osascript -l JavaScript - "$file" "$action" "wechat" "$BIN" "$CONFIG" "$shape" <<'JXA'
function run(argv) {
  const file = argv[0], action = argv[1], server = argv[2], command = argv[3], config = argv[4], shape = argv[5];
  const fm = $.NSFileManager.defaultManager;
  let root = {};
  if (fm.fileExistsAtPath(file)) {
    const text = $.NSString.stringWithContentsOfFileEncodingError(file, $.NSUTF8StringEncoding, null);
    if (!text) throw new Error("无法读取 " + file);
    const raw = text.js;
    if (raw.trim()) root = JSON.parse(raw);
  }

  if (shape === "opencode") {
    if (action === "remove") {
      if (!root.mcp || !root.mcp[server]) return;
      delete root.mcp[server];
    } else {
      if (!root.mcp) root.mcp = {};
      root.mcp[server] = {
        type: "local",
        command: [command],
        environment: { MESSAGE_GATEWAY_CONFIG: config },
        enabled: true
      };
    }
  } else {
    if (action === "remove") {
      if (!root.mcpServers || !root.mcpServers[server]) return;
      delete root.mcpServers[server];
    } else {
      if (!root.mcpServers) root.mcpServers = {};
      root.mcpServers[server] = shape === "claude"
        ? { type: "stdio", command: command, env: { MESSAGE_GATEWAY_CONFIG: config } }
        : { command: command, env: { MESSAGE_GATEWAY_CONFIG: config } };
    }
  }

  const out = JSON.stringify(root, null, 2) + "\n";
  if (!$(out).writeToFileAtomicallyEncodingError(file, true, $.NSUTF8StringEncoding, null)) {
    throw new Error("无法写入 " + file);
  }
}
JXA
  then
    return 0
  fi
  warn "写入 $file 失败（格式不是标准 JSON？请手动合并）"
  return 1
}

json_remove() {
  local file="$1" shape="$2"
  [[ -f "$file" ]] || return 0
  json_edit "$file" remove "$shape"
}

skill_files() {
  case "$1" in
    wechat-operator)
      printf '%s\n' SKILL.md references/messaging.md references/environment.md references/events.md references/bot.md
      ;;
    wechat-article) printf '%s\n' SKILL.md ;;
    *) return 1 ;;
  esac
}

skill_complete() {
  local name="$1" root="$2" resource
  while IFS= read -r resource; do
    [[ -s "$root/$name/$resource" ]] || return 1
  done < <(skill_files "$name")
}

skill_ready() {
  skill_complete "$SKILL_NAME" "$INSTALL_HOME/skills"
}

copy_skill_to() {
  local dest="$1" name root
  skill_ready || return 1
  root="$(dirname "$dest")"
  mkdir -p "$root"
  for name in "${SKILL_NAMES[@]}"; do
    if skill_complete "$name" "$INSTALL_HOME/skills"; then
      rm -rf "${root:?}/$name"
      cp -R "$INSTALL_HOME/skills/$name" "$root/$name"
    fi
  done
}

install_skill_home() {
  local name source_root="${SKILL_SRC%/*}"
  for name in "${SKILL_NAMES[@]}"; do
    if [[ -n "$SKILL_SRC" ]] && skill_complete "$name" "$source_root"; then
      if [[ "$source_root/$name" != "$INSTALL_HOME/skills/$name" ]]; then
        mkdir -p "$INSTALL_HOME/skills"
        rm -rf "$INSTALL_HOME/skills/$name"
        cp -R "$source_root/$name" "$INSTALL_HOME/skills/$name"
      fi
      info "已安装 skill $name"
    elif skill_complete "$name" "$INSTALL_HOME/skills"; then
      info "保留已有 skill $name"
    else
      warn "缺少完整 skill ${name}，跳过该 skill（MCP 注册不受影响）"
    fi
  done
}

# ---------- 各 Agent 的注册 ----------

configure_claude_code() {
  local mcp="failed" skill="skipped" note=""
  if command -v claude >/dev/null 2>&1; then
    claude mcp remove wechat --scope user >/dev/null 2>&1 || true
    if claude mcp add wechat --scope user -e "MESSAGE_GATEWAY_CONFIG=$CONFIG" -- "$BIN" >/dev/null 2>&1; then
      mcp="registered"
      note="claude mcp add --scope user"
    fi
  fi
  if [[ "$mcp" != "registered" ]]; then
    if json_edit "$HOME/.claude.json" set claude; then
      mcp="registered"
      note="写入 $HOME/.claude.json"
    fi
  fi
  if ! skill_ready; then
    skill="skipped"
  elif copy_skill_to "$HOME/.claude/skills/$SKILL_NAME"; then
    skill="installed"
    note="${note:+$note; }skill → $HOME/.claude/skills/$SKILL_NAME"
  else
    skill="failed"
  fi
  [[ "$mcp" == "registered" ]] || FAILED=1
  result "claude-code" "$mcp" "$skill" "$note"
}

configure_codex() {
  local mcp="failed" skill="skipped" note="" skill_dest="$HOME/.codex/skills/$SKILL_NAME"
  if [[ -d "$HOME/.agents/skills" ]]; then
    skill_dest="$HOME/.agents/skills/$SKILL_NAME"
  fi
  if command -v codex >/dev/null 2>&1; then
    codex mcp remove wechat >/dev/null 2>&1 || true
    if codex mcp add wechat --env "MESSAGE_GATEWAY_CONFIG=$CONFIG" -- "$BIN" >/dev/null 2>&1; then
      mcp="registered"
      note="codex mcp add"
    fi
  fi
  if [[ "$mcp" != "registered" ]]; then
    if upsert_codex_toml; then
      mcp="registered"
      note="写入 $HOME/.codex/config.toml"
    fi
  fi
  if skill_ready; then
    if copy_skill_to "$skill_dest"; then
      skill="installed"
      note="${note:+$note; }skill → $skill_dest"
    fi
  fi
  if skill_ready && upsert_block "$HOME/.codex/AGENTS.md"; then
    [[ "$skill" == "installed" ]] || skill="instructions"
    note="${note:+$note; }指令 → $HOME/.codex/AGENTS.md"
  fi
  [[ "$mcp" == "registered" ]] || FAILED=1
  result "codex" "$mcp" "$skill" "$note"
}

configure_gemini() {
  local mcp="failed" skill="skipped" note=""
  if command -v gemini >/dev/null 2>&1; then
    gemini mcp remove -s user wechat >/dev/null 2>&1 || true
    if gemini mcp add -s user -e "MESSAGE_GATEWAY_CONFIG=$CONFIG" wechat "$BIN" >/dev/null 2>&1; then
      mcp="registered"
      note="gemini mcp add -s user"
    fi
  fi
  if [[ "$mcp" != "registered" ]]; then
    if json_edit "$HOME/.gemini/settings.json" set plain; then
      mcp="registered"
      note="写入 $HOME/.gemini/settings.json"
    fi
  fi
  if skill_ready && upsert_block "$HOME/.gemini/GEMINI.md"; then
    skill="instructions"
    note="${note:+$note; }指令 → $HOME/.gemini/GEMINI.md"
  fi
  [[ "$mcp" == "registered" ]] || FAILED=1
  result "gemini" "$mcp" "$skill" "$note"
}

configure_cursor() {
  local mcp="failed" skill="skipped" note=""
  if json_edit "$HOME/.cursor/mcp.json" set plain; then
    mcp="registered"
    note="写入 $HOME/.cursor/mcp.json"
  fi
  [[ "$mcp" == "registered" ]] || FAILED=1
  if skill_ready; then
    skill="manual"
    note="${note:+$note; }Cursor 没有全局规则文件，请在 Settings → Rules → User Rules 添加：个人微信操作先阅读 $SKILL_HOME/SKILL.md；公众号文章使用 $INSTALL_HOME/skills/wechat-article/SKILL.md（如已安装）"
  fi
  result "cursor" "$mcp" "$skill" "$note"
}

configure_windsurf() {
  local mcp="failed" skill="skipped" note=""
  if json_edit "$HOME/.codeium/windsurf/mcp_config.json" set plain; then
    mcp="registered"
    note="写入 $HOME/.codeium/windsurf/mcp_config.json"
  fi
  if skill_ready && upsert_block "$HOME/.codeium/windsurf/memories/global_rules.md"; then
    skill="instructions"
    note="${note:+$note; }指令 → $HOME/.codeium/windsurf/memories/global_rules.md"
  fi
  [[ "$mcp" == "registered" ]] || FAILED=1
  result "windsurf" "$mcp" "$skill" "$note"
}

cline_settings_path() {
  local candidate
  for candidate in \
    "$HOME/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" \
    "$HOME/Library/Application Support/Cursor/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" \
    "$HOME/Library/Application Support/Windsurf/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"; do
    if [[ -f "$candidate" ]]; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  if [[ -d "/Applications/Visual Studio Code.app" ]]; then
    printf '%s' "$HOME/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"
  elif [[ -d "/Applications/Cursor.app" ]]; then
    printf '%s' "$HOME/Library/Application Support/Cursor/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"
  elif [[ -d "/Applications/Windsurf.app" ]]; then
    printf '%s' "$HOME/Library/Application Support/Windsurf/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"
  else
    printf '%s' "$HOME/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"
  fi
}

configure_cline() {
  local mcp="failed" skill="skipped" note="" file rules
  file="$(cline_settings_path)"
  if json_edit "$file" set plain; then
    mcp="registered"
    note="写入 $file"
  fi
  rules="$HOME/Documents/Cline/Rules/wechat-mcp.md"
  if [[ -f "$HOME/Documents/Cline/Rules/wechat-article.md" ]]; then
    backup_file "$HOME/Documents/Cline/Rules/wechat-article.md"
    remove_block "$HOME/Documents/Cline/Rules/wechat-article.md"
  fi
  if skill_ready && upsert_block "$rules"; then
    skill="instructions"
    note="${note:+$note; }指令 → $rules"
  fi
  [[ "$mcp" == "registered" ]] || FAILED=1
  result "cline" "$mcp" "$skill" "$note"
}

configure_opencode() {
  local mcp="failed" skill="skipped" note=""
  if json_edit "$HOME/.config/opencode/opencode.json" set opencode; then
    mcp="registered"
    note="写入 $HOME/.config/opencode/opencode.json"
  fi
  if skill_ready && upsert_block "$HOME/.config/opencode/AGENTS.md"; then
    skill="instructions"
    note="${note:+$note; }指令 → $HOME/.config/opencode/AGENTS.md"
  fi
  [[ "$mcp" == "registered" ]] || FAILED=1
  result "opencode" "$mcp" "$skill" "$note"
}

configure_claude_desktop() {
  local mcp="failed" skill="skipped" note="" zip name
  if json_edit "$HOME/Library/Application Support/Claude/claude_desktop_config.json" set plain; then
    mcp="registered"
    note="写入 Claude Desktop 配置（重启 Claude 生效）"
  fi
  for name in "${SKILL_NAMES[@]}"; do
    if skill_complete "$name" "$INSTALL_HOME/skills" && command -v zip >/dev/null 2>&1; then
      mkdir -p "$HOME/Desktop"
      zip="$HOME/Desktop/$name-skill.zip"
      rm -f "$zip"
      skill="manual"
      if (cd "$INSTALL_HOME/skills" && zip -qr "$zip" "$name"); then
        note="${note:+$note; }在 设置 → Capabilities → Skills 分别上传 $zip"
      else
        note="${note:+$note; }$name 打包失败，请手动压缩上传"
      fi
    fi
  done
  [[ "$mcp" == "registered" ]] || FAILED=1
  result "claude-desktop" "$mcp" "$skill" "$note"
}

configure_one() {
  case "$1" in
    claude-code) configure_claude_code ;;
    codex) configure_codex ;;
    gemini) configure_gemini ;;
    cursor) configure_cursor ;;
    windsurf) configure_windsurf ;;
    cline) configure_cline ;;
    opencode) configure_opencode ;;
    claude-desktop) configure_claude_desktop ;;
    none) result "none" "skipped" "skipped" "未注册（--agent none）" ;;
  esac
}

# ---------- 卸载 ----------

uninstall_agent() {
  local id="$1" note="" mcp="removed" skill="removed" name
  case "$id" in
    claude-code)
      if [[ -f "$HOME/.claude.json" ]] && command -v claude >/dev/null 2>&1; then
        claude mcp remove wechat --scope user >/dev/null 2>&1 || true
        note="claude mcp remove --scope user"
      fi
      json_remove "$HOME/.claude.json" claude
      for name in "${SKILL_NAMES[@]}"; do rm -rf "$HOME/.claude/skills/$name"; done
      note="${note:+$note; }已删除 Claude Code 的个人微信及文章 skills"
      ;;
    codex)
      if [[ -f "$HOME/.codex/config.toml" ]] && command -v codex >/dev/null 2>&1; then
        codex mcp remove wechat >/dev/null 2>&1 || true
        note="codex mcp remove"
      fi
      [[ -f "$HOME/.codex/config.toml" ]] && { backup_file "$HOME/.codex/config.toml"; remove_codex_toml_table; }
      for name in "${SKILL_NAMES[@]}"; do rm -rf "$HOME/.codex/skills/$name" "$HOME/.agents/skills/$name"; done
      [[ -f "$HOME/.codex/AGENTS.md" ]] && { backup_file "$HOME/.codex/AGENTS.md"; remove_block "$HOME/.codex/AGENTS.md"; }
      note="${note:+$note; }已删除 skill 与 AGENTS.md 指令"
      ;;
    gemini)
      if [[ -f "$HOME/.gemini/settings.json" ]] && command -v gemini >/dev/null 2>&1; then
        gemini mcp remove -s user wechat >/dev/null 2>&1 || true
      fi
      json_remove "$HOME/.gemini/settings.json" plain
      [[ -f "$HOME/.gemini/GEMINI.md" ]] && { backup_file "$HOME/.gemini/GEMINI.md"; remove_block "$HOME/.gemini/GEMINI.md"; }
      note="已移除 Gemini 配置与 GEMINI.md 指令"
      ;;
    cursor)
      json_remove "$HOME/.cursor/mcp.json" plain
      note="已移除 ~/.cursor/mcp.json 中的 wechat"
      ;;
    windsurf)
      json_remove "$HOME/.codeium/windsurf/mcp_config.json" plain
      [[ -f "$HOME/.codeium/windsurf/memories/global_rules.md" ]] && { backup_file "$HOME/.codeium/windsurf/memories/global_rules.md"; remove_block "$HOME/.codeium/windsurf/memories/global_rules.md"; }
      note="已移除 Windsurf 配置与指令"
      ;;
    cline)
      json_remove "$(cline_settings_path)" plain
      json_remove "$HOME/Library/Application Support/Cursor/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" plain
      json_remove "$HOME/Library/Application Support/Windsurf/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" plain
      for name in wechat-mcp wechat-article; do
        if [[ -f "$HOME/Documents/Cline/Rules/$name.md" ]]; then
          backup_file "$HOME/Documents/Cline/Rules/$name.md"
          remove_block "$HOME/Documents/Cline/Rules/$name.md"
        fi
      done
      note="已移除 Cline 配置与规则文件"
      ;;
    opencode)
      json_remove "$HOME/.config/opencode/opencode.json" opencode
      [[ -f "$HOME/.config/opencode/AGENTS.md" ]] && { backup_file "$HOME/.config/opencode/AGENTS.md"; remove_block "$HOME/.config/opencode/AGENTS.md"; }
      note="已移除 OpenCode 配置与 AGENTS.md 指令"
      ;;
    claude-desktop)
      json_remove "$HOME/Library/Application Support/Claude/claude_desktop_config.json" plain
      for name in "${SKILL_NAMES[@]}"; do rm -f "$HOME/Desktop/$name-skill.zip"; done
      note="已移除 Claude Desktop 配置与 skill 压缩包"
      ;;
    none)
      mcp="skipped"; skill="skipped"; note="未做修改"
      ;;
  esac
  result "$id" "$mcp" "$skill" "$note"
}

run_uninstall() {
  if [[ -z "$AGENT_SPEC" ]]; then
    AGENT_SPEC="all"
  fi
  parse_agent_spec "$AGENT_SPEC" uninstall
  local id
  for id in "${SELECTED[@]}"; do
    uninstall_agent "$id"
  done
  rm -rf "$BIN_DIR" "$INSTALL_HOME/skills" "$VERSION_FILE"
  info "已删除 ${BIN_DIR}、skill 与 wechat-mcp 版本标记"
  info "配置 $CONFIG 与客服数据库已保留，如需彻底清除请手动删除"
  COMPLETED=1
  if [[ "$JSON_OUT" == "1" ]]; then
    print_json_result "uninstall" "skipped"
  else
    printf '\n✅ 卸载完成\n'
    print_agents_summary
    printf '\n配置 %s 与客服数据库已保留。\n' "$CONFIG"
  fi
}

# ---------- 自检与输出 ----------

run_check() {
  local bin="$BIN_DIR/wechat-mcp" out="" name
  if [[ ! -x "$bin" ]]; then
    CHECK_DETAIL="未找到可执行文件 $bin"
    return 1
  fi
  if [[ -f "$CONFIG" ]]; then
    out="$(printf '%s\n' \
      '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
      '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
      | MESSAGE_GATEWAY_CONFIG="$CONFIG" "$bin" 2>/dev/null || true)"
  else
    out="$(printf '%s\n' \
      '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
      '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
      | "$bin" 2>/dev/null || true)"
  fi
  for name in list_accounts list_conversations get_messages send_message get_send_status; do
    if ! printf '%s' "$out" | grep -q "\"$name\""; then
      CHECK_DETAIL="tools/list 中没有个人微信工具 ${name}（可能是旧版本，或账号配置无法加载）"
      return 1
    fi
  done
  CHECK_DETAIL="tools/list 包含个人微信账号、会话及文字收发工具；未执行桌面操作"
  return 0
}

do_check_only() {
  local ok="false"
  if [[ -f "$VERSION_FILE" ]]; then
    VERSION="$(cat "$VERSION_FILE" 2>/dev/null || true)"
  fi
  if run_check; then
    ok="true"
    info "MCP 自检通过：$CHECK_DETAIL"
  else
    warn "MCP 自检失败：$CHECK_DETAIL"
    FAILED=1
  fi
  if [[ "$JSON_OUT" == "1" ]]; then
    print_json_result "check" "$([[ "$ok" == "true" ]] && printf passed || printf failed)"
  else
    printf '%s\n' "$CHECK_DETAIL"
  fi
  COMPLETED=1
  [[ "$ok" == "true" ]] || exit 1
}

print_agents_summary() {
  local id mcp skill note
  while IFS='|' read -r id mcp skill note; do
    [[ -z "$id" ]] && continue
    printf '     - %-14s MCP: %-11s skill: %-11s %s\n' "$(agent_label "$id")" "$mcp" "$skill" "$note"
  done <<< "$RESULTS"
}

print_manual_config() {
  cat <<EOF

其他 MCP 客户端配置（stdio）：
{
  "mcpServers": {
    "wechat": {
      "command": "$BIN",
      "env": { "MESSAGE_GATEWAY_CONFIG": "$CONFIG" }
    }
  }
}
EOF
}

print_json_result() { # $1 action, $2 check
  local action="$1" check="$2" ok="true" first=1 id mcp skill note
  [[ "$FAILED" -eq 0 ]] || ok="false"
  local skill_path=""
  if skill_ready; then
    skill_path="$SKILL_HOME/SKILL.md"
  fi
  printf '{"ok":%s,"action":"%s","version":"%s","source":"%s","home":"%s","bin":"%s","config":"%s","skill":"%s","check":"%s","agents":[' \
    "$ok" "$(json_escape "$action")" "$(json_escape "$VERSION")" "$(json_escape "$SOURCE_KIND")" \
    "$(json_escape "$INSTALL_HOME")" "$(json_escape "$BIN")" "$(json_escape "$CONFIG")" \
    "$(json_escape "$skill_path")" "$(json_escape "$check")"
  while IFS='|' read -r id mcp skill note; do
    [[ -z "$id" ]] && continue
    [[ "$first" -eq 1 ]] || printf ','
    first=0
    printf '{"id":"%s","mcp":"%s","skill":"%s","note":"%s"}' \
      "$(json_escape "$id")" "$(json_escape "$mcp")" "$(json_escape "$skill")" "$(json_escape "$note")"
  done <<< "$RESULTS"
  printf '],"next":["重启或重新加载上面列出的 Agent，让 wechat 服务与 skill 生效","验证：重新运行安装脚本并加 --check","公众号抓取不需要 macOS 隐私权限；微信消息/界面功能需要给宿主程序授予 辅助功能、屏幕录制、自动化 权限并保持微信已登录"]}\n'
}

# ---------- 安装流程 ----------

resolve_token() {
  if [[ -z "${GITHUB_TOKEN:-}" && -n "${GH_TOKEN:-}" ]]; then
    GITHUB_TOKEN="$GH_TOKEN"
  fi
  if [[ -z "${GITHUB_TOKEN:-}" ]] && command -v gh >/dev/null 2>&1; then
    GITHUB_TOKEN="$(gh auth token 2>/dev/null || true)"
    if [[ -n "$GITHUB_TOKEN" ]]; then
      info "使用 gh 的登录凭据访问私有仓库"
    fi
  fi
  return 0
}

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
  SKILL_SRC="$SRC_DIR/skills/$SKILL_NAME"
  VERSION="$tag"
  SOURCE_KIND="release"
  REF="$tag"
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
  SKILL_SRC="$TMP/src/skills/$SKILL_NAME"
  VERSION="${WECHAT_MCP_VERSION:-main}"
  SOURCE_KIND="source"
  REF="${WECHAT_MCP_VERSION:-main}"
}

prepare_local_fallbacks() {
  local script_path="${BASH_SOURCE[0]:-}" local_root=""
  if [[ -n "$script_path" && -f "$script_path" ]]; then
    local_root="$(cd "$(dirname "$script_path")/.." 2>/dev/null && pwd || true)"
  fi
  [[ -n "$local_root" && -f "$local_root/Cargo.toml" ]] || return 0
  [[ -n "$EXAMPLE_CONFIG" && -f "$EXAMPLE_CONFIG" ]] || EXAMPLE_CONFIG="$local_root/gateway.example.json"
  if [[ -z "$SKILL_SRC" && -f "$local_root/skills/$SKILL_NAME/SKILL.md" ]]; then
    SKILL_SRC="$local_root/skills/$SKILL_NAME"
  fi
}

install_binaries() {
  info "安装到 $BIN_DIR"
  mkdir -p "$BIN_DIR"
  local t optional
  for t in "${TOOLS[@]}"; do
    [[ -f "$SRC_DIR/$t" ]] || die "安装包缺少 $t"
    install -m 755 "$SRC_DIR/$t" "$BIN_DIR/$t"
  done
  # Older releases remain installable; native streaming is explicitly opt-in.
  for optional in desktop_helper wechat_runtime wechat_overlay wechat_bubbles wechat_bot_worker; do
    if [[ -f "$SRC_DIR/$optional" ]]; then
      install -m 755 "$SRC_DIR/$optional" "$BIN_DIR/$optional"
    fi
  done
  xattr -dr com.apple.quarantine "$BIN_DIR" 2>/dev/null || true
  install_layouts
  if [[ -n "$VERSION" ]]; then
    printf '%s\n' "$VERSION" > "$VERSION_FILE"
  fi
  return 0
}

install_layouts() {
  local layout_dir="$HOME/.wechat-rpa/layout"
  local src_layout=""
  local script_path="${BASH_SOURCE[0]:-}" local_root=""
  if [[ -n "$script_path" && -f "$script_path" ]]; then
    local_root="$(cd "$(dirname "$script_path")/.." 2>/dev/null && pwd || true)"
  fi
  if [[ -n "${SRC_DIR:-}" && -d "$SRC_DIR/layout" ]]; then
    src_layout="$SRC_DIR/layout"
  elif [[ -n "${SRC_DIR:-}" && -d "$SRC_DIR/native/layout" ]]; then
    src_layout="$SRC_DIR/native/layout"
  elif [[ -n "$local_root" && -d "$local_root/native/layout" ]]; then
    src_layout="$local_root/native/layout"
  fi
  if [[ -n "$src_layout" ]]; then
    mkdir -p "$layout_dir"
    chmod 700 "$layout_dir"
    for f in "$src_layout"/*.json; do
      [[ -f "$f" ]] || continue
      install -m 600 "$f" "$layout_dir/"
    done
    info "已安装界面地图到 $layout_dir"
  fi
  return 0
}

download_skill_if_missing() {
  local name resource source_root="${SKILL_SRC%/*}" dest="$TMP/skills" found=0
  for name in "${SKILL_NAMES[@]}"; do
    if [[ -n "$SKILL_SRC" ]] && skill_complete "$name" "$source_root"; then
      mkdir -p "$dest"
      cp -R "$source_root/$name" "$dest/$name"
    elif [[ -n "$REF" ]]; then
      while IFS= read -r resource; do
        mkdir -p "$(dirname "$dest/$name/$resource")"
        if ! curl_gh -o "$dest/$name/$resource" "https://raw.githubusercontent.com/$REPO/$REF/skills/$name/$resource"; then
          rm -f "$dest/$name/$resource"
          warn "下载 $name/$resource 失败"
        fi
      done < <(skill_files "$name")
    fi
    if skill_complete "$name" "$dest"; then found=1; fi
  done
  if [[ "$found" == "1" ]]; then SKILL_SRC="$dest/$SKILL_NAME"; fi
  return 0
}

prepare_config() {
  if [[ -f "$CONFIG" ]]; then
    info "保留已有配置 $CONFIG"
    return 0
  fi
  if [[ -n "$EXAMPLE_CONFIG" && -f "$EXAMPLE_CONFIG" ]]; then
    mkdir -p "$(dirname "$CONFIG")"
    install -m 600 "$EXAMPLE_CONFIG" "$CONFIG"
    info "已生成默认配置 $CONFIG"
  else
    warn "未找到 gateway.example.json，跳过默认配置生成（可用 --configure-only 在已安装环境修复）"
  fi
}

run_install() {
  prepare_local_fallbacks
  if [[ "$CONFIGURE_ONLY" == "1" ]]; then
    [[ -x "$BIN" ]] || die "--configure-only 需要已安装的 $BIN"
    VERSION="$(cat "$VERSION_FILE" 2>/dev/null || true)"
    [[ -n "$VERSION" ]] || VERSION="unknown"
    SOURCE_KIND="existing"
    if [[ -z "$SKILL_SRC" && -f "$SKILL_HOME/SKILL.md" ]]; then
      SKILL_SRC="$SKILL_HOME"
    fi
    info "只更新 Agent 配置（跳过二进制安装）"
  else
    resolve_token
    TMP="$(mktemp -d)"
    if [[ "${WECHAT_MCP_FROM_SOURCE:-}" == "1" ]] || ! install_from_release; then
      [[ "${WECHAT_MCP_FROM_SOURCE:-}" == "1" ]] || warn "未找到可用的 Release，改为从源码构建"
      install_from_source
    fi
    install_binaries
    download_skill_if_missing
  fi

  resolve_install_agents
  install_skill_home
  prepare_config

  local id
  for id in "${SELECTED[@]}"; do
    configure_one "$id"
  done
  if printf '%s' "$RESULTS" | grep -qE '\|failed\|'; then
    FAILED=1
  fi

  if [[ "$SKIP_CHECK" != "1" ]]; then
    if run_check; then
      CHECK_STATUS="passed"
      info "MCP 自检通过：$CHECK_DETAIL"
    else
      CHECK_STATUS="failed"
      FAILED=1
      warn "MCP 自检失败：$CHECK_DETAIL"
    fi
  fi

  if [[ "$JSON_OUT" == "1" ]]; then
    print_json_result "install" "$CHECK_STATUS"
  else
    printf '\n✅ 安装完成\n'
    printf '   版本: %s（%s）\n' "${VERSION:-unknown}" "${SOURCE_KIND:-unknown}"
    printf '   二进制: %s\n' "$BIN"
    printf '   配置: %s\n' "$CONFIG"
    if skill_ready; then
      printf '   skill: %s\n' "$SKILL_HOME/SKILL.md"
    else
      printf '   skill: 未安装\n'
    fi
    printf '   Agent:\n'
    print_agents_summary
    if [[ "$CHECK_STATUS" == "passed" ]]; then
      printf '   自检: 通过（个人微信工具已暴露；未执行桌面操作）\n'
    elif [[ "$CHECK_STATUS" == "failed" ]]; then
      printf '   自检: 失败（%s）\n' "$CHECK_DETAIL"
    fi
    if [[ "$AGENT_SPEC" == "none" ]]; then
      print_manual_config
    fi
    cat <<EOF

下一步：
  1. 重启上面列出的 Agent（或重新加载 MCP 服务），让 wechat 服务与 skill 生效。
  2. 验证：重新运行安装脚本并加 --check，仅检查协议和工具列表，不会发送消息。
  3. 只用 fetch_article 抓文章不需要额外权限；微信消息收发/界面功能需要
     系统设置 → 隐私与安全性 中为宿主程序授予 辅助功能、屏幕录制、自动化，并保持微信已登录。
EOF
  fi
  COMPLETED=1
  [[ "$FAILED" -eq 0 ]] || exit 1
}

# ---------- 入口 ----------

[[ "$(uname -s)" == "Darwin" ]] || die "wechat-mcp 仅支持 macOS"
if [[ -z "$INSTALL_HOME" || "$INSTALL_HOME" == "/" || "${INSTALL_HOME%/}" == "$HOME" ]]; then
  die "拒绝使用 ${INSTALL_HOME:-空} 作为安装目录（WECHAT_MCP_HOME）"
fi

case "$ACTION" in
  check) do_check_only ;;
  uninstall) run_uninstall ;;
  install) run_install ;;
esac
