#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "=== Running Task 5 deploy & signing tests ==="

# 1. 检查 setup_signing.sh 是否存在且语法正确
if [ ! -f "scripts/setup_signing.sh" ]; then
  echo "FAIL: scripts/setup_signing.sh does not exist" >&2
  exit 1
fi
bash -n scripts/setup_signing.sh

# 2. setup_signing.sh 参数校验
set +e
./scripts/setup_signing.sh >/dev/null 2>&1
CODE=$?
set -e
if [ $CODE -ne 2 ]; then
  echo "FAIL: scripts/setup_signing.sh without arguments should exit 2, got $CODE" >&2
  exit 1
fi

# 3. 检查 setup_signing.sh 内容是否包含关键配置
grep -q 'WeChat CS Local Signing' scripts/setup_signing.sh || {
  echo "FAIL: scripts/setup_signing.sh must use certificate name 'WeChat CS Local Signing'" >&2
  exit 1
}
grep -q 'codeSigning' scripts/setup_signing.sh || {
  echo "FAIL: scripts/setup_signing.sh must configure codeSigning key usage" >&2
  exit 1
}
grep -q 'security find-certificate' scripts/setup_signing.sh || {
  echo "FAIL: scripts/setup_signing.sh must check if certificate already exists (idempotency)" >&2
  exit 1
}
grep -q 'security import' scripts/setup_signing.sh || {
  echo "FAIL: scripts/setup_signing.sh must import certificate to keychain" >&2
  exit 1
}

# 4. deploy_host.sh dry-run 检查
bash -n scripts/deploy_host.sh

# 检查 --bot 已被删除
set +e
./scripts/deploy_host.sh test-host --bot >/dev/null 2>&1
BOT_CODE=$?
set -e
if [ $BOT_CODE -ne 2 ]; then
  echo "FAIL: deploy_host.sh with --bot should be rejected with exit 2, got $BOT_CODE" >&2
  exit 1
fi

# 检查 dry-run 输出
DRY_RUN_OUTPUT=$(./scripts/deploy_host.sh fake-host)

# 必须不含 --sign - (不能退回临时签名)
if echo "$DRY_RUN_OUTPUT" | grep -q -- '--sign -'; then
  echo "FAIL: deploy_host.sh must NOT use ad-hoc signing '--sign -'" >&2
  exit 1
fi

# 必须包含对 WeChat CS Local Signing 的检查
if ! echo "$DRY_RUN_OUTPUT" | grep -q 'WeChat CS Local Signing'; then
  echo "FAIL: deploy_host.sh dry-run must reference 'WeChat CS Local Signing'" >&2
  exit 1
fi

# 必须在部署时写入 ~/.wechat-rpa/VERSION
if ! echo "$DRY_RUN_OUTPUT" | grep -q 'VERSION'; then
  echo "FAIL: deploy_host.sh dry-run must write to VERSION file" >&2
  exit 1
fi

# 必须对每个二进制产物进行代码签名
if ! echo "$DRY_RUN_OUTPUT" | grep -q 'codesign.*wechat_bot_worker'; then
  echo "FAIL: deploy_host.sh must sign wechat_bot_worker" >&2
  exit 1
fi

if ! echo "$DRY_RUN_OUTPUT" | grep -q 'codesign.*wechat_click'; then
  echo "FAIL: deploy_host.sh must sign wechat_click" >&2
  exit 1
fi

if ! echo "$DRY_RUN_OUTPUT" | grep -q 'codesign.*wechat-mcp'; then
  echo "FAIL: deploy_host.sh must sign wechat-mcp" >&2
  exit 1
fi

if ! echo "$DRY_RUN_OUTPUT" | grep -q 'codesign.*微信客服助手\.app'; then
  echo "FAIL: deploy_host.sh must sign 微信客服助手.app" >&2
  exit 1
fi

echo "=== Task 5 deploy & signing tests passed! ==="
