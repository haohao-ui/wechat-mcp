#!/usr/bin/env bash
# Check tracked and newly added source files, without scanning build/git artifacts.
set -euo pipefail
cd "$(dirname "$0")/.."
# Historical plans, specs, reviews, observations and research preserve audit evidence.
# They are excluded from documentation checks, but never from the .py file ban.
historical="docs/superpowers/plans/ docs/superpowers/specs/ docs/reviews/ docs/observations/ research/"
failed=0
while IFS= read -r -d '' file; do
  [[ -f "$file" ]] || continue
  case "$file" in
    *.py) echo "Python file is forbidden: $file" >&2; failed=1 ;;
    *.md|*.rst|*.txt)
      for prefix in $historical; do
        [[ "$file" != "$prefix"* ]] || continue 2
      done
      case "$file" in
        README.md|docs/*.md|docs/open-api/*|skills/*|scripts/*) ;;
        *) continue ;;
      esac
      if grep -nE 'tests/[^[:space:]`"<>]*\.py|python3' "$file"; then
        echo "Removed Python test reference: $file" >&2
        failed=1
      fi ;;
  esac
done < <(git ls-files -z --cached --others --exclude-standard)
exit "$failed"
