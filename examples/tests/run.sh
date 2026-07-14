#!/usr/bin/env bash
# Plain-bash test suite for deploy.sh — no framework required.
#
# mutash runs whatever `--tests` command you give it; this file shows the
# zero-dependency variant of the bats suite next to it (deploy.bats).
set -u
cd "$(dirname "$0")/.."
# shellcheck source=../deploy.sh
source deploy.sh

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# --- valid_version -----------------------------------------------------------
valid_version 1.2.3 || fail "1.2.3 should be valid"
valid_version 0.0.1 || fail "0.0.1 should be valid"
valid_version 1.2 && fail "1.2 lacks a patch component"
valid_version v1.2.3 && fail "a v prefix is invalid"
valid_version "" && fail "the empty version is invalid"
valid_version 1..3 && fail "empty components are invalid"

# --- healthy: both boundaries, both sides ------------------------------------
healthy 200 || fail "200 is healthy"
healthy 299 || fail "299 is healthy"
healthy 199 && fail "199 is not healthy"
healthy 300 && fail "300 is not healthy"

# --- retry --------------------------------------------------------------------
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
flaky() {
  if [ -f "$tmp/ok" ]; then return 0; fi
  touch "$tmp/ok"
  return 1
}
retry flaky || fail "retry should succeed on the second attempt"
MAX_ATTEMPTS=1 retry false && fail "retry must give up after MAX_ATTEMPTS"
retry false && fail "retrying a failing command must fail"

# --- promote ------------------------------------------------------------------
echo data >"$tmp/app.tar"
promote "$tmp/app.tar" "$tmp/rel" || fail "promote should succeed"
[ -f "$tmp/rel/app.tar" ] || fail "artifact must land in the release dir"
promote "$tmp/missing" "$tmp/rel" 2>/dev/null && fail "a missing artifact must fail"
: >"$tmp/empty.tar"
promote "$tmp/empty.tar" "$tmp/rel" 2>/dev/null && fail "an empty artifact must fail"

# --- main, end to end ----------------------------------------------------------
out=$(bash deploy.sh 2.0.0 "$tmp/app.tar" "$tmp/rel2") || fail "deploy should succeed"
[ "$out" = "deployed 2.0.0" ] || fail "unexpected output: $out"
bash deploy.sh bad-version "$tmp/app.tar" "$tmp/rel2" 2>/dev/null && fail "bad version must be rejected"
bash deploy.sh 1.0.0 2>/dev/null && fail "wrong argument count must be rejected"

echo "all deploy.sh checks passed"
