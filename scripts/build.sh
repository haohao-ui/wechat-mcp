#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=12.0
cargo build --release --locked
swiftc native/wechat_ocr.swift -o target/release/wechat_ocr
swiftc native/click/*.swift native/common/*.swift native/action/*.swift -o target/release/wechat_click
swiftc -O -target "$(uname -m)-apple-macos12" native/interaction/*.swift native/common/Geometry.swift native/action/*.swift -o target/release/desktop_helper
swiftc native/wechat_window.swift -o target/release/wechat_window
swiftc -O -parse-as-library native/wechat_bubbles.swift native/common/*.swift native/eyes/*.swift -o target/release/wechat_bubbles
target/release/wechat_window --self-test
target/release/wechat-updater --version
swiftc -parse-as-library native/macos/*.swift -o target/release/wechat_runtime
target/release/wechat_runtime --self-test
swiftc native/overlay/main.swift -o target/release/wechat_overlay
target/release/wechat_overlay --self-test
swiftc -O -target "$(uname -m)-apple-macos12" -parse-as-library -D BOT_WORKER native/wechat_bubbles.swift native/common/*.swift native/eyes/*.swift native/action/*.swift native/bot/*.swift -o target/release/wechat_bot_worker
