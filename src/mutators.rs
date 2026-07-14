//! Mutation operators: turns the token stream into concrete mutants.
//!
//! Every operator is context-gated so mutants stay syntactically valid and
//! semantically meaningful: test operators only mutate between `[`/`[[`/`test`
//! and their closers, arithmetic operators only inside `$(( ))`/`(( ))`,
//! flags only in argument position, `true`/`false` only in command position.
//! `# mutash: off/on/skip` pragmas recorded by the lexer are honored here.

use crate::lexer::{scan, Pragma, TokKind, Token};
use crate::mutant::Mutant;
use std::collections::BTreeSet;

/// `(id, context, examples)` rows for every mutation operator, in the order
/// they are documented. The ids are what `--only` / `--skip` accept.
pub const OPERATORS: &[(&str, &str, &str)] = &[
    (
        "compare",
        "[ ], [[ ]], test, $(( ))",
        "-eq -> -ne, -lt -> -le, < -> <=",
    ),
    ("unary", "[ ], [[ ]], test", "-z -> -n, -f -> -d, -r -> -w"),
    ("connective", "lists and [ ]", "&& -> ||, -a -> -o"),
    (
        "arith",
        "$(( )), (( ))",
        "+ -> -, * -> /, ++ -> --, += -> -=",
    ),
    ("number", "[ ], [[ ]], $(( ))", "3 -> 4, 3 -> 2, 0 -> 1"),
    (
        "exit",
        "exit / return statuses",
        "exit 1 -> exit 0, return 0 -> return 1",
    ),
    (
        "flag",
        "command arguments",
        "-rf -> -r, drop -q, drop --force",
    ),
    ("negate", "command and test position", "drop !"),
    ("truth", "command position", "true -> false"),
];

/// The set of enabled operator ids.
#[derive(Debug, Clone)]
pub struct OpSet {
    enabled: BTreeSet<&'static str>,
}

impl OpSet {
    /// All operators enabled (the default).
    pub fn all() -> Self {
        OpSet {
            enabled: OPERATORS.iter().map(|(id, _, _)| *id).collect(),
        }
    }

    /// Enable only the comma-separated ids in `list`.
    pub fn only(list: &str) -> Result<Self, String> {
        let mut set = BTreeSet::new();
        for id in Self::split(list)? {
            set.insert(id);
        }
        Ok(OpSet { enabled: set })
    }

    /// All operators except the comma-separated ids in `list`.
    pub fn skip(list: &str) -> Result<Self, String> {
        let mut all = Self::all();
        for id in Self::split(list)? {
            all.enabled.remove(id);
        }
        Ok(all)
    }

    fn split(list: &str) -> Result<Vec<&'static str>, String> {
        let mut out = Vec::new();
        for raw in list.split(',') {
            let name = raw.trim();
            if name.is_empty() {
                continue;
            }
            match OPERATORS.iter().find(|(id, _, _)| *id == name) {
                Some((id, _, _)) => out.push(*id),
                None => {
                    return Err(format!(
                        "unknown operator `{name}` (run `mutash ops` for the list)"
                    ))
                }
            }
        }
        Ok(out)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.enabled.contains(id)
    }
}

/// Which test command the scanner is currently inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestCtx {
    None,
    /// `[ … ]` or `test …`
    Single,
    /// `[[ … ]]`
    Double,
}

