---
schema_version: 1
handoff_id: 1446
parent_handoff_ids: [99a9]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: e9a01e4acdf883844a10321951258c730d0fad3a
created_at: 2026-08-18T14:12:52-0400
writer: claude-code
---

# Handoff — Finding 1 fixed, 0.19.1-sudo.21 released, upstream #373 review closed out

## The Goal

Resume from handoff `99a9` (review-slicing-and-six-fixes): fix the one
remaining ultra-review finding (the sudoers `--all` restore auth bypass),
release/install it, then check whether any upstream PRs/issues needed
attention.

## Where We Are

All three closed out. `sudo-main` is clean at `e9a01e4`, pushed. Boundary is
installed and healthy at `0.19.1-sudo.21`. Upstream PR `cachix/secretspec#373`
has been updated to address the maintainer's review and replied to.

## What We Tried

Nothing failed outright this session, but one judgment call was worth
recording because the wrong answer would have looked like a fix:

- **Considered, then rejected, changing broker.rs's overwrite-protection
  check.** The naive fix for finding 1 (route `--all` through the
  `source-restore-force` verb) also routes it past the `op ==
  "source-restore"` guard that refuses to overwrite a name still holding a
  live value — that guard is currently keyed off the *verb name*, not an
  actual `--force` flag. Before touching it, checked
  `docs/design/infinite-secret-backup-restore.md:387`: the design explicitly
  says `restore --all` is "Necessarily overwrites; never agent-callable."
  So skipping the check for `--all` is the *documented* behavior, not a
  regression — no broker.rs change was needed. Flagging this so a future
  session doesn't independently "fix" it into decoupling `broker.force` from
  the verb name without re-reading the design doc first.

## Key Decisions

- **Reused the existing `source-restore-force` verb for `--all`, not a new
  verb.** This is what parent handoff `99a9` explicitly prescribed ("mirror
  that for `--all`"), and it means zero sudoers policy text changes were
  needed — `source-restore-force` already has no NOPASSWD grant at all, by
  design (falls through to interactive Touch ID/password like `install`).
- **Extracted `restore_verb(force, all) -> &'static str` as its own function**
  in `main.rs` and unit-tested the four `(force, all)` combinations directly,
  rather than leaving the selection inlined in `lifecycle_restore`. Matches
  this codebase's established pattern (see `broker::mutates_vault`) for
  pulling an untestable trust decision out where a test can actually reach
  it — `lifecycle_restore` itself calls `Command::status()` and can't be
  unit-tested without a subprocess harness.
- **Declined an offered second ultra review** (operator asked mid-session:
  "We can do one more ultrareview if you think it would be super-useful").
  Judged it unnecessary for a single well-scoped defect already precisely
  diagnosed by the prior review and handoff; used local review + mutation
  testing instead, consistent with the operator's standing preference
  (recorded in `99a9`) to save the 2 remaining free ultra reviews for other
  projects.
- **Worked the upstream PR fix in a disposable worktree** (`/Users/djbclark/src/ss-373`
  on branch `fix/check-report-to-stdout`), not by switching `sudo-main`'s
  branch — same pattern as the existing `ss-370` worktree for PR #374.
  Removed after the push landed.
- **Did not touch `check`'s SIGPIPE behavior** (domenkozar's review point 3):
  explicitly deferred to a separate upstream PR per his own suggestion, and
  saved as a project memory so it isn't lost:
  `upstream-sigpipe-followup-pr.md` in this project's memory directory.

## Evidence & Data

**Finding 1 fix** (commit `7548caf`):
- `sudo-secretspec-cli` test count: 223 → 228 (4 new `restore_verb` unit
  tests + 1 new `sudoers_text` pinning test, `source_restore_force_has_no_nopasswd_grant`).
