#!/usr/bin/env bash
# Cache-pruning regressions (#1471): fake gh and sleep, no network or delays.
set -euo pipefail

prune="$(cd "$(dirname "$0")" && pwd)/prune-caches.sh"
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir "$fixture/bin"
export PRUNE_FIXTURE=$fixture
# A closed PATH makes it impossible to fall through to a real gh or sleep.
ln -s "$BASH" "$fixture/bin/bash"
for tool in cat chmod grep rm sed sort tail awk; do
  ln -s "$(command -v "$tool")" "$fixture/bin/$tool"
done
export PATH="$fixture/bin"
export GITHUB_REPOSITORY=owner/repo GITHUB_SHA=current

cat >"$fixture/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
root=$PRUNE_FIXTURE
printf '%s\n' "$*" >>"$root/calls"
case "$*" in
  'api repos/owner/repo/commits/dev --jq .sha')
    [ ! -f "$root/head-failure" ] || exit 1
    cat "$root/head" ;;
  'api --paginate repos/owner/repo/actions/caches?per_page=100 --jq '*)
    [ ! -f "$root/list-failure" ] || exit 1
    cat "$root/entries" ;;
  'api -X DELETE repos/owner/repo/actions/caches/'*)
    id=${4##*/}
    count=0
    [ ! -f "$root/count-$id" ] || read -r count <"$root/count-$id"
    count=$((count + 1))
    echo "$count" >"$root/count-$id"
    echo "delete $id" >>"$root/events"
    response=$(sed -n "${count}p" "$root/responses-$id")
    case "$response" in
      ok) exit 0 ;;
      404) message='Not Found' ;;
      403) message='Resource not accessible by integration' ;;
      400|401|409|422|429|500|501|502|503|504) message='Server Error' ;;
      raw-504) echo 'gh: HTTP 504' >&2; exit 1 ;;
      raw-404) echo 'gh: HTTP 404' >&2; exit 1 ;;
      compact-404|compact-504)
        # What gh really prints: a compact body with no trailing newline,
        # then the diagnostic (seen on dev, #2138's merge run).
        message='Server Error'
        [ "$response" != compact-404 ] || message='Not Found'
        printf '{"message":"%s","status":"%s"}' "$message" "${response#compact-}"
        printf 'gh: %s (HTTP %s)\n' "$message" "${response#compact-}" >&2
        exit 1 ;;
      guidance-404|guidance-504)
        message='Server Error'
        [ "$response" != guidance-404 ] || message='Not Found'
        printf 'gh: %s (HTTP %s)\n' "$message" "${response#guidance-}" >&2
        echo 'gh: This API operation needs the "repo" scope. To request it, run: gh auth refresh -h github.com -s repo' >&2
        exit 1 ;;
      earlier-diagnostic)
        # A plain response body can resemble a diagnostic; gh's actual
        # status is the last matching line, before its trailing guidance.
        echo 'gh: upstream failure described in the body (HTTP 502)'
        echo 'gh: Forbidden (HTTP 403)' >&2
        echo 'Request access from your administrator.' >&2
        exit 1 ;;
      body-502|body-404)
        printf '{"message": "an earlier HTTP %s failed"}\n' "${response#body-}"
        echo 'gh: Forbidden (HTTP 403)' >&2
        exit 1 ;;
      message-502) message='an earlier HTTP 502 failed'; response=403 ;;
      long-code) echo 'gh: Server Error (HTTP 5020)' >&2; exit 1 ;;
      plain-mention) echo 'request failed after HTTP 502 was mentioned' >&2; exit 1 ;;
      network) echo 'connection reset by peer' >&2; exit 1 ;;
      *) echo "unexpected fixture response: $response" >&2; exit 1 ;;
    esac
    printf '{\n  "message": "%s"\n}\n' "$message"
    printf 'gh: %s (HTTP %s)\n' "$message" "$response" >&2
    exit 1 ;;
  *) echo "unexpected gh arguments: $*" >&2; exit 1 ;;
