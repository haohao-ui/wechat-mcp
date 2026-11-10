#!/usr/bin/env bash
# 阻止本机信息与敏感文件进入仓库。CI 与本地 pre-push hook 共用。
# 用法: scripts/check_sensitive.sh            # 检查所有已跟踪文件
#       scripts/check_sensitive.sh --staged   # 只检查暂存区（pre-commit）
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "${1:-}" == "--staged" ]]; then
  files=$(git -c core.quotepath=false diff --cached --name-only --diff-filter=ACMR)
else
  files=$(git -c core.quotepath=false ls-files)
fi
[[ -z "$files" ]] && exit 0

fail=0

# 1. 禁止提交的文件类型（真实配置、数据库、证书、原生二进制）
bad_files=$(printf '%s\n' "$files" | grep -E \
  '(^|/)(\.env(\..*)?|gateway\.json|.*\.local\.json|.*\.(sqlite3|sqlite3-wal|sqlite3-shm|db|db-wal|db-shm|pem|key|p12|mobileprovision))$|^bin/' \
  | grep -vE '\.example\.' || true)
if [[ -n "$bad_files" ]]; then
  echo "❌ 以下文件不允许提交（本地配置/数据/证书/二进制）:"
  printf '%s\n' "$bad_files" | sed 's/^/  /'
  fail=1
fi

# 2. 本机路径与个人信息（本脚本自身除外）
patterns=(
  '/Users/[A-Za-z0-9._-]+'
  '/home/[A-Za-z0-9._-]+/'
  '/private/(var|tmp)/'
  '[A-Za-z0-9._%+-]+@(outlook|qq|163|126|gmail|foxmail|icloud)\.com'
  '(^|[^0-9])1[3-9][0-9]{9}([^0-9]|$)'
  '(corpsecret|secret|token|api[_-]?key|password)["'"'"']?[[:space:]]*[:=][[:space:]]*["'"'"'][A-Za-z0-9_\-]{16,}'
)
scan_files=$(printf '%s\n' "$files" | grep -vE '^(scripts/check_sensitive\.sh|Cargo\.lock)$' || true)
for p in "${patterns[@]}"; do
  # One file per line, unquoted (core.quotepath=false): names with spaces or non-ASCII characters
  # are scanned too instead of being skipped as missing files.
  hits=$(while IFS= read -r f; do
    if [[ -n "$f" && -f "$f" ]]; then grep -nIEH -- "$p" "$f" || true; fi
  done <<< "$scan_files")
  if [[ -n "$hits" ]]; then
    echo "❌ 发现疑似本机/个人敏感信息（模式: ${p}）:"
    echo "$hits" | sed 's/^/  /'
    fail=1
  fi
done

if [[ $fail -ne 0 ]]; then
  echo
  echo "请移除上述内容（路径请改用 /path/to/... 占位）后再提交。"
  exit 1
fi
echo "✅ 敏感信息检查通过"
