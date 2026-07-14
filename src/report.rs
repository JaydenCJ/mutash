//! Scoring, grading and report rendering (text and JSON).

use crate::json::{esc, fmt_score};
use crate::mutant::Mutant;
use crate::runner::Outcome;
use std::time::Duration;

/// Aggregate counts for a finished run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    pub total: usize,
    pub killed: usize,
    pub survived: usize,
    pub timeout: usize,
    pub error: usize,
}

impl Tally {
    pub fn count(&mut self, outcome: Outcome) {
        self.total += 1;
        match outcome {
            Outcome::Killed => self.killed += 1,
            Outcome::Survived => self.survived += 1,
            Outcome::Timeout => self.timeout += 1,
            Outcome::Error => self.error += 1,
        }
    }

    /// Mutation score in percent. Timeouts count as detections (the suite
    /// noticed *something*, even if only by hanging the mutant); errors
    /// count against the score as undetected.
    pub fn score(&self) -> f64 {
        if self.total == 0 {
            return 100.0;
        }
        (self.killed + self.timeout) as f64 * 100.0 / self.total as f64
    }

    /// Letter grade for the score.
    pub fn grade(&self) -> &'static str {
        grade(self.score())
    }
}

/// Letter grade for a mutation score.
pub fn grade(score: f64) -> &'static str {
    if score >= 100.0 {
        "A+"
    } else if score >= 90.0 {
        "A"
    } else if score >= 80.0 {
        "B"
    } else if score >= 70.0 {
        "C"
    } else if score >= 60.0 {
        "D"
    } else {
        "F"
    }
}

/// One mutant's result, kept alongside the mutant for reporting.
#[derive(Debug, Clone)]
pub struct MutantResult {
    pub mutant: Mutant,
    pub outcome: Outcome,
    pub duration: Duration,
}

/// Render the survivors section + summary line of a `mutash run`.
/// `sources` maps display file names to their original source text.
pub fn render_run_summary(
    results: &[MutantResult],
    sources: &[(String, String)],
    tally: &Tally,
) -> String {
    let mut out = String::new();
    let survivors: Vec<&MutantResult> = results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Survived))
        .collect();
    if !survivors.is_empty() {
        out.push_str(&format!("\nSurvivors ({}):\n", survivors.len()));
        for r in &survivors {
            let m = &r.mutant;
            out.push_str(&format!(
                "\n  #{}  {}  {}  {}\n",
                m.id,
                m.location(),
                m.op,
                m.descr
            ));
            if let Some((_, src)) = sources.iter().find(|(f, _)| *f == m.file) {
                out.push_str(&format!("      > {}\n", m.excerpt(src)));
            }
        }
    }
    out.push_str(&format!(
        "\nScore: {}%  ({}/{} detected: {} killed, {} survived, {} timeout, {} error)   Grade: {}\n",
        fmt_score(tally.score()),
        tally.killed + tally.timeout,
        tally.total,
        tally.killed,
        tally.survived,
        tally.timeout,
        tally.error,
        tally.grade()
    ));
    out
}

/// Render the full machine-readable report for `mutash run --json`.
pub fn render_run_json(
    tests: &str,
    baseline: Duration,
    timeout: Duration,
    results: &[MutantResult],
    tally: &Tally,
) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!(
        "  \"tool\": \"mutash\",\n  \"version\": \"{}\",\n",
        crate::VERSION
    ));
    out.push_str(&format!("  \"tests\": \"{}\",\n", esc(tests)));
    out.push_str(&format!("  \"baseline_ms\": {},\n", baseline.as_millis()));
    out.push_str(&format!("  \"timeout_ms\": {},\n", timeout.as_millis()));
    out.push_str("  \"mutants\": [\n");
    for (i, r) in results.iter().enumerate() {
        let sep = if i + 1 == results.len() { "" } else { "," };
        out.push_str(&format!(
            "    {}{sep}\n",
            mutant_json(&r.mutant, Some((r.outcome, r.duration)))
        ));
    }
    out.push_str("  ],\n");
    out.push_str(&format!(
        "  \"summary\": {{\"total\": {}, \"killed\": {}, \"survived\": {}, \"timeout\": {}, \"error\": {}, \"score\": {}, \"grade\": \"{}\"}}\n",
        tally.total,
        tally.killed,
        tally.survived,
        tally.timeout,
        tally.error,
        fmt_score(tally.score()),
        tally.grade()
    ));
    out.push_str("}\n");
    out
}

/// Render the machine-readable mutant list for `mutash list --json`.
pub fn render_list_json(mutants: &[Mutant]) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!(
        "  \"tool\": \"mutash\",\n  \"version\": \"{}\",\n",
        crate::VERSION
    ));
    out.push_str("  \"mutants\": [\n");
    for (i, m) in mutants.iter().enumerate() {
        let sep = if i + 1 == mutants.len() { "" } else { "," };
        out.push_str(&format!("    {}{sep}\n", mutant_json(m, None)));
    }
    out.push_str("  ],\n");
    out.push_str(&format!("  \"total\": {}\n", mutants.len()));
    out.push_str("}\n");
    out
}

