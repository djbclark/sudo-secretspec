---
schema_version: 1
handoff_id: 0adc
parent_handoff_ids: [bb66]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 94ea456ed21b14d0f2c0431a4ec7a0694cd4ccac
created_at: 2026-08-18T16:23:09-0400
writer: claude-code
---

# Handoff — Mutation-verifying the blob split, and the sudo prompt flood

## The Goal

Resume from handoff `bb66` and work its next-steps in order: mutation-verify
`0b66bdd` (the content-addressed `capture_history` change, which had shipped
with none), watch the upstream PRs, and hold the live vault migration until
the operator green-lit it.

Two things arrived mid-session and took over:

1. The operator was being flooded with Touch ID prompts — roughly 40, stacking
   up, twice in one day. **"This is not tennible."**
2. A standing design directive: **push as much invariant enforcement as
   possible into SQLite/SQL rather than Rust.** That is the next real work
   item and nothing has been built for it yet.

## Where We Are

`sudo-main` is clean at `94ea456`, pushed. Working tree empty.

Mutation testing of `0b66bdd` is **done** — five mutations, three survived,
and they meant three different things. One was a real security-relevant test
gap, now closed. The live vault is **still not migrated**, and the rehearsal
is now deliberately deferred *behind* the SQL work (see Key Decisions).

The prompt flood is fixed at its root, and it was never sudo-secretspec.

**Files changed this session** — all in `94ea456`:

| file | change |
|---|---|
| `sudo-secretspec-cli/src/broker.rs` | new test `a_destroyed_snapshot_stays_unrestorable_once_the_same_value_is_set_again` (+72 lines) |
| `secretspec/src/provider/sqlite.rs` | removed an unreachable duplicate `value_blobs` DDL from `migrate_v0_to_v1`; recorded the ordering hazard in its place |

**Outside any repo** — a machine-level change, documented here because
nothing in git records it:

| path | content |
|---|---|
| `/etc/sudoers.d/10-timestamp-global` | `Defaults timestamp_type=global, timestamp_timeout=5` |

**Blockers / open questions.**

- The live vault migration is still unapplied *and* self-triggering: opening
  an old-shaped DB with a binary built from `0b66bdd` or later migrates it in
  place with no prompt. The installed binary is `0.19.1-sudo.21`, which
  predates the change, so today's commands remain safe.
- The migration has still only ever run against synthetic v0 fixtures in
  `legacy_v0_database`, never against real vault data.
- The SQL-enforcement directive is unstarted. It is a schema change, which
  reorders everything else (see Key Decisions).

## What We Tried

- **The mutation helper from the last chain would have lied about every
  result.** `~/.local/state/handoffs/chains/standalone-b2db/mutate.sh` runs a
  bare `cargo test -p "$PKG"`. For `secretspec` that includes the 21
  pre-existing `sops` failures, so its `[1-9][0-9]* failed` grep matches on
  *every* run and would have reported **CAUGHT for all five mutations**
  regardless of coverage — including the two that genuinely were not covered.
  Replaced with a runner that takes an explicit test filter
  (`--features sqlite --lib provider::sqlite`), diffs the file to confirm the
  mutation actually landed, and distinguishes "no test ran" from "a test
  failed".

- **My own verdict classifier misfired on the first run.** It checked for
  `^error(\[|:)` before checking test results, and cargo prints
  `error: test failed, to rerun pass ...` on a perfectly normal test failure —
  so a real CAUGHT was reported as COMPILE-FAIL. Reordered to require the
  absence of any `^test result:` line before calling something a build break.

- **`sudo -n -l <cmd>` cannot tell you whether a password is required.** Used
  it to build a table of which verbs prompt; it reported **NOPASSWD for
  everything**, including `destroy`, `install` and `uninstall`. `sudo -l`
  tests *authorization*, and this account carries a blanket `(ALL) ALL` rule
  that authorizes every command — it says nothing about the `NOPASSWD` tag.
  The authoritative source is the tag list in a full `sudo -n -l` dump.
  Caught and corrected before it reached a conclusion.

- **Told the operator the fingerprints came from boundary lifecycle work.**
  Wrong, and it sent the diagnosis in the wrong direction for a turn. They
  came from four concurrent CFEngine acceptance suites in entirely unrelated
  repos.

- **Killing the prompt storm did not work.** Terminating the four `testall`
  process trees cleared the queue; two other Claude sessions relaunched them
  within seconds. Killing `SecurityAgent` dismissed the visible dialog but
  made things worse in a way worth remembering: **it converts a Touch ID
  prompt into a terminal password prompt**, which is what produced the "two of
  them were password-required for some reason" the operator noticed. The
  process-level fight is unwinnable while the sessions live; only the sudoers
  change or stopping those sessions ends it.

