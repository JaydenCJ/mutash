//! Test execution engine: baseline run, per-mutant runs, timeout policy.
//!
//! The test command is the user's own (`bats tests` by default) and is run
//! through `sh -c` inside the sandbox, with stdio silenced. A mutant is
//! *killed* when the command exits non-zero, *survived* when it still
//! passes, and *timeout* when it outlives the per-mutant deadline —
//! mutation loves to turn terminating loops into infinite ones, so the
//! deadline is derived from the measured baseline.

use crate::mutant::Mutant;
use crate::report::{MutantResult, Tally};
use crate::sandbox::Sandbox;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What happened to one mutant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The test command failed — the suite caught the mutant.
    Killed,
    /// The test command still passed — the suite missed the mutant.
    Survived,
    /// The test command exceeded the per-mutant deadline and was killed.
    Timeout,
    /// The test command could not be executed at all.
    Error,
}

impl Outcome {
    pub fn label(&self) -> &'static str {
        match self {
            Outcome::Killed => "killed",
            Outcome::Survived => "survived",
            Outcome::Timeout => "timeout",
            Outcome::Error => "error",
        }
    }
}

/// Result of one invocation of the test command.
#[derive(Debug, Clone, Copy)]
pub struct TestRun {
    pub timed_out: bool,
    pub success: bool,
    /// Exit status code, if the command ran to completion with one.
    pub status: Option<i32>,
    pub duration: Duration,
}