- `cargo test -p secretspec --features sqlite provider::sqlite`: 16, unchanged.
- `pytest tests/sudo_packaging -q`: 26, unchanged.
- Mutation-verified 2/2: reverting `if force || all` to `if force` in
  `restore_verb` was CAUGHT by `all_without_force_still_uses_the_auth_gated_verb`;
  adding a NOPASSWD grant for `source-restore-force` in `sudoers_text` was
  CAUGHT by `source_restore_force_has_no_nopasswd_grant` (had to hand-apply
  this second mutation once — the shared `mutate.sh` helper's Python
  string-replace pattern didn't match due to escaping, silently doing
  nothing and reporting a false MISSED; the direct hand-edit confirmed the
  test genuinely catches it).

**Release 0.19.1-sudo.21**:
- `just release-dry 0.19.1-sudo.21` then `just release 0.19.1-sudo.21` —
  exit 0 through tag/Release/formula/tap/brew test, no hand-finishing needed
  (unlike `.15`/`.18`). Commits `4bc0a27` (stamp), `e9a01e4` (Homebrew
  formula).
- Installed via `sudo "$(brew --prefix sudo-secretspec)/libexec/sudo-secretspec" install`:
  `0.19.1-sudo.20 -> 0.19.1-sudo.21`. Rollback snapshot
  `sudo-secretspec-rollback-1787075211`, 8 artifacts, 1 pruned.
- Post-install verification: `doctor: OK`; `GITHUB_TOKEN` retrieval 40 chars
  (matches baseline); `check` → `47 found, 0 missing, 7 optional`.

**Upstream `cachix/secretspec#373`** (commit `1b1783c` on
`fix/check-report-to-stdout`, pushed to `frdminc/sudo-secretspec`):
- `Secrets::check` signature: `check(&self, no_prompt: bool)` →
  `check(&self, no_prompt: bool, out: &mut dyn io::Write)`. Removed the
  internal `stdout().lock()` that was held across `validate()`'s
  `std::thread::scope` fan-out (`secrets.rs:5412`-ish) — matches
  `export`'s existing shape exactly.
- Call sites updated: `cli/mod.rs` (1, now passes `&mut std::io::stdout()`),
  8 doctest examples across `secrets.rs`, 5 unit test call sites in
  `tests.rs` (now pass `&mut Vec::new()`, matching `export`'s own test
  convention).
- `tests/check_report_stream.rs:181`: `env_path.display()` → `shell_quote(env_path.to_str().unwrap())`.
- `CHANGELOG.md`: "goes through a locked sink" → "goes through a sink
  rather than `println!`" (accurate again now that the lock isn't held
  internally).
- Verified on the `ss-373` worktree: `cargo check -p secretspec` clean;
  `cargo test -p secretspec --lib -- check` 24/24; `cargo test -p secretspec
  --doc` 13/13 (including `Secrets::check`); `cargo test -p secretspec
  --test check_report_stream` 3/3 (including the EPIPE/pipe-close test).
  Pre-existing, unrelated: 21 `sops`-provider test failures (the `sops` CLI
  isn't installed on this machine) and a full-workspace `cargo check
  --workspace` failure on `ext-php-rs`'s build script (no local `php`
  executable) — neither touches anything this session changed.
- Replied on the PR: https://github.com/cachix/secretspec/pull/373#issuecomment-5332171188

## Operator Feedback

- **"We can do one more ultrareview if you think it would be super-useful"**
  — offered mid-turn; declined, see Key Decisions.
- **"Have we reported or updated upstream issues as needed?"** — this
  question is *why* `#373`'s stale "zero maintainer engagement" note (from
  handoff `99a9`) got caught: it genuinely had a substantive review sitting
  unanswered since 2026-08-17T22:25:29Z. Re-verify upstream state at the
  start of any session touching this fork, per the standing
  `keep-up-with-upstream` memory — a session boundary is exactly when a
  note like that goes stale.
- **"Yes, go ahead with your plan."** — approved cutting and installing
  `.21`.
- **"Yes"** — approved pushing the `#373` fix and replying to the review.
- **"Be sure to remember 3. Go ahead and do a handoff."** — point 3 of
  domenkozar's review (the SIGPIPE follow-up) saved as a project memory
  (`upstream-sigpipe-followup-pr.md`) rather than left to rot in
  conversation history; this handoff is the second half of that ask.

## Where We're Going

