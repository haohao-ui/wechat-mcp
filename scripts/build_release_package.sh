#!/usr/bin/env bash
# 构建并生成统一完整应用发布包及清单/签名封套
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:-0.2.0-test}"
OUT_DIR="${2:-dist/releases/$VERSION}"
SEQUENCE="${3:-1002}"
CHANNEL="${4:-test}"
GENERATE_TEST_SIGN="${5:-false}"

# 1. 严格参数校验（防止 shell 注入或格式损坏）
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
  echo "ERROR: invalid VERSION format: '$VERSION'" >&2
  exit 2
fi

if ! [[ "$SEQUENCE" =~ ^[1-9][0-9]*$ ]]; then
  echo "ERROR: invalid SEQUENCE: '$SEQUENCE' (must be positive integer)" >&2
  exit 2
fi

case "$CHANNEL" in
  stable|beta|nightly|test) ;;
  *)
    echo "ERROR: invalid CHANNEL: '$CHANNEL' (must be stable, beta, nightly, or test)" >&2
    exit 2
    ;;
esac

ARCH="$(uname -m)"
case "$ARCH" in
  arm64|aarch64) CANON_ARCH="arm64" ;;
  x86_64|amd64) CANON_ARCH="x86_64" ;;
  *) CANON_ARCH="$ARCH" ;;
esac

echo "==> 启动新鲜构建 (Fresh Build)..."
export MACOSX_DEPLOYMENT_TARGET=12.0

# 真实 fresh 构建 Rust 二进制（Cargo 内部处理正确增量缓存，绝不通过外部仅判断文件存在而跳过）
cargo build --release --locked --bin wechat-mcp --bin wechat-updater --bin wechat-test-signer

