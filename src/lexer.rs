//! Shell-aware lexical scanner: finds the live, mutable tokens in a script.
//!
//! mutash deliberately does not build a bash AST and does not patch an
//! interpreter. A single conservative byte-level pass answers the only
//! question token-level mutation needs answered: *which spans of the file
//! are live shell code*, and in which context they sit (plain command
//! words, `[` / `[[` test expressions, or `$(( ))` arithmetic).
//!
//! Regions that must never be mutated are skipped precisely: comments,
//! single- and double-quoted strings, backslash escapes, backtick bodies
//! and heredoc bodies (including `<<-` tab-stripped and quoted-delimiter
//! forms). Command substitutions `$( … )` are scanned recursively so
//! mutants inside them are still found; anything inside double quotes is
//! left alone.

/// The kind of a scanned token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokKind {
    /// An unquoted shell word (command name, argument, operator word).
    Word,
    /// `&&`
    AndAnd,
    /// `||`
    OrOr,
    /// A separator / control character: `;` `;;` `|` `&` `(` `)` `<` `>` or newline.
    Sep,
    /// An operator inside `$(( ))` / `(( ))` arithmetic.
    ArithOp,
    /// An integer literal inside arithmetic.
    ArithNum,
}

/// One token with its exact byte span in the source.
#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokKind,
    /// Token text. For tainted words this may omit quoted content; use
    /// `offset`/`len` for the true source span.
    pub text: String,
    /// Byte offset of the token start in the source.
    pub offset: usize,
    /// Byte length of the full token span in the source.
    pub len: usize,
    /// 1-based line number.
    pub line: u32,
    /// 1-based byte column on that line.
    pub col: u32,
    /// The word contains quotes, expansions or escapes — never mutate it.
    pub tainted: bool,
    /// The word sits in command position (start of a simple command).
    pub cmd_pos: bool,
}

/// An in-source `# mutash: …` directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pragma {
    /// `# mutash: off` — disable mutation from this line on.
    Off,
    /// `# mutash: on` — re-enable mutation from this line on.
    On,
    /// `# mutash: skip` — disable mutation on this line only.
    SkipLine,
}

/// Result of scanning a script.
#[derive(Debug, Default)]
pub struct ScanResult {
    pub tokens: Vec<Token>,
    /// `(line, directive)` pairs in source order.
    pub pragmas: Vec<(u32, Pragma)>,
}

/// Scan a shell script into mutable tokens plus pragma directives.
pub fn scan(src: &str) -> ScanResult {
    let mut s = Scanner {
        src: src.as_bytes(),
        i: 0,
        line: 1,
        line_start: 0,
        tokens: Vec::new(),
        pragmas: Vec::new(),
        pending: Vec::new(),
        cmd_pos: true,
        word: None,
    };
    s.scan_code(None);
    s.flush_word();
    ScanResult {
        tokens: s.tokens,
        pragmas: s.pragmas,
    }
}

/// Words after which the *next* word is still in command position.
const CMD_KEEPERS: &[&str] = &[
    "if", "then", "else", "elif", "do", "while", "until", "!", "time", "{", "exec", "eval",
];

struct WordAcc {
    start: usize,
    end: usize,
    line: u32,
    col: u32,
    text: String,
    tainted: bool,
    cmd_pos: bool,
}

struct Scanner<'a> {
    src: &'a [u8],
    i: usize,
    line: u32,
    line_start: usize,
    tokens: Vec<Token>,
    pragmas: Vec<(u32, Pragma)>,
    /// Heredocs registered on the current line: `(delimiter, strip_tabs)`.
    pending: Vec<(String, bool)>,
    cmd_pos: bool,
    word: Option<WordAcc>,
}

impl<'a> Scanner<'a> {
    fn peek(&self, ahead: usize) -> Option<u8> {
        self.src.get(self.i + ahead).copied()
    }

    fn col(&self) -> u32 {
        (self.i - self.line_start + 1) as u32
    }

