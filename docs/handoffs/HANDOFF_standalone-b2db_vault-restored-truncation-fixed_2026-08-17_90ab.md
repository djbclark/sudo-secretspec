---
schema_version: 1
handoff_id: 90ab
parent_handoff_ids: [e439]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: ce38516c75600a1549ab34465ca5f92af82246ba
created_at: 2026-08-17T09:21:50-0400
writer: claude-code
---

# Handoff — vault restored from backup, truncation bug fixed and pushed

## The Goal

Resume `e439`, whose single next action was the open data-loss incident: a plain
`sudo-secretspec install` (no `--adopt-existing`) had truncated
`/var/db/sudo-secretspec/.env` to 0 bytes, destroying every stored secret value.
The operator additionally asked, mid-session, to install a command-line Arq
restore tool.

**Both landed. The incident is closed.** The vault is fully restored and
verified, and the bug that caused it is fixed, tested, documented and pushed.

## Where We Are

Branch `sudo-main` at `ce38516`, **tree clean, pushed** (`6f5ae76..ce38516` to
`frdminc/sudo-secretspec`). Two commits:

| SHA | What |
|-----|------|
| `ca2b6ab` | `fix(install)`: refuse a fresh install onto a populated vault + 3 regression tests |
| `ce38516` | `docs`: SKILL.md → 0.7.0 and AI-GUIDANCE.md state what omitting `--adopt-existing` *does* |

**Vault: RESTORED AND VERIFIED.**

```
Summary: 45 found, 0 missing, 8 optional
```

| | found | missing | optional |
|---|---|---|---|
| historical (Aug 14–15 handoffs) | 43 | 0 | 7 |
| after the truncation | 8 | 7 | 37 |
| **now** | **45** | **0** | **8** |

**Correction to `e439`, carry this forward:** `e439` said to expect "7 required
still missing (that gap predates the incident)". **That is false.** Those 7 were
part of the truncation damage and came back with the restore. `missing` is 0. A
future session must not go hunting for a phantom pre-existing gap.

`.env` holds 38 values; `check` reports 45 found because the other 7 resolve
through defaults and paths rather than the dotenv vault. The file-count and
check-count differing is expected, not a partial restore.

**NOT done:** `0.19.1-sudo.16` is not cut. Version stamp is still `.15`. The
`Spec`-shaped manifest-edit reference implementation for #370 remains untouched
(carried unworked from `7c73` through `e439` to here — three sessions).

### Files changed this session

- `sudo-secretspec-cli/src/install.rs` — guard hoisted out of the dry-run
  branch; `create_fresh_runtime_files()` extracted; 3 tests added
- `CHANGELOG.md` — Unreleased → Fixed entry
- `skills/sudo-secretspec/SKILL.md` — version 0.6.0 → 0.7.0, new
  `--adopt-existing` pitfall bullet
- `sudo-secretspec/AI-GUIDANCE.md` — paragraph on the flag's necessity and on
  why rollback snapshots cannot recover a truncated vault

Nothing else in the repo was touched.

## The Fix

`sudo-secretspec-cli/src/install.rs`, three changes:

1. **The real fix.** The guard refusing a fresh install onto an existing
   identity or vault sat *inside* the `if req.dry_run` block. Hoisted above it,
   so the live path enforces it too. This was the whole defect: `--dry-run`
   refused precisely what the live command went on to perform, so a green
   rehearsal read as permission to proceed.
2. **Defense in depth.** Extracted `create_fresh_runtime_files()`, which refuses
   when `secretspec.toml` or `.env` already exists instead of calling
   `fs::File::create` on it. The existence test is `symlink_metadata()`, **not**
   `Path::exists()` — `exists()` follows symlinks and returns false for a
   dangling one, which would have let `fs::copy`/`create_new` write *through*
   the link to a path outside the vault.
3. **Testability.** The destructive call sat mid-way through a root-only
   function no test could reach. Extraction is what makes the regression
   testable at all.

Tests (all passing, 34/34 in the module):

```
installing_over_a_populated_vault_leaves_env_byte_identical   ok
a_dangling_env_symlink_is_refused_rather_than_written_through ok
a_fresh_vault_still_gets_both_runtime_files                   ok
```

**Deliberately NOT done:** `--adopt-existing` was not made the default. The
guard now produces a clear refusal naming the flag, which removes the danger
without changing what an existing command means. Flipping the default remains
open if the operator wants it.

## What We Tried

Chronological. The restore path burned most of the session and the first three
approaches all failed.

