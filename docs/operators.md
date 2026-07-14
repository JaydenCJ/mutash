# Mutation operator reference

mutash mutates at the token level: it never builds a bash AST. A
shell-aware scanner finds the live tokens (skipping comments, quoted
strings, heredoc bodies and escapes) and tags the context each token sits
in — plain command words, `[` / `[[` / `test` expressions, or `$(( ))` /
`(( ))` arithmetic. Every operator below is gated on that context, which is
what keeps mutants syntactically valid without an interpreter in the loop.

Run `mutash ops` for the compact version of this table.

## `compare` — comparison operators

| Original | Mutant | Context |
|---|---|---|
| `-eq` / `-ne` | `-ne` / `-eq` | `[ ]`, `[[ ]]`, `test` |
| `-lt` / `-le` | `-le` / `-lt` | `[ ]`, `[[ ]]`, `test` |
| `-gt` / `-ge` | `-ge` / `-gt` | `[ ]`, `[[ ]]`, `test` |
| `=` / `==` | `!=` | `[ ]`, `[[ ]]`, `test` |
| `!=` | `==` | `[ ]`, `[[ ]]`, `test` |
| `<` / `>` | `>` / `<` | `[[ ]]` only (elsewhere they are redirections) |
| `<` / `<=` / `>` / `>=` / `==` / `!=` | boundary/negation partner | `$(( ))`, `(( ))` |

The ordered pairs are deliberately *boundary* mutants: `-lt` becomes `-le`,
not `-gt`. A suite kills them only if it tests the exact boundary value —
which is where real off-by-one bugs live.

## `unary` — unary test operators

| Original | Mutant | Meaning drift |
|---|---|---|
| `-z` / `-n` | `-n` / `-z` | empty ↔ non-empty |
| `-f` / `-d` | `-d` / `-f` | regular file ↔ directory |
| `-e` | `-d` | exists → is a directory |
| `-r` / `-w` | `-w` / `-r` | readable ↔ writable |
| `-x` | `-r` | executable → readable |
| `-s` | `-f` | non-empty file → any regular file |

## `connective` — logical connectives

| Original | Mutant | Context |
|---|---|---|
| `&&` | `\|\|` | command lists and `[[ ]]` |
| `\|\|` | `&&` | command lists and `[[ ]]` |
| `-a` / `-o` | `-o` / `-a` | `[ ]` only (outside a test they are ordinary flags) |

## `arith` — arithmetic operators

| Original | Mutant |
|---|---|
| `+` / `-` | `-` / `+` (binary only; unary signs are left alone) |
| `*` / `/` | `/` / `*` |
| `%` | `*` |
| `++` / `--` | `--` / `++` |
| `+=` / `-=` | `-=` / `+=` |

Applied inside `$(( ))` and `(( ))` only. Shifts (`<<`, `>>`), bitwise
operators and base literals (`16#ff`) are recognized and left alone.

## `number` — integer literals

`n` becomes `n+1` and `n-1` — but only inside test expressions and
arithmetic, where the number is a *decision*. A bare `head -n 5` argument
is data and is not touched.

## `exit` — exit and return statuses

`exit 0` → `exit 1`; any non-zero `exit N` / `return N` → `0`. These are
the mutants that catch suites which never assert on exit codes — the single
most common gap in shell test suites.

## `flag` — command flags

| Original | Mutants |
|---|---|
| `-q` | drop the flag |
| `-rf` | drop the flag; shrink the cluster to `-r` |
| `--force`, `--depth=1` | drop the flag |

Only in argument position: a word starting with `-` inside `[ ]` is a test
operator, a leading `-N` number is data, and both are excluded.

## `negate` — negation removal

`! cmd` and `[[ ! -f x ]]` lose the `!`. Survivors here mean the suite
never exercises the negative path.

## `truth` — builtin truth values

`true` ↔ `false`, in command position only (`echo true` is data).

## Suppressing mutants

Some lines are noise by design (cleanup traps, logging). Silence them in
the source rather than in every invocation:

```bash
rm -rf "$TMPDIR"            # mutash: skip

# mutash: off
log_debug "state: $state"
log_debug "attempt: $attempt"
# mutash: on
```

`--only <ids>` and `--skip <ids>` select operators per invocation; pragmas
select lines permanently. Both compose.
