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

…you get nothing, and it looks like `check` found no secrets. It found them
fine — the text just went somewhere `grep` wasn't looking.

### The bug

Every line `check` emits goes to stderr. stdout is completely empty. Captured
against `main` at `dfa4b10`, with a two-secret manifest and a `dotenv` provider:

```console
$ secretspec check --reason "deploy preflight" 2>/dev/null   # stdout only
[exit 0]

$ secretspec check --reason "deploy preflight" | grep DATABASE_URL
[grep exit 1]
```

Both silent. The report is there — it's on stderr — so the exit status says
"all good" while every pipeline sees an empty report.

Source, on `main` at `dfa4b10` (`secretspec/src/secrets.rs`):

- `check()` — the header, line 3678
- `display_validation_success()` — every status line and the summary, 3701
- `display_validation_errors()` — same for the failure path, plus constraint
  violations, 3731

11 `eprintln!` calls across the three.

With them writing to stdout instead, same manifest, same commands:

```console
$ secretspec check --reason "deploy preflight" 2>/dev/null   # stdout only
Checking secrets in demo (profile: default)...

✓ DATABASE_URL - app database
○ SENTRY_DSN - error reporting (optional)

Summary: 1 found, 0 missing, 1 optional
[exit 0]

$ secretspec check --reason "deploy preflight" | grep DATABASE_URL
✓ DATABASE_URL - app database
[grep exit 0]
```

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

`eprintln!` → stdout at the 11 call sites across those three functions. No
change to exit codes, no change to what is printed, no new dependency, no
colour rework. `ensure_secrets` is deliberately untouched — its output is
prompts and diagnostics, not report. `check --json` and `--explain` return
early at `cli/mod.rs:1443-1465`, before the header, so neither is affected.

**One thing worth doing at the same time, and the reason I'd not just swap the
macros.** Putting the report on stdout puts it on a stream a reader can close
early, and `println!` panics on EPIPE. Since Rust's stdout is a `LineWriter`,
each report line is its own write, so this needs no large output:

```console
$ secretspec check --reason r | head -1
Checking secrets in demo (profile: default)...
thread 'main' panicked at library/std/src/io/stdio.rs:1165:9:
failed printing to stdout: Broken pipe (os error 32)
[exit 101]
```

That pipeline exits 0 today, precisely because stdout gets nothing. So a naive
macro swap trades a silent-empty-report bug for a panic on `| head`.

My patch therefore writes the report through an injected sink, which is the
pattern the codebase already documents for this exact reason —
`write_export`'s doc comment says an injected sink "turns a broken pipe into a
returned error instead of a panic". `check` locks stdout once and passes
`&mut dyn io::Write` to both display helpers. A closed pipe then behaves as
`export | head` already does: a clean `IO error: Broken pipe`, exit 1.

Two related things I noticed while doing this, both pre-existing and neither
touched by my patch, flagging in case you want them: `check --json | head`
already panics the same way (`cli/mod.rs:1456`), and if you'd rather a broken
pipe exit 0 silently — as `git` and `ls` do — that's a different and slightly
larger change, since `export` would presumably want to match. Happy either way;
tell me which and I'll do it consistently across both.

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
- Transcripts above are REAL, captured 2026-08-16 from two binaries built from
  the merged tree: one with upstream's `secrets.rs` verbatim, one with the fix.
  The first-run `note: secretspec is now recording secret access to ...` stderr
  line was trimmed deliberately (it fires once per state dir and is noise here).
- The broken-pipe panic was found by review, not by me, AFTER I had already
  committed the naive swap and verified it with `| grep` — which reads to EOF
  and so cannot reproduce it. Reproduced 20/20 with `| head -1`. Fixed in
  `be901ba`; regression test confirmed to fail against the pre-fix commit.
- READY TO POST. Nothing outstanding.
