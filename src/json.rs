//! Minimal JSON emission helpers (std-only, no serialization framework).
//!
//! mutash only ever *writes* JSON — a flat, stable report schema — so a
//! string escaper plus `format!` is all it needs.

/// Escape a string for inclusion inside a JSON string literal.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Format a score with one decimal place, JSON- and human-friendly.
pub fn fmt_score(score: f64) -> String {
    format!("{score:.1}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_every_class_of_special_character() {
        assert_eq!(esc(r#"a "b" \c"#), r#"a \"b\" \\c"#);
        assert_eq!(esc("a\nb\tc\r"), "a\\nb\\tc\\r");
        assert_eq!(esc("\u{1}"), "\\u0001");
    }

    #[test]
    fn passes_plain_and_multibyte_text_through() {
        assert_eq!(esc("plain -eq text"), "plain -eq text");
        assert_eq!(esc("café ✓"), "café ✓");
    }

    #[test]
    fn scores_format_with_one_decimal() {
        assert_eq!(fmt_score(91.30434), "91.3");
        assert_eq!(fmt_score(100.0), "100.0");
        assert_eq!(fmt_score(0.0), "0.0");
    }
}
