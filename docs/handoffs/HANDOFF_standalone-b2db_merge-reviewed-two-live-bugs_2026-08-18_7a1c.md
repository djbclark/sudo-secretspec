---
schema_version: 1
handoff_id: 7a1c
parent_handoff_ids: [ccbb]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: ad6c368518016c09bf256d19d2a4fa0a69686492
created_at: 2026-08-18T19:13:20-0400
writer: claude-code
---

# Handoff — the merge got reviewed and grew conditions; two live bugs fell out

## The Goal

Resume from `ccbb` and execute its stated next action: the two-database file
layout (`secrets.db` alone | new `broker.sqlite3` taking both broker ledgers).

**No code was written, on purpose.** The design review found enough that
implementing first would have been wrong. The session produced: a corrected
understanding of the merge's failure mode, four conditions on whether to do it
at all, and **two live bugs in shipped `0.19.1-sudo.22` that have nothing to do
with the merge**. The operator then redirected to shipping those two first,
alone.

## Where We Are

`sudo-main` clean at `ad6c368`, nothing staged, nothing in flight, **no commits
this session** (this handoff is the first). Live vault untouched — never opened,
never probed, not even `ls`'d (it is `0700`; the attempt was denied and not
retried under sudo).

Test baseline, re-derived and green: **`cargo test -p sudo-secretspec-cli` =
229 passed / 0 failed** across 8 binaries (163+4+19+10+6+11+16+0).

## What We Tried

- **`cargo test --all` as the baseline — it FAILED, and I then misdiagnosed
  why.** `ext-php-rs` cannot build (`Could not find PHP executable`). I first
  reported "exit code 0" because I had piped through `tail`, which masked the
  real status — **always use `${PIPESTATUS[0]}` or don't pipe.** I then told the
  operator "the command in CLAUDE.md doesn't work here," blaming the docs. Wrong
  twice: `devenv.nix:61` enables `languages.php` and line 87 pins
  `pkgs.php.unwrapped.dev` precisely so `ext-php-rs`'s bindgen step works, and
  CLAUDE.md says `devenv shell` *first*. The real cause was that `devenv` was not
  installed (nix was). **I nearly `brew install php`'d, which would have been the
  wrong fix** — duplicating a dependency devenv manages, at a possibly different
  major version than `pkgs.php`. Checking `devenv.nix` before installing is what
  caught it. `devenv 2.2.1` is now installed via
  `nix profile install --accept-flake-config nixpkgs#devenv`.

- **Claimed the `head`-table collision "fails loudly — better than I feared."
  That was wrong and is the most important correction in this document.** See
  Key Decisions; the operational failure is silent.

- **Proposed "my tables missing = empty" as the fix for F5. Both reviewers
  independently rejected it** and produced a better design (single-owner
  `broker_db`). Rejected because it converts a deliberate corruption signal into
  silence — `audit.rs:807-813` documents that a ledger missing its tables is *a
  finding for the caller to report*, and `audit.rs:1122-1142`
  (`read_only_verify_does_not_create_the_schema`) pins that behavior.

- **One of four reviews produced nothing.** `gpt-5.6-sol-xhigh` exited rc=0
  having written a single byte. `cursor-grok-4.6-xhigh` was still running when
  this was written. So the conclusions rest on **two** substantive reviews
  (`gpt-5.3-codex-xhigh` and a Fable 5 xhigh agent), not four. Do not cite "four
  reviewers" downstream.

## Key Decisions

- **The `head` collision's real failure mode is SILENT, not loud.** Both ledgers
  create a table named `head` with different columns (`audit.rs:552`
  `event_hash`; `history.rs:104` `entry_hash`), both `IF NOT EXISTS`. Probed on
  3.53.4: the second CREATE is a silent no-op and the mismatched INSERT fails
  with `table head has no column named entry_hash`. **But**: `broker.rs:691`
  runs the audit attempt before any secret operation, so audit's `head` always
  wins and *history deterministically always loses* — not a race. And
  `Mutation::commit` (`broker.rs:428-443`) **swallows** archive failures into a
  stderr warning while the operation reports success. Net effect of a naive
  merge: **every mutation reports success while manifest archiving is
  permanently dead**, surfacing only as warnings and advisory `PENDING_ROLLBACK`
  findings. Tolerated-forever, which is worse than loud.

