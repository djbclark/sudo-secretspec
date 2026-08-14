---
schema_version: 1
handoff_id: f3eb
parent_handoff_ids: []
lineage: none
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: sudo-secretspec
branch: sudo-main
head_sha: 1248ed6ed26e1b105ba378e847b23641975684d1
created_at: 2026-08-13T21:22:14-0400
writer: claude-code
---

# Handoff — sudo-secretspec privilege-boundary hardening

## The Goal

Started as a documentation cleanup of the downstream fork's `sudo`-related
markdown. Grew, at the operator's direction, into: a full code review of the
`sudo-secretspec-cli` companion crate driven by the repo's own
`PROMPT-REVIEW.md`; fixing every finding; deploying the hardened build to the
live macOS boundary; and consolidating the fork's git branches.

## Where We Are

**Predecessor.** No Tier 2 parent exists (hence `parent_handoff_ids: []`), but
this session continues work logged by a different agent: the Tier 1 pointer at
`~/.local/state/handoffs/sudo-secretspec/privilege-boundary/SESSION_LOG.md`,
written by `hermes` at 2026-08-13T11:00-0400 in the pre-v0.4 legacy format. Its
`next_action` was "finish generic drift/release packaging, integrate ops site
config, **review**, release v0.19.1-djbclark.1" — this session executed the
review step (via `PROMPT-REVIEW.md`) and the fixes arising from it. That pointer
was stale on arrival: it recorded branch `feature/sudo-privilege-boundary` at
`b90afe3`, which has since been merged (PR #1) and deleted. Release
`v0.19.1-djbclark.1` remains **un-cut**.

`sudo-main` at `1248ed6`, working tree **clean**, in sync with origin. Local
branches: `sudo-main` (trunk) and `main` (upstream mirror, `b51c378`).

Five commits landed this session:

| SHA | Subject |
|---|---|
| `96f7a91` | docs: deduplicate downstream fork documentation |
| `3adb6ae` | fix: harden the sudo-secretspec privilege boundary (17 files, +708/−166) |
| `76175a6` | docs: consolidate the fork onto a single sudo-main trunk |
| `32ff51f` | fix: validate sudoers before it can take effect |
| `1248ed6` | fix: scope the sudoers combined check to our own policy |

The hardened build is **installed and live** on this host. `doctor` exits 0,
lifecycle operations work, and boundary lifecycle is blocked through the
NOPASSWD path.

### Files changed this session

Rust: `sudo-secretspec-cli/src/{main,broker,audit,drift,install,rollback}.rs`,
`sudo-secretspec-cli/tests/install_rollback.rs`.
CI/build: `.github/workflows/{sudo-release,test}.yml`, `devenv.nix`.
Docs: `CHANGELOG.md`, `CLAUDE.md`, `FORK-AI.md`, `README.downstream.md`,
`packaging/README.md`, `packaging/release.py`, `skills/sudo-secretspec/SKILL.md`,
`sudo-secretspec/{README,AI-GUIDANCE}.md`.

### Review findings and disposition

All 15 fixed. Ordered as delivered:

1. **SECURITY — `sudo` resolved via `PATH`.** All five call sites in `main.rs`
   used `Command::new("sudo")`. Demonstrated live: a fake `sudo` earlier in
   `PATH` satisfied `run` with forged env, exit 0, zero audit events. Silent
   because `run_target` uses `.output()` and only prints stderr on failure.
   Fixed to `/usr/bin/sudo` (const `SUDO`).
2. **SECURITY — NOPASSWD wildcard.** `sudoers_text` granted
   `sudo-secretspec *`; client and broker are the same binary, so `install`
   and `rollback` ran with no Touch ID. Now per-subcommand (`__broker *`,
   `doctor`, `doctor *`) plus an in-binary guard: `invoked_as_privileged_broker()`
   refuses anything but `__broker`/`doctor` when `current_exe()` is the libexec path.
3. **SECURITY — rollback arbitrary root write/exec.** Executed a `restore`
   program from the snapshot; `N.path` supplied unconstrained destinations; no
   manifest verification despite the doc claim. Now `plan_restore` requires a
   manifest, allowlists destinations against `installed_artifacts()`, verifies
   SHA-256, and takes modes from the installer's table. Exec removed.
4. **SECURITY — empty rollback snapshots.** `install` created the dir and never
   captured priors, so every rollback failed. Now `capture_snapshot` runs before
   any overwrite.
5. **BUG — `Report.ok` ignored no advisories.** `ok = findings.is_empty()`.
   Contradicted the stated invariant and, combined with "treat drift as a hard
   stop", would wedge every agent permanently. Added `ADVISORY_CODES` +
   `Finding.advisory`.
6. **BUG — `run` mangled `--`.** Double-split after clap already consumed the
   delimiter. Reproduced: `run --reason t -- /bin/echo hello -- world` exited
   127 trying to exec `world`. Manual re-split deleted.
7. `require_boundary` checked neither ownership nor mode → now enforces vault
   mode 0700, vault + runtime-file ownership, no group/world access, and
   `canonicalize() == vault_realpath`.
8. `expected_uid` was always `None` from the broker → now passes the service uid.
9. Audit doc claimed an inode re-check; code re-stat'd by path and discarded
   `_existed` → real `(dev, ino)` comparison; ownership asserted *after* the
   chown so an older root-owned ledger repairs instead of failing.
10. Ledger truncation-to-empty is undetectable → documented as residual risk in
    `audit.rs` and `AI-GUIDANCE.md`.
11. Terminal-audit failure masked the real rc → exits 126 and reports it.
12. `std::env::vars()` panics on non-UTF-8 env → `vars_os()`.
13. `sudo-release.yml` ran shellcheck/shfmt on deleted shell scripts and
    ruff/pytest on a nonexistent `tests/sudo_secretspec`, and never ran
    `cargo test` → repaired, now runs the crate's suite on macOS.
14. macOS-only crate broke upstream's Linux and Windows CI → excluded from
    `test.yml` (both jobs) and `devenv.nix`.
15. `packaging/README.md` version + formula claims were false → corrected, plus
    `release.py`'s docstring.

Then, from operator follow-up: the installer wrote `sudoers.d` **before**
validating it (`32ff51f`), and my first fix for that was too strict (`1248ed6`).

## What We Tried

Chronological, including what failed — this is the expensive part to rediscover.

1. **`grep -ril` for the initial file inventory — incomplete.** macOS `grep -r`
   does not follow symlinks, so `AGENTS.md` (→ `CLAUDE.md`) and `README.md`
   (→ `secretspec/README.md`) were silently missed. `-R` follows. Two other
   files (`PROMPT-REVIEW.md`, `PROMPT-SECREV.md`) appeared mid-session from
   commit `29c6e9b`, not from the grep flag.
2. **`cargo test ... | tail -30` — wrong conclusion.** The pipe truncated the
   log, so only 4 of 8 test binaries were visible and I reported 13 tests when
   the real number was 58. Always capture the full run and sum `^test result:`.
3. **Release build failed on a corrupt cargo registry.** `FORK-AI.md` lesson 3
   at unexpected scale: **546** extracted crates under
   `~/.cargo/registry/src/index.crates.io-*` were missing `Cargo.toml`. Removed
   every one that had a matching `.crate` archive in `registry/cache/` (all 546
   did, so no network needed); rebuild took 1m30s. Root cause of the corruption
   is unknown — possibly the disk-space cleanup noted in `~/CLAUDE.md`.
4. **First sudoers hardening was too strict — install denied.** Staging +
   isolated `visudo -c -f` was right, but I also failed the install on a bare
   `visudo -c`. That fails on this host because
   `/private/etc/sudoers.d/yabai` has the wrong mode. Our installer refused to
   run over another package's misconfiguration and restored the previous policy.
   Fixed in `1248ed6`: on combined-check failure, re-validate our own file and
   continue with a warning if it is clean.
5. **Could not test two fixes end-to-end after fixing them.** Post-fix, `run`
   and the `PATH` hardening can only be exercised by invoking the real broker
   against the real vault, which prints live secrets. Both were demonstrated
   empirically *before* the fix; afterwards verified by source grep and by
   confirming `/usr/bin/sudo` is the only sudo string in the binary.
6. **Revert vs rebase for undoing the `main` merge.** Rejected `git revert -m 1`:
   a revert commit would silently prevent those same upstream changes from ever
   merging cleanly again, and the operator explicitly wants them in a future
   release. Used `git rebase --onto 3adb6ae 7fd91e8 sudo-main` +
   `--force-with-lease`.

## Key Decisions

- **Trunk stays `sudo-main`.** Rejected renaming to `master` and folding into
  `main`. Chosen because `packaging/homebrew/sudo-secretspec.rb`
  (`head ... branch: "sudo-main"`) and `packaging/release.py` then need no changes.
- **Branch deletion limited to the fork's own branches.** Rejected deleting all
  ~117 origin branches; ~115 inherited from the upstream fork are untouched.
- **`main` restored** at `b51c378` as the upstream mirror after the operator
  judged its deletion a mistake. It must **not** be merged into `sudo-main`
  unasked — it carries post-0.19.1 upstream work (shell completions,
  age-provider delete, social card) held for a future release.
- **`SKILL.md` stays self-contained**, deliberately duplicating policy from
  `AI-GUIDANCE.md`, because the skill installs on machines without this repo.
  Rejected pointer-only and copy-at-package-time. Rationale recorded in `FORK-AI.md`
  so a later cleanup does not "fix" it.
- **macOS-only crate excluded from workspace-wide test commands.** Rejected
  making it build on Linux via `cfg(unix)` libc: `/bin/ls -lde`, `dscl`, and
  the `/private/var` layout are macOS-specific, so a green Linux run would be
  meaningless.
- **Two commits, split by file not perfectly by concern.** `README.downstream.md`
  is pure dedup; the dedup edits in `FORK-AI.md` and `sudo-secretspec/README.md`
  are entangled with hardening notes and ride in `3adb6ae`.

## Evidence & Data

- **Tests:** 58 passed / 0 failed at baseline (matching `PROMPT-REVIEW.md`'s
  claim) → 66 after the fixes (+8) → **68 after the sudoers work**. `cargo fmt`
  clean; clippy warnings are pre-existing lint classes also present in the
  untouched `secretspec` lib.
- **Rollback snapshot fix, live proof:** install reports `rollback_artifacts=8`;
  `sudo-secretspec-rollback-1786670348` is 608 bytes / 19 links (17 files =
  8 `.prior` + 8 `.path` + manifest). The three pre-fix snapshots
  (`1786659819`, `1786663736`, `1786664372`) are still 64 bytes — empty.
- **sudoers policy:** installed file is 329 bytes, byte-identical to the
  template validated with `visudo` beforehand. Four lines: the `Defaults!`
  hardening line plus `__broker *`, `doctor`, `doctor *`.
- **Boundary lifecycle blocked:** `sudo -n /usr/local/libexec/sudo-secretspec
  install --dry-run --adopt-existing` and the `rollback` equivalent both return
  `sudo: a password is required`. Before the fix both ran freely.
- **Advisory fix, live proof:** `doctor` exits 0 and prints
  `[advisory] LEGACY_VAULT_CLUTTER` for `/var/db/stayturgid-secrets/.local` and
  `/.ansible` — exactly the residual state `FORK-AI.md` predicted. The
  previously installed binary reported bare `OK` with no findings, so it was not
  detecting that clutter; the old binary is preserved in the new snapshot.
- **Pre-existing host issue:** `visudo -c` reports
  `/private/etc/sudoers.d/yabai: bad permissions, should be mode 0440`, so sudo
  is ignoring that policy. Not modified by this session. Fix is
  `sudo chmod 0440 /etc/sudoers.d/yabai`.
- **Live config unchanged by the reinstall:** vault `/var/db/stayturgid-secrets`,
  `_secretspec:staff`, declarations pinned from
  `/usr/local/share/sudo-secretspec/secretspec.toml` (verified byte-identical to
  `~/ops/site-private/secretspec.toml.example` before installing, so
  auto-detection was a no-op).

## Operator Feedback

- All downstream work commits **directly to `sudo-main`** — no feature branches,
  no PRs. If the operator says "main" or "master", they mean `sudo-main`; they
  said they may misname it. Saved to memory as `trunk-is-sudo-main`.
- Deleting `main` was a mistake; it is restored and is the upstream mirror only.
- Upstream's post-0.19.1 files are "not ours and meant for a future release" —
  removed from `sudo-main`.
- The operator handles Touch ID themselves for privileged installs.
- Wanted the installer made structurally incapable of breaking sudoers, not just
  checked afterwards.

## Where We're Going

1. **Run `sudo-release.yml` and confirm it passes.** It was substantially
   repaired this session but has **never executed**: it triggers on PR paths,
   tag push, or `workflow_dispatch`, and everything went straight to `sudo-main`
   with no PR. The repaired steps (cargo test with Homebrew SQLite, the removed
   shellcheck/pytest paths) are therefore unverified.
2. Decide on `PROMPT-SECREV.md` — the Rust privilege-boundary *security* review
   prompt, which sits next to `PROMPT-REVIEW.md` and was never run. It targets
   the same crate this session rewrote, so it would now review the hardened code.
3. Resolve the **Touch ID question**: the privileged install completed without
   any prompt appearing on the agent side. Either the operator authenticated, or
   something grants passwordless sudo for arbitrary binary paths — worth
   confirming, because the elevation design assumes `install`/`rollback` are
   authentication-gated.
4. Fix the unrelated host issue: `sudo chmod 0440 /etc/sudoers.d/yabai`.
5. Clean up the three stale empty rollback snapshots under
   `/usr/local/libexec/` (they now fail with "snapshot manifest missing", which
   is a clear error, but they are clutter).
6. Decide the release story: `sudo-main` is versioned `0.19.1-djbclark.1` and
   the upstream merge was undone, so the fork is pinned to upstream 0.19.1.
   Folding `main` in later will need a version decision.
7. Optional: post the review findings somewhere durable. PR #1 is **merged**, so
   inline comments would land on a closed diff; nothing was posted anywhere.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -5          # expect 1248ed6 at tip, branch sudo-main

# Build/test — ALWAYS use system SQLite, never rusqlite `bundled`
export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
export LIBRARY_PATH="/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH"
cargo test -p sudo-secretspec-cli          # expect 68 passed, 0 failed
cargo build -p sudo-secretspec-cli --release

# Next action: exercise the repaired workflow
gh workflow run sudo-release.yml --repo djbclark/sudo-secretspec
gh run list --workflow=sudo-release.yml --repo djbclark/sudo-secretspec --limit 3

# Inspect the live boundary (read-only, NOPASSWD, safe)
sudo -n /usr/local/libexec/sudo-secretspec doctor

# Reinstall after changing installer/broker code (operator does Touch ID)
./target/release/sudo-secretspec install --adopt-existing
```

If `cargo build` fails with `failed to read .../<crate>/Cargo.toml`, the
registry extract cache is corrupt again — remove every directory under
`~/.cargo/registry/src/index.crates.io-*/` that lacks a `Cargo.toml` **and** has
a matching `.crate` in `~/.cargo/registry/cache/`, then rebuild offline.

Do not commit on a branch here, and do not merge `main` into `sudo-main`.