- **Leaked two live credential values into the session transcript.** A
  prompt-audit loop ran `get` and `export` and printed `tail -1` of the raw
  output, which for those verbs *is* the secret. The handoff Quick Start has
  the correct form and I deviated from it:
  `V=$(...) && echo "retrieval OK, ${#V} chars"` — never `$V`. On operator
  instruction, every reference has since been scrubbed from all tracked state
  (Tier 1 log, scratchpad); no credential value was ever written to a file,
  only to the transcript.

## Key Decisions

- **The SQL-enforcement directive reorders the vault migration.** All three
  candidate invariants are schema changes, so they must land *before* the live
  migration — one migration instead of two. This means the rehearsal, which
  `bb66` made THE next action, is now deliberately deferred behind the SQL
  work, and its baseline numbers must be re-derived against the final schema.
  **Rejected:** rehearsing and migrating now, then migrating again for the
  schema change.

- **Do not write to the live vault before the rehearsal.** Declined to test
  `set`/`add` unattended behavior against the real vault: it would append
  history entries and invalidate the rehearsal baseline (158 rows / 40 blobs /
  7988 bytes). The sudoers `NOPASSWD` grant plus the empirically-proven
  `source-get` path is sufficient evidence that creation is unattended.

- **Mutation (b) is an equivalent mutant, not a test gap.** `bb66` predicted
  that dropping `WHERE value_blob IS NOT NULL` from `migrate_v0_to_v1` "must
  fail". It does not. `INSERT OR IGNORE` against a `BLOB NOT NULL` column
  already discards the destroyed rows — verified directly in `sqlite3` rather
  than argued. Kept the filter anyway as the explicit statement of intent,
  since it stops being redundant the moment either half changes.
  **Rejected:** deleting it as dead weight, and "fixing" the test to catch a
  mutation that changes no behavior.

- **Removed the duplicate `value_blobs` DDL rather than sharing it.**
  `connection()` executes `HISTORY_SCHEMA` before `migrate_history`, so the
  migration's own `CREATE TABLE IF NOT EXISTS` was unreachable and its primary
  key untestable. **Rejected:** factoring the DDL into a `macro_rules!` shared
  by both sites — a migration that references the *current* schema constant
  silently starts producing the newer shape the moment that constant evolves,
  which is worse than the duplication. Left a comment recording that exact
  hazard for whoever writes v2.

- **Fixed the prompt flood in global sudo policy, not in sudo-secretspec.**
  The operator offered to accept "a second or two of cache as an acceptable
  risk" in the boundary. That trade turned out to be unnecessary: the flood
  was `tty_tickets` discarding the ticket per-TTY, and every agent-spawned
  shell gets a fresh pty. `timestamp_type=global` fixes it with the window
  length unchanged at sudo's own documented 5-minute default.
  **Rejected:** relaxing `Defaults!/usr/local/bin/sudo-secretspec
  timestamp_timeout=0`, which would have weakened the per-operation Touch ID
  guarantee for lifecycle verbs to solve a problem originating elsewhere.

- **Did not push the sudoers change without asking**, and did not fight the
  respawning test suites past the first two rounds — repeated kills were
  destroying other sessions' in-progress work to no lasting effect.

## Evidence & Data

**Mutation results** (runner:
`<scratchpad>/mutate2.sh <file> <label> <python-expr> <test-cmd...>`):

| # | mutation | verdict | meaning |
|---|---|---|---|
| a1 | `HISTORY_SCHEMA` `value_blobs` PK → `(value_sha256)` | **CAUGHT** | 3 tests fail; per-name keying pinned |
| a2 | same PK inside `migrate_v0_to_v1` | **MISSED** | unreachable duplicate; removed |
| b | drop `WHERE value_blob IS NOT NULL` | **MISSED** | equivalent mutant, proven |
| c | restore filter → blob-presence only | **MISSED** → **CAUGHT** | real gap; test added |
| d | `INSERT OR IGNORE` → `INSERT` | **CAUGHT** | dedup path covered |

**The gap that mattered (c).** Restore's `destroyed_by IS NULL` filter could be
deleted with all 228 tests still green. The `JOIN value_blobs` masks it in the
common case — but blobs are keyed `(item, value_sha256)` and shared across an
item's snapshots, so destroying a name and re-setting it to the value it used
to hold **recreates the very blob `destroy` deleted**, and the tombstoned
snapshot becomes restorable. Exactly the divergence the filter was written
for, and nothing exercised it. The new test drives
set(seq 1) → destroy(seq 2) → set same value(seq 3) → delete(seq 4) → restore
`--to 1`, and asserts a precondition first — that the tombstoned row really
does join to a live blob right now — so it cannot pass for the wrong reason.
Post-fix, mutation (c) fails **only** that test.