fn mutant_json(m: &Mutant, result: Option<(Outcome, Duration)>) -> String {
    let mut s = format!(
        "{{\"id\": {}, \"file\": \"{}\", \"line\": {}, \"col\": {}, \"op\": \"{}\", \"original\": \"{}\", \"replacement\": \"{}\"",
        m.id,
        esc(&m.file),
        m.line,
        m.col,
        m.op,
        esc(&m.original),
        esc(&m.replacement),
    );
    if let Some((outcome, duration)) = result {
        s.push_str(&format!(
            ", \"outcome\": \"{}\", \"ms\": {}",
            outcome.label(),
            duration.as_millis()
        ));
    }
    s.push('}');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mutant(id: usize, file: &str) -> Mutant {
        Mutant {
            id,
            file: file.into(),
            op: "compare",
            line: 2,
            col: 6,
            offset: 10,
            len: 3,
            original: "-eq".into(),
            replacement: "-ne".into(),
            descr: "`-eq` -> `-ne`".into(),
        }
    }

    fn result(id: usize, outcome: Outcome) -> MutantResult {
        MutantResult {
            mutant: mutant(id, "s.sh"),
            outcome,
            duration: Duration::from_millis(40),
        }
    }

    #[test]
    fn tally_counts_every_outcome_bucket() {
        let mut t = Tally::default();
        for o in [
            Outcome::Killed,
            Outcome::Killed,
            Outcome::Survived,
            Outcome::Timeout,
            Outcome::Error,
        ] {
            t.count(o);
        }
        assert_eq!(
            (t.total, t.killed, t.survived, t.timeout, t.error),
            (5, 2, 1, 1, 1)
        );
    }

    #[test]
    fn score_counts_timeouts_as_detections() {
        let mut t = Tally::default();
        t.count(Outcome::Killed);
        t.count(Outcome::Timeout);
        t.count(Outcome::Survived);
        t.count(Outcome::Survived);
        assert!((t.score() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn grade_boundaries_are_inclusive() {
        // An empty run has nothing to miss.
        assert_eq!(Tally::default().score(), 100.0);
        assert_eq!(Tally::default().grade(), "A+");
        assert_eq!(grade(100.0), "A+");
        assert_eq!(grade(99.9), "A");
        assert_eq!(grade(90.0), "A");
        assert_eq!(grade(89.9), "B");
        assert_eq!(grade(80.0), "B");
        assert_eq!(grade(70.0), "C");
        assert_eq!(grade(60.0), "D");
        assert_eq!(grade(59.9), "F");
        assert_eq!(grade(0.0), "F");
    }

    #[test]
    fn summary_lists_survivors_with_excerpts() {
        let results = vec![result(1, Outcome::Killed), result(2, Outcome::Survived)];
        let sources = vec![(
            "s.sh".to_string(),
            "line one\nif [ -eq ]; then\n".to_string(),
        )];
        let text = render_run_summary(&results, &sources, &tally_of(&results));
        assert!(text.contains("Survivors (1):"), "{text}");
        assert!(text.contains("#2  s.sh:2:6"), "{text}");
        assert!(text.contains("> if [ -eq ]; then"), "{text}");
        // Buckets sum to the total; `detected` = killed + timeout.
        assert!(
            text.contains("Score: 50.0%  (1/2 detected: 1 killed, 1 survived, 0 timeout, 0 error)"),
            "{text}"
        );
        assert!(text.contains("Grade: F"), "{text}");
        // With no survivors the section disappears entirely.
        let clean = vec![result(1, Outcome::Killed)];
        let text = render_run_summary(&clean, &[], &tally_of(&clean));
        assert!(!text.contains("Survivors"), "{text}");
        assert!(text.contains("Grade: A+"), "{text}");
    }

    #[test]
    fn run_json_is_well_formed_and_complete() {
        let results = vec![result(1, Outcome::Killed), result(2, Outcome::Survived)];
        let json = render_run_json(
            "bats tests",
            Duration::from_millis(300),
            Duration::from_millis(2900),
            &results,
            &tally_of(&results),
        );
        assert!(json.contains("\"tool\": \"mutash\""));
        assert!(json.contains("\"baseline_ms\": 300"));
        assert!(json.contains("\"timeout_ms\": 2900"));
        assert!(json.contains("\"outcome\": \"killed\""));
        assert!(json.contains("\"outcome\": \"survived\""));
        assert!(json.contains("\"score\": 50.0"));
        // Balanced braces/brackets — a cheap structural sanity check.
        assert_eq!(json.matches('{').count(), json.matches('}').count());
        assert_eq!(json.matches('[').count(), json.matches(']').count());
    }

    #[test]
    fn list_json_has_no_outcome_fields() {
        let json = render_list_json(&[mutant(1, "s.sh")]);
        assert!(json.contains("\"total\": 1"));
        assert!(!json.contains("outcome"));
        assert_eq!(json.matches('{').count(), json.matches('}').count());
    }

    #[test]
    fn json_escapes_special_characters_in_fields() {
        let mut m = mutant(1, "dir/my \"odd\" script.sh");
        m.replacement = "a\\b".into();
        let json = render_list_json(&[m]);
        assert!(json.contains("my \\\"odd\\\" script.sh"), "{json}");
        assert!(json.contains("a\\\\b"), "{json}");
    }

    fn tally_of(results: &[MutantResult]) -> Tally {
        let mut t = Tally::default();
        for r in results {
            t.count(r.outcome);
        }
        t
    }
}
