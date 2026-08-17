# DRAFT — upstream bug report for cachix/secretspec
# Status: NOT POSTED. Gated on the 2-review process per operator.

## Title

`check` writes its entire report to stderr, so `secretspec check | grep ...` silently returns nothing

## Body

### ELI5

`secretspec check` prints a nice report — a header, a ✓/○/✗ line per secret, and a
summary. It looks like normal program output. But all of it is going out the
"error" pipe instead of the "normal" pipe.

So if you try to do the obvious thing:

```console
$ secretspec check | grep DATABASE_URL
$
```

…you get nothing, and it looks like `check` found no secrets. It found them
fine — the text just went somewhere `grep` wasn't looking.

### The bug

Every line `check` emits goes to stderr. stdout is completely empty.

```console
$ secretspec check 2>/dev/null          # stdout only — silent
$ secretspec check 2>&1 >/dev/null      # stderr only — the whole report
Checking secrets in demo (profile: default)...

✓ DATABASE_URL
○ SENTRY_DSN (optional)

Summary: 1 found, 0 missing, 1 optional
```

Source, on current `main` (`secretspec/src/secrets.rs`):

- `check()` — the header, ~line 3679
- `display_validation_success()` — every status line and the summary, ~3701
- `display_validation_errors()` — same for the failure path, plus constraint
  violations, ~3731

All use `eprintln!`.

### Why this is worth fixing beyond the piping annoyance

The colour handling is currently self-contradictory. `colored` decides whether
to emit ANSI escapes by testing **stdout**:

```rust
// colored-3.1.1/src/control.rs:108
&& io::stdout().is_terminal(),
```

But the report is written to **stderr**. The two streams are independent, so
both mistakes are reachable today:

- `secretspec check 2>log` with a terminal stdout → `colored` sees a TTY and
  emits escapes, which land as raw `\x1b[32m` bytes **inside the log file**.
- `secretspec check >file` with a terminal stderr → `colored` sees a pipe and
  strips colour from output the user is actually reading on their terminal.

Routing the report to stdout makes the existing TTY detection correct as a side
effect, rather than requiring any new colour logic.

### `check` already contradicts itself

The clearest evidence that this is an oversight rather than a decision is that
the *same subcommand* routes the *same report* to different streams depending
on a flag:

- `check --json` → `println!` (`cli/mod.rs:1507`) — stdout
- `check --explain` → `print!` (`cli/mod.rs:1509`) — stdout
- `check` → `eprintln!` (`secrets.rs:3678` and the two `display_validation_*`
  helpers) — stderr

So a caller can already pipe the machine-readable and the explain renderings,
but not the default human one. Whatever the right stream is, it should not
depend on which flag was passed.

More broadly, a command's primary output — the thing the user asked for — goes
to stdout, and stderr carries diagnostics, progress and errors. `export` writes
to stdout via `io::stdout()` (`cli/mod.rs:1466`), as do `get`, `schema` and
`completions`.

**In fairness, `check` is not the only stderr reporter.** `import`'s summary is
also on stderr (`secrets.rs:4313`, `:4320`, `:4328`), as are the interactive
set/generation confirmations. I've left `import` alone here because I don't want
to widen a focused fix, but if you consider its summary primary output too, I'm
happy to include it — or to be told the house rule is the opposite of what I've
assumed, in which case the consistent fix is to move `--json`/`--explain` to
stderr instead and I'll send that patch.

### Proposed fix

Mechanical: `eprintln!` → `println!` at the 11 call sites across those three
functions. No change to exit codes, no change to what is printed, no new
dependency, no colour rework. `ensure_secrets` is deliberately untouched — its
output is prompts and diagnostics, not report.

Open question for maintainers, happy to go either way:

- The constraint-violation lines in `display_validation_errors` are arguably
  diagnostics rather than report. I'd keep them on **stdout** with the rest of
  the report, on the grounds that splitting one human-readable report across two
  streams is the same class of bug — but say the word and I'll send them to
  stderr.

### Use cases this unblocks

1. **CI / automation.** `secretspec check | grep -q MISSING_KEY` and friends
   currently misreport. Today every wrapper has to know to add `2>&1`, which is
   non-obvious precisely because the command looks like it is printing normally.
2. **Logging with colour.** Capturing the report to a file while keeping a clean
   terminal is currently impossible without escape codes leaking into the file
   (see above).

### Library API

`Secrets::check()` is public (`secrets.rs:3674`) and appears in the SDK doc
examples, so this makes a *library* write to its consumer's stdout — a fair
objection, and arguably worse than writing to stderr, since a consumer emitting
JSON on stdout would have the report interleaved into it.

Two things make me think it's still right as proposed. The printing already
happens unconditionally today, just to the other stream, so no consumer is
currently spared it; and in practice the CLI is the only caller — the one
non-CLI caller I know of is my own downstream broker.

That said, if you'd rather not have a library print at all, the cleaner shape is
to move rendering into the CLI layer, or have `check` take an `impl Write`
(defaulting to `io::stdout()`) so an embedder can redirect it. I'm happy to do
either instead; say which and I'll send that patch.

### Compatibility

Anyone capturing via `2>&1` keeps working — that captures both streams. The
breaking case is a script capturing stderr *specifically* (`2>report.txt` with
stdout discarded).

I won't claim that's rare, because it caught me: my own downstream post-install
suite asserted the report was on stderr and asserted stdout was empty, and I had
to update three fixtures. Worth a `Changed` note rather than only a `Fixed` one,
and I'd expect a small number of wrappers to need the same edit.

I have a patch with tests and can open a PR immediately if the approach looks
right.

---

## Notes to self (NOT part of the issue)

- Verified the defect exists on upstream `main` independently of our fork:
  `git show upstream/main:secretspec/src/secrets.rs` has the same 37 `eprintln!`
  calls and the same three functions.
- `secrets.rs` is upstream-owned, and after the dfa4b10 merge our copy is
  **byte-identical to upstream**, so the patch applies cleanly. That is the
  strong form of the argument for upstreaming rather than patching locally.
- Line numbers CONFIRMED against dfa4b10 (2026-08-16): `display_validation_success`
  3701, `display_validation_errors` 3731, the `check()` header 3678.
  `export` → stdout at `cli/mod.rs:1466`. `check --json` → `cli/mod.rs:1507`,
  `--explain` → `:1509`. `import`'s stderr summary at `secrets.rs:4313/4320/4328`.
- Still need a real reproduction transcript from the merged tree before posting —
  do NOT post the illustrative block above as if it were captured output. Note
  the real output includes descriptions (`✓ DATABASE_URL - app database`) and a
  first-run `note: secretspec is now recording secret access to ...` line on
  stderr; include or trim that deliberately rather than by accident.
