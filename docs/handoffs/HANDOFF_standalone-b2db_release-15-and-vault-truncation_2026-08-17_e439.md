---
schema_version: 1
handoff_id: e439
parent_handoff_ids: [7c73]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 9702fe9f6c0735c9ee315ae9ee974c82b3cddbd4
created_at: 2026-08-17T06:57:57-0400
writer: claude-code
---

# Handoff — `0.19.1-sudo.15` shipped, and a plain `install` destroyed the vault

## The Goal

Resume `7c73`'s remaining four items: post the check-stdout issue upstream, open
the PR, reimplement manifest-edit on the `Spec` shape, and cut
`0.19.1-sudo.15`. The operator additionally asked, early on, for the command to
install `.15` so the post-install stdout gate would pass, and later said to use
the available headroom freely (explicitly permitting Fable 5 at any effort).

Three of the four shipped. **The session also caused real data loss**, which is
now the most important thing in this document.

## Where We Are

Branch `sudo-main` at `9702fe9`, **tree clean, everything pushed**. Three
commits:

| SHA | What |
|-----|------|
| `2789765` | Posted issue #372 + PR #373; tracked upstream #362; design doc |
| `d6f763e` | Stamp workspace `0.19.1-sudo.15`, skill to 0.6.0 |
| `9702fe9` | Homebrew formula for `v0.19.1-sudo.15` |

**Released:** `0.19.1-sudo.15` — tag, GitHub Release, formula, tap all published;
`brew test` passes; installed and verified (`doctor` OK, ledger chain intact).