1. **THE NEXT ACTION: the `capture_history` content-addressed blob
   migration.** Approved, not started. `secretspec/src/provider/sqlite.rs:422`,
   Hindsight page `kp-4591bfacb8f64679a468c647a5dc617f`. Baseline to verify
   against: 158 `captured_values` rows / 40 distinct `(item, digest)` pairs,
   7988 blob bytes vs 1997 deduped, 4 entries, 39 secrets. Consider folding
   in finding-5's functional gap from `99a9` (destroying copies of an
   already-source-deleted name) while the schema is open. **ANNOUNCE-FIRST
   — migrates the live vault.**
2. Watch `cachix/secretspec#373` for domenkozar's response to `1b1783c`; no
   action needed unless he replies with more feedback.
3. Open the SIGPIPE follow-up PR against `cachix/secretspec` whenever
   picked up — see memory `upstream-sigpipe-followup-pr.md` for the exact
   scope (restore default `SIGPIPE` disposition in the CLI entry point,
   unix-only via `libc`, already a transitive dependency; covers `check`,
   `check --json`, and `export` in one place). Check
   `gh pr list --repo cachix/secretspec --search "sigpipe"` first in case
   someone else already opened it.
4. `cachix/secretspec#374` (format-preserving edits) and `#372` (issue)
   still have zero maintainer engagement as of this session — nothing to
   action, just keep watching each time upstream state is re-checked.
5. Rebase `/Users/djbclark/src/ss-370` (`dfa4b10` → `35791a2`) before any
   upstream round on `#374`.
6. Tear down the stale, unused-this-session review worktree:
   `git worktree remove /Users/djbclark/src/sudo-secretspec-review && git branch -D review/base-core review/slice-core review/base-rest review/slice-rest`.
7. Standing don't-touch list: `PROMPT-REVIEW.md`, `PROMPT-SECREV.md`,
   `REVIEW_REPORT.md` (prior review artifacts, ask before removing);
   `/var/db/sudo-secretspec/.env` (last pre-migration reference copy of all
   39 values, confirm before removing); branch
   `explore/pr-334-rust-first-spec` (do not delete — `bf0b25c` is linked by
   SHA from a public comment on upstream `#357`).
8. **DEFERRED per explicit operator instruction, do not resurface:**
   cross-platform sudo/Linux port.
9. 2 free ultra reviews remain; operator wants them saved for other
   projects — prefer local review + mutation testing here, as done this
   session.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect e9a01e4 at tip, clean, pushed

# 1. IS IT UP? Do this first — other agents depend on it.
sudo-secretspec --version                       # expect 0.19.1-sudo.21
sudo-secretspec doctor
sudo -k && V=$(sudo-secretspec get GITHUB_TOKEN --reason "availability check") \
  && echo "retrieval OK, ${#V} chars"           # never echo $V itself
sudo-secretspec check --reason "availability check" | tail -1
#   expect: doctor: OK / retrieval OK, 40 chars / 47 found, 0 missing, 7 optional

# 2. Build — system SQLite. Required for EVERY cargo command here:
export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
export LIBRARY_PATH="/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH"
export CPATH="/opt/homebrew/opt/sqlite/include:$CPATH"
cargo test -p sudo-secretspec-cli                 # expect 228 passed, 0 failed
cargo test -p secretspec --features sqlite provider::sqlite   # expect 16
pytest tests/sudo_packaging -q                    # expect 26

# 3. capture_history migration entry point:
grep -n "422" secretspec/src/provider/sqlite.rs
# then read docs/design/infinite-secret-backup-restore.md's schema section

# 4. Upstream SIGPIPE follow-up — confirm still unopened before starting:
gh pr list --repo cachix/secretspec --search "sigpipe"

# 5. Mutation-verify any fix before believing it (2/2 this session):
#    ~/.local/state/handoffs/chains/standalone-b2db/mutate.sh <file> <label> <python-expr> <pkg>
#    NB: it backs up with `cp`, not `git checkout` — `git checkout` will eat
#    unstaged work. Also: verify the mutation actually landed in the file
#    before trusting a MISSED result — a silently-failed string replace
#    reports MISSED for a fix that is actually covered.
```