/// Generate every enabled mutant for one script source.
///
/// Mutants come back in source order with `id = 0`; the caller assigns
/// globally unique ids and the display file name.
pub fn generate(src: &str, ops: &OpSet) -> Vec<Mutant> {
    let result = scan(src);
    let mut out: Vec<Mutant> = Vec::new();
    let mut ctx = TestCtx::None;
    let mut prev_word: Option<(String, bool)> = None; // (text, cmd_pos)

    for tok in &result.tokens {
        match tok.kind {
            TokKind::Sep => {
                match tok.text.as_str() {
                    "\n" | ";" | ";;" | "|" | "&" | "(" | ")" => {
                        ctx = TestCtx::None;
                        prev_word = None;
                    }
                    "<" | ">" => {
                        // Inside `[[ ]]` these are string comparisons.
                        if ctx == TestCtx::Double && ops.contains("compare") {
                            let to = if tok.text == "<" { ">" } else { "<" };
                            out.push(swap(tok, "compare", to));
                        }
                    }
                    _ => {}
                }
            }
            TokKind::AndAnd | TokKind::OrOr => {
                if ops.contains("connective") {
                    let to = if tok.kind == TokKind::AndAnd {
                        "||"
                    } else {
                        "&&"
                    };
                    out.push(swap(tok, "connective", to));
                }
                if ctx == TestCtx::Single {
                    ctx = TestCtx::None;
                }
                prev_word = None;
            }
            TokKind::ArithOp => {
                if let Some((op_id, to)) = arith_swap(&tok.text) {
                    if ops.contains(op_id) {
                        out.push(swap(tok, op_id, to));
                    }
                }
            }
            TokKind::ArithNum => {
                if ops.contains("number") {
                    number_mutants(tok, &mut out);
                }
            }
            TokKind::Word => {
                word_mutants(tok, src, ctx, &prev_word, ops, &mut out);
                match tok.text.as_str() {
                    "[[" if tok.cmd_pos => ctx = TestCtx::Double,
                    "[" if tok.cmd_pos => ctx = TestCtx::Single,
                    "test" if tok.cmd_pos => ctx = TestCtx::Single,
                    "]]" | "]" => ctx = TestCtx::None,
                    _ => {}
                }
                prev_word = Some((tok.text.clone(), tok.cmd_pos));
            }
        }
    }

    apply_pragmas(&mut out, &result.pragmas);
    out
}

fn word_mutants(
    tok: &Token,
    src: &str,
    ctx: TestCtx,
    prev_word: &Option<(String, bool)>,
    ops: &OpSet,
    out: &mut Vec<Mutant>,
) {
    if tok.tainted {
        return;
    }
    let text = tok.text.as_str();
    let in_test = ctx != TestCtx::None;

    // `!` removal: `if ! cmd`, `[[ ! -f x ]]`.
    if text == "!" && (tok.cmd_pos || in_test) {
        if ops.contains("negate") {
            out.push(delete(tok, src, "negate"));
        }
        return;
    }

    // `true` / `false` as a command.
    if tok.cmd_pos && (text == "true" || text == "false") {
        if ops.contains("truth") {
            let to = if text == "true" { "false" } else { "true" };
            out.push(swap(tok, "truth", to));
        }
        return;
    }

    // Binary comparison and unary test operators.
    if in_test {
        if ops.contains("compare") {
            if let Some(to) = compare_swap(text) {
                out.push(swap(tok, "compare", to));
                return;
            }
        }
        if ops.contains("unary") {
            if let Some(to) = unary_swap(text) {
                out.push(swap(tok, "unary", to));
                return;
            }
        }
        if ops.contains("connective") && (text == "-a" || text == "-o") {
            let to = if text == "-a" { "-o" } else { "-a" };
            out.push(swap(tok, "connective", to));
            return;
        }
    }

    // Integer literals: exit/return statuses first, then test-expression numbers.
    if let Ok(n) = text.parse::<i64>() {
        if let Some((prev, prev_cmd)) = prev_word {
            if *prev_cmd && (prev == "exit" || prev == "return") {
                if ops.contains("exit") {
                    let to = if n == 0 {
                        "1".to_string()
                    } else {
                        "0".to_string()
                    };
                    out.push(swap_owned(tok, "exit", to));
                }
                return;
            }
        }
        if in_test && ops.contains("number") {
            number_mutants(tok, out);
        }
        return;
    }

    // Command flags (argument position only; in a test these are operators).
    if !in_test && !tok.cmd_pos && ops.contains("flag") {
        flag_mutants(tok, src, out);
    }
}

