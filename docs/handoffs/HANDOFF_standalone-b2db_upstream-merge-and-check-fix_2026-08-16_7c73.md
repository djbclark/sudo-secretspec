---
schema_version: 1
handoff_id: 7c73
parent_handoff_ids: [3c9d]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 7ff973153af6af9dcefbf4c496107e1630a9a045
created_at: 2026-08-16T21:42:35-0400
writer: claude-code
---

# Handoff — upstream merge executed, `check` stdout fix shipped and twice reviewed

## The Goal

Execute the upstream realignment that `3c9d` had researched but not started.
Upstream closed PRs #356 and #357 sixty seconds apart in favour of the merged
#334, stating the public API will be `Spec` rather than the internal `Config`.
Operator directive, carried forward from `3c9d`: **do not push back on
upstream's direction — reshape our code to match.**

Concretely: merge upstream, fix the `check` stdout/stderr bug, get both
reviewed, file the owed upstream issues, and cut `0.19.1-sudo.15`.

## Where We Are

Branch `sudo-main` at `7ff9731`, **tree clean, everything pushed**. Six commits
this session:

| SHA | What |
|-----|------|
| `92eee84` | Merge upstream `dfa4b10` — 87 commits, 129 files, 6 conflicts |
| `8c177e4` | `check` report → stdout (11 `eprintln!` → `println!`) + regression test |
| `bde608d` | Upstream issues #370 and #371 filed; contact ledger rewritten |
| `f1a615b` | Repair the post-install gate that `8c177e4` broke |
| `be901ba` | Write the report through a sink so a closed pipe cannot panic |
| `7ff9731` | Finish the upstream check-stdout draft (real transcript + EPIPE section) |

Done: the merge, the `emit_schema` repair, the `check` fix, two independent
reviews with **all** findings remediated, and two upstream issues filed.

Not done: posting the check-stdout issue + PR, the `Spec`-shaped manifest-edit
reference implementation, and the `0.19.1-sudo.15` release.

