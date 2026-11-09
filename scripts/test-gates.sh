#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
for file in scripts/build.sh .github/workflows/ci.yml .github/workflows/release.yml; do
  if grep -Fq 'native/click/main.swift' "$file"; then
    echo "FAIL: $file omits click/Input.swift" >&2
    exit 1
  fi
done
bash scripts/check_no_python.sh
for file in .github/workflows/ci.yml .github/workflows/release.yml; do
  grep -Fq 'dtolnay/rust-toolchain@1.99.0' "$file"
  if grep -Eq 'python3|macos13' "$file"; then
    echo "FAIL: stale Python or deployment target in $file" >&2
    exit 1
  fi
done
grep -Fq 'continue-on-error: true' .github/workflows/ci.yml
if grep -Fq 'needs: security' .github/workflows/ci.yml; then
  echo 'FAIL: test job still depends on security' >&2
  exit 1
fi
# Execute the release guard with exactly the environment GitHub supplies.
guard=$(sed -n '/if \[\[ "$GITHUB_REF_TYPE"/,/          fi/p' .github/workflows/release.yml)
if GITHUB_REF_TYPE=branch GITHUB_REF_NAME=main bash -c "$guard"; then
  echo 'FAIL: release accepted a branch' >&2
  exit 1
fi
GITHUB_REF_TYPE=tag GITHUB_REF_NAME=v1.2.3 bash -c "$guard"
echo 'PASS build, CI and release gates'
fixture=$(mktemp -d scripts/.gate-test.XXXXXX)
trap 'rm -rf "$fixture"' EXIT
: > "$fixture/probe.py"
if bash scripts/check_no_python.sh > /dev/null 2>&1; then
  echo 'FAIL: Python source escaped guard' >&2
  exit 1
fi
rm "$fixture/probe.py"
printf '%s\n' 'python3 tests/removed.py' > "$fixture/probe.md"
if bash scripts/check_no_python.sh > /dev/null 2>&1; then
  echo 'FAIL: removed test command escaped guard' >&2
  exit 1
fi
rm "$fixture/probe.md"
echo 'PASS Python source and active-document negative fixtures'