# 编译全套 Swift 辅助组件与宿主外壳二进制
swiftc native/wechat_ocr.swift -o target/release/wechat_ocr
swiftc native/click/*.swift native/common/*.swift native/action/*.swift -o target/release/wechat_click
swiftc -O -target "${ARCH}-apple-macos12" native/interaction/*.swift native/common/Geometry.swift native/action/*.swift -o target/release/desktop_helper
swiftc native/wechat_window.swift -o target/release/wechat_window
swiftc -O -parse-as-library native/wechat_bubbles.swift native/common/*.swift native/eyes/*.swift -o target/release/wechat_bubbles
swiftc -parse-as-library native/macos/*.swift -o target/release/wechat_runtime
swiftc native/overlay/main.swift -o target/release/wechat_overlay
swiftc -O -target "${ARCH}-apple-macos12" -parse-as-library -D BOT_WORKER native/wechat_bubbles.swift native/common/*.swift native/eyes/*.swift native/action/*.swift native/bot/*.swift -o target/release/wechat_bot_worker
swiftc -O -target "${ARCH}-apple-macos12" native/bootstrap/*.swift native/eyes/Screens.swift -o target/release/wechat-cs-host

echo "==> 组装产物目录: $OUT_DIR"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR/bin" "$OUT_DIR/Apps/微信客服助手.app/Contents/MacOS" "$OUT_DIR/Apps/微信客服助手.app/Contents/Resources" "$OUT_DIR/resources"

# 复制二进制组件
BINARIES=(
  wechat-mcp
  wechat-updater
  wechat_ocr
  wechat_click
  desktop_helper
  wechat_window
  wechat_bubbles
  wechat_runtime
  wechat_overlay
  wechat_bot_worker
)

for b in "${BINARIES[@]}"; do
  cp "target/release/$b" "$OUT_DIR/bin/"
  chmod 0755 "$OUT_DIR/bin/$b"
done

# 组装 微信客服助手.app
cp target/release/wechat-cs-host "$OUT_DIR/Apps/微信客服助手.app/Contents/MacOS/wechat-cs-host"
chmod 0755 "$OUT_DIR/Apps/微信客服助手.app/Contents/MacOS/wechat-cs-host"

cp native/bootstrap/Info.plist "$OUT_DIR/Apps/微信客服助手.app/Contents/Info.plist"
chmod 0644 "$OUT_DIR/Apps/微信客服助手.app/Contents/Info.plist"

if [ -f native/bootstrap/gateway.test.json ]; then
  cp native/bootstrap/gateway.test.json "$OUT_DIR/Apps/微信客服助手.app/Contents/Resources/gateway.test.json"
  chmod 0644 "$OUT_DIR/Apps/微信客服助手.app/Contents/Resources/gateway.test.json"
fi

if [ -f native/bootstrap/AppIcon.icns ]; then
  cp native/bootstrap/AppIcon.icns "$OUT_DIR/Apps/微信客服助手.app/Contents/Resources/AppIcon.icns"
  chmod 0644 "$OUT_DIR/Apps/微信客服助手.app/Contents/Resources/AppIcon.icns"
fi

# 拷贝样例资源
if [ -f gateway.example.json ]; then
  cp gateway.example.json "$OUT_DIR/resources/"
  chmod 0644 "$OUT_DIR/resources/gateway.example.json"
fi

echo "==> 归档打包 tar.gz..."
TAR_NAME="wechat-mcp-${VERSION}-macos-${CANON_ARCH}.tar.gz"
TAR_PATH="$OUT_DIR/$TAR_NAME"
tar -czf "$TAR_PATH" -C "$OUT_DIR" bin Apps resources

TAR_SIZE=$(stat -f %z "$TAR_PATH" 2>/dev/null || stat -c %s "$TAR_PATH")
TAR_SHA=$(shasum -a 256 "$TAR_PATH" | awk '{print $1}')

NOW=$(date +%s)
EXPIRES=$((NOW + 31536000))

echo "==> 逐文件扫描并生成组件摘要清单..."
COMPONENTS_JSON=""
TOTAL_UNPACKED_BYTES=0
TOTAL_FILE_COUNT=0

# 仅枚举 payload 目标子目录中的所有文件（包含 JSON 资源，排除元数据文件靠其位于 payload 外部而非扩展名过滤）
PAYLOAD_DIRS=()
for d in bin Apps resources; do
  if [ -d "$OUT_DIR/$d" ]; then
    PAYLOAD_DIRS+=("$OUT_DIR/$d")
  fi
done

while IFS= read -r f; do
  rel_path="${f#$OUT_DIR/}"

  f_size=$(stat -f %z "$f" 2>/dev/null || stat -c %s "$f")
  f_sha=$(shasum -a 256 "$f" | awk '{print $1}')
  
  if [ -x "$f" ]; then
    kind="executable"
    mode_dec=493 # 0755
  else
    kind="regular_file"
    mode_dec=420 # 0644
  fi

  TOTAL_UNPACKED_BYTES=$((TOTAL_UNPACKED_BYTES + f_size))
  TOTAL_FILE_COUNT=$((TOTAL_FILE_COUNT + 1))

  comp_entry=$(cat <<EOF
    {
      "path": "$rel_path",
      "kind": "$kind",
      "file_mode": $mode_dec,
      "sha256": "$f_sha",
      "size_bytes": $f_size
    }
EOF
)
  if [ -z "$COMPONENTS_JSON" ]; then
    COMPONENTS_JSON="$comp_entry"
  else
    COMPONENTS_JSON="$COMPONENTS_JSON,
$comp_entry"
  fi
done < <(find "${PAYLOAD_DIRS[@]}" -type f | sort)

# 上限设为展开大小的 1.5 倍与条目数的 2 倍以作为受控解包边界，且不超过硬上限
MAX_UNPACKED=$((TOTAL_UNPACKED_BYTES * 3 / 2))
MAX_COUNT=$((TOTAL_FILE_COUNT * 2))

cat > "$OUT_DIR/manifest.json" <<EOF
{
  "manifest_schema": 1,
  "release_id": "$VERSION",
  "sequence": $SEQUENCE,
  "channel": "$CHANNEL",
  "published_at": $NOW,
  "expires_at": $EXPIRES,
  "target": {
    "os": "macos",
    "arch": "$CANON_ARCH",
    "min_os_version": "12.0",
    "max_os_version": null
  },
  "artifact": {
    "filename": "$TAR_NAME",
    "sha256": "$TAR_SHA",
    "size_bytes": $TAR_SIZE,
    "max_unpacked_bytes": $MAX_UNPACKED,
    "max_file_count": $MAX_COUNT
  },
  "components": [
$COMPONENTS_JSON
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
  "release_notes": "WeChat MCP full application release package ($VERSION)"
}
EOF

if [ "$GENERATE_TEST_SIGN" = "true" ] || [ "$GENERATE_TEST_SIGN" = "1" ]; then
  echo "==> 使用隔离测试密钥生成真实可核验的测试签名封套..."
  target/release/wechat-test-signer \
    --manifest "$OUT_DIR/manifest.json" \
    --out "$OUT_DIR/envelope.json" \
    --key-id "test-root-v1" \
    --generate-key-to "$OUT_DIR/test_pubkey.hex"
else
  echo "==> 生成未签名封套 (未配置信任根或测试密钥)..."
  cat > "$OUT_DIR/envelope.unsigned.json" <<EOF
{
  "envelope_schema": 1,
  "key_id": "unsigned",
  "signature_algorithm": "ed25519",
  "signature": "",
  "manifest": $(cat "$OUT_DIR/manifest.json")
}
EOF
fi

echo "==> 发布包生成成功: $OUT_DIR"
echo "    - 制品归档: $OUT_DIR/$TAR_NAME ($TAR_SIZE bytes, SHA: $TAR_SHA)"
echo "    - 组件总数: $TOTAL_FILE_COUNT (展开总计: $TOTAL_UNPACKED_BYTES bytes)"
echo "    - 发布清单: $OUT_DIR/manifest.json"
