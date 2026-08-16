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

### Convention

The usual split is that a command's primary output — the thing the user asked
for — goes to stdout, and stderr carries diagnostics, progress and errors. The
`check` report *is* the requested output. (This is also why `git status`,
`cargo tree`, `kubectl get` etc. are all pipeable.) Note `export` already writes
to stdout via `io::stdout()`, so `check` is the outlier within secretspec
itself.

### Proposed fix

Mechanical: `eprintln!` → `println!` at those three sites. No change to exit
codes, no change to what is printed, no new dependency, no colour rework.

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

### Compatibility

Anyone currently capturing via `2>&1` keeps working unchanged — that captures
both streams. The breaking case is a script that captures stderr *specifically*
(`2>report.txt` with stdout discarded); I'd expect that to be rare, and it is
arguably relying on the bug.

I have a patch with tests and can open a PR immediately if the approach looks
right.

---

## Notes to self (NOT part of the issue)

- Verified the defect exists on upstream `main` independently of our fork:
  `git show upstream/main:secretspec/src/secrets.rs` has the same 37 `eprintln!`
  calls and the same three functions.
- `secrets.rs` is upstream-owned: the fork has touched it once since the merge
  base (0a17ce0, unrelated), upstream 7 times. Fork-local-only fix = permanent
  conflict site. This is the argument for upstreaming rather than just patching.
- Need to confirm the exact upstream line numbers against dfa4b10 before posting
  (numbers above are from the pre-merge read; re-check after the merge lands).
- Need a real reproduction transcript from the merged tree before posting —
  do NOT post the illustrative block above as if it were captured output.
