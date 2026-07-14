# Contributing to mutash

Thanks for your interest in improving mutash. Issues, discussions and pull requests are all welcome.

## Getting started

Prerequisites: Rust 1.75 or newer (stable toolchain) and bash 4+ (the tests spawn real `sh`/`bash` processes on local files).

```bash
git clone https://github.com/JaydenCJ/mutash.git
cd mutash
cargo build
cargo test
bash scripts/smoke.sh
```

`scripts/smoke.sh` exercises the real CLI end to end against a fixture project — list, a full mutation run with known kills/survivors/timeouts, JSON output, pragmas and the `--min-score` gate. It finishes in well under a minute and must print `SMOKE OK`.

## Before you open a pull request

1. `cargo fmt` — formatting is enforced.
2. `cargo clippy --all-targets -- -D warnings` — clippy must be clean.
3. `cargo test` — unit tests and the CLI integration tests must pass.
4. `bash scripts/smoke.sh` — the smoke test must print `SMOKE OK`.
5. Add tests for behavior changes. Scanning and mutation logic lives in pure modules (`lexer`, `mutators`, `mutant`, `report`) that are easy to unit-test; please keep it that way.

## Ground rules

- Keep dependencies at zero. mutash is a std-only binary and that is a headline feature; adding a crate needs a very strong justification in the PR description.
- No network calls, ever. mutash only spawns the user's own test command and touches temp directories it created.
- New mutation operators must be context-gated (never emit a syntactically invalid mutant on plausible input) and documented in `docs/operators.md`, `mutators::OPERATORS` and the README table together.
- Code comments and doc comments are written in English.
- The scanner is deliberately conservative: when in doubt, skip a region rather than emit a wrong mutant. False negatives are acceptable; corrupted scripts are not.

## Reporting bugs

Please include the `mutash --version` output, the smallest script that reproduces the problem, the exact command line, and — for wrong-mutant bugs — the `mutash list` line for the offending mutant. Scanner bugs are much easier to fix with a minimal snippet ("this heredoc/quote/arithmetic combination is mis-tokenized").

## Security

mutash executes your test command and writes only inside temp sandboxes, but if you find a way to make it touch files outside them (or otherwise escalate), please do not open a public issue. Use GitHub's private vulnerability reporting on this repository instead.