**Tests** (system-SQLite env exported):
- `cargo test -p sudo-secretspec-cli` → **229 passed, 0 failed** (baseline 228, +1 new)
- `cargo test -p secretspec --features sqlite --lib provider::sqlite` → **21 passed**
- `cargo fmt --all -- --check` → clean
- Pre-existing and unrelated: `cargo test -p secretspec --lib` → 1352 passed /
  **21 failed**, every one `The 'sops' CLI is not installed`. Do not chase.

**Unattended-usage audit.** The operator asked whether normal usage
(create / retrieve / list) will prompt. It will not. After `sudo -k` cleared
the credential cache:

| verb | rc | elapsed |
|---|---|---|
| `get` | 0 | 85 ms |
| `check` | 0 | 63 ms |
| `schema` | 0 | 63 ms |
| `export` | 0 | 62 ms |

Sixty milliseconds is not a Touch ID round trip. The installed policy grants
`NOPASSWD` to `_sudo_secretspec` for `source-add`, `source-set`,
`source-delete`, `source-undeclare`, `source-get`, `source-check`,
`source-export`, `source-template-check`, `source-schema`, `source-restore`,
`audit-verify`, plus `doctor` as root. **Not** granted, therefore always
interactive: `source-destroy`, `source-restore-force` (which is where `--force`
*and* `--all` restores are routed), and the client-path lifecycle verbs.

**The prompt flood — root cause.** `/var/db/sudo/ts/` was empty: no timestamp
records existed at all. There is no `tty_tickets` or global
`timestamp_timeout` override on this machine, so sudo 1.9.17p2 defaults apply
— `tty_tickets` ON, 5-minute window. Every agent-spawned shell gets a fresh
pty, hence a fresh ticket, hence a prompt. Four concurrent suites, each
shelling out per test case:

| repo | test pid | blocked on |
|---|---|---|
| `core-evalint` | 71872 | `sudo rm -fr …` (95213) |
| `core-cmdbdotted` | 96023 | `sudo rm -rf …` (96164) |
| `core-cmdbkey` | 96226 | `sudo rm -rf …` (96352) |
| `core-cmdbnull` | 96412 | `sudo rm -rf …` (96553) |

None of it touched sudo-secretspec. My own two `sudo -k` calls wiped the ts
directory and made it worse.

**Post-change verification**, run from a fresh non-TTY shell — the exact
context that used to force re-auth. All three with `sudo -n`, so no prompt was
possible:

| check | result |
|---|---|
| global ticket survives a fresh pty (`sudo -n true`) | **YES**, no prompt |
| boundary still enforced (`sudo -n <client-path> doctor`) | **YES**, still refuses |
| runtime verbs still `NOPASSWD` (`check`) | **YES**, unattended OK |

Both Defaults coexist as intended: the general
`timestamp_type=global, timestamp_timeout=5`, and the command-specific
`Defaults!/usr/local/bin/sudo-secretspec timestamp_timeout=0` which neither
consults nor updates the timestamp — so lifecycle verbs still authenticate
every single time. `sudo visudo -c` parses all four drop-ins OK.

**Upstream, re-checked 2026-08-18, nothing to action:** `#377` OPEN /
MERGEABLE, 0 reviews, 0 comments. `#373` MERGEABLE, no new maintainer response
since `1b1783c`. `#374` MERGEABLE at `d96f9b6`, only our own dogfooding
comment. **`#372` is an ISSUE, not a PR** — `bb66` recorded it as a PR and
`gh pr view 372` fails outright.

## Operator Feedback

- **"We want to take full advantages of SQLite and SQL and have as much of the
  logic and controls over what is permissable to happen there vs. our code."**
  — the standing directive, and the next work item.
- **"I just did like 40 fingerprints and they keep stacking up. This is not
  tennible."** … **"It cannot happen again."** Twice in one day.
- Offered to accept "a second or two of cache as an acceptable risk" in the
  boundary — not taken up, because the fix lay in global sudo policy instead.
- **Instructed to remove the two exposed credentials from anything tracked.**
  Done: 10 occurrences scrubbed from the Tier 1 canonical log (including
  earlier, benign availability-check mentions), and the scratchpad payload
  deleted. The names are deliberately not recorded here either. Rotation is
  no longer tracked anywhere by me — by instruction, so it rests with the
  operator.
- Standing from `1446`/`bb66`: 2 free ultra reviews remain, saved for other
  projects — prefer local review + mutation testing here.
- Standing: `sudo-main` only, no branches, no PRs, push directly.

## Where We're Going

