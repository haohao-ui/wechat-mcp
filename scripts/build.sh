#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked
swiftc native/wechat_ocr.swift -o target/release/wechat_ocr
swiftc native/wechat_click.swift -o target/release/wechat_click