    /// Consume one byte, keeping line bookkeeping exact.
    fn bump(&mut self) {
        if self.src[self.i] == b'\n' {
            self.line += 1;
            self.line_start = self.i + 1;
        }
        self.i += 1;
    }

    fn ensure_word(&mut self) {
        if self.word.is_none() {
            self.word = Some(WordAcc {
                start: self.i,
                end: self.i,
                line: self.line,
                col: self.col(),
                text: String::new(),
                tainted: false,
                cmd_pos: self.cmd_pos,
            });
        }
    }

    /// Append `n` raw bytes to the current word.
    fn push_word_raw(&mut self, n: usize) {
        self.ensure_word();
        for _ in 0..n {
            if self.i >= self.src.len() {
                break;
            }
            let b = self.src[self.i];
            self.bump();
            if let Some(w) = self.word.as_mut() {
                w.text.push(b as char);
                w.end = self.i;
            }
        }
    }

    /// Consume `n` bytes into the word span without recording text
    /// (used for quoted content — the word is tainted anyway).
    fn swallow_into_word(&mut self, n: usize) {
        self.ensure_word();
        self.taint_word();
        for _ in 0..n {
            if self.i >= self.src.len() {
                break;
            }
            self.bump();
        }
        if let Some(w) = self.word.as_mut() {
            w.end = self.i;
        }
    }

    fn taint_word(&mut self) {
        if let Some(w) = self.word.as_mut() {
            w.tainted = true;
        }
    }

    fn flush_word(&mut self) {
        if let Some(w) = self.word.take() {
            if w.end > w.start {
                let keeper = CMD_KEEPERS.contains(&w.text.as_str());
                self.tokens.push(Token {
                    kind: TokKind::Word,
                    text: w.text,
                    offset: w.start,
                    len: w.end - w.start,
                    line: w.line,
                    col: w.col,
                    tainted: w.tainted,
                    cmd_pos: w.cmd_pos,
                });
                if !keeper {
                    self.cmd_pos = false;
                }
            }
        }
    }

    fn emit_here(&mut self, kind: TokKind, text: &str) {
        self.tokens.push(Token {
            kind,
            text: text.to_string(),
            offset: self.i,
            len: text.len(),
            line: self.line,
            col: self.col(),
            tainted: false,
            cmd_pos: false,
        });
        for _ in 0..text.len() {
            self.bump();
        }
    }