**Blockers: none.** Both operator decisions that `3c9d` was waiting on were
answered this session (post #370 as drafted; keep #371 separate), and neither
remaining work item is gated on anything external.

### Files changed by hand this session

Beyond the 129 files the merge brought in mechanically, these were authored or
edited deliberately:

- `secretspec/src/secrets.rs` — the `check` fix; otherwise reverted to exact
  upstream parity
- `secretspec/tests/check_report_stream.rs` — new, 3 regression tests
- `secretspec/src/codegen.rs`, `secretspec/src/lib.rs`, `secretspec/Cargo.toml` —
  the `codegen-schema` feature
- `secretspec/src/manifest_edit.rs`, `secretspec/src/cli/mod.rs` — conflict
  resolution + two orphaned doc comments refiled
- `sudo-secretspec-cli/src/broker.rs`, `sudo-secretspec-cli/Cargo.toml` —
  `emit_schema` moved onto `Spec`
- `.github/workflows/test.yml` — took upstream's structure, kept the
  `sudo-secretspec-cli` exclusion and extended it to Clippy
- `CHANGELOG.md` — two `## [Unreleased]` sections merged, plus new entries
- `tests/sudo_postinstall/{conftest.py,test_postinstall.py,README.md}` — gate repair
- `skills/sudo-secretspec/SKILL.md` — corrected the now-backwards `check` pitfall
- `sudo-secretspec/UPSTREAM-CONTACT.md` and three files under
  `sudo-secretspec/drafts/`

## What We Tried

Chronological, including what failed — this is the section most expensive to
rediscover.

### 1. `__private` alone does not reach `schema::emit` (the recorded stopgap was wrong)

`3c9d` predicted `emit_schema` would break and recorded the fix as "go through
`__private`". That is **insufficient**, and believing it would waste a cycle:

- `build_ir` *is* exported from `__private::codegen` upstream — free to use.
- `schema::emit` is `pub(crate)` **and** `#[cfg(any(feature = "cli", test))]`,
  so no amount of `__private` reaches it, and enabling `cli` does not help
  either (it makes the module *exist*, still `pub(crate)`).

Resolved with a fork-local `codegen-schema` feature mirroring the existing
`manifest-edit` one. So the shape debt is a **patch on upstream internals**, not
merely use of a disclaimed surface. The ledger entry has been corrected to say
so.

### 2. Verifying the stdout fix with `| grep` — the verification that could not fail

`8c177e4` was verified live with `secretspec check | grep DATABASE_URL`, which
passed. That check is **structurally incapable** of catching the bug that was
actually introduced: `grep` reads to EOF, so it never closes the pipe early and
never triggers EPIPE. `| head -1` does, and panics.

Lesson worth carrying: when a change moves output onto a pipe, the verification
must include a reader that *exits early*, not merely one that reads.

### 3. Two doc comments orphaned by an earlier extraction

The merge surfaced two doc comments in `cli/mod.rs` left dangling by an earlier
`manifest_edit` extraction — one directly above a blank line (`empty line after
doc comment` clippy warning), one describing a function that had moved. Folded
back into the functions they document in `manifest_edit.rs`.

### 4. `cargo test` without `--no-fail-fast` hides the CLI crate

The first post-merge run stopped at the `secretspec` lib failure and never
reached `sudo-secretspec-cli`, so the CLI crate's tests appeared to have run
when they had not. Always pass `--no-fail-fast` here.

## Key Decisions

### Chosen: take upstream's exact text wherever our delta had already been merged

`provider/mod.rs` was the big conflict — upstream split it into nine modules.
Our only delta there was `supports_delete` (our PR #354), which upstream had
**already merged**. Verified present in `traits.rs:213/220/243-248`, the `Arc`
impl at `:722`, and `preflight.rs:166` *before* discarding our side. Result: the
file is byte-identical to upstream.

Eight provider backends (`dotenv`, `file`, `gopass`, `keeper`, `keyring`,
`openbao`, `pass`, `vault`) had `supports_delete` **twice** — git auto-merged
ours alongside upstream's merged copy of the same PR, producing E0201. Each
diverged from upstream by exactly those 6 lines, so all eight were reset to
upstream's text. That removed 8 future conflict sites at zero functional cost.

### Chosen: point the broker at `Spec`, not `Secrets::config()`

Rather than keep the fork's `#[doc(hidden)] pub fn config()`, the broker now
does `Spec::try_from(manifest.as_path())`. This let **`secretspec/src/secrets.rs`
revert to exact upstream parity**, retiring a permanent conflict site on a file
upstream edits often (7 upstream commits vs 1 fork commit since the merge base).
Keep it that way.

### Chosen: an injected sink, not a bare macro swap

Rejected `writeln!(...).ok()` (silently swallow EPIPE) in favour of an injected
`&mut dyn io::Write` propagating the error, because the codebase already
documents that exact rationale at `write_export`: *"Writing to an injected sink
(rather than `print!`) … turns a broken pipe into a returned error instead of a
panic."* Matching an existing in-repo pattern is far easier to defend upstream
than inventing a second convention. Consequence: `check | head` behaves exactly
like `export | head` does today — clean `IO error: Broken pipe`, exit 1.

Rejected making broken-pipe exit 0 silently (as `git`/`ls` do): defensible, but
it would want `export` changed to match, which is scope creep on a focused fix.
Raised as an open question in the draft instead.

### Chosen: two upstream issues, not one

Per explicit operator decision this session: post the `Spec::to_toml()` ask as
drafted, and keep the `Spec::schema_json(profile)` ask **separate** so the
focused one is not diluted. Filed as **#370** and **#371**.

### Rejected: the draft's "check is the outlier within secretspec" claim

Overstated, and a maintainer grepping `secrets.rs` would have found it wrong in
thirty seconds — `import`'s summary is on stderr too (`secrets.rs:4313/4320/4328`),
as are the set/generation confirmations. Replaced with a claim that cannot be
argued with, and named `import` honestly.

### Chosen (upgraded argument): `check` already contradicts itself

The strongest available argument, found by review and verified independently:
`check --json` → `println!` (`cli/mod.rs:1507`) and `check --explain` → `print!`
(`:1509`) **already write to stdout**. The same subcommand routes the same report
to different streams depending on a flag. This is now the draft's lead.

## Evidence & Data

### Merge

87 commits, 129 files, +14550/−2981. Six conflicts: `.github/workflows/test.yml`,
`CHANGELOG.md`, `secretspec/Cargo.toml`, `secretspec/src/cli/mod.rs`,
`secretspec/src/provider/mod.rs`, `secretspec/src/provider/tests.rs`.

CHANGELOG merge verified lossless programmatically — both sides' bullets
extracted to sets and diffed: **0 fork bullets missing, 0 upstream bullets
missing.**

### Tests

| Run | Result |
|---|---|
| Baseline (pre-merge, from `3c9d`) | 1213 passed / 22 failed |
| Post-merge | 1583 passed / 21 failed |
| After the check fix | 1584 passed / 22 failed |
| Final (`7ff9731`) | **1586 passed / 21 failed** |

All failures are `provider::sops::*`, every one reporting `The 'sops' CLI is not
installed`. **`provider::vault_common::approle_get_many_refreshes_a_token_between_slow_waves`
is FLAKY** — run alone 3×, it gave FAILED, FAILED, ok. Its mock Vault loses a
race on an ephemeral port. It may or may not appear; it is not a regression.

Clippy: `cargo clippy -p secretspec --all-targets` = **16 warnings**, measured
identical to a `git worktree` of `upstream/main` (16 = 16). The fork adds none.
Compare against that worktree, never against zero.

### The EPIPE regression, measured

```console
$ secretspec check --reason r --no-prompt --provider … | head -1
Checking secrets in demo (profile: default)...
thread 'main' panicked at library/std/src/io/stdio.rs:1165:9:
failed printing to stdout: Broken pipe (os error 32)
[pipeline exit 101]
```

Reproduced 20/20 by the reviewer on a six-line report — Rust's stdout is a
`LineWriter`, so every line is its own write syscall. The same pipeline exited
**0** before `8c177e4`, because stdout received no bytes at all. After
`be901ba`: graceful `IO error: Broken pipe`, exit 1, no panic.

### The ANSI leak, proven not merely reasoned

Under a pty with stderr redirected, the **pre-fix** binary wrote raw escapes
into the log file:

```
Checking secrets in ^[[1mdemo^[[0m (profile: ^[[36mdefault^[[0m)...
```

Post-fix the same log is **0 bytes** and the report reaches redirected stdout
uncoloured. `colored 3.1.1` decides on ANSI by testing *stdout*
(`control.rs:108`), which is why routing the report to stdout fixes the colour
handling as a side effect rather than needing colour logic.

### Regression tests, each confirmed to fail without its fix

`secretspec/tests/check_report_stream.rs`, 3 tests:

- `a_passing_check_writes_its_whole_report_to_stdout`
- `a_failing_check_reports_on_stdout_but_errors_on_stderr`
- `a_reader_that_closes_the_pipe_early_does_not_panic` — `#[cfg(unix)]`, drives
  a real `sh`/`head` pipeline

Verified by reverting `secrets.rs` and re-running: the first two FAIL against
the pre-`8c177e4` tree; the third FAILS against `8c177e4` itself and passes
against `be901ba`.

### The silent-skip defect in our own gate

`f1a615b`'s worst finding was not the two failing assertions but a *silent* one:
`_check_report()` parsed stderr only, so post-change it returned an empty set,
which fed `resolved_name`, which calls `pytest.skip` — **quietly disabling four
value-reading tests** rather than failing. `resolved_name` now asserts the report
named *some* secret before concluding none resolved, so a parse failure can
never again masquerade as an empty vault.

### Upstream

- Filed **#370** — format-preserving single-declaration edits on `Spec`
- Filed **#371** — no library path from `Spec` to a JSON Schema
- **#64** remains the only open thread we don't own
- Re-checked all threads; no state changes since `3c9d`

Fact-checking #370's draft against `dfa4b10` before posting caught two errors:
`generate_toml_with_comments` is at `cli/mod.rs:480` (not `:479`), and
`SpecBuilder::build()` does not simply end at `from_config_document` — it also
sets `base_dir`. Everything else in the draft held.

## Operator Feedback

- **Reviews:** chose "two subagents now" over CodeRabbit or skipping. Vindicated
  — the two reviews found a shipped regression and a silently-degrading test
  gate that no amount of self-review had caught.
- **Upstream issues:** "approved, and keep separate."
- **Standing, from `3c9d` and unchanged:** follow upstream's stated direction
  rather than arguing it; re-check all upstream contact every session; do not
  delete `explore/pr-334-rust-first-spec` (commit `bf0b25c` is linked by SHA
  from a public comment on #357); item 12 (cross-platform sudo/Linux port) is
  DEFERRED — do not resurface.
- **Late in session:** wrap up with a handoff; headroom is fine, no need to rush.

## Where We're Going

1. **THE NEXT ACTION — post the check-stdout issue upstream.**
   `sudo-secretspec/drafts/upstream-check-stdout-issue.md` is marked
   **READY TO POST**; nothing is outstanding. Real transcripts captured, line
   numbers exact against `dfa4b10`, EPIPE section written, public-library-API
   objection pre-answered.

2. **Then open the upstream PR.** The patch is the `secrets.rs` +
   `secretspec/tests/check_report_stream.rs` hunks from `8c177e4`, `f1a615b` and
   `be901ba` combined.
   - **Do NOT cherry-pick the CHANGELOG hunk** — `git diff --stat upstream/main
     8c177e4~1 -- CHANGELOG.md` shows 458 fork-local insertions. Re-place the
     entry under upstream's own Unreleased by hand, and prefer **Changed** over
     **Fixed** (it changes observable output routing).
   - **Do NOT upstream** `tests/sudo_postinstall/*` or `skills/*` — fork-only.

3. **Reimplement manifest-edit on the `Spec` shape** — the reference
   implementation for #370. Design (already adjudicated in `3c9d`): `toml_edit`
   surgery on retained source text, then re-derive the semantic view by
   reparsing. Wrinkle recorded in #370's body: `Spec::from_toml` rejects
   non-empty `project.extends`, so the internal reparse needs an
   `extends`-aware variant seeded from `self.base_dir`, and the retained text
   must be the **root file only** (`Config::try_from` merges parents).

4. **Cut `0.19.1-sudo.15`.** Update `skills/sudo-secretspec/SKILL.md` and
   `sudo-secretspec/AI-GUIDANCE.md` **before** tagging — they ship inside the
   release, so a post-tag doc fix never reaches the installed copy (the trap the
   .12 release hit). SKILL.md's `check` pitfall was **already corrected** in
   `be901ba`; `AI-GUIDANCE.md` had no stdout/stderr mentions when grepped, but
   re-check.

5. **Note:** `tests/sudo_postinstall/test_postinstall.py::test_check_writes_its_report_to_stdout`
   **requires a boundary at .15+** and will fail against the installed .14 until
   the release is installed. That is intended.

6. **Optional hardening (task #11, not scoped this session):** wrap the broker's
   `execute()` in `catch_unwind`. There is no `catch_unwind` anywhere in
   `sudo-secretspec-cli` and the workspace does not set `panic = "abort"`, so
   **any** panic unwinds past the terminal audit event at `broker.rs:558` and
   strands an attempt in the protected ledger — a state the ledger's own docs
   call indistinguishable from a broker killed mid-operation. The EPIPE instance
   is fixed at source, but the class of hazard remains.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -6            # expect 7ff9731 at the tip, tree clean

# Standing directive — re-check upstream contact FIRST, every session:
cat sudo-secretspec/UPSTREAM-CONTACT.md      # then run the two gh search
                                             # commands in "How to re-check"
git fetch upstream main && git log --oneline -1 upstream/main   # was dfa4b10

# Tests (--no-fail-fast is REQUIRED or the CLI crate never runs):
cargo test --no-fail-fast -p secretspec -p secretspec-derive -p sudo-secretspec-cli
# expect ~1586 passed / 21 failed, all provider::sops::* (sops CLI absent);
# provider::vault_common::approle_get_many... is flaky and may add a 22nd.
# `cargo test --all` CANNOT run here: ext-php-rs needs a PHP toolchain.

# Clippy — compare against upstream, not zero:
cargo clippy -p secretspec --all-targets 2>&1 | grep -c '^warning: '   # 16

# THE NEXT ACTION — post the ready draft:
sed -n '1,12p' sudo-secretspec/drafts/upstream-check-stdout-issue.md
# extract body as bde608d did (after the title block's ---, before "Notes to self"), then:
#   gh issue create --repo cachix/secretspec --title '<Title line>' --body-file <body>
```
