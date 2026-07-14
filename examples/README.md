# mutash examples

A complete, offline example project: a small deploy script and two
equivalent test suites for it — one in [bats](https://github.com/bats-core/bats-core),
one in plain bash (no framework at all).

| File | What it is |
|---|---|
| `deploy.sh` | The script under test: version validation, health-check ranges, retry loop, artifact promotion |
| `tests/deploy.bats` | The suite as bats tests (needs `bats` installed) |
| `tests/run.sh` | The identical suite in plain bash — works everywhere |

## Run it

From this directory (`examples/`):

```bash
# with bats installed:
mutash run deploy.sh --tests "bats tests/deploy.bats"

# with nothing but bash:
mutash run deploy.sh --tests "bash tests/run.sh"
```

Both suites pass on the pristine script, so the baseline is green — and then
mutash generates 35 mutants and reports which ones the suite fails to kill.

## What the survivors teach you

With `tests/run.sh` the score is 88.6% (grade B) and four mutants survive,
each one a concrete, actionable test gap:

1. `` `-le` -> `-lt` `` and `` `1` -> `2` `` in the retry loop — nothing pins
   down that `retry` makes *exactly* `MAX_ATTEMPTS` attempts.
2. ``drop `-p` `` on `mkdir -p` — no test promotes into an already-existing
   release directory twice.
3. `` `return 1` -> `return 0` `` in `main` — the suite never exercises a
   deploy that fails *after* validation, so the failure exit code is untested.

Add the missing cases, rerun, and watch the grade climb. That loop — mutate,
read survivors, strengthen the suite — is the whole point of mutation testing.