    /// Main scanner. `term == Some(b')')` when inside a `$( … )` substitution.
    fn scan_code(&mut self, term: Option<u8>) {
        let mut depth = 0usize;
        while self.i < self.src.len() {
            let c = self.src[self.i];
            match c {
                b' ' | b'\t' | b'\r' => {
                    self.flush_word();
                    self.bump();
                }
                b'\n' => {
                    self.flush_word();
                    self.emit_here(TokKind::Sep, "\n");
                    self.consume_heredocs();
                    self.cmd_pos = true;
                }
                b'\\' => {
                    if self.peek(1) == Some(b'\n') {
                        // Line continuation: the word carries on.
                        self.bump();
                        self.bump();
                    } else {
                        // Escaped char: part of the word, never mutable.
                        self.ensure_word();
                        self.taint_word();
                        self.push_word_raw(2);
                    }
                }
                b'#' if self.word.is_none() => self.scan_comment(),
                b'\'' => self.scan_single_quote(),
                b'"' => self.scan_double_quote(),
                b'`' => {
                    self.ensure_word();
                    self.taint_word();
                    self.skip_backtick();
                    if let Some(w) = self.word.as_mut() {
                        w.end = self.i;
                    }
                }
                b'$' => {
                    if self.peek(1) == Some(b'(') && self.peek(2) == Some(b'(') {
                        self.swallow_into_word(3);
                        self.scan_arith();
                        if let Some(w) = self.word.as_mut() {
                            w.end = self.i;
                        }
                    } else if self.peek(1) == Some(b'(') {
                        self.swallow_into_word(2);
                        // Scan the substitution body as real code (recursively),
                        // stashing the partially-built outer word.
                        let saved_word = self.word.take();
                        let saved_cmd = self.cmd_pos;
                        self.cmd_pos = true;
                        self.scan_code(Some(b')'));
                        self.word = saved_word;
                        self.cmd_pos = saved_cmd;
                        if let Some(w) = self.word.as_mut() {
                            w.end = self.i;
                        }
                    } else {
                        self.ensure_word();
                        self.taint_word();
                        self.push_word_raw(1);
                    }
                }
                b'(' => {
                    if self.word.is_none() && self.peek(1) == Some(b'(') {
                        // `(( … ))` arithmetic command.
                        self.bump();
                        self.bump();
                        self.scan_arith();
                        self.cmd_pos = false;
                    } else {
                        self.flush_word();
                        self.emit_here(TokKind::Sep, "(");
                        depth += 1;
                        self.cmd_pos = true;
                    }
                }
                b')' => {
                    self.flush_word();
                    if depth == 0 && term == Some(b')') {
                        self.bump();
                        return;
                    }
                    depth = depth.saturating_sub(1);
                    self.emit_here(TokKind::Sep, ")");
                    self.cmd_pos = true;
                }
                b';' => {
                    self.flush_word();
                    if self.peek(1) == Some(b';') {
                        self.emit_here(TokKind::Sep, ";;");
                    } else {
                        self.emit_here(TokKind::Sep, ";");
                    }
                    self.cmd_pos = true;
                }
                b'&' => {
                    self.flush_word();
                    if self.peek(1) == Some(b'&') {
                        self.emit_here(TokKind::AndAnd, "&&");
                    } else {
                        self.emit_here(TokKind::Sep, "&");
                    }
                    self.cmd_pos = true;
                }
                b'|' => {
                    self.flush_word();
                    if self.peek(1) == Some(b'|') {
                        self.emit_here(TokKind::OrOr, "||");
                    } else {
                        self.emit_here(TokKind::Sep, "|");
                    }
                    self.cmd_pos = true;
                }
                b'<' => {
                    self.flush_word();
                    if self.peek(1) == Some(b'<') && self.peek(2) == Some(b'<') {
                        // Herestring: the following word is data, keep scanning.
                        self.bump();
                        self.bump();
                        self.bump();
                    } else if self.peek(1) == Some(b'<') {
                        self.bump();
                        self.bump();
                        self.register_heredoc();
                    } else {
                        self.emit_here(TokKind::Sep, "<");
                    }
                }
                b'>' => {
                    self.flush_word();
                    self.emit_here(TokKind::Sep, ">");
                    if self.peek(0) == Some(b'>') {
                        self.bump(); // `>>` append: consume the second `>`
                    }
                }
                _ => self.push_word_raw(1),
            }
        }
        self.flush_word();
    }

    /// `#` comment to end of line; records `# mutash: …` pragmas.
    fn scan_comment(&mut self) {
        let start_line = self.line;
        let mut text = String::new();
        while self.i < self.src.len() && self.src[self.i] != b'\n' {
            text.push(self.src[self.i] as char);
            self.bump();
        }
        let body = text.trim_start_matches('#').trim();
        if let Some(rest) = body.strip_prefix("mutash:") {
            match rest.trim() {
                "off" => self.pragmas.push((start_line, Pragma::Off)),
                "on" => self.pragmas.push((start_line, Pragma::On)),
                "skip" => self.pragmas.push((start_line, Pragma::SkipLine)),
                _ => {}
            }
        }
    }

    fn scan_single_quote(&mut self) {
        self.ensure_word();
        self.taint_word();
        self.bump(); // opening '
        while self.i < self.src.len() && self.src[self.i] != b'\'' {
            self.bump();
        }
        if self.i < self.src.len() {
            self.bump(); // closing '
        }
        if let Some(w) = self.word.as_mut() {
            w.end = self.i;
        }
    }

