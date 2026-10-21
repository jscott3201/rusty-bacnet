#!/usr/bin/env bash
# plan-skip.sh regressions (#1466), in a throwaway repository with a stub for
# gh's API calls. No network.
set -euo pipefail

plan="$(cd "$(dirname "$0")" && pwd)/plan-skip.sh"
fixture=$(mktemp -d)
# So the no-repository case can't find an enclosing one.
export GIT_CEILING_DIRECTORIES=$fixture
trap 'rm -rf "$fixture"' EXIT
cd "$fixture"

# The stub answers by endpoint: compare with $STUB_COMPARE, workflow runs
# with $STUB_RUNS and a check suite's check runs with $STUB_CHECKS, each
# printed as given (several JSON documents stand for --paginate's pages), or
# fails when that variable is empty. It applies --jq as gh would and logs
# its arguments so the tests can check the queries.
cat >stub-gh <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"$STUB_LOG"
filter='' url=''
while [ $# -gt 0 ]; do
  case $1 in
    --jq) filter=$2; shift 2 ;;
    api | --paginate) shift ;;
    *) url=$1; shift ;;
  esac
done
case $url in
  */compare/*) body=${STUB_COMPARE:-} ;;
  */actions/runs\?*) body=${STUB_RUNS:-} ;;
  */check-suites/*/check-runs\?*) body=${STUB_CHECKS:-} ;;
  *) body='' ;;
esac
[ -n "$body" ] || { echo "HTTP 502" >&2; exit 1; }
if [ -n "$filter" ]; then jq -r "$filter" <<<"$body"; else printf '%s\n' "$body"; fi
EOF
chmod +x stub-gh
export PLAN_SKIP_GH=$fixture/stub-gh STUB_LOG=$fixture/gh.log
export GITHUB_EVENT_NAME=push GITHUB_REF=refs/heads/dev GITHUB_REPOSITORY=owner/repo
export GITHUB_OUTPUT=$fixture/output GITHUB_STEP_SUMMARY=$fixture/summary

# run <path> <event> <head repository> <started> <check suite id>: one
# workflow run object.
run() {
  printf '{"path":"%s","event":"%s","head_repository":{"full_name":"%s"},"run_started_at":"%s","id":%s,"check_suite_id":%s,"pull_requests":[]}' \
    "$1" "$2" "$3" "$4" "$5" "$5"
}
runs() { local IFS=,; echo "{\"workflow_runs\":[$*]}"; }
ci_run=$(run .github/workflows/ci.yml pull_request owner/repo 2026-10-20T10:00:00Z 11)
native_run=$(run .github/workflows/native-tests.yml pull_request owner/repo 2026-10-20T10:00:00Z 22)
# check <conclusion> <started> <id> [app]: one check run object.
check() {
  local conclusion=\"$1\"
  [ "$1" = null ] && conclusion=null
  printf '{"id":%s,"started_at":"%s","conclusion":%s,"app":{"slug":"%s"}}' "$3" "$2" "$conclusion" "${4:-github-actions}"
}
checks() { local IFS=,; echo "{\"check_runs\":[$*]}"; }

good_compare='{"behind_by":0,"ahead_by":1,"status":"ahead"}'
good_runs=$(runs "$ci_run" "$native_run")
green=$(checks "$(check success 2026-10-20T10:00:00Z 1)")
export STUB_COMPARE=$good_compare STUB_RUNS=$good_runs STUB_CHECKS=$green

cases=0
fail() {
  echo "FAIL: plan-skip regression: $1" >&2
  [ -f result.log ] && cat result.log >&2
  exit 1
}

# expect <true|false> <case> [check name] [workflow]: runs plan-skip.sh in
# repo/ and checks the one skip= line it wrote and that it exited 0.
expect() {
  : >"$GITHUB_OUTPUT"
  : >"$GITHUB_STEP_SUMMARY"
  : >"$STUB_LOG"
  (cd repo && bash "$plan" "${3:-CI OK}" "${4:-.github/workflows/ci.yml}") >result.log 2>&1 \
    || fail "$2: exited non-zero"
  [ "$(cat "$GITHUB_OUTPUT")" = "skip=$1" ] || fail "$2: expected skip=$1, wrote '$(cat "$GITHUB_OUTPUT")'"
  grep -q "^skip=$1: " result.log || fail "$2: no log line"
  grep -q '^### Plan: ' "$GITHUB_STEP_SUMMARY" || fail "$2: no summary"
  cases=$((cases + 1))
  echo "ok: $2 -> skip=$1"
}
# reason <text>: the last case's log names it.
reason() { grep -qF -- "$1" result.log || fail "reason should say '$1'"; }
queried() { grep -qF -- "$1" "$STUB_LOG" || fail "no query with '$1'"; }

# A dev history with a crate, then a PR branch off it.
reset_repo() {
  rm -rf repo
  git init -q -b dev repo
  (
    cd repo
    git config user.email ci@example.invalid
    git config user.name CI
    mkdir -p crates/a/src scripts/ci .github/workflows
    echo 'fn main() {}' >crates/a/src/main.rs
    echo '[package]' >crates/a/Cargo.toml
    echo 'version = 3' >Cargo.lock
    echo 'features=a' >scripts/ci/local-macos.sh
    echo 'name: CI' >.github/workflows/ci.yml
    git add -A && git commit -q -m base
    git checkout -q -b pr
  )
}

# pr_commit <path> <line>: appends to a file on the PR branch.
pr_commit() {
  (cd repo && echo "$2" >>"$1" && git add -A && git commit -q -m "pr: $1")
}

# merge_pr: the lead's --no-ff merge of the PR into dev.
merge_pr() {
  (cd repo && git checkout -q dev && git merge -q --no-ff --no-edit pr)
}

# A PR that changes only code, up to date with dev, then merged: the merge
# could have been a fast-forward.
reset_repo
pr_commit crates/a/src/main.rs '// change'
merge_pr
expect true 'tree-equal descendant merge, green CI OK on HEAD^2'
first=$(cd repo && git rev-parse 'HEAD^1')
pr_head=$(cd repo && git rev-parse 'HEAD^2')
queried "repos/owner/repo/compare/$first...$pr_head"
queried "--paginate repos/owner/repo/actions/runs?head_sha=$pr_head&event=pull_request&per_page=100"
queried "--paginate repos/owner/repo/check-suites/11/check-runs?check_name=CI%20OK&filter=latest&per_page=100"
grep -qF 'Plan: skip the heavy jobs' "$GITHUB_STEP_SUMMARY" || fail 'skip summary'
expect true 'Native OK reads native-tests.yml'"'"'s run' 'Native OK' .github/workflows/native-tests.yml
queried 'check-suites/22/check-runs?check_name=Native%20OK'

# Ancestry: dev's tip before the merge must be in the PR head's history.
STUB_COMPARE='{"behind_by":2,"ahead_by":1,"status":"diverged"}' \
  expect false 'PR head not a descendant of dev (trees equal)'
reason 'not a descendant'
STUB_COMPARE='' expect false 'compare API call fails'
reason 'Could not compare'
STUB_COMPARE='{"status":"ahead"}' expect false 'compare response without behind_by'

# Which workflow run counts.
STUB_RUNS=$(runs "$native_run") expect false 'only another workflow ran on HEAD^2'
reason 'No pull_request run of .github/workflows/ci.yml'
STUB_RUNS=$(runs "$(run .github/workflows/ci.yml pull_request fork/repo 2026-10-20T10:00:00Z 11)") \
  expect false 'only a fork'"'"'s run'
STUB_RUNS=$(runs "$(run .github/workflows/ci.yml push owner/repo 2026-10-20T10:00:00Z 11)") \
  expect false 'only a push run'
STUB_RUNS='{"workflow_runs":[]}' expect false 'no runs on HEAD^2'
STUB_RUNS=$(runs "$ci_run" "$(run .github/workflows/ci.yml pull_request owner/repo 2026-10-20T12:00:00Z 33)") \
  expect true 'the newest of two runs counts'
queried 'check-suites/33/check-runs'
# --paginate prints one JSON document per page.
STUB_RUNS="$(runs "$native_run")
$(runs "$ci_run")" expect true 'the run is on the second page'
queried 'check-suites/11/check-runs'
STUB_RUNS='' expect false 'workflow runs API call fails'
STUB_RUNS='not json' expect false 'unparseable workflow runs'

# The aggregate's check run in that run.
STUB_CHECKS=$(checks "$(check failure 2026-10-20T10:00:00Z 1)") expect false 'red aggregate on HEAD^2'
reason 'CI OK on the PR head'
reason 'is failure'
STUB_CHECKS='{"check_runs":[]}' expect false 'missing aggregate on HEAD^2'
reason 'is missing'
STUB_CHECKS=$(checks "$(check success 2026-10-20T11:00:00Z 2)" "$(check failure 2026-10-20T10:00:00Z 1)") \
  expect true 'a failed attempt, then a green re-run'
STUB_CHECKS=$(checks "$(check success 2026-10-20T10:00:00Z 1)" "$(check null 2026-10-20T11:00:00Z 2)") \
  expect false 'newest aggregate run still in progress'
STUB_CHECKS=$(checks "$(check success 2026-10-20T10:00:00Z 1 someone-else)") \
  expect false 'same-named check from another app'
reason 'is missing'
STUB_CHECKS="$(checks "$(check success 2026-10-20T10:00:00Z 1)")
$(checks "$(check failure 2026-10-20T11:00:00Z 2)")" expect false 'a newer red attempt on the second page'
STUB_CHECKS='not json' expect false 'unparseable check runs'
STUB_CHECKS='' expect false 'check runs API call fails'
reason 'Could not read the check runs'
grep -qF 'Plan: run every job' "$GITHUB_STEP_SUMMARY" || fail 'run summary'

GITHUB_EVENT_NAME=pull_request expect false 'pull_request event'
GITHUB_EVENT_NAME=schedule expect false 'scheduled run'
GITHUB_EVENT_NAME=workflow_dispatch expect false 'manual run'
GITHUB_REF=refs/heads/main expect false 'push to main'
GITHUB_REF=refs/tags/v1.0.0 expect false 'tag push'
[ ! -s "$STUB_LOG" ] || fail 'non-dev events must not call the API'

# A behind merge: dev moved after the PR branched, so the merge tree is new.
reset_repo
pr_commit crates/a/src/main.rs '// change'
(cd repo && git checkout -q dev && echo '// dev moved' >>crates/a/src/lib.rs && git add -A && git commit -q -m 'dev moves')
merge_pr
expect false 'behind merge (trees differ)'
reason 'different tree'
[ ! -s "$STUB_LOG" ] || fail 'behind merge must not call the API'

# A plain commit pushed to dev.
reset_repo
(cd repo && git checkout -q dev && echo '// direct' >>crates/a/src/main.rs && git commit -q -am direct)
expect false 'non-merge push'
reason 'is not a merge commit'

# An octopus merge whose tree equals its second parent's.
reset_repo
pr_commit crates/a/src/main.rs '// change'
(cd repo && git checkout -q -b empty dev && git commit -q --allow-empty -m empty \
  && git checkout -q dev && git merge -q --no-ff --no-edit pr empty) >/dev/null
expect false 'octopus merge'
reason 'more than two parents'

# Tree-equal merges whose PR changed a cache-key input.
for input in Cargo.lock crates/a/Cargo.toml rust-toolchain rust-toolchain.toml \
  .cargo/config .cargo/config.toml .github/workflows/ci.yml .github/ci-pins.env \
  scripts/ci/local-macos.sh; do
  reset_repo
  (cd repo && mkdir -p "$(dirname "$input")")
  pr_commit "$input" 'changed = true'
  merge_pr
  expect false "cache-key input $input changed"
  reason "$input"
done

# A file outside the cache inputs, beside them, still skips.
reset_repo
pr_commit scripts/ci/other.sh 'echo'
merge_pr
expect true 'non-key script change, tree-equal merge'

# Outside a git repository: fails open.
rm -rf repo && mkdir repo
expect false 'no git repository'

echo "plan-skip regressions passed ($cases cases)"