/// `n -> n+1` and `n -> n-1`, the classic boundary mutants.
fn number_mutants(tok: &Token, out: &mut Vec<Mutant>) {
    let Ok(n) = tok.text.parse::<i64>() else {
        return;
    };
    for delta in [1i64, -1] {
        let Some(v) = n.checked_add(delta) else {
            continue;
        };
        out.push(swap_owned(tok, "number", v.to_string()));
    }
}

fn flag_mutants(tok: &Token, src: &str, out: &mut Vec<Mutant>) {
    let s = tok.text.as_str();
    if !s.starts_with('-') || s == "-" || s == "--" {
        return;
    }
    if let Some(long) = s.strip_prefix("--") {
        // `--force`, `--depth=1`: drop the whole flag.
        if long.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            out.push(delete(tok, src, "flag"));
        }
        return;
    }
    let body = &s[1..];
    if !body.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return; // `-5`, `-@foo`: not a flag
    }
    out.push(delete(tok, src, "flag"));
    if body.len() >= 2 && body.chars().all(|c| c.is_ascii_alphabetic()) {
        // Shrink a cluster: `-rf` -> `-r`.
        let shrunk = &s[..s.len() - 1];
        out.push(swap_owned(tok, "flag", shrunk.to_string()));
    }
}

fn compare_swap(text: &str) -> Option<&'static str> {
    Some(match text {
        "-eq" => "-ne",
        "-ne" => "-eq",
        // Boundary mutants: only survivable with an exact-boundary test case.
        "-lt" => "-le",
        "-le" => "-lt",
        "-gt" => "-ge",
        "-ge" => "-gt",
        "=" | "==" => "!=",
        "!=" => "==",
        _ => return None,
    })
}

fn unary_swap(text: &str) -> Option<&'static str> {
    Some(match text {
        "-z" => "-n",
        "-n" => "-z",
        "-f" => "-d",
        "-d" => "-f",
        "-e" => "-d",
        "-r" => "-w",
        "-w" => "-r",
        "-x" => "-r",
        "-s" => "-f",
        _ => return None,
    })
}

fn arith_swap(text: &str) -> Option<(&'static str, &'static str)> {
    Some(match text {
        "+" => ("arith", "-"),
        "-" => ("arith", "+"),
        "*" => ("arith", "/"),
        "/" => ("arith", "*"),
        "%" => ("arith", "*"),
        "++" => ("arith", "--"),
        "--" => ("arith", "++"),
        "+=" => ("arith", "-="),
        "-=" => ("arith", "+="),
        // Relational boundary mutants inside arithmetic.
        "<" => ("compare", "<="),
        "<=" => ("compare", "<"),
        ">" => ("compare", ">="),
        ">=" => ("compare", ">"),
        "==" => ("compare", "!="),
        "!=" => ("compare", "=="),
        _ => return None,
    })
}

fn swap(tok: &Token, op: &'static str, to: &str) -> Mutant {
    swap_owned(tok, op, to.to_string())
}

fn swap_owned(tok: &Token, op: &'static str, to: String) -> Mutant {
    Mutant {
        id: 0,
        file: String::new(),
        op,
        line: tok.line,
        col: tok.col,
        offset: tok.offset,
        len: tok.len,
        original: tok.text.clone(),
        descr: format!("`{}` -> `{}`", tok.text, to),
        replacement: to,
    }
}

/// Delete a token, swallowing one adjacent space so the line stays valid.
fn delete(tok: &Token, src: &str, op: &'static str) -> Mutant {
    let bytes = src.as_bytes();
    let mut offset = tok.offset;
    let mut len = tok.len;
    if bytes.get(offset + len) == Some(&b' ') {
        len += 1;
    } else if offset > 0 && bytes.get(offset - 1) == Some(&b' ') {
        offset -= 1;
        len += 1;
    }
    Mutant {
        id: 0,
        file: String::new(),
        op,
        line: tok.line,
        col: tok.col,
        offset,
        len,
        original: tok.text.clone(),
        replacement: String::new(),
        descr: format!("drop `{}`", tok.text),
    }
}