    fn scan_double_quote(&mut self) {
        self.ensure_word();
        self.taint_word();
        self.bump(); // opening "
        while self.i < self.src.len() {
            match self.src[self.i] {
                b'\\' => {
                    self.bump();
                    if self.i < self.src.len() {
                        self.bump();
                    }
                }
                b'"' => {
                    self.bump();
                    break;
                }
                b'`' => self.skip_backtick(),
                b'$' if self.peek(1) == Some(b'(') => self.skip_cmdsubst(),
                _ => self.bump(),
            }
        }
        if let Some(w) = self.word.as_mut() {
            w.end = self.i;
        }
    }

    /// Skip a balanced `$( … )` (or `$(( … ))`) without emitting tokens.
    /// Used inside double quotes, where mutation is off-limits.
    fn skip_cmdsubst(&mut self) {
        self.bump(); // $
        self.bump(); // (
        let mut depth = 1usize;
        while self.i < self.src.len() && depth > 0 {
            match self.src[self.i] {
                b'(' => {
                    depth += 1;
                    self.bump();
                }
                b')' => {
                    depth -= 1;
                    self.bump();
                }
                b'\\' => {
                    self.bump();
                    if self.i < self.src.len() {
                        self.bump();
                    }
                }
                b'\'' => {
                    self.bump();
                    while self.i < self.src.len() && self.src[self.i] != b'\'' {
                        self.bump();
                    }
                    if self.i < self.src.len() {
                        self.bump();
                    }
                }
                b'"' => {
                    self.bump();
                    while self.i < self.src.len() {
                        match self.src[self.i] {
                            b'\\' => {
                                self.bump();
                                if self.i < self.src.len() {
                                    self.bump();
                                }
                            }
                            b'"' => {
                                self.bump();
                                break;
                            }
                            b'$' if self.peek(1) == Some(b'(') => self.skip_cmdsubst(),
                            _ => self.bump(),
                        }
                    }
                }
                b'`' => self.skip_backtick(),
                _ => self.bump(),
            }
        }
    }

    fn skip_backtick(&mut self) {
        self.bump(); // opening `
        while self.i < self.src.len() {
            match self.src[self.i] {
                b'\\' => {
                    self.bump();
                    if self.i < self.src.len() {
                        self.bump();
                    }
                }
                b'`' => {
                    self.bump();
                    return;
                }
                _ => self.bump(),
            }
        }
    }

    /// After consuming `<<`, read the (possibly quoted) delimiter word.
    fn register_heredoc(&mut self) {
        let strip = if self.peek(0) == Some(b'-') {
            self.bump();
            true
        } else {
            false
        };
        while matches!(self.peek(0), Some(b' ') | Some(b'\t')) {
            self.bump();
        }
        let mut delim = String::new();
        while let Some(b) = self.peek(0) {
            match b {
                b' ' | b'\t' | b'\n' | b';' | b'&' | b'|' | b'<' | b'>' | b'(' | b')' => break,
                b'\'' => {
                    self.bump();
                    while self.i < self.src.len() && self.src[self.i] != b'\'' {
                        delim.push(self.src[self.i] as char);
                        self.bump();
                    }
                    if self.i < self.src.len() {
                        self.bump();
                    }
                }
                b'"' => {
                    self.bump();
                    while self.i < self.src.len() && self.src[self.i] != b'"' {
                        delim.push(self.src[self.i] as char);
                        self.bump();
                    }
                    if self.i < self.src.len() {
                        self.bump();
                    }
                }
                b'\\' => {
                    self.bump();
                    if self.i < self.src.len() {
                        delim.push(self.src[self.i] as char);
                        self.bump();
                    }
                }
                _ => {
                    delim.push(b as char);
                    self.bump();
                }
            }
        }
        if !delim.is_empty() {
            self.pending.push((delim, strip));
        }
    }

