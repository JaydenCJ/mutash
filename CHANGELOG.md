# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-07-13

### Added

- Shell-aware token scanner: quotes (single/double, nested command substitution), comments (with `$#`/`${#var}` disambiguation), backslash escapes and line continuations, heredocs (`<<`, `<<-`, quoted delimiters, several per line), herestrings, backticks, `$( )` bodies scanned recursively, `$(( ))`/`(( ))` arithmetic with longest-first operator matching — no bash AST, no interpreter patching.
- Nine context-gated mutation operators: `compare` (boundary swaps like `-lt` → `-le`, `[[ ]]`-only `<`/`>`), `unary` (`-z`/`-n`, `-f`/`-d`, …), `connective` (`&&`/`||`, `-a`/`-o`), `arith` (`+`/`-`, `*`/`/`, `++`/`--`, `+=`/`-=`), `number` (±1 in decision positions only), `exit` (status flips), `flag` (drop/shrink short, cluster and long flags), `negate` (drop `!`), `truth` (`true`/`false`).
- Sandboxed runner: the project root is copied once into a temp directory (skipping `.git`, `target`, `node_modules`, …, preserving executable bits); each mutant rewrites one file, runs the user's test command via `sh -c`, and restores the original. The user's working tree is never touched.
- Baseline enforcement (a failing suite is refused before any mutation) and a per-mutant deadline derived from the measured baseline (3× + 2s, overridable with `--timeout`) so infinite-loop mutants are cut off and counted as detections.
- CLI: `mutash run` (progress lines, survivor report with source excerpts, mutation score and letter grade, `--min-score` gate with exit code 1), `mutash list` (preview mutants without executing anything), `mutash ops` (operator reference), `--only`/`--skip` operator selection, stable `--json` reports for both `run` and `list`.
- `# mutash: skip` / `# mutash: off` / `# mutash: on` source pragmas honored at generation time.
- Runnable example project (`examples/`): a deploy script with equivalent bats and plain-bash suites, scoring 88.6% with four instructive survivors.
- Test suite: 80 unit tests, 9 CLI integration tests, and `scripts/smoke.sh`.

[0.1.0]: https://github.com/JaydenCJ/mutash/releases/tag/v0.1.0
