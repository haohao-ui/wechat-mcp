#!/bin/sh
# Dev-only: capture the WeChat main window (no shadow) plus its window JSON for fixture calibration.
# Usage: capture_frame.sh <ssh-host|local> <out-dir> <name>
# Output stays in <out-dir> (use a scratch dir); redact before copying anything into tests/fixtures.
set -eu
host=$1 out=$2 name=$3
mkdir -p "$out"
remote='set -eu
info=$("$HOME/.wechat-rpa/bin/wechat_window")
id=$(printf "%s" "$info" | /usr/bin/plutil -extract window_id raw -o - -)
tmp=$(mktemp -d)
/usr/sbin/screencapture -l "$id" -o -x "$tmp/f.png"
printf "%s" "$info" > "$tmp/f.json"
echo "$tmp"'
if [ "$host" = local ]; then
  tmp=$(sh -c "$remote")
  mv "$tmp/f.png" "$out/$name.png"; mv "$tmp/f.json" "$out/$name.window.json"; rmdir "$tmp"
else
  tmp=$(ssh -o LogLevel=ERROR "$host" "$remote")
  scp -q "$host:$tmp/f.png" "$out/$name.png"; scp -q "$host:$tmp/f.json" "$out/$name.window.json"
  ssh -o LogLevel=ERROR "$host" "rm -rf '$tmp'"
fi
chmod 600 "$out/$name.png" "$out/$name.window.json"
echo "$out/$name.png"
