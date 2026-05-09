#!/usr/bin/env bash
# Baseline no-secret scan. Greps tracked files for common secret-shaped
# patterns. `_spec/` is excluded — it's an untracked working area, but
# the exclusion is defense-in-depth in case anyone accidentally tracks
# coordination notes that quote credentials.

set -euo pipefail

violations=0

scan() {
  local pattern="$1"
  local label="$2"
  if git grep -nE "$pattern" -- ':!_spec' >/dev/null 2>&1; then
    echo "FAIL: matched $label pattern in tracked files:"
    git grep -nE "$pattern" -- ':!_spec' || true
    violations=$((violations + 1))
  fi
}

scan 'AKIA[0-9A-Z]{16}' 'AWS access key id'
scan '-----BEGIN (RSA|EC|OPENSSH|PGP) PRIVATE KEY-----' 'private key block'
scan 'xox[abpr]-[A-Za-z0-9-]{10,}' 'Slack token'
scan 'gh[pousr]_[A-Za-z0-9]{36,}' 'GitHub token'
scan 'sk-[A-Za-z0-9_-]{20,}' 'API token (sk- prefix)'

if [ "$violations" -gt 0 ]; then
  echo
  echo "Remove secrets from the working tree, rotate compromised credentials, and rewrite history."
  exit 1
fi

echo "OK: baseline no-secret scan clean."