### 1. `arqc` — wrong tool entirely (fast, cheap)

The operator's pasted note claimed `arqc` was "the command-line restore
program". It is not, and it was **already installed** — symlinked at
`/opt/homebrew/bin/arqc` since Jul 16. Its complete verb list is licensing and
backup control: `listBackupPlans`, `startBackupPlan`, `stopBackupPlan`,
`pauseBackups`, `resumeBackups`, `stats`, `latestBackupActivityLog/JSON`,
`setAppPassword`, `setGroup`, `acceptLicenseAgreement`, `activateLicense`,
`refreshLicense`, `deactivateLicense`. **No restore verb, no record browsing,
no file extraction.**

It was still useful: it gave the SYSTEM plan UUID and proved the newest record
was the pre-truncation one.

### 2. `arq_restore` against the Google Drive mount — EDEADLK

I first asserted `arq_restore` was Arq-5-only. **Wrong** — the repo was updated
2026-04-13 and its README states Arq 5, 6, and 7 support. Built it from source
(Xcode 26.6, `xcodebuild -configuration Release`, BUILD SUCCEEDED, arm64).

Registered the Drive mount as a `local` target. `listcomputers` worked (it reads
cached directory metadata) but `listfolders` died:

```
arq_restore: The file "backupconfig.json" couldn't be opened.
OSError: [Errno 11] Resource deadlock avoided
```

### 3. Chasing the EDEADLK as a permissions problem — two wrong theories

**Theory A: the tool sandbox.** Disproved — identical failure with sandboxing
disabled.

**Theory B: a launchd-detached process context.** The ancestry here is
`launchd → herdr → bash → claude` (herdr's parent is PID 1, `SECURITYSESSIONID`
and `TERM_SESSION_ID` both empty), so I reasoned a File Provider download
request had no responsible GUI app to attribute to. **Disproved by the
operator**: the same `head -c 80` from a fresh Ghostty tab, outside herdr,
failed identically.

**And Full Disk Access was never the issue.** I verified this process tree
*already has* FDA — `~/Library/Application Support/com.apple.TCC/TCC.db` and
`~/Library/Safari` both read fine. Ghostty and herdr both had FDA already. The
lesson: **FDA does not govern File Provider materialization**, and no additional
TCC grant would have helped.

The actual cause: every file in the backup set is a streaming placeholder —
`stat -f "%z %b %Sf"` reports `blocks=0 flags=compressed,dataless`. Reading one
needs a synchronous download through Drive's File Provider extension, and *that*
returns EDEADLK for command-line readers on this machine. Directory listings
work because metadata is cached locally.

`fileproviderctl` has **no** `materialize` verb (commands are `dump`,
`diagnose`, `evaluate`, `check`/`repair`, `obfuscate`), so there is no supported
CLI way to force materialization.

### 4. Finder-mediated local staging — worked, then hit a hard wall

Finder **can** materialize (proved by an `osascript` Finder `duplicate` of
`backupconfig.json`, which then read as real JSON). Staged 8.3 GB locally; the
local target then identified the plan correctly as
`[arq7] plan … mac - SYSTEM to Google Drive (encrypted)` where the Drive mount
had shown `[arq5]` with null names. The password decrypted the keyset, and it
selected the right record.

Then:

```
restoring backup from 2026-08-17 07:53:27 +0000
arq_restore: absurd string length 13042424520865021952 in [StringIO newString:]
             from <BufferedInputStream <DataInputStream: 13860 bytes: tree data>>
```

**`arq_restore` cannot parse this backup's trees.** `Arq7BlobLoc.m:81` gates
field parsing on `if (theTreeVersion >= 2)`, and both the parser and the
bundled `arq7_data_format.html` top out at tree version 2. Arq here is
**7.47.3**, an August 2026 build. A v3+ tree read by a v2 parser desyncs the
stream and then reads a garbage string length — textbook. Upstream has no fix:
last functional commits are March 2026 and no open issue matches.

**Three AppleScript bugs cost real time along the way**, all in the same
`osascript` Finder call, each surfacing only after fixing the previous:
`-10006` (a `folder`/`POSIX file` coercion), then `-1728` (`POSIX file` being
evaluated *inside* the Finder `tell` block, where it must be outside), then a
`repeat with n in {...}` loop variable being a *reference*, not text, so
`contents of n` was required.

### 5. Arq's own GUI — this is what actually worked

The route `e439` named in the first place. Arq.app reads the format Arq.app
wrote. The operator restored `.env` to a scratch path from the Aug 17 02:10
record, and `check` came back clean.

