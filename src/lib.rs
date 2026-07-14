//! mutash — mutation testing for shell scripts.
//!
//! Flips operators and flags token-by-token (no bash AST, no interpreter
//! patching), runs your test command against every mutant in a disposable
//! sandbox, and grades the suite by how many mutants it kills.
//!
//! Modules, in pipeline order:
//! - [`lexer`] — shell-aware scan: live tokens, contexts, pragmas
//! - [`mutators`] — mutation operators over the token stream
//! - [`mutant`] — the mutant itself: span rewrite + display helpers
//! - [`sandbox`] — disposable project copies
//! - [`runner`] — baseline, per-mutant runs, timeout policy
//! - [`report`] — scoring, grading, text/JSON rendering
//! - [`cli`] — argument parsing and command dispatch

pub mod cli;
pub mod json;
pub mod lexer;
pub mod mutant;
pub mod mutators;
pub mod report;
pub mod runner;
pub mod sandbox;

/// Crate version, single source of truth for `--version` and reports.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