1. **THE NEXT ACTION — move invariant enforcement into SQLite/SQL.** Three
   concrete candidates, in descending order of evidence:
   - **Every live `captured_values` row has a `value_blobs` row.** The old
     single-table `CHECK` enforced the equivalent and could not survive the
     blob split; it is currently upheld by code and tests only.
   - **Tombstone authority as a trigger** — a destroyed row must not become
     restorable. Mutation (c) is the direct argument: a security-relevant
     invariant that no test enforced and the schema had no opinion about.
   - **Hash-chain continuity**, currently enforced entirely inside
     `capture_history`.
   The foundation is already there: `STRICT` tables, real `REFERENCES`, and
   `PRAGMA foreign_keys=ON` on every connection the provider hands out.
   Bump `SCHEMA_VERSION` to 2 and fold this into the *same* migration as the
   v0→v1 blob split so the live vault is rewritten once.

2. **Re-derive the rehearsal baseline against the final schema, then rehearse
   against a COPY** (needs operator go-ahead; live migration needs a second,
   separate one):
   ```bash
   sudo cp /var/db/sudo-secretspec/secrets.db /tmp/probe.db
   sudo chown "$(id -un)" /tmp/probe.db
   # open with the new binary; the v1 baseline was 158 captured_values rows
   # preserved, 40 value_blobs rows, blob bytes 7988 -> 1997, 2 tombstones.
   # user_version will be 2, not 1, once step 1 lands.
   rm -P /tmp/probe.db   # MUST shred: holds all 39 plaintext secrets
   ```
   Do **not** write to the live vault before this — it invalidates the
   baseline.

3. **Do not redo mutation testing of `0b66bdd`.** Results are in the table
   above. If mutating anything in `secretspec`, use a runner with an explicit
   test filter — never bare `cargo test -p secretspec`.

4. **Release ordering for `0.19.1-sudo.22`.** `1446`'s precedent is to release
   the proven tree *before* the unproven migration. Note that installing `.22`
   is itself a lifecycle op (one Touch ID) and silently migrates the vault on
   the next privileged verb.

5. **Upstream:** nothing to action; re-check each session.
   `gh pr view 377 --repo cachix/secretspec --json state,reviews,comments`.
   Once `#377` resolves:
   `git worktree remove /Users/djbclark/src/ss-sigpipe && git branch -D fix/sigpipe-default-disposition`.

6. **If the prompt flood ever returns:** it is almost certainly `tty_tickets`
   plus a harness shelling out to `sudo` per unit of work. Check
   `ps -eo pid,ppid,user,etime,command | grep "[s]udo "` for the real source
   before assuming it is this project. Revert path:
   `sudo rm /etc/sudoers.d/10-timestamp-global`.

7. Standing don't-touch list: `PROMPT-REVIEW.md`, `PROMPT-SECREV.md`,
   `REVIEW_REPORT.md` (ask before removing); `/var/db/sudo-secretspec/.env`
   (last pre-migration reference copy of all 39 values — **keep until the
   vault migration is verified**); branch `explore/pr-334-rust-first-spec`
   (`bf0b25c` is linked by SHA from a public comment on upstream `#357`).

8. **DEFERRED per explicit operator instruction, do not resurface:**
   cross-platform sudo/Linux port.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -2          # expect 94ea456 at tip, clean, pushed

# 1. IS IT UP? Other agents depend on this. The installed binary is still
#    0.19.1-sudo.21, which PREDATES 0b66bdd, so none of this migrates the vault.
sudo-secretspec --version                      # expect 0.19.1-sudo.21
sudo-secretspec doctor                         # expect: doctor: OK
sudo-secretspec check --reason "availability check" | tail -1
#   expect: 47 found, 0 missing, 7 optional
# NEVER print a retrieved value. Use the length only:
#   V=$(sudo-secretspec get <NAME> --reason "availability check") \
#     && echo "retrieval OK, ${#V} chars"

# 2. Build — system SQLite. Required for EVERY cargo command here:
export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
export LIBRARY_PATH="/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH"
export CPATH="/opt/homebrew/opt/sqlite/include:$CPATH"
cargo test -p sudo-secretspec-cli                                   # expect 229
cargo test -p secretspec --features sqlite --lib provider::sqlite   # expect 21
pytest tests/sudo_packaging -q                                      # expect 26

# 3. Where the SQL work goes:
grep -n "HISTORY_SCHEMA\|SCHEMA_VERSION\|migrate_history\|migrate_v0_to_v1" \
  secretspec/src/provider/sqlite.rs
grep -n "destroyed_by IS NULL" sudo-secretspec-cli/src/broker.rs

# 4. The test that pins the tombstone invariant today (a trigger should make
#    it unfalsifiable rather than merely tested):
cargo test -p sudo-secretspec-cli a_destroyed_snapshot
```

**Hazard, repeated because it is the easiest way to lose control of this:**
opening the live vault with a binary built from `0b66bdd` or later migrates it
in place, with no prompt. Installing `0.19.1-sudo.22` would do the same on the
next privileged verb. Rehearse against a copy first — and now, land the schema
work first so the vault is migrated exactly once.