## Key Decisions

- **Pivoted to the GUI rather than patching `arq_restore`'s tree parser.**
  Writing a v3 parser against an undocumented format, to recover one 2852-byte
  file, when the vendor's own app restores it in a few clicks, is the wrong
  trade. Rejected.
- **Paused Arq backups before restoring** (`arqc pauseBackups 480`, resumed
  after verification). `arq_restore restore` takes the *most recent complete
  backup* with no record picker — a scheduled run would have made the truncated
  0-byte `.env` the newest record. This mattered less once the GUI took over,
  but it protected the window.
- **Backed up the truncated `.env` before overwriting** — the operator's call,
  and correct. My belief it was 0 bytes came from `e439`'s report, not from
  anything verified this session; had it differed, the plan needed rethinking.
  Saved to `/var/root/env-truncated-<timestamp>.bak`.
- **Restored to a scratch path, never the original.** `ArqRestoreCommand.m:809`
  confirms `arq_restore` writes to `cwd` + last path component and refuses if
  the destination exists — safer than the README's "defaults to the original
  path" implied. The GUI restore used the same discipline.
- **Documented the *consequence*, not the flag.** Both SKILL.md and
  AI-GUIDANCE.md already specified `--adopt-existing` in their upgrade command,
  and `e439` records that the previous session read past it while editing
  SKILL.md ~40 lines above that exact line. Restating the flag would change
  nothing. What was missing: what omitting it *does*, and that a green
  `--dry-run` said nothing about the live path.
- **Held the `.16` release.** Cutting it while `check` still reported a broken
  vault would make a release regression indistinguishable from the incident.
  Restore first, verify, then release.
- **Did not make `--adopt-existing` the default.** See The Fix.

## Evidence & Data

- Arq **7.47.3**. SYSTEM plan `47DBF853-A72C-459E-985D-5073B2F94660`; HOME plan
  `5803D830-0D59-4C89-AF19-2B404345723F`.
- The good record: started `2026-08-17 02:10:05 EDT`, finished `03:57:27`,
  0 errors, not aborted, 865756 files / 188.4 GB. Truncation ran at 06:21 and
  06:47 — the record predates it. It was also still the *newest* record, so no
  date-hunting was needed.
- `/private/var` backup folder UUID: `ADF5485A-BAE9-43F5-A408-D62662A65162`
  (the vault lives at `/private/var/db/sudo-secretspec/`).
- Backup set sizes, measured by summing `st_size` (dataless files report logical
  size, so `du` is useless here): `standardobjects` 105.52 GB / 4147 files,
  `largeblobpacks` 90.75 GB / 2386, `blobpacks` 6.45 GB / 1648, `treepacks`
  2.46 GB / 997, metadata ~1 MB. **Total 205 GB.**
- `backupconfig.json` has `maxPackedItemLength: 256000`, so sub-256 KB files are
  packed into `blobpacks` — which is why staging metadata + `treepacks` +
  `blobpacks` (8.9 GB) was sufficient and the 196 GB of large-object storage
  could be skipped. Staged copy verified: 997 + 1648 files, **0 dataless files
  remaining**.
- Restored `.env`: 38 lines, 2852 bytes, 38 distinct keys, no empty values.
- Disk: 70–81 Gi free throughout; never approached the 100% that killed the
  `.15` release.
- Tests: `cargo test -p sudo-secretspec-cli --lib install::` → **34 passed,
  0 failed**. Full-workspace suite NOT run this session.
- Two `arq_restore` targets remain registered in the operator's config:
  `arqdrive` (Drive mount — **does not work**, dataless reads fail) and
  `arqlocal` (`~/arq-staging`, **now deleted**). Both are stale; re-add
  `arqlocal` if staging is ever recreated.
- `arq_restore` source clone at `~/src/arq_restore`, binary symlinked at
  `~/.local/bin/arq_restore`. Kept — harmless, and it correctly reads Arq 5/6
  and Arq 7 *metadata*; only tree parsing is too old.

## Operator Feedback

- Reaffirmed the `arq_restore` install after I raised the Arq-5-only concern
  (which was wrong). Correct call — building it produced all the diagnostic
  value even though it could not finish the restore.
- **"Don't we want to backup the current one before replacing it?"** — caught a
  real gap in my instructions. I had gone straight to `sudo install` over the
  live vault without a preserving step.
- "Sure do the cheap things" → pause backups + start the fix, in parallel with
  the download.
