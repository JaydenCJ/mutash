//! Command-line interface: argument parsing and command dispatch.
//!
//! Hand-rolled on purpose — mutash has zero runtime dependencies, and the
//! surface is three commands with a handful of flags.

use crate::mutators::{generate, OpSet, OPERATORS};
use crate::report;
use crate::runner::{self, FilePlan, Outcome, Plan};
use std::path::{Path, PathBuf};
use std::time::Duration;

const USAGE: &str = "\
mutash — mutation testing for shell scripts

USAGE:
  mutash <COMMAND> [OPTIONS] <script>...

COMMANDS:
  run <script>...    Mutate the scripts, run the test command against every
                     mutant, list survivors and grade the suite
  list <script>...   Show the mutants that would be generated (nothing runs)
  ops                Print the mutation operator reference

OPTIONS (run):
  --tests <CMD>      Test command run per mutant via `sh -c` [default: bats tests]
  --root <DIR>       Project root copied into the sandbox [default: .]
  --timeout <SECS>   Per-mutant timeout [default: 3x baseline + 2s]
  --min-score <PCT>  Exit 1 when the mutation score falls below this

OPTIONS (run, list):
  --only <OPS>       Comma-separated operator ids to enable exclusively
  --skip <OPS>       Comma-separated operator ids to disable
  --json             Machine-readable report on stdout (progress suppressed)

GLOBAL:
  -h, --help         Print this help
  -V, --version      Print version

EXIT CODES:
  0  success (score met)   1  score below --min-score   2  usage or setup error
";

/// Entry point: returns the process exit code.
pub fn run(args: Vec<String>) -> i32 {
    let mut it = args.iter();
    let Some(cmd) = it.next() else {
        eprint!("{USAGE}");
        return 2;
    };
    match cmd.as_str() {
        "-V" | "--version" => {
            println!("mutash {}", crate::VERSION);
            0
        }
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            0
        }
        "ops" => {
            print_ops();
            0
        }
        "run" => match cmd_run(&args[1..]) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("mutash: {e}");
                2
            }
        },
        "list" => match cmd_list(&args[1..]) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("mutash: {e}");
                2
            }
        },
        other => {
            eprintln!("mutash: unknown command `{other}`\n");
            eprint!("{USAGE}");
            2
        }
    }
}

fn print_ops() {
    println!("mutash mutation operators\n");
    println!("  {:<12} {:<26} EXAMPLES", "ID", "CONTEXT");
    for (id, ctx, examples) in OPERATORS {
        println!("  {id:<12} {ctx:<26} {examples}");
    }
    println!();
    println!(
        "Disable one line with `# mutash: skip`, a block with `# mutash: off` / `# mutash: on`."
    );
    println!("Select at run time with `--only <ids>` or `--skip <ids>` (comma-separated).");
}

/// Options shared by `run` and `list`.
#[derive(Debug)]
struct Common {
    scripts: Vec<String>,
    ops: OpSet,
    json: bool,
}

#[derive(Debug)]
struct RunOpts {
    tests: String,
    root: PathBuf,
    timeout: Option<Duration>,
    min_score: Option<f64>,
}

fn parse_common(args: &[String], allow: &[&str]) -> Result<(Common, RunOpts), String> {
    let mut scripts = Vec::new();
    let mut only: Option<String> = None;
    let mut skip: Option<String> = None;
    let mut json = false;
    let mut tests = "bats tests".to_string();
    let mut root = PathBuf::from(".");
    let mut timeout = None;
    let mut min_score = None;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let mut value = |name: &str| -> Result<String, String> {
            i += 1;
            args.get(i)
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match arg {
            "--only" => only = Some(value("--only")?),
            "--skip" => skip = Some(value("--skip")?),
            "--json" => json = true,
            "--tests" if allow.contains(&"--tests") => tests = value("--tests")?,
            "--root" if allow.contains(&"--root") => root = PathBuf::from(value("--root")?),
            "--timeout" if allow.contains(&"--timeout") => {
                let secs: f64 = value("--timeout")?
                    .parse()
                    .map_err(|_| "--timeout expects seconds (e.g. 30 or 2.5)".to_string())?;
                if secs <= 0.0 {
                    return Err("--timeout must be positive".into());
                }
                timeout = Some(Duration::from_secs_f64(secs));
            }
            "--min-score" if allow.contains(&"--min-score") => {
                let pct: f64 = value("--min-score")?
                    .parse()
                    .map_err(|_| "--min-score expects a percentage (0-100)".to_string())?;
                if !(0.0..=100.0).contains(&pct) {
                    return Err("--min-score must be between 0 and 100".into());
                }
                min_score = Some(pct);
            }
            s if s.starts_with('-') => {
                // A run-only option under `list` deserves better than "unknown".
                return Err(
                    if ["--tests", "--root", "--timeout", "--min-score"].contains(&s) {
                        format!("{s} only applies to `mutash run`")
                    } else {
                        format!("unknown option `{s}`")
                    },
                );
            }
            s => scripts.push(s.to_string()),
        }
        i += 1;
    }

    if scripts.is_empty() {
        return Err("no script given".into());
    }
    if only.is_some() && skip.is_some() {
        return Err("--only and --skip are mutually exclusive".into());
    }
    let ops = match (only, skip) {
        (Some(o), _) => OpSet::only(&o)?,
        (_, Some(s)) => OpSet::skip(&s)?,
        _ => OpSet::all(),
    };
    let common = Common { scripts, ops, json };
    // RunOpts carries the run-only knobs; `list` ignores it.
    let run = RunOpts {
        tests,
        root,
        timeout,
        min_score,
    };
    Ok((common, run))
}

