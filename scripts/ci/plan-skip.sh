#!/usr/bin/env bash
# Decides whether a push to dev can skip the heavy jobs (#1466). A --no-ff
# merge of a PR head that already contained dev's tip (a merge that could
# have been a fast-forward) has exactly that head's tree, and the PR's own
# runs already tested that tree. It skips only when all of these hold, and
# runs on anything else, errors included:
#
#   - the event is a push to refs/heads/dev;
#   - HEAD is a merge of exactly two parents, and its tree equals HEAD^2's
#     (the PR head);
#   - HEAD^1 (dev before the merge) is an ancestor of HEAD^2, by the compare
#     API (the depth-2 checkout has no history to ask git). A PR run tests
#     refs/pull/<n>/merge, the head merged with dev as dev stood then. dev
#     only moves forward, so that older dev is an ancestor of HEAD^1 and so
#     of the head, and the tree it tested is the head's. Tree equality alone
#     isn't enough: a commit on dev that was reverted before the merge could
#     be in the tested tree but not in this one;
#   - no rust-cache key input changed between HEAD^1 and HEAD, so the dev
#     caches PRs restore stay current without this run saving new ones;
#   - in the newest pull_request run of <workflow> on HEAD^2 from this
#     repository, the check run named <check> concluded success.
#
# The checkout needs fetch-depth: 2 for both parents. Writes skip=true or
# skip=false to $GITHUB_OUTPUT and the reason to $GITHUB_STEP_SUMMARY, and
# logs it. It exits 0 whatever it decides; a failure to decide means run.
#
#   GH_TOKEN=... GITHUB_EVENT_NAME=push GITHUB_REF=refs/heads/dev \
#     GITHUB_REPOSITORY=owner/repo \
#     bash scripts/ci/plan-skip.sh "CI OK" .github/workflows/ci.yml
#
# PLAN_SKIP_GH names a command to run in place of gh (the tests stub it).
# scripts/ci/test-plan-skip.sh covers each case.
set -euo pipefail

check=${1:?usage: plan-skip.sh "<aggregate check name>" <workflow path>}
workflow=${2:?usage: plan-skip.sh "<aggregate check name>" <workflow path>}
gh_cmd=${PLAN_SKIP_GH:-gh}

# What feeds the workflows' rust-cache keys. This list is about caches only:
# the tree and ancestry checks are what make a skip correct. A change here
# runs the merge so it saves the caches for the new keys. It is every
# Cargo.lock and Cargo.toml in the repository, the root toolchain and cargo
# config files (with their legacy names), the workflow files (the env-vars
# values such as LINUX_FEATURES, the CARGO_* and RUST* variables, the job
# ids) and the pins beside them, and local-macos.sh, whose features= line is
# native-tests.yml's NATIVE_FEATURES. The runner image's own toolchains
# (native-tests.yml keeps them) also enter the native key; no diff can see an
# image update.
cache_inputs=(
  ':(glob)**/Cargo.lock'
  ':(glob)**/Cargo.toml'
  rust-toolchain
  rust-toolchain.toml
  .cargo/config
  .cargo/config.toml
  .github/
  scripts/ci/local-macos.sh
)

decide() {
  local skip=$1 reason=$2
  echo "skip=$skip" >>"${GITHUB_OUTPUT:-/dev/null}"
  if [ "$skip" = true ]; then
    echo "### Plan: skip the heavy jobs" >>"${GITHUB_STEP_SUMMARY:-/dev/null}"
  else
    echo "### Plan: run every job" >>"${GITHUB_STEP_SUMMARY:-/dev/null}"
  fi
  echo "$reason" >>"${GITHUB_STEP_SUMMARY:-/dev/null}"
  echo "skip=$skip: $reason"
  exit 0
}

event=${GITHUB_EVENT_NAME:-}
ref=${GITHUB_REF:-}
if [ "$event" != push ] || [ "$ref" != refs/heads/dev ]; then
  decide false "Not a push to dev (${event:-no event} on ${ref:-no ref}); only those can skip."
