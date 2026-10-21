#!/usr/bin/env bash
# Build and start the NAT topology (docker-compose.yml here), run both
# probes, and tear it down. Exits non-zero when either probe misses a
# device. A manual check (#942): CI runs it only through the
# workflow_dispatch workflow .github/workflows/nat-docker.yml.
set -euo pipefail

cd "$(dirname "$0")"
compose=(docker compose -p bacnet-nat -f docker-compose.yml)

cleanup() {
  "${compose[@]}" --profile probe down --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

"${compose[@]}" --profile probe build
"${compose[@]}" up -d nat-a nat-b nat-c bbmd-a bbmd-b
# Let the BBMDs bind before the probes register.
sleep 3

status=0
for probe in probe probe-behind-nat; do
  echo "--- $probe"
  if ! "${compose[@]}" --profile probe run --rm "$probe"; then
    status=1
  fi
done
if [ "$status" -ne 0 ]; then
  echo "--- BBMD logs"
  "${compose[@]}" logs bbmd-a bbmd-b || true
fi
exit "$status"
