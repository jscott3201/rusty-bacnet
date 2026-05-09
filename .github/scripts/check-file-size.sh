#!/usr/bin/env bash
# 700-LOC cap per source file (modularity guideline). Counts non-empty,
# non-comment lines in tracked Rust source files; flags any that exceed
# the cap. Runs from repo root. Compatible with macOS bash 3.x (no
# mapfile / no readarray).
#
# Two-mode operation:
#   - CHECK_FILE_SIZE_WARN=1  : print violations, exit 0 (warn-only).
#                                Use during the rollout while track-2
#                                splits land.
#   - default                 : print violations, exit 1 (strict).

set -euo pipefail

CAP=700
violations=0

while IFS= read -r f; do
  [ -f "$f" ] || continue
  loc=$(grep -cvE '^\s*(//.*)?$' "$f" || true)
  if [ "$loc" -gt "$CAP" ]; then
    echo "FAIL: $f has $loc LOC (cap: $CAP)"
    violations=$((violations + 1))
  fi
done < <(git ls-files '*.rs' 2>/dev/null | grep -v -E '^(target|generated|out)/' || true)

if [ "$violations" -gt 0 ]; then
  echo
  echo "Refactor or split files exceeding the $CAP LOC cap."
  if [ "${CHECK_FILE_SIZE_WARN:-0}" = "1" ]; then
    echo "(warn-only mode: not failing CI)"
    exit 0
  fi
  exit 1
fi

echo "OK: all tracked .rs files within the $CAP LOC cap."