- **Rename all four tables** — `audit_events`/`audit_head`,
  `manifest_entries`/`manifest_head`. Only `head` actually collides (`events` vs
  `entries` do not), but rename all four: F2 makes renames free, it matches
  `1acd` line 98, and this codebase already produced one panic-scare over two
  same-named `entries` tables (`1acd` lines 82-85).
  **Rejected:** one shared `head` with a discriminator column — a single
  `UPDATE` missing its `WHERE chain = ?` would clobber the sibling chain's head,
  and it is exactly the "argument you can omit and get a wrong answer" class this
  codebase keeps excising (cf. the removed no-op filter param, `history.rs:299-308`).

- **Version guard belongs in neither module.** Create a single `broker_db::open`
  owning the file, and delete both modules' `ensure_schema`. Only that makes
  "exactly once" true by construction. `PRAGMA user_version` was **probe-verified
  transactional on 3.53.4** (rollback leaves it 0), so create-all-four-tables +
  stamp is genuinely atomic. Behavior: `version==1` proceed; `version==0` and
  `sqlite_master` empty → create+stamp; `version==0` **with tables present →
  fail closed, never adopt, never stamp** (unlike the provider, `broker.sqlite3`
  is a new filename with no legitimate unstamped predecessor); `version>1` fail
  closed.

- **This dissolves F5 rather than accommodating it.** If any ReadWrite touch
  creates both table families atomically, "file exists, my tables missing" is
  never legitimate, so ReadOnly keeps today's strict semantics unchanged. The
  only surviving early-return is `!db_path.exists()` → empty.