    /// At the newline that ends a command line: skip every pending heredoc body.
    fn consume_heredocs(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        for (delim, strip) in pending {
            loop {
                if self.i >= self.src.len() {
                    return;
                }
                let start = self.i;
                let mut j = self.i;
                while j < self.src.len() && self.src[j] != b'\n' {
                    j += 1;
                }
                let mut line = &self.src[start..j];
                if strip {
                    while let Some((b'\t', rest)) = line.split_first() {
                        line = rest;
                    }
                }
                let matched = line == delim.as_bytes();
                while self.i < j {
                    self.bump();
                }
                if self.i < self.src.len() {
                    self.bump(); // the newline
                }
                if matched {
                    break;
                }
            }
        }
    }

    /// Inside `$(( … ))` / `(( … ))`: emit arithmetic operator and integer
    /// tokens, consume everything else, and stop at the matching `))`.
    fn scan_arith(&mut self) {
        let mut depth = 0usize;
        let mut prev_operand = false;
        while self.i < self.src.len() {
            let c = self.src[self.i];
            match c {
                b')' => {
                    if depth == 0 {
                        self.bump();
                        if self.peek(0) == Some(b')') {
                            self.bump();
                        }
                        return;
                    }
                    depth -= 1;
                    prev_operand = true;
                    self.bump();
                }
                b'(' => {
                    depth += 1;
                    prev_operand = false;
                    self.bump();
                }
                b'0'..=b'9' => {
                    let start = self.i;
                    let (line, col) = (self.line, self.col());
                    while matches!(self.peek(0), Some(b'0'..=b'9')) {
                        self.bump();
                    }
                    // `16#ff`, `0x1f`, `2abc` — base or malformed literals: skip.
                    if matches!(self.peek(0), Some(b) if b == b'#' || b.is_ascii_alphanumeric() || b == b'_')
                    {
                        while matches!(self.peek(0), Some(b) if b == b'#' || b.is_ascii_alphanumeric() || b == b'_')
                        {
                            self.bump();
                        }
                    } else {
                        let text: String =
                            self.src[start..self.i].iter().map(|&b| b as char).collect();
                        self.tokens.push(Token {
                            kind: TokKind::ArithNum,
                            len: text.len(),
                            text,
                            offset: start,
                            line,
                            col,
                            tainted: false,
                            cmd_pos: false,
                        });
                    }
                    prev_operand = true;
                }
                b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                    while matches!(self.peek(0), Some(b) if b.is_ascii_alphanumeric() || b == b'_')
                    {
                        self.bump();
                    }
                    prev_operand = true;
                }
                b'$' => {
                    self.bump();
                    if self.peek(0) == Some(b'{') {
                        while self.i < self.src.len() && self.src[self.i] != b'}' {
                            self.bump();
                        }
                        if self.i < self.src.len() {
                            self.bump();
                        }
                    } else if self.peek(0) == Some(b'(') {
                        // Re-borrowing `$(cmd)` inside arithmetic: skip it whole.
                        self.i -= 1;
                        self.skip_cmdsubst();
                    } else {
                        while matches!(self.peek(0), Some(b) if b.is_ascii_alphanumeric() || b == b'_' || b == b'#')
                        {
                            self.bump();
                        }
                    }
                    prev_operand = true;
                }
                b'\'' | b'"' => {
                    let quote = c;
                    self.bump();
                    while self.i < self.src.len() && self.src[self.i] != quote {
                        if self.src[self.i] == b'\\' {
                            self.bump();
                        }
                        if self.i < self.src.len() {
                            self.bump();
                        }
                    }
                    if self.i < self.src.len() {
                        self.bump();
                    }
                    prev_operand = true;
                }
                b' ' | b'\t' | b'\r' | b'\n' | b';' => self.bump(),
                b'#' => {
                    // Base separator (`16#ff`): swallow the digits that follow.
                    self.bump();
                    while matches!(self.peek(0), Some(b) if b.is_ascii_alphanumeric()) {
                        self.bump();
                    }
                    prev_operand = true;
                }
                _ => {
                    prev_operand = self.scan_arith_op(prev_operand);
                }
            }
        }
    }

    /// Match one arithmetic operator (longest first). Returns the new
    /// `prev_operand` state (always false: an operator expects an operand).
    fn scan_arith_op(&mut self, prev_operand: bool) -> bool {
        let rest = &self.src[self.i..];
        let starts = |p: &str| rest.starts_with(p.as_bytes());
        // Consume-only compound operators first so `<<=` is not seen as `<` `<=`.
        for p in [
            "<<=", ">>=", "**=", "<<", ">>", "**", "&=", "|=", "^=", "&&", "||",
        ] {
            if starts(p) {
                for _ in 0..p.len() {
                    self.bump();
                }
                return false;
            }
        }
        // Mutable compound operators.
        for p in [
            "==", "!=", "<=", ">=", "++", "--", "+=", "-=", "*=", "/=", "%=",
        ] {
            if starts(p) {
                let emit = !matches!(p, "*=" | "/=" | "%=");
                if emit {
                    self.emit_here(TokKind::ArithOp, p);
                } else {
                    for _ in 0..p.len() {
                        self.bump();
                    }
                }
                return false;
            }
        }
        let c = self.src[self.i];
        match c {
            b'+' | b'-' if !prev_operand => {
                // Unary sign: not a mutation target.
                self.bump();
            }
            b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' => {
                let text = (c as char).to_string();
                self.emit_here(TokKind::ArithOp, &text);
            }
            _ => self.bump(), // = ! & | ^ ~ ? : , and anything else
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(src: &str) -> Vec<String> {
        scan(src)
            .tokens
            .into_iter()
            .filter(|t| t.kind == TokKind::Word)
            .map(|t| t.text)
            .collect()
    }

    fn kinds(src: &str) -> Vec<(TokKind, String)> {
        scan(src)
            .tokens
            .into_iter()
            .map(|t| (t.kind, t.text))
            .collect()
    }

    #[test]
    fn splits_words_and_tracks_positions_across_lines() {
        let r = scan("echo hello\nbb ccc\n");
        let t: Vec<_> = r
            .tokens
            .iter()
            .filter(|t| t.kind == TokKind::Word)
            .collect();
        assert_eq!(t.len(), 4);
        assert_eq!(t[1].text, "hello");
        assert_eq!((t[1].offset, t[1].line, t[1].col), (5, 1, 6));
        assert_eq!((t[2].line, t[2].col), (2, 1));
        assert_eq!((t[3].line, t[3].col), (2, 4));
    }

    #[test]
    fn comment_detection_matches_shell_rules() {
        // A real comment hides everything to end of line…
        assert_eq!(words("echo hi # rm -rf / && true"), vec!["echo", "hi"]);
        // …but `$#` (argc), `${#a}` (length) and mid-word `#` are not comments.
        assert_eq!(words("echo $# after").last().unwrap(), "after");
        assert_eq!(words("echo ${#a} after").last().unwrap(), "after");
        assert_eq!(words("echo a#b after"), vec!["echo", "a#b", "after"]);
    }

    #[test]
    fn quoted_text_is_tainted_and_opaque() {
        // Single quotes: one word, tainted, nothing inside leaks out.
        let r = scan("echo '-f && exit 1'");
        let w: Vec<_> = r
            .tokens
            .iter()
            .filter(|t| t.kind == TokKind::Word)
            .collect();
        assert_eq!(w.len(), 2, "quoted string is one word: {:?}", w);
        assert!(w[1].tainted);
        assert!(!r.tokens.iter().any(|t| t.kind == TokKind::AndAnd));
        // Double quotes behave the same.
        let r = scan("echo \"a && b -eq c\"");
        assert!(!r.tokens.iter().any(|t| t.kind == TokKind::AndAnd));
        assert!(r
            .tokens
            .iter()
            .any(|t| t.kind == TokKind::Word && t.tainted));
    }

    #[test]
    fn nested_quotes_in_command_substitution_inside_double_quotes() {
        // The inner `"bar"` must not terminate the outer double quote.
        let r = scan("x=\"$(foo \"bar\")\" after");
        let w: Vec<_> = r
            .tokens
            .iter()
            .filter(|t| t.kind == TokKind::Word)
            .collect();
        assert_eq!(w.last().unwrap().text, "after");
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn command_substitution_body_is_scanned_for_tokens() {
        // `&&` inside an unquoted $( ) is live code and must be visible.
        let r = scan("out=$(a && b)");
        assert!(r.tokens.iter().any(|t| t.kind == TokKind::AndAnd));
        let w: Vec<_> = r
            .tokens
            .iter()
            .filter(|t| t.kind == TokKind::Word)
            .collect();
        assert!(w.iter().any(|t| t.text == "a" && t.cmd_pos));
    }

    #[test]
    fn word_resumes_after_command_substitution() {
        let r = scan("f=$(date).log next");
        let w: Vec<_> = r
            .tokens
            .iter()
            .filter(|t| t.kind == TokKind::Word)
            .collect();
        assert_eq!(w.last().unwrap().text, "next");
        // The assignment word is tainted (contains an expansion).
        assert!(w.iter().any(|t| t.tainted));
    }

    #[test]
    fn backtick_body_is_skipped() {
        let r = scan("x=`a && b` after");
        assert!(!r.tokens.iter().any(|t| t.kind == TokKind::AndAnd));
        let w: Vec<_> = r
            .tokens
            .iter()
            .filter(|t| t.kind == TokKind::Word)
            .collect();
        assert_eq!(w.last().unwrap().text, "after");
    }

    #[test]
    fn heredoc_body_is_skipped() {
        let src = "cat <<EOF\n[ 1 -eq 1 ] && true\nEOF\necho after\n";
        let w = words(src);
        assert_eq!(w, vec!["cat", "echo", "after"]);
    }

    #[test]
    fn heredoc_variants_dash_quoted_and_stacked() {
        // `<<-` strips leading tabs before matching the delimiter.
        let src = "cat <<-END\n\tbody && text\n\tEND\necho after\n";
        assert_eq!(words(src), vec!["cat", "echo", "after"]);
        // Quoted delimiters are unquoted before matching.
        let src = "cat <<'EOF'\n$x && $y\nEOF\necho after\n";
        assert_eq!(words(src), vec!["cat", "echo", "after"]);
        // Two heredocs registered on one line are consumed in order.
        let src = "diff <(cat <<A\n1 -eq 1\nA\n) - <<B\n2 -ne 2\nB\necho after\n";
        let w = words(src);
        assert_eq!(w.last().unwrap(), "after");
        assert!(!w.iter().any(|t| t == "-eq" || t == "-ne"));
    }

    #[test]
    fn herestring_is_not_a_heredoc() {
        let w = words("cmd <<< data\necho after\n");
        assert_eq!(w, vec!["cmd", "data", "echo", "after"]);
    }

    #[test]
    fn escapes_taint_words_and_continuations_keep_line_numbers() {
        let r = scan("echo \\-f");
        let w: Vec<_> = r
            .tokens
            .iter()
            .filter(|t| t.kind == TokKind::Word)
            .collect();
        assert!(w[1].tainted, "escaped chars must taint the word");
        let r = scan("echo a \\\nb\necho c\n");
        let c = r
            .tokens
            .iter()
            .find(|t| t.kind == TokKind::Word && t.text == "c")
            .unwrap();
        assert_eq!(
            c.line, 3,
            "a `\\` line continuation still counts its newline"
        );
    }

    #[test]
    fn command_position_tracking() {
        let r = scan("if grep -q x f; then echo y && ls; fi\n");
        let pos: Vec<(String, bool)> = r
            .tokens
            .iter()
            .filter(|t| t.kind == TokKind::Word)
            .map(|t| (t.text.clone(), t.cmd_pos))
            .collect();
        let get = |name: &str| pos.iter().find(|(t, _)| t == name).unwrap().1;
        assert!(get("grep"), "word after `if` is a command");
        assert!(get("echo"), "word after `then` is a command");
        assert!(get("ls"), "word after `&&` is a command");
        assert!(!get("-q"), "flag is not in command position");
        assert!(!get("y"), "argument is not in command position");
    }

    #[test]
    fn and_or_separators_are_distinct_tokens() {
        let k = kinds("a && b || c; d | e & f");
        assert!(k.contains(&(TokKind::AndAnd, "&&".into())));
        assert!(k.contains(&(TokKind::OrOr, "||".into())));
        assert!(k.contains(&(TokKind::Sep, ";".into())));
        assert!(k.contains(&(TokKind::Sep, "|".into())));
        assert!(k.contains(&(TokKind::Sep, "&".into())));
    }

    #[test]
    fn redirection_angles_are_seps_not_words() {
        let r = scan("sort < in > out");
        let seps: Vec<_> = r
            .tokens
            .iter()
            .filter(|t| t.kind == TokKind::Sep && (t.text == "<" || t.text == ">"))
            .collect();
        assert_eq!(seps.len(), 2);
        assert_eq!(words("sort < in > out"), vec!["sort", "in", "out"]);
    }

    fn arith_ops(src: &str) -> Vec<String> {
        scan(src)
            .tokens
            .into_iter()
            .filter(|t| t.kind == TokKind::ArithOp)
            .map(|t| t.text)
            .collect()
    }

    fn arith_nums(src: &str) -> Vec<String> {
        scan(src)
            .tokens
            .into_iter()
            .filter(|t| t.kind == TokKind::ArithNum)
            .map(|t| t.text)
            .collect()
    }

    #[test]
    fn arithmetic_ops_and_numbers_are_tokenized() {
        assert_eq!(arith_ops("x=$((a + 3 * b))"), vec!["+", "*"]);
        assert_eq!(arith_nums("x=$((a + 3 * b))"), vec!["3"]);
        // Nested parens do not end the expression early.
        assert_eq!(arith_ops("x=$(( (a + b) * 2 )) after=1"), vec!["+", "*"]);
        let w = words("x=$(( (a + b) * 2 )) after=1");
        assert_eq!(w.last().unwrap(), "after=1");
    }

    #[test]
    fn arith_signs_and_base_literals_are_not_mutation_targets() {
        // A leading `-` is a sign, not a binary operator.
        assert_eq!(arith_ops("x=$(( -3 + y ))"), vec!["+"]);
        // `16#ff` is a base literal, not an integer to nudge.
        assert_eq!(arith_nums("x=$(( 16#ff + 2 ))"), vec!["2"]);
    }

    #[test]
    fn arith_compound_operators_are_matched_longest_first() {
        let ops = arith_ops("x=$(( a <= b )) y=$(( c << 2 ))");
        assert_eq!(
            ops,
            vec!["<="],
            "`<<` shift is consumed, not split into `<` `<`"
        );
    }

    #[test]
    fn arith_command_form_for_loop() {
        let src = "for ((i=0; i<n; i++)); do :; done\n";
        assert_eq!(arith_ops(src), vec!["<", "++"]);
        assert_eq!(arith_nums(src), vec!["0"]);
    }

    #[test]
    fn pragmas_are_recorded_with_line_numbers() {
        let src = "# mutash: off\na\n# mutash: on\nb # mutash: skip\n";
        let r = scan(src);
        assert_eq!(
            r.pragmas,
            vec![(1, Pragma::Off), (3, Pragma::On), (4, Pragma::SkipLine)]
        );
    }

    #[test]
    fn degenerate_sources_scan_cleanly() {
        assert_eq!(words("#!/usr/bin/env bash\necho hi\n"), vec!["echo", "hi"]);
        assert!(scan("").tokens.is_empty());
        let r = scan("   \n\t\n");
        assert!(r.tokens.iter().all(|t| t.kind == TokKind::Sep));
    }
}