/// Remove mutants on lines disabled by `# mutash: off/on/skip`.
fn apply_pragmas(out: &mut Vec<Mutant>, pragmas: &[(u32, Pragma)]) {
    if pragmas.is_empty() {
        return;
    }
    let mut skip_lines: BTreeSet<u32> = BTreeSet::new();
    let mut off_ranges: Vec<(u32, u32)> = Vec::new(); // inclusive start, exclusive end
    let mut off_since: Option<u32> = None;
    for &(line, p) in pragmas {
        match p {
            Pragma::SkipLine => {
                skip_lines.insert(line);
            }
            Pragma::Off => {
                if off_since.is_none() {
                    off_since = Some(line);
                }
            }
            Pragma::On => {
                if let Some(start) = off_since.take() {
                    off_ranges.push((start, line));
                }
            }
        }
    }
    if let Some(start) = off_since {
        off_ranges.push((start, u32::MAX));
    }
    out.retain(|m| {
        !skip_lines.contains(&m.line) && !off_ranges.iter().any(|&(a, b)| m.line >= a && m.line < b)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen(src: &str) -> Vec<Mutant> {
        generate(src, &OpSet::all())
    }

    fn descrs(src: &str) -> Vec<String> {
        gen(src).into_iter().map(|m| m.descr).collect()
    }

    fn ops_of(src: &str) -> Vec<&'static str> {
        gen(src).into_iter().map(|m| m.op).collect()
    }

    #[test]
    fn numeric_comparisons_swap_inside_single_bracket() {
        let d = descrs("[ \"$n\" -eq 0 ]\n");
        assert!(d.contains(&"`-eq` -> `-ne`".to_string()), "{d:?}");
        // `-lt` -> `-le` only differs at the boundary value; a test suite
        // without an exact-boundary case cannot kill it.
        let d = descrs("if [ \"$n\" -lt 10 ]; then :; fi\n");
        assert!(d.contains(&"`-lt` -> `-le`".to_string()), "{d:?}");
    }

    #[test]
    fn string_equality_swaps_inside_double_bracket() {
        let d = descrs("[[ $a == foo ]] && [[ $b != bar ]]\n");
        assert!(d.contains(&"`==` -> `!=`".to_string()));
        assert!(d.contains(&"`!=` -> `==`".to_string()));
    }

    #[test]
    fn test_builtin_counts_as_a_test_context() {
        let d = descrs("test -f /etc/hosts && echo ok\n");
        assert!(d.contains(&"`-f` -> `-d`".to_string()), "{d:?}");
    }

    #[test]
    fn eq_outside_a_test_is_untouched() {
        // `-eq` as a literal argument to a random command is not a test op.
        assert!(!descrs("printf '%s' x -eq y\n")
            .iter()
            .any(|d| d.contains("-ne")));
    }

    #[test]
    fn angle_brackets_swap_only_inside_double_bracket() {
        let d = descrs("[[ $a < $b ]]\n");
        assert!(d.contains(&"`<` -> `>`".to_string()), "{d:?}");
        // Plain redirections must never be flipped.
        assert!(descrs("sort < in > out\n").is_empty());
    }

    #[test]
    fn unary_string_and_file_tests_swap() {
        let d = descrs("[ -z \"$x\" ] || [ -n \"$y\" ]\n");
        assert!(d.contains(&"`-z` -> `-n`".to_string()));
        assert!(d.contains(&"`-n` -> `-z`".to_string()));
        let d = descrs("[[ -d $dir && -r $file ]]\n");
        assert!(d.contains(&"`-d` -> `-f`".to_string()));
        assert!(d.contains(&"`-r` -> `-w`".to_string()));
    }

    #[test]
    fn connectives_swap_in_lists_and_single_brackets() {
        let d = descrs("make build && make test || notify\n");
        assert!(d.contains(&"`&&` -> `||`".to_string()));
        assert!(d.contains(&"`||` -> `&&`".to_string()));
        let d = descrs("[ -n \"$a\" -a -n \"$b\" ]\n");
        assert!(d.contains(&"`-a` -> `-o`".to_string()), "{d:?}");
        // Outside a test, `-a` is an ordinary flag (e.g. `ls -a`), not a connective.
        let outside = gen("ls -a\n");
        assert!(outside.iter().all(|m| m.op == "flag"), "{outside:?}");
    }

    #[test]
    fn arithmetic_operator_swaps() {
        let d = descrs("total=$((total + i))\n");
        assert!(d.contains(&"`+` -> `-`".to_string()), "{d:?}");
        let d = descrs("x=$(( a * b / c ))\n");
        assert!(d.contains(&"`*` -> `/`".to_string()));
        assert!(d.contains(&"`/` -> `*`".to_string()));
        let d = descrs("(( i++, total += n ))\n");
        assert!(d.contains(&"`++` -> `--`".to_string()), "{d:?}");
        assert!(d.contains(&"`+=` -> `-=`".to_string()), "{d:?}");
    }

    #[test]
    fn arithmetic_relational_ops_are_compare_mutants() {
        let m = gen("while (( i < n )); do :; done\n");
        let rel = m.iter().find(|m| m.original == "<").unwrap();
        assert_eq!(rel.op, "compare");
        assert_eq!(rel.replacement, "<=");
    }

    #[test]
    fn numbers_in_tests_and_arithmetic_get_off_by_one_mutants() {
        let d = descrs("[ \"$#\" -eq 2 ]\n");
        assert!(d.contains(&"`2` -> `3`".to_string()), "{d:?}");
        assert!(d.contains(&"`2` -> `1`".to_string()), "{d:?}");
        let d = descrs("x=$(( y + 10 ))\n");
        assert!(d.contains(&"`10` -> `11`".to_string()));
        assert!(d.contains(&"`10` -> `9`".to_string()));
    }

    #[test]
    fn bare_argument_numbers_are_left_alone() {
        // `head -n 5`: the 5 is data, not a decision — mutating it is noise.
        let m = gen("head -n 5 file\n");
        assert!(m.iter().all(|m| m.op == "flag"), "{m:?}");
    }

    #[test]
    fn exit_statuses_flip_between_zero_and_nonzero() {
        let d = descrs("exit 0\n");
        assert!(d.contains(&"`0` -> `1`".to_string()));
        let d = descrs("exit 3\n");
        assert!(d.contains(&"`3` -> `0`".to_string()));
        let d = descrs("return 1\n");
        assert!(d.contains(&"`1` -> `0`".to_string()));
        // The status flip must not also produce number mutants for the token.
        let m = gen("exit 2\n");
        assert_eq!(m.len(), 1, "{m:?}");
        assert_eq!(m[0].op, "exit");
    }

    #[test]
    fn flags_are_dropped_and_clusters_shrunk() {
        let m = gen("grep -q pattern file\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].op, "flag");
        assert_eq!(m[0].descr, "drop `-q`");
        let d = descrs("rm -rf build\n");
        assert!(d.contains(&"drop `-rf`".to_string()), "{d:?}");
        assert!(d.contains(&"`-rf` -> `-r`".to_string()), "{d:?}");
        let d = descrs("git clone --depth=1 url\n");
        assert!(d.contains(&"drop `--depth=1`".to_string()), "{d:?}");
    }

    #[test]
    fn non_flags_are_never_dropped() {
        // Negative numbers and bare dashes are data.
        assert!(gen("seq -5 5\n").is_empty());
        assert!(gen("cmd - --\n").is_empty());
        // A `--help`-style word in command position is a command, not an argument.
        let m = gen("if true; then --weird; fi\n");
        assert!(m.iter().all(|m| m.op != "flag"), "{m:?}");
        // Quoted flags are string data.
        assert!(gen("printf '%s' '-q'\n").is_empty());
        assert!(gen("echo \"-rf\"\n").is_empty());
    }

    #[test]
    fn negation_is_dropped_in_command_and_test_position() {
        let d = descrs("if ! grep -q x f; then exit 1; fi\n");
        assert!(d.contains(&"drop `!`".to_string()), "{d:?}");
        let d = descrs("[[ ! -f $x ]]\n");
        assert!(d.contains(&"drop `!`".to_string()), "{d:?}");
    }

    #[test]
    fn true_false_swap_only_in_command_position() {
        let d = descrs("true && false\n");
        assert!(d.contains(&"`true` -> `false`".to_string()));
        assert!(d.contains(&"`false` -> `true`".to_string()));
        // As a plain argument, `true` is data.
        let m = gen("echo true\n");
        assert!(m.is_empty(), "{m:?}");
    }

    #[test]
    fn deletion_swallows_one_adjacent_space() {
        let src = "grep -q pat file\n";
        let m = gen(src);
        let mutated = m[0].apply(src);
        assert_eq!(mutated, "grep pat file\n");
    }

    #[test]
    fn mutants_inside_command_substitution_are_found() {
        let d = descrs("count=$(grep -c -q pat file || true)\n");
        assert!(d.contains(&"drop `-q`".to_string()), "{d:?}");
        assert!(d.contains(&"`||` -> `&&`".to_string()), "{d:?}");
    }

    #[test]
    fn comments_heredocs_and_strings_yield_no_mutants() {
        let src = "# rm -rf && exit 1\ncat <<EOF\n[ 1 -eq 1 ] && true\nEOF\necho 'a && b'\n";
        assert!(gen(src).is_empty());
    }

    #[test]
    fn pragma_skip_disables_a_single_line() {
        let src = "[ \"$a\" -eq 1 ] # mutash: skip\n[ \"$b\" -eq 2 ]\n";
        let m = gen(src);
        assert!(m.iter().all(|m| m.line == 2), "{m:?}");
        assert!(!m.is_empty());
    }

    #[test]
    fn pragma_off_on_disables_a_range() {
        let src = "\
[ \"$a\" -eq 1 ]
# mutash: off
[ \"$b\" -eq 2 ]
[ \"$c\" -eq 3 ]
# mutash: on
[ \"$d\" -eq 4 ]
";
        let m = gen(src);
        let lines: BTreeSet<u32> = m.iter().map(|m| m.line).collect();
        assert!(lines.contains(&1));
        assert!(lines.contains(&6));
        assert!(!lines.contains(&3) && !lines.contains(&4), "{lines:?}");
    }

    #[test]
    fn pragma_off_without_on_runs_to_end_of_file() {
        let src = "# mutash: off\n[ \"$a\" -eq 1 ]\n[ \"$b\" -eq 2 ]\n";
        assert!(gen(src).is_empty());
    }

    #[test]
    fn opset_only_and_skip_select_operators() {
        let ops = OpSet::only("flag").unwrap();
        let m = generate("rm -rf x && [ -f y ]\n", &ops);
        assert!(m.iter().all(|m| m.op == "flag"), "{m:?}");
        assert!(!m.is_empty());
        let ops = OpSet::skip("flag,connective").unwrap();
        let m = generate("rm -rf x && [ -f y ]\n", &ops);
        assert!(m.iter().all(|m| m.op == "unary"), "{m:?}");
    }

    #[test]
    fn opset_rejects_unknown_ids() {
        let err = OpSet::only("compare,bogus").unwrap_err();
        assert!(err.contains("bogus"));
    }

    #[test]
    fn mutants_are_reported_in_source_order() {
        let src = "[ -f a ]\n[ -f b ]\n";
        let m = gen(src);
        let lines: Vec<u32> = m.iter().map(|m| m.line).collect();
        let mut sorted = lines.clone();
        sorted.sort_unstable();
        assert_eq!(lines, sorted);
    }

    #[test]
    fn every_generated_op_id_is_documented() {
        let src = "\
#!/usr/bin/env bash
if ! [ \"$1\" -lt 3 ]; then exit 1; fi
[[ $a == b && -z $c ]]
x=$(( x + 1 ))
rm -rf tmp || true
";
        let known: BTreeSet<&str> = OPERATORS.iter().map(|(id, _, _)| *id).collect();
        for m in gen(src) {
            assert!(known.contains(m.op), "undocumented op id {}", m.op);
        }
        assert_eq!(ops_of(src).len(), gen(src).len());
    }
}