- **Merge scope stays "file merge first, `transactions` FK as a follow-up"**
  (operator's call this session), but **both reviewers independently concluded
  the case AGAINST the merge survives unless the FK follow-up is genuinely
  next**, not aspirational.

- **Operator redirect: ship the two live bugs first, alone, before the merge.**
  Follows the `secure_delete` precedent from `1acd`/`ccbb`.

## Evidence & Data

**LIVE BUG 1 — no `busy_timeout` anywhere in the CLI crate.** `grep -rn
"busy_timeout\|busy_handler" sudo-secretspec-cli/src/` → **nothing**. The
provider has one (`secretspec/src/provider/sqlite.rs:158`, 5s); neither ledger
connection does, so `SQLITE_BUSY` returns immediately. `open_connection`
(`audit.rs:431-529`) sets five pragmas and no busy timeout. Aggravator: every
append takes `BEGIN IMMEDIATE` and re-walks the **entire chain under the lock**
(`verify_rows`, `audit.rs:932` / `history.rs:387`), re-hashing every
`manifest_blob` (up to 16MB each, `history.rs:23`). Today the two O(n) lock
holds are in separate files; **the merge serializes them onto one lock.**
Failure modes, all verified in source: busy attempt → rc 2 denial
(`broker.rs:706-709`); busy terminal → **rc 126, outcome permanently
unrecorded, unterminated attempt left in the ledger** (`broker.rs:763-773`);
busy capture → swallowed warning, op reports success (`broker.rs:428-443`).

**LIVE BUG 2 — the manifest chain is never verified in production.**
`drift.rs:1036` verifies only `audit::verify_read_only`; there is no history
equivalent anywhere. Every caller of `history::verify` is at `broker.rs:1805`,
`1832`, `1855`, and `#[cfg(test)]` starts at `broker.rs:1560` — **all three are
test-only.** `history::verify_read_only` and `history::list` have zero
production callers. The manifest chain can rot, be truncated, or be tampered
with and nothing in `drift`, `doctor`, or any command path notices.

**F1-F5 verdicts** (F1/F2/F3/F5 confirmed by both reviewers; F4 downgraded):

- **F4 is real but nearly vacuous here.** Probe: source `sqlite_sequence`=3
  after a tail delete; after `INSERT INTO new SELECT * FROM old` the new value
  is 2 and the next insert re-allocates 3. **But** both ledgers compute
  `sequence = count + 1` in Rust (`audit.rs:933`, `history.rs:388`) and never
  DELETE, so `sqlite_sequence` is not the allocator of record. Keep the
  assertion (free), but it guards a state this code cannot produce.
- **F2 confirmed and load-bearing.** `canonical_event_json` (`audit.rs:572-629`)
  and `canonical_entry_json` (`history.rs:117-142`) hash field values only — no
  table or file name. So a row-preserving copy reproduces both chains, and
  tip-hash + count equality via the *existing* verifiers is a real proof. Bonus:
  `history.rs:187` binds every `manifest_blob` byte through its stored digest,
  so blob bytes are covered by re-verification despite not being in the entry hash.

**Findings that weigh against the merge, not yet resolved:**

- **The merge destroys the audit file's "digest-only, handable as-is" property**
  — the exact property `1acd` (lines 106-108) used to reject the three-way
  merge. Handing someone `broker.sqlite3` hands them every historical
  `secretspec.toml`. The "is manifest history shareable" question is still
  **unanswered**. A mitigation exists but is unbuilt: since `manifest_blob` is
  not in the entry hash, a strip-export could copy, drop the manifest tables,
  and `VACUUM`, leaving an audit-only artifact whose chain still verifies.
- **Old-binary-reports-clean hazard.** After migration a stale binary looks for
  `broker-audit.sqlite3`, doesn't find it, and reports **"0 events, tip ZERO" —
  clean** (`audit.rs:795-802` early return). An audit trail that vanishes *and
  reports clean* is the worst failure shape for this artifact. Migration must be
  gated on the new binary already being installed, and announced.
- **Single corruption domain**: one bad page can now take out both
  tamper-evidence chains; a hot journal from either blocks read-only
  verification of both (`audit.rs:777-779`'s documented trade widens).
- **The merge buys no cross-ledger atomicity as scoped** — attempt, capture and
  terminal remain three separate connections and transactions (`broker.rs:691`,
  `448`, `748`). One file does not make them one commit.
- **`1acd` line 276 concedes** the `vault_entry_hash` citation works *across
  files*, so a Rust-side transaction-UUID join is an alternative that needs no
  migration at all.

**Migration protocol** (agreed by both reviewers, for when the merge happens):
lock both sources with `BEGIN IMMEDIATE` in fixed order (audit then history) →
`cp` backups *under the locks* (a `cp` of a DELETE-journal db is only a valid
snapshot with no write txn in flight) → verify both chains, record
`(count, tip)`, **abort if either source chain fails** → build
`broker.sqlite3.migrating-<uuid>` in the same dir → one transaction: create four
tables, copy rows with explicit column lists preserving `sequence`, copy heads,
assert F4 high-water equality, stamp `user_version=1` → verify both chains in
the new file and require **exact `(count, tip)` equality** plus
`integrity_check` → fsync, chown/chmod 0600, fsync dir, `rename()` (the single
commit point), fsync, unlink old files. **Recovery rule belongs in the opener,
not a human**: both-present → compare tips, finish unlink if equal, hard abort
if not (split-brain from an old binary appending after the copy).

**Disk**: freed 33Gi (42Gi → 75Gi available) by deleting
`sudo-secretspec/target/debug/incremental` (19G), `~/src/ss-370/target` (5.9G),
`~/src/ss-sigpipe/target` (6.3G), and `brew cleanup --prune=all` (3.1G).
86M of `incremental` survived: it is **root-owned**
(`sudo_secretspec-0vsjdcov2n1xm`), left by something that built under sudo.

## Operator Feedback

- Wants second opinions from other models/TUIs on database work — "I know it can
  be very hard." Full model list must be enumerated, never `head`'d, and
  `-fast` variants never chosen (standing rule).
- Chose **Opus 5 / high effort** for this work, explicitly not Fast mode.
- Authorized the disk deletions and the `devenv` install.
- Chose: ship the two live bugs first, alone; write this handoff, then continue.

## Where We're Going

1. **THE NEXT ACTION — fix the missing `busy_timeout`.** Set it on the
   connections `open_connection` returns (`audit.rs:431-529`), which serves
   **both** ledgers via `open_protected_db`. Match the provider's 5s
   (`secretspec/src/provider/sqlite.rs:158`) unless there's reason to differ.
   This is a live gap in shipped `0.19.1-sudo.22` under concurrent agents, and
   it is a named precondition for the merge. Mutation-verify the test.
2. **Wire the manifest chain into production verification.** Add a
   `history::verify_read_only` call beside `drift.rs:1036`'s audit check, with
   its own finding code (e.g. `HISTORY_VERIFY_FAILED`). Today the manifest chain
   is verified only by test code.
3. **Then re-open the merge go/no-go** against the four conditions: (a)
   `busy_timeout` landed, (b) single-owner `broker_db` with atomic create+stamp
   replacing both `ensure_schema`s, (c) the both-present recovery rule
   implemented in the opener, (d) **the `transactions` FK follow-up is genuinely
   next, not aspirational.** If (d) is false, prefer the cross-file Rust-side
   transaction-UUID join and keep the files separate.
4. **Answer the unanswered question before merging**: is manifest history
   shareable? The merge forecloses the cheap `cp` answer for the audit ledger.
5. Read `cursor-grok-4.6-xhigh`'s review if it completed — it was still running
   when this was written. Raw reviews are in the session scratchpad
   (ephemeral; re-run if gone).
6. Unchanged from `ccbb` and still true: don't touch `PROMPT-REVIEW.md`,
   `PROMPT-SECREV.md`, `REVIEW_REPORT.md`, or branch
   `explore/pr-334-rust-first-spec` without asking. **DEFERRED, do not
   resurface:** cross-platform sudo/Linux port. Upstream `#377` still open;
   once it resolves,
   `git worktree remove /Users/djbclark/src/ss-sigpipe && git branch -D fix/sigpipe-default-disposition`.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec && git log --oneline -3   # expect ad6c368 + this handoff

# EVERY cargo command needs this env (system SQLite, not bundled):
export PKG_CONFIG_PATH=/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH \
       LIBRARY_PATH=/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH \
       CPATH=/opt/homebrew/opt/sqlite/include:$CPATH

# The baseline that actually works here. Expect 229 passed / 0 failed.
cargo test -p sudo-secretspec-cli
# `cargo test --all` needs `devenv shell` first (devenv 2.2.1 now installed);
# outside it, ext-php-rs fails on a missing php. Never pipe through `tail`
# without ${PIPESTATUS[0]} — it masks the exit code.

# Probe with the RIGHT binary. Bare `sqlite3` on PATH is the Android SDK build
# (3.50.6) and reports the OPPOSITE secure_delete default from what cargo links.
/opt/homebrew/opt/sqlite/bin/sqlite3 --version   # expect 3.53.4

# Confirm the two live bugs still exist before fixing them:
grep -rn "busy_timeout" sudo-secretspec-cli/src/            # expect NOTHING (bug 1)
grep -rn "history::verify" --include=*.rs . | grep -v ./target  # expect only broker.rs:1805/1832/1855, all after #[cfg(test)] at :1560 (bug 2)
```

**NEVER print a retrieved secret value.** `V=$(sudo-secretspec get <NAME>
--reason '...')` then `echo ${#V}` only. The live vault was not touched this
session; any live work must be announced to the operator first (the installed
boundary is shared infrastructure other agents call constantly).