- "commit, then update, then commit again" → the two-commit split above.
- Cleanup of the restored plaintext was done by the operator after I flagged
  that `~/arq-restore-out/.env` (mode 0600, owned by the login user) sat
  **outside the privilege boundary** and was readable by any process running as
  them — including every agent session. A second copy existed at
  `~/arq-restore-out/NEW/sudo-secretspec/` (0700) from the GUI restore. Both
  removed, along with `~/arq-staging`.

## Where We're Going

1. **THE NEXT ACTION: cut `0.19.1-sudo.16`.** The vault is healthy, so a
   release is now attributable. Stamp `Cargo.toml`, `Cargo.lock`,
   `secretspec-derive/Cargo.toml`, `sudo-secretspec-cli/Cargo.toml` from
   `.15` → `.16`; run `release.py`; then `brew reinstall` and install the
   boundary with `--adopt-existing`. **Check `df -h /System/Volumes/Data`
   BEFORE starting** — the `.15` release died at 100% disk *after* the tag,
   Release, formula and tap had already published, and `release.py` must not be
   re-run in that state. SKILL.md and AI-GUIDANCE.md are already updated and
   committed, so the doc-before-tag requirement is satisfied.
2. **Reimplement manifest-edit on the `Spec` shape** as #370's reference
   implementation — untouched across three sessions now. Wrinkle:
   `Spec::from_toml` rejects non-empty `project.extends`, so the reparse needs
   an extends-aware variant seeded from `self.base_dir`, and the retained text
   must be the ROOT file only.
3. **Comment on upstream PR #362** per `docs/design/upstream-ipc-v1-and-the-fork.md`
   "Next actions". Carry the ancestor-chain point: its registration trust check
   validates only the immediate parent dir (`external.rs:335-361`,
   symlink-following `fs::metadata`) while macOS `/Library/Application Support`
   is admin-group writable; this fork walks the full chain with
   `symlink_metadata` (`drift.rs check_ancestor_chain`).
4. **Watch #372 and #373** for a maintainer response. Branch
   `fix/check-report-to-stdout` is pushed; recreate its worktree with
   `git worktree add <dir> fix/check-report-to-stdout` if changes are requested.
5. **Optional, open question:** make `--adopt-existing` the default. The guard
   removes the danger either way; this is purely about ergonomics.
6. **Do NOT delete branch `explore/pr-334-rust-first-spec`** — `bf0b25c` is
   linked by SHA from a public comment on upstream #357.
7. **DEFERRED per explicit operator instruction, do NOT resurface:** item 12
   cross-platform sudo/Linux port
   (`docs/design/privilege-boundary-and-packaging.md:487`).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect ce38516 at tip, tree clean, pushed

# Vault is HEALTHY as of this handoff — confirm, don't assume:
sudo-secretspec check --reason "orientation"    # expect 45 found, 0 missing, 8 optional
sudo-secretspec --version                       # 0.19.1-sudo.15 (.16 not yet cut)

# The install truncation bug is FIXED at ca2b6ab, but the fix is NOT RELEASED.
# The INSTALLED boundary is still .15 and still carries the bug, so on this
# machine plain `install` remains destructive until .16 is installed:
#   sudo .../libexec/sudo-secretspec install --adopt-existing   <- the safe form

# Standing directive — re-check upstream contact FIRST, every session:
cat sudo-secretspec/UPSTREAM-CONTACT.md
git fetch upstream main && git log --oneline -1 upstream/main   # was dfa4b10

# Tests (--no-fail-fast REQUIRED or the CLI crate never runs):
cargo test --no-fail-fast -p secretspec -p secretspec-derive -p sudo-secretspec-cli
# expect ~1586 passed / 21 failed, all provider::sops::* (sops CLI absent)
# `cargo test --all` CANNOT run here: ext-php-rs needs a PHP toolchain
# This session ran only:  cargo test -p sudo-secretspec-cli --lib install::  (34/34)

# Post-install suite — use this pytest, NOT `python3 -m pytest`:
~/.local/bin/pytest tests/sudo_postinstall -q

# Disk: check BEFORE any release. The .15 release hit 100% mid-flight.
df -h /System/Volumes/Data

# If a vault restore is ever needed again: use Arq's GUI, NOT arq_restore.
# arq_restore (~/.local/bin/arq_restore) cannot parse Arq 7.47's tree format
# (v2 parser, v3+ data) and CLI reads of ~/Library/CloudStorage fail with
# EDEADLK regardless of Full Disk Access. Finder can materialize; the CLI cannot.
```