/// Run `cmd` via `sh -c` in `dir`, killing it after `timeout` if given.
pub fn run_tests(cmd: &str, dir: &Path, timeout: Option<Duration>) -> std::io::Result<TestRun> {
    let start = Instant::now();
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(TestRun {
                timed_out: false,
                success: status.success(),
                status: status.code(),
                duration: start.elapsed(),
            });
        }
        if let Some(limit) = timeout {
            if start.elapsed() >= limit {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(TestRun {
                    timed_out: true,
                    success: false,
                    status: None,
                    duration: start.elapsed(),
                });
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Per-mutant deadline: three times the healthy baseline plus slack, so a
/// mutant that merely doubles runtime still finishes, but an infinite loop
/// is cut off quickly.
pub fn derive_timeout(baseline: Duration) -> Duration {
    let auto = baseline * 3 + Duration::from_secs(2);
    auto.max(Duration::from_secs(1))
}

/// Everything the engine needs for one run.
pub struct Plan {
    /// Project root that gets copied into the sandbox.
    pub root: PathBuf,
    /// The test command, run with `sh -c` inside the sandbox.
    pub tests: String,
    /// Explicit per-mutant timeout; derived from the baseline when `None`.
    pub timeout: Option<Duration>,
    /// Scripts under test with their mutants.
    pub files: Vec<FilePlan>,
}

/// One script under test.
pub struct FilePlan {
    /// Path relative to the project root (also the display name).
    pub rel: PathBuf,
    /// Original source text.
    pub source: String,
    /// Mutants for this file, ids already assigned.
    pub mutants: Vec<Mutant>,
}

/// A finished run.
#[derive(Debug)]
pub struct RunReport {
    pub baseline: Duration,
    pub timeout: Duration,
    pub results: Vec<MutantResult>,
    pub tally: Tally,
}

/// Execute the plan. `on_baseline` is called once with the measured
/// baseline duration and the derived per-mutant timeout; `progress` is
/// called once per finished mutant.
pub fn execute(
    plan: &Plan,
    mut on_baseline: impl FnMut(Duration, Duration),
    mut progress: impl FnMut(&Mutant, Outcome, Duration),
) -> Result<RunReport, String> {
    let sandbox = Sandbox::create(&plan.root).map_err(|e| format!("cannot create sandbox: {e}"))?;

    // Baseline: the untouched suite must pass, and its duration anchors the
    // per-mutant deadline. A generous fixed cap guards against a hung suite.
    let baseline_cap = plan.timeout.unwrap_or(Duration::from_secs(300));
    let base = run_tests(&plan.tests, &sandbox.dir, Some(baseline_cap))
        .map_err(|e| format!("cannot run test command `{}`: {e}", plan.tests))?;
    if base.timed_out {
        return Err(format!(
            "baseline run of `{}` exceeded {:.0}s; pass a larger --timeout",
            plan.tests,
            baseline_cap.as_secs_f64()
        ));
    }
    if !base.success {
        // `sh -c` reports a missing command as exit 127; "fix the suite"
        // would send the user in exactly the wrong direction.
        if base.status == Some(127) {
            return Err(format!(
                "test command `{}` was not found (exit 127) — is it installed and on PATH?",
                plan.tests
            ));
        }
        return Err(format!(
            "baseline run of `{}` failed on the unmutated project — fix the suite first, then mutate",
            plan.tests
        ));
    }
    let timeout = plan
        .timeout
        .unwrap_or_else(|| derive_timeout(base.duration));
    on_baseline(base.duration, timeout);

    let mut results = Vec::new();
    let mut tally = Tally::default();
    for file in &plan.files {
        let target = sandbox.path_of(&file.rel);
        for mutant in &file.mutants {
            let mutated = mutant.apply(&file.source);
            let outcome;
            let duration;
            if fs::write(&target, &mutated).is_err() {
                outcome = Outcome::Error;
                duration = Duration::ZERO;
            } else {
                match run_tests(&plan.tests, &sandbox.dir, Some(timeout)) {
                    Ok(run) if run.timed_out => {
                        outcome = Outcome::Timeout;
                        duration = run.duration;
                    }
                    Ok(run) => {
                        outcome = if run.success {
                            Outcome::Survived
                        } else {
                            Outcome::Killed
                        };
                        duration = run.duration;
                    }
                    Err(_) => {
                        outcome = Outcome::Error;
                        duration = Duration::ZERO;
                    }
                }
            }
            // Always restore the pristine file before the next mutant.
            fs::write(&target, &file.source)
                .map_err(|e| format!("cannot restore {} in sandbox: {e}", file.rel.display()))?;
            tally.count(outcome);
            progress(mutant, outcome, duration);
            results.push(MutantResult {
                mutant: mutant.clone(),
                outcome,
                duration,
            });
        }
    }
    Ok(RunReport {
        baseline: base.duration,
        timeout,
        results,
        tally,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mutators::{generate, OpSet};
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn temp_project() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mutash-runner-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn run_tests_reports_status_and_runs_in_the_given_directory() {
        let dir = temp_project();
        let ok = run_tests("exit 0", &dir, None).unwrap();
        assert!(ok.success && !ok.timed_out);
        let bad = run_tests("exit 7", &dir, None).unwrap();
        assert!(!bad.success && !bad.timed_out);
        assert_eq!(bad.status, Some(7), "exit status must be reported");
        fs::write(dir.join("marker"), "x").unwrap();
        let r = run_tests("test -f marker", &dir, None).unwrap();
        assert!(r.success, "command must run with the sandbox as cwd");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn run_tests_kills_a_command_that_overruns_the_deadline() {
        let dir = temp_project();
        // `sleep 5` would make the suite hang for 5s; the 100ms deadline
        // must cut it off deterministically.
        let r = run_tests("sleep 5", &dir, Some(Duration::from_millis(100))).unwrap();
        assert!(r.timed_out);
        assert!(r.duration < Duration::from_secs(3), "kill was not prompt");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn derive_timeout_scales_with_baseline_but_has_a_floor() {
        assert_eq!(derive_timeout(Duration::ZERO), Duration::from_secs(2));
        assert_eq!(
            derive_timeout(Duration::from_secs(4)),
            Duration::from_secs(14)
        );
        assert!(derive_timeout(Duration::from_millis(1)) >= Duration::from_secs(1));
    }

    /// Build a tiny real project: a script with one killable and one
    /// unkillable mutant, plus a plain-shell test suite.
    fn seeded_plan() -> (PathBuf, Plan) {
        let dir = temp_project();
        let script = "#!/bin/sh\nif [ \"$1\" -eq 0 ]; then echo zero; else echo other; fi\n";
        fs::write(dir.join("classify.sh"), script).unwrap();
        // The suite checks input 0 but never a non-zero input, so `-eq`->`-ne`
        // dies while number mutants on the 0 survive partially.
        fs::write(
            dir.join("check.sh"),
            "#!/bin/sh\nout=$(sh classify.sh 0)\ntest \"$out\" = zero\n",
        )
        .unwrap();
        let ops = OpSet::only("compare").unwrap();
        let mut mutants = generate(script, &ops);
        for (i, m) in mutants.iter_mut().enumerate() {
            m.id = i + 1;
            m.file = "classify.sh".into();
        }
        let plan = Plan {
            root: dir.clone(),
            tests: "sh check.sh".into(),
            timeout: Some(Duration::from_secs(10)),
            files: vec![FilePlan {
                rel: PathBuf::from("classify.sh"),
                source: script.to_string(),
                mutants,
            }],
        };
        (dir, plan)
    }

    #[test]
    fn execute_kills_a_detectable_mutant_and_reports_progress() {
        let (dir, plan) = seeded_plan();
        let mut calls = 0;
        let report = execute(&plan, |_, _| {}, |_, _, _| calls += 1).unwrap();
        assert_eq!(report.tally.total, 1, "one compare mutant expected");
        assert_eq!(report.tally.killed, 1);
        assert_eq!(report.results[0].outcome, Outcome::Killed);
        assert_eq!(calls, 1, "progress must fire once per mutant");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn execute_restores_the_file_between_mutants() {
        let (dir, mut plan) = seeded_plan();
        // Duplicate the mutant: if restoration failed, the second application
        // would compound with the first and corrupt offsets.
        let m = plan.files[0].mutants[0].clone();
        plan.files[0].mutants.push(Mutant { id: 2, ..m });
        let report = execute(&plan, |_, _| {}, |_, _, _| {}).unwrap();
        assert_eq!(report.tally.killed, 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn execute_reports_survivors_for_a_weak_suite() {
        let (dir, mut plan) = seeded_plan();
        plan.tests = "exit 0".into(); // a suite that asserts nothing
        let report = execute(&plan, |_, _| {}, |_, _, _| {}).unwrap();
        assert_eq!(report.tally.survived, report.tally.total);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn execute_refuses_a_failing_baseline() {
        let (dir, mut plan) = seeded_plan();
        plan.tests = "exit 1".into();
        let err = execute(&plan, |_, _| {}, |_, _, _| {}).unwrap_err();
        assert!(err.contains("fix the suite first"), "{err}");
        // A missing test runner (exit 127) is not the suite's fault and
        // must be diagnosed as such, not as a red suite.
        plan.tests = "definitely-not-a-real-test-runner".into();
        let err = execute(&plan, |_, _| {}, |_, _, _| {}).unwrap_err();
        assert!(err.contains("was not found"), "{err}");
        assert!(!err.contains("fix the suite"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn execute_never_touches_the_original_tree() {
        let (dir, plan) = seeded_plan();
        let before = fs::read_to_string(dir.join("classify.sh")).unwrap();
        execute(&plan, |_, _| {}, |_, _, _| {}).unwrap();
        let after = fs::read_to_string(dir.join("classify.sh")).unwrap();
        assert_eq!(before, after);
        fs::remove_dir_all(&dir).unwrap();
    }
}