**Upstream:** [#372](https://github.com/cachix/secretspec/issues/372) (issue) and
[#373](https://github.com/cachix/secretspec/pull/373) (PR) both posted.

**NOT done:** the `Spec`-shaped manifest-edit reference implementation for #370.
Untouched this session.

### Files changed by hand this session

- `sudo-secretspec/drafts/upstream-check-stdout-issue.md` — restored the missing
  ELI5 example block; status header now records #372/#373
- `sudo-secretspec/UPSTREAM-CONTACT.md` — #372, #373 and #362 added to the open
  table; the owed-item entry replaced with the #362 comment action; a
  "checked against #362" note added to the `codegen-schema` debt section
- `docs/design/upstream-ipc-v1-and-the-fork.md` — **new**, the #362 analysis
- `skills/sudo-secretspec/SKILL.md` — version 0.5.0 → 0.6.0, verified-against
  stamp → `.15`, broken-pipe consequence added to the `check` pitfall
- `Cargo.toml`, `Cargo.lock`, `secretspec-derive/Cargo.toml`,
  `sudo-secretspec-cli/Cargo.toml` — version stamp `.14` → `.15`
- `packaging/homebrew/sudo-secretspec.rb` — rewritten by `release.py` (version +
  sha256 `3a93fae1…`)
- On the separate `fix/check-report-to-stdout` branch (off `upstream/main`, for
  PR #373, not on `sudo-main`): `secretspec/src/secrets.rs`,
  `secretspec/tests/check_report_stream.rs`, `CHANGELOG.md`

No code under `secretspec/src/` or `sudo-secretspec-cli/src/` was modified on
`sudo-main` this session — the truncation bug is **diagnosed, not yet fixed**.

### THE OPEN INCIDENT — the vault's `.env` was truncated to 0 bytes

`/var/db/sudo-secretspec/.env` is **0 bytes**. Every stored secret value is gone
from the vault. `check` reports `8 found, 7 missing, 37 optional` where four
Aug 14–15 handoffs recorded `43 found, 0 missing, 7 optional`.

**Cause: I told the operator to run `sudo /opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec install`
WITHOUT `--adopt-existing`.** Both shipped docs specify the flag —
`skills/sudo-secretspec/SKILL.md:197` and `sudo-secretspec/AI-GUIDANCE.md:87`.
I had been editing SKILL.md earlier in the same session, ~40 lines above that
line, and still did not read it before handing over a privileged command. It ran
twice (06:21:27 and 06:47:08).

Recovery is available and NOT yet performed — see Where We're Going item 1.

## What We Tried

Chronological, including the two things I got wrong. These are the expensive
parts to rediscover.

### 1. FAILED HYPOTHESIS: "the `dfa4b10` merge broke provider resolution"

On seeing `8 found` against a historical `43 found`, I reasoned: `.15` is the
first release carrying the 87-commit merge (verified — `git merge-base
--is-ancestor 92eee84 v0.19.1-sudo.14` fails, `...v0.19.1-sudo.15` succeeds), and
upstream's changelog says profile application now "preserves each provider's
public URI and storage/cache identities". Plausible, and **wrong**.

Worse, I presented that hypothesis to the operator in an `AskUserQuestion` asking
whether to roll back — **before running the one cheap test that would have
falsified it**. They chose rollback. `.14` produced byte-identical numbers
(`8 found, 7 missing, 37 optional`). The rollback cost a Touch ID and a downgrade
to learn something a single `check` on `.14` would have shown for free.

**Lesson:** when a hypothesis is cheaply falsifiable, falsify it before asking
the operator to spend a privileged operation on it.

### 2. FAILED READING: `template-check` green was taken as reassurance

`template-check` reported "runtime manifest matches tracked declaration example",
exit 0, and I read that as evidence the boundary was healthy. It was **evidence
of the overwrite**: the install had just `fs::copy`'d the declarations file over
the runtime manifest, so of course they matched byte-for-byte. A hand-managed
runtime manifest matching a template exactly should have been suspicious, not
comforting.

### 3. FAILED CONCLUSION: "the loss predates tonight"

Because the vault's `secretspec.toml` had mtime `Aug 15 20:45` — apparently
untouched — I concluded the fresh-install branch had not run and the loss dated
to the Aug 16 00:58 install. Both halves were wrong:

- Rust's `fs::copy` on macOS (`fcopyfile`, `COPYFILE_ALL`) **preserves source
  mtime**. The manifest is byte-identical in size (9663) *and* mtime
  (`2026-08-15 20:45:42`) to `/usr/local/share/sudo-secretspec/secretspec.toml`.
  It *was* overwritten; it just doesn't look it.
- The audit ledger dates the loss exactly (see Evidence). The Aug 16 installs
  were harmless because they used the documented `--adopt-existing` form.

### 4. The release helper died mid-run on a full disk

`packaging/release.py` publishes tag → Release → formula → tap **before** the
local Homebrew steps. `/System/Volumes/Data` was at 100% (777Mi free), so
`brew reinstall` failed with "No space left on device" *after* everything public
had shipped. Do **not** re-run `release.py` in that state — it fails on the
existing tag. Finish by hand: `brew update --force`,
`brew reinstall frdminc/sudo-secretspec/sudo-secretspec`, `brew test ...`.

Freed 62Gi by deleting `target/debug` (58G of pure build cache). **The next
`cargo test` in this repo is a full rebuild.**

### 5. A draft marked READY TO POST had a hole in it

`sudo-secretspec/drafts/upstream-check-stdout-issue.md` had, in its ELI5 lead:
"So if you try to do the obvious thing:" followed immediately by "…you get
nothing" — the example command block was **missing entirely**. Verified against
raw bytes (`od -c`) before concluding it wasn't a rendering artifact. Restored
before posting. A previous session's "nothing outstanding" is not a substitute
for reading the artifact.

## Key Decisions

### Chosen: build the upstream PR on a worktree off `upstream/main`, not cherry-pick

`git worktree add -b fix/check-report-to-stdout <dir> upstream/main`, then
`git checkout sudo-main -- secretspec/src/secrets.rs secretspec/tests/check_report_stream.rs`.
Deliberate, per `7c73`: the fork's CHANGELOG diff against upstream is **458
fork-local insertions**, so the entry had to be hand-placed under upstream's own
Unreleased. Filed under **Changed**, not Fixed — it changes observable output
routing. Did not upstream `tests/sudo_postinstall/*` or `skills/*`.

Verified all 3 regression tests pass against a **pure `dfa4b10` base**, not
merely against our merged tree. That is the claim that matters for a PR.

### Chosen: write proper release notes rather than accept `default_notes()`

`packaging/release.py:496` produces generic packaging boilerplate. `.15` carries
a behaviour change (report stream) that scripts can break on, so notes were
written to `--notes-file` covering the stdout change, the broken-pipe exit, the
`codegen-schema` feature, and the correct upgrade command.

### Chosen: roll back rather than diagnose forward — on a bad premise

Recorded as a decision because the *process* was wrong, not the option. See
What We Tried §1. The operator chose from options I framed; the framing was the
defect.

### Rejected: yanking the `.15` release or tap after the incident

`.15` is exonerated — `.14` reproduces the same resolution state. The truncation
bug is in `install.rs` and predates `.15` entirely. Unpublishing is messier than
rolling a machine back, and `.15` is correct for a fresh install. PR #373 is also
untouched by any of this — it is the `check` fix on a clean upstream base.

### Chosen fix shape for the truncation bug (NOT yet implemented)

Not "make the dry-run guard fire on the live path" — plain `install` from libexec
*is* the documented upgrade path, so it must work on an existing deployment.
Instead: runtime-file creation becomes **create-only-if-missing**, never
truncating, independent of `adopt_existing`; reconcile `--dry-run` with live
semantics; add a regression test that install over a populated vault leaves
`.env` byte-identical.

## Evidence & Data

### The truncation bug, located

`sudo-secretspec-cli/src/install.rs:1045-1049`, live path:

```rust
// Runtime files for fresh install only.
if !req.adopt_existing {
    let manifest_rt = req.vault.join("secretspec.toml");
    let env_rt = req.vault.join(".env");
    fs::copy(&declarations_dst, &manifest_rt)?;
    fs::File::create(&env_rt)?;        // truncates an existing .env to 0 bytes
```

The refusal that should prevent this —
`"fresh-install identity or vault already exists; use --adopt-existing"` at
`install.rs:914` — is inside `if req.dry_run {` (line 856), which returns at 927.
Confirmed only two `return Err` exist on the live path between 950 and 1044, and
neither covers an existing vault. **So `--dry-run` refuses what the live path
does.** Same failure shape as verifying an EPIPE fix with `| grep` (see `7c73`):
a check structurally incapable of catching the thing it exists for.

`--adopt-existing` takes the `else` branch (`install.rs:1060-1070`), which only
*verifies* the runtime files exist. That is the safe form and the documented one.

### The audit ledger dates the loss to this session

Value-free by design (`names_json` holds names, never values; schema at
`audit.rs:502-521`).

| operation/phase | n | first_seen | last_seen |
|---|---|---|---|
| `source-export` / success | 66 | 2026-08-15 12:21:44 | **2026-08-17 04:15:23** |
| `source-export` / failure | 9 | **2026-08-17 06:21:53** | 2026-08-17 06:47:19 |
| `source-set` / success | 6 | 2026-08-15 17:35:26 | 2026-08-16 18:31:08 |
| `source-check` / success | 94 | 2026-08-15 11:44:22 | 2026-08-16 18:32:17 |
| `source-get` / success | 14 | 2026-08-15 14:20:37 | 2026-08-16 18:35:47 |

`export` succeeded at **04:15:23**; the first failure is **06:21:53**, 26 seconds
after the install at 06:21:27. That brackets the loss to this session's first
install.

Note `source-check | success` last occurred **Aug 16 18:32**, so the **7 missing
required secrets predate this session** and are a separate matter from the
truncation.

### Vault state after the incident

```
-rw-------  1 _sudo_secretspec  _sudo_secretspec       0 Aug 17 06:47 .env
-rw-------  1 _sudo_secretspec  _sudo_secretspec  249856 Aug 17 06:47 broker-audit.sqlite3
-rw-------  1 _sudo_secretspec  _sudo_secretspec    9663 Aug 15 20:45 secretspec.toml
```

`sudo wc -lc .env` → `0 0`. The 7 required names with no value:
`ATLASSIAN_CFENGINE_API_TOKEN`, `SPOTIFY_CLIENT_ID`, `SPOTIFY_CLIENT_SECRET`,
`TELEGRAM_BOT_TOKEN`, `TWILIO_ACCOUNT_SID`, `TWILIO_AUTH_TOKEN`,
`WEBUI_SECRET_KEY`.

### Arq coverage — recovery IS available

`arqc` is on PATH at `/opt/homebrew/bin/arqc`. It has **no restore command**
(commands: listBackupPlans, stats, latestBackupActivity{Log,JSON},
start/stop/pauseBackups, license/appPassword) — restore is GUI-only in Arq 7.

Two plans: `5803D830-…` "djbclark HOME to Google Drive", `47DBF853-…`
"SYSTEM to Google Drive" (dbId 3). The SYSTEM plan's log confirms coverage:

```
17-Aug-2026 02:10:05 EDT Backup plan: SYSTEM to Google Drive
17-Aug-2026 03:53:35 EDT /private/var (3 exclusions): 100.53 GB, 180,526 files backed up
17-Aug-2026 03:53:36 EDT Total scanned: 188.401 GB, 865,756 files
```
errorCount 0, finished 03:57:27.

Runs are **twice daily at 02:10 and 03:14** (68 logs in
`/Library/Application Support/ArqAgent/logs/backup/`). The **Aug 17 02:10 SYSTEM
record finished 03:57 — before the 06:21 loss — so it holds the intact `.env`.**

Also still on disk, as origins for many declarations (existence/size only; never
read): `~/.hermes/.env` (958 B), `~/.config/stayturgid/play.env` (351 B),
`~/.config/stayturgid/observability.env` (101 B),
`~/.config/stayturgid/adbkey` (1704 B), `~/.ssh/stayturgid_ca` (399 B),
`~/.ssh/termux_key` (432 B).

### Release verification

`.15` installed cleanly: `installed sudo-secretspec 0.19.1-sudo.14 ->
0.19.1-sudo.15` (the arrow is the evidence the upgrade was real), `doctor: OK`
exit 0 with `UPGRADE_AVAILABLE` correctly gone once staged == installed,
`audit-verify` 560 events chain intact. `brew test` passed.

Post-install suite (`~/.local/bin/pytest tests/sudo_postinstall`, **not**
`python3 -m pytest` — system python 3.14 has no pytest): **17 passed, 3 failed,
11 skipped**. `test_check_writes_its_report_to_stdout` **PASSES** on `.15` and
fails on `.14` by design. The 3 failures
(`test_export_emits_every_declared_secret`,
`test_run_forwards_double_dash_to_the_child`,
`test_run_propagates_the_child_exit_code`) all fail only because required
secrets have no value — they will clear when the vault is restored.

Rust suite at `d6f763e`: **1586 passed / 21 failed**, all `provider::sops::*`
(sops CLI absent). Matches `7c73`'s baseline exactly.

### Upstream state (re-checked this session)

`upstream/main` **unmoved at `dfa4b10`**. #370 and #371 open, zero comments. #64
open. Newly tracked: **#362** "feat: add versioned client and provider IPC"
(SecretSpec IPC v1 for 0.20+, `secretspec broker --stdio`) — the maintainer's own
PR, previously visible only as a pointer inside a comment on #64. A Fable 5 pass
read both codebases; analysis in
`docs/design/upstream-ipc-v1-and-the-fork.md`. Headline findings:

- Upstream's "broker" is an IPC endpoint **inside the caller's trust domain**,
  not a privilege boundary: session authority is possession of inherited pipe
  handles (`ipc-architecture.md:150-152`), no caller identity (`:191`),
  forwarding secret authority a stated non-goal (`:211-223`), and `initialize`
  accepts a **caller-supplied** manifest/provider/profile (`broker.rs:67-87`).
  Its audit is fail-open (`audit.rs:8-19`); ours is fail-closed and hash-chained.
- `secretspec.provider/1` **is** the `exec://` mechanism #345 asked for, and a
  privileged endpoint is registrable as **data** (root-owned
  `providers.d`, `external.rs:473-483`) with **no upstream patching**.
- It does **NOT** retire the `codegen-schema` shape debt — no manifest-shape
  reflection anywhere in the five-method client protocol.
- Review point worth offering: its registration trust check validates only the
  **immediate parent** directory (`external.rs:335-361`, symlink-following
  `fs::metadata`), but macOS `/Library/Application Support` is admin-group
  writable. This fork walks the full ancestor chain with `symlink_metadata`
  (`drift.rs` `check_ancestor_chain`).

## Operator Feedback

- Asked for the `.15` install command up front; `.15` did not exist yet, so it
  was cut this session specifically to satisfy that.
- "Resume the plan," plus: use the headroom freely, **Fable 5 permitted at any
  effort level** (the standing "never use `-fast` variants" note is about `-fast`
  suffixes, not Fable).
- Chose **rollback** over diagnose-forward when offered — on a premise I had not
  tested. Framing defect was mine.
- Corrected me on Arq: it **does** cover `/var/db`, "although keep in mind on mac
  it is actually `/private/var/db`". Confirmed — `vault_realpath` in the config
  already agrees.
- Pointed out an Arq CLI exists, which is how `arqc` was found.
- Standing, carried forward and unchanged: follow upstream's stated direction
  rather than arguing it; re-check all upstream contact every session; do **not**
  delete `explore/pr-334-rust-first-spec` (`bf0b25c` is linked by SHA from a
  public comment on #357); item 12 (cross-platform sudo/Linux port) is
  **DEFERRED — do not resurface**.

## Where We're Going

1. **THE NEXT ACTION — restore the vault from Arq.** Operator-driven (GUI +
   privileged move). In Arq: **SYSTEM to Google Drive** → the **Aug 17 02:10**
   backup record → restore `/private/var/db/sudo-secretspec/.env` to a scratch
   path. Then, as root, because a restore landing with operator ownership breaks
   the boundary:
   ```bash
   sudo install -o _sudo_secretspec -g _sudo_secretspec -m 0600 \
     /path/to/restored/.env /var/db/sudo-secretspec/.env
   sudo-secretspec check --reason "post-restore verification"
   ```
   Restore **`.env` only**, not `secretspec.toml` (the manifest matches tracked
   declarations; reintroducing an older copy creates drift). Expect `found` to
   return to roughly 43–45 with the 7 required ones still missing — that gap
   predates the incident. If Arq's record is unusable, fall back to re-`set`ting
   from the on-disk origin files listed in Evidence.

2. **Fix the truncation bug and cut `0.19.1-sudo.16`.** Design in Key Decisions.
   Until then the ONLY safe upgrade form is:
   ```bash
   sudo /opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec install --adopt-existing
   ```
   Update SKILL.md / AI-GUIDANCE.md **before** tagging (the .12 trap). Consider
   making `--adopt-existing` the default and requiring an explicit
   `--fresh`/`--force-fresh` for the destructive path, since the safe case is
   overwhelmingly the common one.

3. **Reimplement manifest-edit on the `Spec` shape** — #370's reference
   implementation, the one `7c73` item still untouched. Wrinkle recorded in
   #370's body: `Spec::from_toml` rejects non-empty `project.extends`, so the
   internal reparse needs an `extends`-aware variant seeded from `self.base_dir`,
   and the retained text must be the **root file only** (`Config::try_from`
   merges parents).

4. **Comment on upstream #362** per
   `docs/design/upstream-ipc-v1-and-the-fork.md` → "Next actions". Announce
   intent to ship `sudo-secretspec` as the first out-of-tree *privileged*
   `secretspec.provider/1` endpoint, closing the loop on #345. Carry the
   ancestor-chain review point from Evidence. Offer, don't demand.

5. **Watch #372/#373** for maintainer response. Recreate the PR worktree if
   changes are requested: `git worktree add <dir> fix/check-report-to-stdout`
   (branch is pushed; its worktree was removed).

6. **Optional hardening, still unscoped (from `7c73`):** wrap the broker's
   `execute()` in `catch_unwind`. No `catch_unwind` exists in
   `sudo-secretspec-cli` and the workspace does not set `panic = "abort"`, so any
   panic unwinds past the terminal audit event at `broker.rs:558` and strands an
   attempt in the protected ledger.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect 9702fe9 at tip, tree clean

# INCIDENT STATE — check this first:
sudo wc -lc /var/db/sudo-secretspec/.env     # 0 0 means not yet restored
sudo-secretspec check --reason "orientation" # 8 found/7 missing/37 optional if not restored
sudo-secretspec --version                    # 0.19.1-sudo.15

# NEVER run plain `install` on this machine until item 2 lands:
#   sudo .../libexec/sudo-secretspec install --adopt-existing    <- the safe form

# Standing directive — re-check upstream contact FIRST, every session:
cat sudo-secretspec/UPSTREAM-CONTACT.md      # then run the two gh search commands
git fetch upstream main && git log --oneline -1 upstream/main   # was dfa4b10

# Tests (--no-fail-fast REQUIRED or the CLI crate never runs;
# target/debug was deleted, so this is a FULL REBUILD, ~4min+ for deps):
cargo test --no-fail-fast -p secretspec -p secretspec-derive -p sudo-secretspec-cli
# expect ~1586 passed / 21 failed, all provider::sops::* (sops CLI absent)
# `cargo test --all` CANNOT run here: ext-php-rs needs a PHP toolchain

# Post-install suite — use this pytest, NOT `python3 -m pytest`:
~/.local/bin/pytest tests/sudo_postinstall -q

# Disk: check BEFORE any release. It hit 100% mid-release this session.
df -h /System/Volumes/Data
```