esac
STUB
cat >"$fixture/bin/sleep" <<'STUB'
#!/usr/bin/env bash
printf 'sleep %s\n' "$*" >>"$PRUNE_FIXTURE/events"
[ ! -f "$PRUNE_FIXTURE/sleep-failure" ]
STUB
chmod +x "$fixture/bin/gh" "$fixture/bin/sleep"

cases=0
fail() {
  echo "FAIL: prune-caches regression: $1" >&2
  cat "$fixture/result" >&2
  exit 1
}
reset() {
  rm -f "$fixture"/count-* "$fixture"/responses-* "$fixture"/*-failure
  : >"$fixture/calls"
  : >"$fixture/events"
  : >"$fixture/entries"
  echo current >"$fixture/head"
}
# entry <id> <ref> <key> <day>: fixtures are deliberately supplied unsorted.
entry() { printf '%s\t%s\t%s\t2026-10-%sT00:00:00Z\t1048576\n' "$@" >>"$fixture/entries"; }
responses() { local id=$1; shift; printf '%s\n' "$@" >"$fixture/responses-$id"; }
expect() {
  local expected=$1 name=$2 status=0
  shift 2
  bash "$prune" "$@" >"$fixture/result" 2>&1 || status=$?
  [ "$status" -eq "$expected" ] || fail "$name: expected exit $expected, got $status"
  cases=$((cases + 1))
  echo "ok: $name"
}
events() { [ "$(cat "$fixture/events")" = "$1" ] || fail "unexpected delete/wait sequence"; }
output() { grep -qF -- "$1" "$fixture/result" || fail "missing output: $1"; }
errors() { [ "$(grep -c '^::error::' "$fixture/result" || :)" -eq "$1" ] || fail "wrong error count"; }
single() {
  reset
  entry 1 refs/pull/1/merge v0-rust-test-Linux-x64-tool-lock 01
}

single
responses 1 ok
expect 0 'immediate success has no retry'
events 'delete 1'
output 'removed 1 entries, 1 MiB, leaving 0 MiB'
errors 0

for status in 502 503 504 raw-504 guidance-504 compact-504; do
  single
  responses 1 "$status" ok
  expect 0 "$status is retried and recovers"
  events $'delete 1\nsleep 1\ndelete 1'
  output 'attempt 2/3 in 1s'
  output 'removed 1 entries, 1 MiB, leaving 0 MiB'
  errors 0
done

single
responses 1 502 503 ok
expect 0 'two transient failures recover on the last attempt'
events $'delete 1\nsleep 1\ndelete 1\nsleep 2\ndelete 1'
output 'retrying cache 1 after HTTP 503: attempt 3/3 in 2s'
output 'removed 1 entries, 1 MiB, leaving 0 MiB'
errors 0

for status in 502 503 504; do
  single
  responses 1 "$status" "$status" "$status" ok
  expect 1 "persistent HTTP $status stops at three attempts"
  events $'delete 1\nsleep 1\ndelete 1\nsleep 2\ndelete 1'
  output "gh: Server Error (HTTP $status)"
  output 'removed 0 entries, 0 MiB, leaving 1 MiB'
  errors 1
done

for status in 400 401 403 409 422 429 500 501 network \
  body-502 body-404 message-502 long-code plain-mention earlier-diagnostic; do
  single
  responses 1 "$status" ok
  expect 1 "$status fails without retry or success accounting"
  events 'delete 1'
  output 'removed 0 entries, 0 MiB, leaving 1 MiB'
  errors 1
done

for status in 404 raw-404 guidance-404 compact-404; do
  single
  responses 1 "$status"
  expect 0 "$status is already-gone success without retry"
  events 'delete 1'
  output 'already gone:'
  output 'removed 1 entries, 1 MiB, leaving 0 MiB'
  errors 0
done

single
responses 1 502 404
expect 0 'ambiguous deletion followed by 404 counts once'
events $'delete 1\nsleep 1\ndelete 1'
output 'already gone:'
output 'removed 1 entries, 1 MiB, leaving 0 MiB'
errors 0

reset
entry 1 refs/pull/1/merge first 02
entry 2 refs/pull/2/merge second 01
responses 1 502 503 504
responses 2 ok
expect 1 'exhaustion preserves failure and continues to the next entry'
events $'delete 1\nsleep 1\ndelete 1\nsleep 2\ndelete 1\ndelete 2'
output 'deleted second'
output 'removed 1 entries, 1 MiB, leaving 1 MiB'
errors 1

reset
entry 1 refs/pull/1/merge first 02
entry 2 refs/pull/2/merge second 01
responses 1 502 503 ok
responses 2 504 502 ok
expect 0 'each entry receives its own three-attempt budget'
events $'delete 1\nsleep 1\ndelete 1\nsleep 2\ndelete 1\ndelete 2\nsleep 1\ndelete 2\nsleep 2\ndelete 2'
output 'removed 2 entries, 2 MiB, leaving 0 MiB'
errors 0

single
responses 1 502 ok
: >"$fixture/sleep-failure"
expect 1 'a failed wait does not continue retrying'
events $'delete 1\nsleep 1'
output 'could not wait before retrying cache 1'
output 'removed 0 entries, 0 MiB, leaving 1 MiB'
errors 1

# Hand-selected expected IDs verify sorting and retention independently.
reset
entry 1 refs/heads/dev v0-rust-test-Linux-x64-old-lock 01
entry 2 refs/heads/dev v0-rust-test-Linux-x64-new-lock 03
entry 3 refs/heads/dev v0-rust-test-Linux-x64-mid-lock 02
entry 4 refs/heads/dev v0-rust-clippy-Linux-x64-old-lock 04
entry 5 refs/heads/dev v0-rust-clippy-Linux-x64-new-lock 06
entry 6 refs/heads/dev v0-rust-clippy-Linux-x64-mid-lock 05
entry 7 refs/heads/dev node-cache 07
entry 8 refs/heads/feature feature-cache 08
entry 9 refs/pull/9/merge pull-cache 09
entry 10 refs/heads/main main-cache 10
for id in 1 4 8 9 10; do responses "$id" ok; done
expect 0 'keep two newest per dev Rust family and other dev tools'
events $'delete 10\ndelete 9\ndelete 8\ndelete 4\ndelete 1'
output 'removed 5 entries, 5 MiB, leaving 5 MiB'

: >"$fixture/events"
expect 0 'dry run retains selection and never mutates' --dry-run
events ''
output 'would remove 5 entries, 5 MiB, leaving 5 MiB'
[ "$(grep -c '^would delete ' "$fixture/result")" -eq 5 ] || fail 'dry-run selection'

single
responses 1 ok
GITHUB_SHA=stale expect 0 'an older dev run exits before listing or deleting'
events ''
[ "$(cat "$fixture/calls")" = 'api repos/owner/repo/commits/dev --jq .sha' ] || fail 'stale-head API calls'
output 'dev is now at current'

single
unset GITHUB_SHA
expect 1 'missing GITHUB_SHA refuses destructive operation'
events ''
[ "$(cat "$fixture/calls")" = 'api repos/owner/repo/commits/dev --jq .sha' ] || fail 'missing-SHA API calls'
output 'GITHUB_SHA is not set'
expect 0 'missing GITHUB_SHA still permits dry run' --dry-run
events ''
output 'would remove 1 entries, 1 MiB, leaving 0 MiB'
export GITHUB_SHA=current

for stage in head list; do
  single
  : >"$fixture/$stage-failure"
  expect 1 "$stage lookup failure stops before deletion"
  events ''
done

echo "prune-caches regressions passed ($cases cases)"
