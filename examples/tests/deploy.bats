#!/usr/bin/env bats
# bats suite for deploy.sh — the same checks as tests/run.sh, written for
# bats users. Run the example with:
#
#   mutash run deploy.sh --tests "bats tests/deploy.bats"

setup() {
  cd "$BATS_TEST_DIRNAME/.."
  source deploy.sh
  TMP=$(mktemp -d)
}

teardown() {
  rm -rf "$TMP"
}

@test "accepts well-formed versions" {
  valid_version 1.2.3
  valid_version 0.0.1
}

@test "rejects malformed versions" {
  # `! cmd` cannot fail a bats test (bash skips errexit on `!`), so assert
  # through `run` + $status instead — otherwise these checks kill nothing.
  run valid_version 1.2;    [ "$status" -ne 0 ]
  run valid_version v1.2.3; [ "$status" -ne 0 ]
  run valid_version "";     [ "$status" -ne 0 ]
  run valid_version 1..3;   [ "$status" -ne 0 ]
}

@test "healthy covers exactly the 2xx range" {
  healthy 200
  healthy 299
  run healthy 199; [ "$status" -ne 0 ]
  run healthy 300; [ "$status" -ne 0 ]
}

@test "retry succeeds once the command recovers" {
  flaky() {
    if [ -f "$TMP/ok" ]; then return 0; fi
    touch "$TMP/ok"
    return 1
  }
  retry flaky
}

@test "retry gives up after MAX_ATTEMPTS" {
  MAX_ATTEMPTS=1
  run retry false; [ "$status" -ne 0 ]
  MAX_ATTEMPTS=3
  run retry false; [ "$status" -ne 0 ]
}

@test "promote refuses missing and empty artifacts" {
  echo data >"$TMP/app.tar"
  promote "$TMP/app.tar" "$TMP/rel"
  [ -f "$TMP/rel/app.tar" ]
  run promote "$TMP/missing" "$TMP/rel";   [ "$status" -ne 0 ]
  : >"$TMP/empty.tar"
  run promote "$TMP/empty.tar" "$TMP/rel"; [ "$status" -ne 0 ]
}

@test "main deploys end to end" {
  echo data >"$TMP/app.tar"
  run bash deploy.sh 2.0.0 "$TMP/app.tar" "$TMP/rel2"
  [ "$status" -eq 0 ]
  [ "$output" = "deployed 2.0.0" ]
}

@test "main rejects bad input" {
  echo data >"$TMP/app.tar"
  run bash deploy.sh bad-version "$TMP/app.tar" "$TMP/rel2"
  [ "$status" -eq 2 ]
  run bash deploy.sh 1.0.0
  [ "$status" -eq 2 ]
}