fi
repo=${GITHUB_REPOSITORY:-}
[ -n "$repo" ] || decide false "GITHUB_REPOSITORY is not set."

head=$(git rev-parse HEAD) || decide false "No HEAD commit."
if ! second=$(git rev-parse -q --verify 'HEAD^2^{commit}'); then
  decide false "$head is not a merge commit."
fi
if git rev-parse -q --verify 'HEAD^3^{commit}' >/dev/null; then
  decide false "$head merges more than two parents."
fi
first=$(git rev-parse 'HEAD^1') || decide false "Cannot read HEAD^1 of $head."

head_tree=$(git rev-parse 'HEAD^{tree}') || decide false "Cannot read the tree of $head."
second_tree=$(git rev-parse "$second^{tree}") || decide false "Cannot read the tree of $second."
if [ "$head_tree" != "$second_tree" ]; then
  decide false "The merge $head has a different tree from its PR head $second: the PR was behind dev, so this tree is untested."
fi

status=0
git diff --quiet "$first" HEAD -- "${cache_inputs[@]}" || status=$?
if [ "$status" -eq 1 ]; then
  changed=$(git diff --name-only "$first" HEAD -- "${cache_inputs[@]}" | head -n 5 | paste -sd ' ' -)
  decide false "Cache-key inputs changed since $first ($changed), so this run saves new dev caches."
elif [ "$status" -ne 0 ]; then
  decide false "git diff $first HEAD failed (status $status)."
fi

# behind_by counts the commits of $first that $second lacks.
if ! behind=$("$gh_cmd" api "repos/$repo/compare/$first...$second" --jq .behind_by); then
  decide false "Could not compare $first with $second."
fi
if [ "$behind" != 0 ]; then
  decide false "The PR head $second is not a descendant of dev's $first (behind by ${behind:-unknown}), so its runs may have tested another tree."
fi

# The workflow's pull_request runs on the head from this repository, not
# another workflow's check of the same name or a fork's run. The newest of
# them counts. A run can't be told apart by its PR's base: GitHub empties a
# run's pull_requests once the PR is merged, which it is by now. So a PR
# into main from the same commit, if it ran last, would count too
# (docs/ci.md, "Skipping identical merge runs").
if ! runs=$("$gh_cmd" api --paginate "repos/$repo/actions/runs?head_sha=$second&event=pull_request&per_page=100"); then
  decide false "Could not read the workflow runs on $second."
fi
if ! suite=$(jq -rs --arg path "$workflow" --arg repo "$repo" '[.[].workflow_runs[]
    | select(.path == $path and .event == "pull_request"
      and .head_repository.full_name == $repo)]
    | sort_by(.run_started_at // .created_at // "", .id) | last | .check_suite_id // empty' <<<"$runs"); then
  decide false "Could not parse the workflow runs on $second."
fi
[ -n "$suite" ] || decide false "No pull_request run of $workflow on $second."

# Within the run, the latest check run of the name: a re-run of failed jobs
# replaces the attempt it re-ran.
if ! checks=$("$gh_cmd" api --paginate "repos/$repo/check-suites/$suite/check-runs?check_name=${check// /%20}&filter=latest&per_page=100"); then
  decide false "Could not read the check runs of $workflow's run on $second."
fi
if ! conclusion=$(jq -rs '[.[].check_runs[] | select(.app.slug == "github-actions")]
    | sort_by(.started_at // "", .id) | last | .conclusion // "missing"' <<<"$checks"); then
  decide false "Could not parse the check runs on $second."
fi
if [ "$conclusion" != success ]; then
  decide false "$check on the PR head $second is $conclusion, not success."
fi

decide true "The merge $head could have been a fast-forward to its PR head $second, where $workflow's $check passed, and no cache-key input changed since $first."
