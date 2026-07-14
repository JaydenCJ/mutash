//! End-to-end CLI tests against the compiled `mutash` binary.
//!
//! Each test builds a tiny real shell project in a temp directory (a script
//! plus a plain-shell test suite), runs the binary, and asserts on stdout,
//! stderr and exit codes. Everything is offline and deterministic: the only
//! processes spawned are `mutash` itself and `sh`/`bash` on local files.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mutash"))
}

fn temp_project() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mutash-cli-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A realistic little script: sums 1..N, rejects negative input.
const COUNTER_SH: &str = r#"#!/usr/bin/env bash
# Sum the integers 1..N; negative N is an error.
set -u
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
"#;

/// A decent (but imperfect) suite for it, in plain shell — no test
/// framework needed. It checks 3 -> 6, 0 -> 0, and that -2 fails.
const SUITE_SH: &str = r#"#!/bin/sh
set -e
out=$(bash counter.sh 3); test "$out" = 6
out=$(bash counter.sh 0); test "$out" = 0
if bash counter.sh -2 2>/dev/null; then exit 1; fi
exit 0
"#;

/// Write the counter fixture into a fresh project dir.
fn counter_project() -> PathBuf {
    let dir = temp_project();
    fs::write(dir.join("counter.sh"), COUNTER_SH).unwrap();
    fs::write(dir.join("check.sh"), SUITE_SH).unwrap();
    dir
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn version_and_help() {
    let out = bin().arg("--version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        stdout_of(&out).trim(),
        format!("mutash {}", env!("CARGO_PKG_VERSION"))
    );

    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = stdout_of(&out);
    for needle in [
        "COMMANDS:",
        "run",
        "list",
        "ops",
        "--min-score",
        "EXIT CODES:",
    ] {
        assert!(text.contains(needle), "help missing {needle}: {text}");
    }
}

#[test]
fn ops_reference_lists_every_operator_id() {
    let out = bin().arg("ops").output().unwrap();
    assert!(out.status.success());
    let text = stdout_of(&out);
    for id in [
        "compare",
        "unary",
        "connective",
        "arith",
        "number",
        "exit",
        "flag",
        "negate",
        "truth",
    ] {
        assert!(text.contains(id), "ops output missing {id}");
    }
    assert!(
        text.contains("# mutash: skip"),
        "ops output should mention pragmas"
    );
}

#[test]
fn list_prints_mutants_with_locations_and_total() {
    let dir = counter_project();
    let script = dir.join("counter.sh");
    let out = bin()
        .args(["list", script.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let text = stdout_of(&out);
    assert!(text.contains("`-lt` -> `-le`"), "{text}");
    assert!(text.contains("`-le` -> `-lt`"), "{text}");
    assert!(text.contains("`+` -> `-`"), "{text}");
    assert!(text.contains("drop `-u`"), "{text}");
    // Locations are line:col and every listing ends with a total.
    assert!(
        text.contains("6:13"),
        "expected -lt at line 6 col 13: {text}"
    );
    assert!(text.contains("Total: "), "{text}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn list_json_is_well_formed_and_honors_only() {
    let dir = counter_project();
    let script = dir.join("counter.sh");
    let out = bin()
        .args([
            "list",
            script.to_str().unwrap(),
            "--json",
            "--only",
            "compare",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let json = stdout_of(&out);
    assert!(json.contains("\"tool\": \"mutash\""), "{json}");
    assert!(json.contains("\"op\": \"compare\""), "{json}");
    assert!(
        !json.contains("\"op\": \"flag\""),
        "--only compare must exclude flags: {json}"
    );
    assert_eq!(json.matches('{').count(), json.matches('}').count());
    assert_eq!(json.matches('[').count(), json.matches(']').count());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn run_grades_a_real_suite_and_lists_survivors() {
    let dir = counter_project();
    let out = bin()
        .current_dir(&dir)
        .args([
            "run",
            "counter.sh",
            "--tests",
            "sh check.sh",
            "--timeout",
            "1",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let text = stdout_of(&out);
    // The suite kills the comparison mutants…
    assert!(text.contains("`-lt` -> `-le`"), "{text}");
    assert!(text.contains("=> killed"), "{text}");
    // …but misses the untested boundary: `0` -> `-1` only differs at N=-1.
    assert!(text.contains("Survivors ("), "{text}");
    assert!(text.contains("`0` -> `-1`"), "{text}");
    // The `set -u` flag mutant is exercised too (progress line present).
    assert!(text.contains("drop `-u`"), "{text}");
    // Mutating `i + 1` into an infinite loop must be caught by the deadline.
    assert!(text.contains("=> timeout"), "{text}");
    assert!(text.contains("Score: "), "{text}");
    assert!(text.contains("Grade: "), "{text}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn run_json_reports_outcomes_and_summary() {
    let dir = counter_project();
    let out = bin()
        .current_dir(&dir)
        .args([
            "run",
            "counter.sh",
            "--tests",
            "sh check.sh",
            "--timeout",
            "1",
            "--json",
            "--only",
            "number",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let json = stdout_of(&out);
    assert!(json.contains("\"tests\": \"sh check.sh\""), "{json}");
    assert!(json.contains("\"outcome\": \"killed\""), "{json}");
    assert!(json.contains("\"outcome\": \"survived\""), "{json}");
    assert!(json.contains("\"summary\": {"), "{json}");
    assert!(json.contains("\"grade\":"), "{json}");
    // JSON mode keeps stdout machine-readable: no progress lines.
    assert!(
        !json.contains("=>"),
        "progress leaked into --json output: {json}"
    );
    assert_eq!(json.matches('{').count(), json.matches('}').count());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn run_min_score_gate_sets_the_exit_code() {
    let dir = counter_project();
    // --only number yields four mutants: `0`->`1` (killed by the N=0 case),
    // `0`->`-1` (survives: nobody tests N=-1), `1`->`2` (killed) and
    // `1`->`0` (an infinite loop, killed by the deadline): score 75%.
    let mut fail = bin();
    fail.current_dir(&dir).args([
        "run",
        "counter.sh",
        "--tests",
        "sh check.sh",
        "--timeout",
        "1",
        "--only",
        "number",
        "--min-score",
        "95",
    ]);
    let out = fail.output().unwrap();
    assert_eq!(out.status.code(), Some(1), "stdout: {}", stdout_of(&out));
    assert!(
        stdout_of(&out).contains("Result: FAIL"),
        "{}",
        stdout_of(&out)
    );

    let mut pass = bin();
    pass.current_dir(&dir).args([
        "run",
        "counter.sh",
        "--tests",
        "sh check.sh",
        "--timeout",
        "1",
        "--only",
        "number",
        "--min-score",
        "40",
    ]);
    let out = pass.output().unwrap();
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout_of(&out));
    assert!(
        stdout_of(&out).contains("Result: PASS"),
        "{}",
        stdout_of(&out)
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn run_refuses_a_failing_baseline() {
    let dir = counter_project();
    let out = bin()
        .current_dir(&dir)
        .args(["run", "counter.sh", "--tests", "exit 1"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr_of(&out).contains("fix the suite first"),
        "{}",
        stderr_of(&out)
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn usage_errors_exit_2_with_a_reason() {
    // Unknown command.
    let out = bin().arg("frobnicate").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_of(&out).contains("unknown command"));

    // Missing script file.
    let out = bin().args(["list", "no-such-script.sh"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_of(&out).contains("cannot read no-such-script.sh"));

    // Unknown option.
    let out = bin()
        .args(["run", "x.sh", "--frobnicate"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_of(&out).contains("unknown option"));

    // No arguments at all prints usage.
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr_of(&out).contains("USAGE:"));
}
