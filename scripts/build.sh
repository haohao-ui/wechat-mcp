#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked
swiftc native/wechat_ocr.swift -o target/release/wechat_ocr
swiftc native/wechat_click.swift -o target/release/wechat_click
swiftc native/wechat_window.swift -o target/release/wechat_window
swiftc -O -parse-as-library native/wechat_bubbles.swift -o target/release/wechat_bubbles
target/release/wechat_window --self-test
swiftc -parse-as-library native/macos/*.swift -o target/release/wechat_runtime
target/release/wechat_runtime --self-test
swiftc native/overlay/main.swift -o target/release/wechat_overlay
target/release/wechat_overlay --self-test
swiftc -O -target "$(uname -m)-apple-macos12" -parse-as-library -D BOT_WORKER native/wechat_bubbles.swift native/action/*.swift native/bot/*.swift -o target/release/wechat_bot_worker
install -m 755 native/rapidocr_server.py target/release/rapidocr_server.py
