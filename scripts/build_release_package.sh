#!/usr/bin/env bash
# 构建并生成统一发布包及签名封套（用于本地合成包验证与 Release 打包）
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:-0.2.0-test}"
ARCH="$(uname -m)"
OUT_DIR="${2:-dist/releases/$VERSION}"
KEY_ID="${3:-test-root-v1}"

mkdir -p "$OUT_DIR/bin" "$OUT_DIR/resources"

echo "==> 准备版本二进制组件..."
# 确保目标二进制存在
[ -f target/release/wechat-mcp ] || cargo build --release --locked --bin wechat-mcp
[ -f target/release/wechat-updater ] || cargo build --release --locked --bin wechat-updater

cp target/release/wechat-mcp "$OUT_DIR/bin/"
cp target/release/wechat-updater "$OUT_DIR/bin/"
chmod 0755 "$OUT_DIR/bin/wechat-mcp" "$OUT_DIR/bin/wechat-updater"

# 拷贝样例资源
if [ -f gateway.example.json ]; then
  cp gateway.example.json "$OUT_DIR/resources/"
  chmod 0644 "$OUT_DIR/resources/gateway.example.json"
fi

echo "==> 打包 tar.gz..."
TAR_NAME="wechat-mcp-${VERSION}-macos-${ARCH}.tar.gz"
TAR_PATH="$OUT_DIR/$TAR_NAME"
tar -czf "$TAR_PATH" -C "$OUT_DIR" bin resources

TAR_SIZE=$(stat -f %z "$TAR_PATH" 2>/dev/null || stat -c %s "$TAR_PATH")
TAR_SHA=$(shasum -a 256 "$TAR_PATH" | awk '{print $1}')

MCP_SIZE=$(stat -f %z "$OUT_DIR/bin/wechat-mcp" 2>/dev/null || stat -c %s "$OUT_DIR/bin/wechat-mcp")
MCP_SHA=$(shasum -a 256 "$OUT_DIR/bin/wechat-mcp" | awk '{print $1}')

UPDATER_SIZE=$(stat -f %z "$OUT_DIR/bin/wechat-updater" 2>/dev/null || stat -c %s "$OUT_DIR/bin/wechat-updater")
UPDATER_SHA=$(shasum -a 256 "$OUT_DIR/bin/wechat-updater" | awk '{print $1}')

NOW=$(date +%s)
EXPIRES=$((NOW + 31536000))

echo "==> 生成规范化 release manifest..."
cat > "$OUT_DIR/manifest.json" <<EOF
{
  "manifest_schema": 1,
  "release_id": "$VERSION",
  "sequence": 1002,
  "channel": "stable",
  "published_at": $NOW,
  "expires_at": $EXPIRES,
  "target": {
    "os": "macos",
    "arch": "$ARCH",
    "min_os_version": "12.0",
    "max_os_version": null
  },
  "artifact": {
    "filename": "$TAR_NAME",
    "sha256": "$TAR_SHA",
    "size_bytes": $TAR_SIZE,
    "max_unpacked_bytes": 150000000,
    "max_file_count": 500
  },
  "components": [
    {
      "path": "bin/wechat-mcp",
      "kind": "executable",
      "file_mode": 493,
      "sha256": "$MCP_SHA",
      "size_bytes": $MCP_SIZE
    },
    {
      "path": "bin/wechat-updater",
      "kind": "executable",
      "file_mode": 493,
      "sha256": "$UPDATER_SHA",
      "size_bytes": $UPDATER_SIZE
    }
  ],
  "compatibility": {
    "config_schema_min": 1,
    "config_schema_max": 2,
    "profile_schema_min": 1,
    "profile_schema_max": 2,
    "history_schema_min": 1,
    "history_schema_max": 3,
    "min_updater_version": 1,
    "in_place_rollback_supported": true
  },
  "release_notes": "WeChat MCP production updater package"
}
EOF

# 生成简单签名封套示例（纯测试封套）
cat > "$OUT_DIR/envelope.json" <<EOF
{
  "envelope_schema": 1,
  "key_id": "$KEY_ID",
  "signature_algorithm": "ed25519",
  "signature": "$(printf '0%.0s' {1..128})",
  "manifest": $(cat "$OUT_DIR/manifest.json")
}
EOF

echo "==> 发布包生成完成: $OUT_DIR"
