//! The mutant itself: one byte-span rewrite of a script, plus helpers to
//! apply it and to show the source line it lives on.

/// A single mutation: replace `len` bytes at `offset` with `replacement`.
#[derive(Debug, Clone)]
pub struct Mutant {
    /// 1-based id, unique across the whole run (assigned by the CLI).
    pub id: usize,
    /// Display path of the script, relative to the project root.
    pub file: String,
    /// Operator id (`compare`, `flag`, …) — see `mutators::OPERATORS`.
    pub op: &'static str,
    /// 1-based line of the mutated token.
    pub line: u32,
    /// 1-based byte column of the mutated token.
    pub col: u32,
    /// Byte offset of the span to rewrite.
    pub offset: usize,
    /// Byte length of the span to rewrite.
    pub len: usize,
    /// The original token text.
    pub original: String,
    /// Replacement text; empty for deletions.
    pub replacement: String,
    /// Human-readable summary, e.g. `` `-eq` -> `-ne` `` or `` drop `-q` ``.
    pub descr: String,
}

impl Mutant {
    /// Apply this mutation to the original source, returning the mutated text.
    pub fn apply(&self, src: &str) -> String {
        debug_assert!(self.offset + self.len <= src.len());
        let mut out = String::with_capacity(src.len() + self.replacement.len());
        out.push_str(&src[..self.offset]);
        out.push_str(&self.replacement);
        out.push_str(&src[self.offset + self.len..]);
        out
    }

    /// The (trimmed) source line containing the mutated span.
    pub fn excerpt<'a>(&self, src: &'a str) -> &'a str {
        let bytes = src.as_bytes();
        let mut start = self.offset.min(src.len());
        while start > 0 && bytes[start - 1] != b'\n' {
            start -= 1;
        }
        let mut end = self.offset.min(src.len());
        while end < src.len() && bytes[end] != b'\n' {
            end += 1;
        }
        src[start..end].trim()
    }

    /// `file:line:col` location string used across all report formats.
    pub fn location(&self) -> String {
        format!("{}:{}:{}", self.file, self.line, self.col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(offset: usize, len: usize, replacement: &str) -> Mutant {
        Mutant {
            id: 1,
            file: "s.sh".into(),
            op: "compare",
            line: 1,
            col: 1,
            offset,
            len,
            original: "x".into(),
            replacement: replacement.into(),
            descr: String::new(),
        }
    }

    #[test]
    fn apply_replaces_deletes_and_grows_exactly_the_span() {
        assert_eq!(
            sample(5, 3, "-ne").apply("[ $a -eq 0 ]\n"),
            "[ $a -ne 0 ]\n"
        );
        assert_eq!(sample(3, 4, "").apply("rm -rf x\n"), "rm x\n");
        assert_eq!(
            sample(3, 3, "--recursive").apply("rm -rf x\n"),
            "rm --recursive x\n"
        );
        // Boundary spans: very start and very end of the source.
        assert_eq!(sample(0, 4, "false").apply("true"), "false");
    }

    #[test]
    fn excerpt_returns_the_trimmed_line_of_the_mutation() {
        let src = "first\n  if [ $a -eq 0 ]; then\nlast\n";
        let m = sample(15, 3, "-ne"); // points at `-eq`
        assert_eq!(m.excerpt(src), "if [ $a -eq 0 ]; then");
    }

    #[test]
    fn excerpt_on_first_and_last_lines() {
        let src = "alpha beta\ngamma";
        assert_eq!(sample(0, 5, "x").excerpt(src), "alpha beta");
        assert_eq!(sample(11, 5, "x").excerpt(src), "gamma");
    }

    #[test]
    fn location_formats_file_line_col() {
        let mut m = sample(0, 1, "y");
        m.file = "deploy.sh".into();
        m.line = 12;
        m.col = 8;
        assert_eq!(m.location(), "deploy.sh:12:8");
    }
}
