# 让 Agent 自动安装 wechat-mcp

把下面整段提示词发给任意一个能执行终端命令的 AI Agent（Claude Code、Codex CLI、Gemini CLI、Cursor、Windsurf、Cline、OpenCode 等）。安装、MCP 注册、skill 安装和自检都由 `scripts/install.sh` 完成，Agent 不再手改各客户端配置；Claude Desktop 没有终端，用它自己的手动命令并上传打包好的 skill。

仓库是私有的，需要一个有 `repo`（或 Contents: Read）权限的 GitHub Token。建议先在终端里 `export GITHUB_TOKEN=...`（或提前 `gh auth login`）再启动 Agent；没有的话，Agent 会向你索取。

## 安装提示词

````text
请帮我安装 wechat-mcp（macOS 微信 MCP，含公众号文章抓取工具 fetch_article）和配套的 wechat-article skill。安装、MCP 注册、skill 安装和自检都由安装脚本完成，请不要手动修改任何客户端配置。

【第 1 步：准备 Token】
如果环境变量 GITHUB_TOKEN 为空，先执行：
  command -v gh >/dev/null 2>&1 && export GITHUB_TOKEN="$(gh auth token)"
如果仍为空，停下来向我要一个有 repo（或 Contents: Read）权限的 GitHub Token；拿到后 export 再继续。不要把 Token 写进文件，也不要在回复里回显。

【第 2 步：执行唯一一条安装命令】
原样执行下面的命令（一行），不要改参数，不要拆成手动步骤：
  set -o pipefail; curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash -s -- --agent auto --json

【第 3 步：根据结果处理】
- 退出码 0：脚本最后一行是 JSON。用简短列表汇报 version、每个 agents[].id 的 mcp/skill 状态；skill 为 manual 的 Agent 把 note 一起告诉我。然后提醒我重启当前 Agent，让 wechat 服务和 skill 生效。
- 如果报“无法自动识别当前 Agent”：按你的身份把 --agent auto 换成下面之一，再执行一次第 2 步：
  Claude Code→claude-code  Codex CLI→codex  Gemini CLI→gemini  Cursor→cursor
  Windsurf→windsurf  Cline→cline  OpenCode→opencode
- 如果 curl 报 401/404，或命令没有输出 JSON：回到第 1 步向我要 Token。
- 其他失败：把 stderr 最后几行和已有 JSON 原样告诉我，不要自行改用其他安装方式。
````

## 安装后

- 重启 Agent（或重新加载 MCP 服务），wechat 服务和 skill 才会生效。
- 自检：把上面的第 2 步命令中的 `--agent auto` 换成 `--check` 再运行一次（输出 `"check":"passed"` 即正常）。
- 只用 `fetch_article` 抓公众号文章不需要 macOS 隐私权限；微信消息收发、界面操作等功能需要为运行 MCP 的宿主程序授予「辅助功能」「屏幕录制」「自动化」权限，并保持微信已登录。

## 卸载提示词

````text
请帮我卸载 wechat-mcp。先准备 GITHUB_TOKEN（为空时执行 command -v gh >/dev/null 2>&1 && export GITHUB_TOKEN="$(gh auth token)"，仍为空就向我要），然后执行：
  set -o pipefail; curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash -s -- --uninstall --json
用简短列表汇报 JSON 里每个 agents[].id 和 note；gateway.json 与客服数据库默认保留。如需要，再提醒我重启相关 Agent。
````

## 手动安装 / 指定 Agent

```bash
export GITHUB_TOKEN=ghp_xxx
set -o pipefail

# 只安装并自动识别当前 Agent（识别不到时退回配置 Claude Code）
curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" \
  https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash

# 一次配置所有检测到已安装的客户端
curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" \
  https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash -s -- --agent all

# 指定客户端（可逗号分隔）
curl -fsSL -H "Authorization: Bearer $GITHUB_TOKEN" \
  https://raw.githubusercontent.com/haohao-ui/wechat-mcp/main/scripts/install.sh | bash -s -- --agent codex,gemini
```

`--agent` 取值：`claude-code`、`codex`、`gemini`、`cursor`、`windsurf`、`cline`、`opencode`、`claude-desktop`、`auto`、`all`、`none`。

常用选项：

| 选项 | 作用 |
|---|---|
| `--agent <id>` | 指定目标 Agent；`auto` 识别当前 Agent，`all` 配置所有已安装客户端，`none` 只装二进制和 skill |
| `--check` | 只做 MCP 自检（initialize + tools/list，确认 `fetch_article` 可用） |
| `--configure-only` | 不下载二进制，只写 Agent 配置和 skill（修复配置时用） |
| `--uninstall` | 卸载二进制、skill 和所有已知 Agent 中的 `wechat` 配置，保留 `gateway.json` 与数据库 |
| `--json` | 输出机器可读的 JSON 摘要，人类日志走 stderr |

`--json` 成功时最后一行形如：

```json
{"ok":true,"action":"install","version":"v0.1.0","source":"release","home":"/path/to/.wechat-mcp","bin":"/path/to/.wechat-mcp/bin/wechat-mcp","config":"/path/to/.wechat-mcp/gateway.json","skill":"/path/to/.wechat-mcp/skills/wechat-article/SKILL.md","check":"passed","agents":[{"id":"codex","mcp":"registered","skill":"installed","note":"codex mcp add"}],"next":["重启或重新加载上面列出的 Agent，让 wechat 服务与 skill 生效"]}
```

退出码：`0` 表示全部成功；`1` 表示下载/安装/自检或某个 Agent 配置失败，具体原因在 stderr。

## 说明

- 脚本会按客户端选择最稳妥的注册方式：Claude Code / Codex / Gemini CLI 优先用各自的 `mcp add` 命令，Cursor / Windsurf / Cline / OpenCode / Claude Desktop 直接合并对应的 JSON 配置；失败时自动回退到直接写配置，并在修改前生成 `.bak` 备份。
- skill 安装位置：Claude Code → `~/.claude/skills/`；Codex → `~/.agents/skills/` 或 `~/.codex/skills/`；Gemini / Windsurf / Cline / OpenCode 没有统一 skill 目录，改为写入各自的全局指令文件（GEMINI.md、global_rules.md、Cline Rules、AGENTS.md）；Cursor 的用户规则只能手动粘贴；Claude Desktop 会打包成 `~/Desktop/wechat-article-skill.zip` 并提示上传。
- 各客户端配置路径以官方文档为准。`--agent <id>` 显式指定时会创建该客户端的配置目录；`--agent all` 只包含检测到已安装的客户端，不会凭空创建无关文件。
- skill 的原始文件在仓库 [`skills/wechat-article/SKILL.md`](../skills/wechat-article/SKILL.md)，安装后位于 `~/.wechat-mcp/skills/wechat-article/`。