fn read_script(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))
}

/// `1 mutant`, `2 mutants` — a count with a correctly pluralized noun.
fn counted(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn cmd_list(args: &[String]) -> Result<(), String> {
    let (common, _) = parse_common(args, &[])?;
    let mut all = Vec::new();
    let mut sources: Vec<(String, String)> = Vec::new();
    for script in &common.scripts {
        let src = read_script(script)?;
        let mut mutants = generate(&src, &common.ops);
        for m in &mut mutants {
            m.file = script.clone();
        }
        sources.push((script.clone(), src));
        all.extend(mutants);
    }
    for (i, m) in all.iter_mut().enumerate() {
        m.id = i + 1;
    }
    if common.json {
        print!("{}", report::render_list_json(&all));
        return Ok(());
    }
    let mut current_file = String::new();
    for m in &all {
        if m.file != current_file {
            let count = all.iter().filter(|x| x.file == m.file).count();
            println!("{} — {}\n", m.file, counted(count, "mutant"));
            current_file = m.file.clone();
        }
        println!(
            "  #{:<4} {}:{}  {:<11} {}",
            m.id, m.line, m.col, m.op, m.descr
        );
        if let Some((_, src)) = sources.iter().find(|(f, _)| *f == m.file) {
            println!("        {}", m.excerpt(src));
        }
    }
    if !all.is_empty() {
        println!();
    }
    println!(
        "Total: {} across {}",
        counted(all.len(), "mutant"),
        counted(common.scripts.len(), "file")
    );
    Ok(())
}

fn cmd_run(args: &[String]) -> Result<i32, String> {
    let (common, opts) = parse_common(args, &["--tests", "--root", "--timeout", "--min-score"])?;
    let root = opts
        .root
        .canonicalize()
        .map_err(|e| format!("cannot resolve --root {}: {e}", opts.root.display()))?;

    let mut files = Vec::new();
    let mut sources: Vec<(String, String)> = Vec::new();
    let mut next_id = 1usize;
    let mut total = 0usize;
    for script in &common.scripts {
        let src = read_script(script)?;
        let rel = relative_to(script, &root)?;
        let display = rel.display().to_string();
        let mut mutants = generate(&src, &common.ops);
        for m in &mut mutants {
            m.file = display.clone();
            m.id = next_id;
            next_id += 1;
        }
        total += mutants.len();
        sources.push((display, src.clone()));
        files.push(FilePlan {
            rel,
            source: src,
            mutants,
        });
    }

    let plan = Plan {
        root,
        tests: opts.tests.clone(),
        timeout: opts.timeout,
        files,
    };
    let json = common.json;

    if !json {
        println!("mutash {} — mutation run", crate::VERSION);
        println!("  tests:    {}", opts.tests);
        for f in &plan.files {
            println!(
                "  target:   {} ({})",
                f.rel.display(),
                counted(f.mutants.len(), "mutant")
            );
        }
        if total == 0 {
            println!("\nNo mutants generated — nothing to grade.");
            return Ok(0);
        }
    }

    let report = runner::execute(
        &plan,
        |baseline, timeout| {
            if !json {
                println!(
                    "  baseline: pass in {:.2}s (per-mutant timeout: {:.1}s)\n",
                    baseline.as_secs_f64(),
                    timeout.as_secs_f64()
                );
            }
        },
        |m, outcome, duration| {
            if !json {
                println!(
                    "  #{:<4} {:<24} {:<11} {:<22} => {} ({:.2}s)",
                    m.id,
                    m.location(),
                    m.op,
                    m.descr,
                    outcome.label(),
                    duration.as_secs_f64()
                );
            }
        },
    )?;

    if json {
        print!(
            "{}",
            report::render_run_json(
                &opts.tests,
                report.baseline,
                report.timeout,
                &report.results,
                &report.tally
            )
        );
    } else {
        print!(
            "{}",
            report::render_run_summary(&report.results, &sources, &report.tally)
        );
    }

    if report.results.iter().any(|r| r.outcome == Outcome::Error) {
        eprintln!(
            "mutash: warning: some mutants could not be executed (counted against the score)"
        );
    }
    if let Some(min) = opts.min_score {
        if report.tally.score() < min {
            if !json {
                println!(
                    "Result: FAIL — score {}% is below --min-score {}%",
                    crate::json::fmt_score(report.tally.score()),
                    crate::json::fmt_score(min)
                );
            }
            return Ok(1);
        }
        if !json {
            println!(
                "Result: PASS — score {}% meets --min-score {}%",
                crate::json::fmt_score(report.tally.score()),
                crate::json::fmt_score(min)
            );
        }
    }
    Ok(0)
}

/// Express `script` relative to the (canonical) project root.
fn relative_to(script: &str, root: &Path) -> Result<PathBuf, String> {
    let canon = Path::new(script)
        .canonicalize()
        .map_err(|e| format!("cannot resolve {script}: {e}"))?;
    canon
        .strip_prefix(root)
        .map(|p| p.to_path_buf())
        .map_err(|_| {
            format!(
                "{script} is outside the project root {} — pass --root to a directory containing both the script and its tests",
                root.display()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parse_rejects_unknown_options() {
        let err = parse_common(&s(&["x.sh", "--frobnicate"]), &[]).unwrap_err();
        assert!(err.contains("--frobnicate"));
    }

    #[test]
    fn parse_rejects_missing_values() {
        let err = parse_common(&s(&["x.sh", "--only"]), &[]).unwrap_err();
        assert!(err.contains("--only needs a value"));
    }

    #[test]
    fn parse_requires_a_script() {
        let err = parse_common(&s(&["--json"]), &[]).unwrap_err();
        assert!(err.contains("no script"));
    }

    #[test]
    fn parse_rejects_only_plus_skip() {
        let err = parse_common(&s(&["x.sh", "--only", "flag", "--skip", "exit"]), &[]).unwrap_err();
        assert!(err.contains("mutually exclusive"));
    }

    #[test]
    fn parse_rejects_run_options_where_not_allowed() {
        // `list` does not take --tests, and the error says so precisely.
        let err = parse_common(&s(&["x.sh", "--tests", "true"]), &[]).unwrap_err();
        assert_eq!(err, "--tests only applies to `mutash run`");
    }

    #[test]
    fn parse_validates_timeout_and_min_score() {
        let allow = ["--tests", "--root", "--timeout", "--min-score"];
        let err = parse_common(&s(&["x.sh", "--timeout", "nope"]), &allow).unwrap_err();
        assert!(err.contains("seconds"));
        let err = parse_common(&s(&["x.sh", "--timeout", "-1"]), &allow).unwrap_err();
        assert!(err.contains("positive"));
        let err = parse_common(&s(&["x.sh", "--min-score", "150"]), &allow).unwrap_err();
        assert!(err.contains("between 0 and 100"));
    }

    #[test]
    fn parse_accepts_a_full_run_invocation() {
        let allow = ["--tests", "--root", "--timeout", "--min-score"];
        let (common, run) = parse_common(
            &s(&[
                "deploy.sh",
                "--tests",
                "sh t.sh",
                "--timeout",
                "2.5",
                "--min-score",
                "90",
                "--json",
            ]),
            &allow,
        )
        .unwrap();
        assert_eq!(common.scripts, vec!["deploy.sh"]);
        assert!(common.json);
        assert_eq!(run.tests, "sh t.sh");
        assert_eq!(run.timeout, Some(Duration::from_secs_f64(2.5)));
        assert_eq!(run.min_score, Some(90.0));
    }

    #[test]
    fn usage_names_all_commands_and_exit_codes() {
        for needle in [
            "COMMANDS:",
            "run",
            "list",
            "ops",
            "--min-score",
            "EXIT CODES:",
        ] {
            assert!(USAGE.contains(needle), "usage missing {needle}");
        }
    }
}
