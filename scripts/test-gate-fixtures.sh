#!/usr/bin/env bash
# Independent repositories: never mutate the checkout or real user state.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/.github/workflows" "$fixture/docs/business"
cp "$root"/scripts/{test-gates,check_no_python}.sh "$fixture/scripts/"
cp "$root"/.github/workflows/{ci,release}.yml "$fixture/.github/workflows/"
cp "$root/scripts/build.sh" "$fixture/scripts/"
git -C "$fixture" init -q
for name in build_model build_compliance_model render_report; do
  : > "$fixture/docs/business/$name.py"
done
git -C "$fixture" add .
pass() { (cd "$fixture" && bash scripts/test-gates.sh) > "$fixture/output" 2>&1 || { cat "$fixture/output"; echo "FAIL: $1"; exit 1; }; }
reject() { if (cd "$fixture" && bash scripts/test-gates.sh) > "$fixture/output" 2>&1; then echo "FAIL: $1 accepted"; exit 1; fi; }
pass 'three offline helpers and hard CI'
for path in docs/business/new.py scripts/probe.py native/probe.py src/probe.py docs/other/probe.py; do
  mkdir -p "$fixture/$(dirname "$path")"
  : > "$fixture/$path"
  reject "$path"
  rm "$fixture/$path"
done
cp "$fixture/.github/workflows/ci.yml" "$fixture/ci.original"
for step in 'cargo fmt' 'cargo clippy'; do
  awk -v step="$step" '{print; if ($0 == "      - name: " step) print "        continue-on-error: true"}' "$fixture/ci.original" > "$fixture/.github/workflows/ci.yml"
  reject "$step soft gate"
done
for command in 'rustfmt --edition 2024 --check' 'cargo clippy --all-targets --locked'; do
  awk -v command="$command" 'index($0, command) {print "        run: true"; next} {print}' "$fixture/ci.original" > "$fixture/.github/workflows/ci.yml"
  reject "missing $command execution"
done
cp "$fixture/ci.original" "$fixture/.github/workflows/ci.yml"
for entry in scripts/build.sh scripts/install.sh src/probe.rs native/probe.swift .github/workflows/ci.yml .github/workflows/release.yml; do
  mkdir -p "$fixture/$(dirname "$entry")"
  if [[ -f "$fixture/$entry" ]]; then cp "$fixture/$entry" "$fixture/saved"; fi
  printf '\n# docs/business/build_model.py\n' >> "$fixture/$entry"
  reject "runtime reference in $entry"
  if [[ -f "$fixture/saved" ]]; then mv "$fixture/saved" "$fixture/$entry"; else rm "$fixture/$entry"; fi
done
printf 'python3 removed\n' > "$fixture/scripts/probe.md"
reject 'active document Python reference'
rm "$fixture/scripts/probe.md"
pass 'restored hard CI'
echo 'PASS gate regression fixtures'
