#!/usr/bin/env bash
# Smoke test: builds mutash, then exercises the real CLI end to end against
# a fixture project — list, a full mutation run with known kills/survivors/
# timeouts, JSON output, the pragma escape hatch and the --min-score gate.
# Self-contained: temp dirs only, no network, finishes in well under a minute.
set -euo pipefail

cd "$(dirname "$0")/.."

fail() {
  echo "SMOKE FAIL: $*" >&2
  exit 1
}

echo "[smoke] building..."
cargo build --quiet
BIN=$PWD/target/debug/mutash

WORK=$(mktemp -d "${TMPDIR:-/tmp}/mutash-smoke.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

# --- 1. version/help/ops sanity ----------------------------------------------
"$BIN" --version | grep -q '^mutash 0\.1\.0$' || fail "--version mismatch"
"$BIN" --help | grep -q 'COMMANDS:' || fail "--help missing sections"
"$BIN" ops | grep -q 'compare' || fail "ops reference missing operators"
echo "[smoke] version/help/ops OK"

# --- 2. fixture: a script with a deliberately incomplete suite ----------------
cat >"$WORK/counter.sh" <<'EOF'
#!/usr/bin/env bash
# Sum the integers 1..N; negative N is an error.
main() {
  local n="$1"
  if [ "$n" -lt 0 ]; then
    echo "negative" >&2
    return 1
  fi
  local total=0
  local i=1
  while [ "$i" -le "$n" ]; do
    total=$((total + i))
    i=$((i + 1))
  done
  echo "$total"
}
main "$@"
EOF
cat >"$WORK/check.sh" <<'EOF'
#!/bin/sh
set -e
out=$(bash counter.sh 3); test "$out" = 6
out=$(bash counter.sh 0); test "$out" = 0
if bash counter.sh -2 2>/dev/null; then exit 1; fi
exit 0
EOF

# --- 3. list: mutants are found with locations, nothing is executed ----------
echo "[smoke] mutash list"
"$BIN" list "$WORK/counter.sh" | tee "$WORK/list.out" >/dev/null
grep -q -- '`-lt` -> `-le`' "$WORK/list.out" || fail "list missing the -lt boundary mutant"
grep -q -- '`+` -> `-`' "$WORK/list.out" || fail "list missing the arithmetic mutant"
grep -q 'Total: 9 mutants across 1 file$' "$WORK/list.out" || fail "expected 9 mutants, got: $(tail -1 "$WORK/list.out")"

# --- 4. full run: kills, survivors, timeouts, grade ---------------------------
echo "[smoke] mutash run (full operator set)"
(cd "$WORK" && "$BIN" run counter.sh --tests "sh check.sh" --timeout 1) \
  | tee "$WORK/run.out" >/dev/null
grep -q 'baseline: pass' "$WORK/run.out" || fail "baseline line missing"
grep -q -- '`-lt` -> `-le`.*=> killed' "$WORK/run.out" || fail "boundary mutant not killed"
grep -q '=> timeout' "$WORK/run.out" || fail "infinite-loop mutant not cut off by the deadline"
grep -q 'Survivors (1):' "$WORK/run.out" || fail "expected exactly one survivor"
grep -q -- '`0` -> `-1`' "$WORK/run.out" || fail "the untested N=-1 boundary must survive"
grep -q 'Score: 88\.9%.*Grade: B' "$WORK/run.out" || fail "score/grade mismatch: $(tail -1 "$WORK/run.out")"

# --- 5. JSON report -----------------------------------------------------------
echo "[smoke] mutash run --json"
(cd "$WORK" && "$BIN" run counter.sh --tests "sh check.sh" --timeout 1 --only number --json) \
  >"$WORK/run.json"
grep -q '"tool": "mutash"' "$WORK/run.json" || fail "json header missing"
grep -q '"outcome": "killed"' "$WORK/run.json" || fail "json missing killed outcome"
grep -q '"outcome": "survived"' "$WORK/run.json" || fail "json missing survived outcome"
grep -q '"outcome": "timeout"' "$WORK/run.json" || fail "json missing timeout outcome"
grep -q '"score": 75.0' "$WORK/run.json" || fail "json score mismatch"

# --- 6. pragma escape hatch ----------------------------------------------------
echo "[smoke] pragma # mutash: skip"
sed 's/if \[ "$n" -lt 0 \]; then/if [ "$n" -lt 0 ]; then # mutash: skip/' \
  "$WORK/counter.sh" >"$WORK/counter2.sh"
"$BIN" list "$WORK/counter2.sh" | grep -q 'Total: 6 mutants' \
  || fail "pragma did not remove the 3 mutants on its line"

# --- 7. min-score gate ----------------------------------------------------------
echo "[smoke] --min-score gate"
if (cd "$WORK" && "$BIN" run counter.sh --tests "sh check.sh" --timeout 1 --only number --min-score 95 >/dev/null); then
  fail "--min-score 95 should have failed (score is 75%)"
fi
(cd "$WORK" && "$BIN" run counter.sh --tests "sh check.sh" --timeout 1 --only number --min-score 70 >/dev/null) \
  || fail "--min-score 70 should have passed"

# --- 8. a broken suite is refused, and the user's tree is never touched --------
echo "[smoke] failing baseline is refused"
BEFORE=$(cat "$WORK/counter.sh")
if (cd "$WORK" && "$BIN" run counter.sh --tests "exit 1" >/dev/null 2>"$WORK/baseline.err"); then
  fail "failing baseline was accepted"
fi
grep -q 'fix the suite first' "$WORK/baseline.err" || fail "baseline error message missing"
[ "$BEFORE" = "$(cat "$WORK/counter.sh")" ] || fail "original script was modified"

echo "SMOKE OK"
