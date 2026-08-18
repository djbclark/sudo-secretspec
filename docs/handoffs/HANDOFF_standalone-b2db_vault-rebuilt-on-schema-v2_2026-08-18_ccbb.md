---
schema_version: 1
handoff_id: ccbb
parent_handoff_ids: [1acd]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 840447b8bf1eba81c597af339904387b0d006e9d
created_at: 2026-08-18T18:35:21-0400
writer: claude-code
---

# Handoff — vault rebuilt on schema v2, `destroy` finally shreds

## The Goal

Resume from `1acd` and execute its stated next action: ship the
`secure_delete` fix, then implement the v2 schema that had been designed but
not written.

All of that landed, plus three things `1acd` did not anticipate: the release
it planned turned out to collide with its own migration plan; the operator
redirected the live migration into a **rebuild**; and the migration code became
dead and was removed. **The live vault now runs schema v2 and `.env` is gone.**

## Where We Are

`sudo-main` clean at `840447b`, everything pushed, nothing in flight.

**Five commits this session:**

| commit | what |
|---|---|
| `b5bdf61` | `PRAGMA secure_delete=ON` on all three connection paths + test |
| `9f29747` | `review/` (the four schema reviews), committed on operator go-ahead |
| `6a683f8` | schema v2 — invariants moved into the schema |
| `7794b52` + `5fe7b23` | release machinery for `v0.19.1-sudo.22` |
| `840447b` | migrations removed, version guard in their place |

**Live system state, all verified after the cutover:**

| thing | state |
|---|---|
| installed binary | `0.19.1-sudo.22` |
| `secrets.db` | **schema v2**, 49152 bytes, `_sudo_secretspec:_sudo_secretspec` `0600` |
| `user_version` | 2, both triggers present |
| history | **0 entries, 0 captured_values** — discarded by the rebuild, on purpose |
| `doctor` | OK |
| `check` | 47 found / 0 missing / 7 optional |
| value round-trip | verified on two credentials, lengths only, values never printed |
| `.env` | **deleted** (`rm -P`) after proving redundancy |
| rollback snapshot | `/usr/local/libexec/sudo-secretspec-rollback-1787091723`, 8 artifacts |

Tests: **229 / 25 / 26** green, `cargo fmt` clean. The provider count moved
21 → 22 → 28 → 25 across the session: +1 for the `secure_delete` test, +6 for
the v2 invariant and migration tests, then −3 when the migration tests were
removed along with the code they covered.

**Blockers: none.** The one consequence worth stating plainly is that `restore`
has nothing to restore until new history accumulates, because the rebuild
discarded 158 snapshots and 2 tombstones. That was the operator's explicit
choice. The audit ledger (`broker-audit.sqlite3`) and manifest chain
(`broker-history.sqlite3`) are **separate files** and were never touched.

## What We Tried

- **`1acd` said "release `0.19.1-sudo.22` now." I stopped and did not.** Its
  own plan also said "migrate the live vault v0→v2 direct, one irreversible
  rewrite." Those collide: `connection()` calls `migrate_history()`
  unconditionally and `SCHEMA_VERSION` was 1, so *installing* `.22` would have
  silently taken the live vault v0→v1 on the next privileged verb and destroyed
  the v0→v2-direct path. Writing the pragma was zero-risk; shipping the binary
  carrying it was not. Raised as a three-option gate; operator chose **(a) hold
  the release**, which also folded away the open `VACUUM` question, since the
  migration already vacuums after commit.

- **`1acd` named two connection sites that need the pragma. There are three.**
  `audit::open_protected_db` (`sudo-secretspec-cli/src/audit.rs` ~479) serves
  **both** ledgers — `history.rs` routes through it. Defense-in-depth there
  rather than a live gap, since both ledgers are append-only and hold only
  digests and the declaration manifest, but any future per-connection pragma
  must be set in all three.

- **Put `CREATE INDEX ... ON captured_values(blob_id)` in the schema. Wrong
  place, and it failed loudly.** The schema runs *before* migration, against a
  table `CREATE TABLE IF NOT EXISTS` leaves in whatever shape it already had —
  so on an unmigrated database it died with `no such column: blob_id`. This is
  exactly the ordering hazard `1acd` warned about. Moved to
  `HISTORY_POST_MIGRATION`, applied after the version check.

- **Wrote a test that immediately started passing for the wrong reason.**
  `foreign_keys_are_enforced_on_every_connection_this_provider_hands_out` insert
  an orphan row to prove the FK bites — but my new `CHECK (blob_id IS NOT NULL
  OR destroyed_by IS NOT NULL)` fired *first*, so the assertion passed without
  the FK ever being consulted. This is the third time this exact class of defect
  has appeared in this file (the comment above that test documents the prior
  two). Fixed by supplying a real blob and asserting on `FOREIGN KEY`
  specifically, not just `ConstraintViolation`.

- **Hand-wrote an entry in a test without advancing `head`.** `capture_history`
  derives the next sequence from `head`, so the subsequent `set` collided with
  `UNIQUE constraint failed: entries.sequence`. Any test that seeds an entry by
  hand must update `head` too.

- **Tried `sha256()` in SQLite for the `.env` redundancy check.** Not compiled
  into either local build. Did the comparison in a single in-memory Python
  process instead — which is strictly better anyway, because the obvious
  alternative (dump values to a file, digest the file) would have written all 39
  plaintext values back to disk while trying to prove a plaintext file was safe
  to delete.

- **Rehearsed the cutover the obvious way first, and measured it as wrong.** See
  Key Decisions.

## Key Decisions

- **Hold `.22` rather than release it mid-schema (operator, option (a)).**
  Rejected: releasing and accepting a v0→v1 hop (two irreversible rewrites);
  releasing with the migration gated off behind a flag.

- **Identity `blob_id` + a generated `live_blob_id`, which is neither option
  `1acd` framed.** `1acd` left "generated `blob_sha256` vs identity `blob_id`"
  as the open call. Probing on the linked build found a shape that takes both
  wins:

  ```sql
  value_blobs(blob_id INTEGER PRIMARY KEY AUTOINCREMENT, item, value_sha256,
              value_blob, UNIQUE(item, value_sha256))
  captured_values(..., blob_id INTEGER, destroyed_by INTEGER,
    live_blob_id INTEGER GENERATED ALWAYS AS
      (CASE WHEN destroyed_by IS NULL THEN blob_id END) VIRTUAL
      REFERENCES value_blobs(blob_id) ON DELETE RESTRICT)
  ```

  `destroy` stays a single-column write; `ON DELETE RESTRICT` replaces the
  broker's hand-written `NOT IN` guard; and **resurrection becomes a foreign-key
  failure instead of a trigger's responsibility.** Under the generated-
  `blob_sha256` variant `1acd` verified the identical sequence *resurrected* —
  which is why it concluded the write-once trigger was load-bearing. Here the
  trigger drops to belt-and-braces. Kept anyway.

- **`AUTOINCREMENT` on `value_blobs` is load-bearing.** Mutation-verified: with
  a plain `INTEGER PRIMARY KEY` the recreated blob reclaims the freed rowid, the
  dangling reference becomes valid again, and the tombstone **resurrects**.
  SQLite's own documentation discourages `AUTOINCREMENT` on performance grounds;
  following that advice here silently reopens the hole. Documented as such in
  the schema comment.

- **Rebuild the live vault instead of migrating it (operator).** "Just export
  the current database and import into the new one. We don't have to worry about
  keeping audit details."

- **Provider-to-provider `import`, not an `.env` round trip (mine, on the
  operator's idea).** `secretspec import` reads one provider and writes another
  directly, so no plaintext file is ever created. An `.env` intermediate would
  have recreated precisely the exposure we were about to delete.

- **Import with history OFF, then stamp with history ON — two phases.**
  Rejected after measuring it: importing straight into a `history=true` database
  is value-perfect but manufactures **39 entries and 780 `captured_values`
  rows**, because `import` calls `set()` per secret and every `set` snapshots
  every live secret. The operator asked for a clean rebuild; that is its
  opposite.

- **Snapshot the source under the OLD binary, before installing `.22`.**
  Improves on my own recorded procedure, which had install first and rested on
  "the in-place migration is harmless since we're replacing the file anyway."
  Copying first makes the source read *provably* pristine v0 and removes the
  need to trust that argument at all.

- **Replace the removed migrations with a guard, not with nothing.** The
  operator asked for the dead code deleted. Deleting it outright would have been
  unsafe: `CREATE TABLE IF NOT EXISTS` silently accepts an old-shaped table, so
  an upgraded binary would write against columns that are not there — the exact
  failure the migration machinery was added to prevent.
  `check_history_version` now refuses a database older *or* newer than
  `SCHEMA_VERSION`.

- **Table bodies defined once and shared with the migrations** (while they
  existed). An earlier revision kept a second copy of the schema inside the
  migration; it was unreachable and drifted undetectably, which is how mutation
  testing found it.

## Evidence & Data

**`destroy` never shredded — the finding `1acd` opened, now closed.**
`PRAGMA secure_delete` is 0 in the linked Homebrew SQLite 3.53.4 and appeared
nowhere in the codebase. Mutation check on the new test is itself the proof:
with the pragma removed, the **anti-vacuity precondition passed** (canary
readable in the raw file) and the **post-delete assertion failed** (still
readable). That combination demonstrates the live bug, not merely a missing
line.

**Mutation results this session — 3 run, 3 CAUGHT:**

| mutation | result |
|---|---|
| remove `PRAGMA secure_delete=ON` | CAUGHT — deleted plaintext readable in file |
| revert restore's join to `(item, value_sha256)` matching | CAUGHT — "a destroyed snapshot must never restore", with **no** `destroyed_by` filter present anywhere |
| `AUTOINCREMENT` → plain `INTEGER PRIMARY KEY` | CAUGHT — tombstone resurrects |
| disable the older-version branch in `check_history_version` | CAUGHT |

The second is the one that matters most: it shows the *structure* carries the
invariant, rather than a `WHERE` clause that mutation testing had already shown
was deletable with the suite green.

**Cutover verification, before swapping anything in:**

| check | result |
|---|---|
| `user_version` | 2 |
| secrets | 39 |
| entries / captured_values | 0 / 0 |
| triggers + `captured_values_blob_id` index | present |
| exact item+value matches vs source | **39 / 39** |
| mismatches / only-in-new / only-in-old | 0 / 0 / 0 |
| `integrity_check`, `foreign_key_check` | ok, no violations |

Source snapshot taken under `.21` was pristine v0: 39 secrets, 158
`captured_values`, 4 entries, 2 tombstones — matching `1acd`'s recorded
baseline exactly. `import` reported `39 imported, 0 already exists, 15 not found
in source` in both rehearsal and live run.

**`.env` retirement.** 39 entries in the file, 39 in the vault, **39/39 exact
SHA-256 matches, 0 differing, 0 present only in `.env`.** Compared in one
in-memory Python process; no plaintext written to disk. Then `rm -P`, along with
every scratch copy (`/tmp/cut-src.db`, `/tmp/cut-new.db`,
`/tmp/cut-old-rollback.db`, the rehearsal copies).

**Vault directory now:** `.state/`, `broker-audit.sqlite3` (700416),
`broker-history.sqlite3` (745472), `secrets.db` (49152), `secretspec.toml`
(9799). No `.env`.

## Operator Feedback

- **"a"** — chose holding `.22` over releasing it mid-schema, when given the
  three-option gate.
- **"You can commit review."**
- **"I think we know what we are doing now, I thought it was a clear decision,
  tell me if i am wrong, but I think we are ready to implement v2."** — they
  were right; answered directly and started.
- **"I think it might more sense to just export the current database to a .env
  file and then import the .env file into the new database. We don't have to
  worry about keeping audit details."** — took the idea, dropped the `.env`
  intermediate for a direct provider-to-provider import, and corrected one
  premise: the audit ledger is a separate file and survives regardless; what
  the rebuild discards is `secrets.db`'s *value* history.
- **"once we are migrated, we should remove all of the code regarding previous
  schema versions, as I am the only person using this so far."** — done, with a
  guard in its place.
- **"i want to change the account to one with more usage left right before the
  actual migration, so pause and tell me when that is."** — paused immediately
  before the first write to the live vault, with the full procedure recorded so
  it survived the switch.
- Standing, unchanged: `sudo-main` only, no branches, no PRs, push directly.
  2 free ultra reviews remain, saved for other projects.

## Where We're Going

1. **THE NEXT ACTION — the two-database file layout.** The last unstarted item
   from `1acd`'s design, fully reviewed and unblocked: `secrets.db` (generic
   provider) stays alone; a new `broker.sqlite3` takes `transactions` +
   `manifest_entries` + `audit_events` and their heads. Cut by **owner** — all
   four reviews rejected one file, they split 2–2 on where to cut, and cut-by-
   owner wins because the `transaction_id` FK is broker↔broker while the
   competing cut leaves those tables in different files. Rationale and rejected
   alternatives: `HANDOFF_standalone-b2db_sql-invariants-designed_2026-08-18_1acd.md`,
   Key Decisions.

2. **Optional hygiene: `VACUUM` the two ledgers.** `secure_delete` only zeroes
   pages freed from now on. The rebuilt `secrets.db` is a fresh file and carries
   no residue, but `broker-history.sqlite3` and `broker-audit.sqlite3` predate
   the pragma. They hold no secret values — digests and the declaration manifest
   only — so this is tidiness, not exposure.

3. **Upstream, unchanged, nothing to action.** `#377` open, `#373`/`#374`
   mergeable, `#372` is an issue not a PR.
   `gh pr view 377 --repo cachix/secretspec --json state,reviews,comments`

4. **Once `#377` resolves:** `git worktree remove /Users/djbclark/src/ss-sigpipe
   && git branch -D fix/sigpipe-default-disposition`

5. Standing don't-touch list, unchanged: `PROMPT-REVIEW.md`, `PROMPT-SECREV.md`,
   `REVIEW_REPORT.md` (ask before removing); branch
   `explore/pr-334-rust-first-spec`. **DEFERRED, do not resurface:**
   cross-platform sudo/Linux port.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect 840447b at tip, clean tree

# 1. IS IT UP? Other agents depend on this.
sudo-secretspec --version                      # expect 0.19.1-sudo.22
sudo-secretspec doctor                         # expect: doctor: OK
sudo-secretspec check --reason "availability check" | tail -1
#   expect: 47 found, 0 missing, 7 optional
# NEVER print a retrieved value:
#   V=$(sudo-secretspec get <NAME> --reason "...") && echo "OK, ${#V} chars"

# 2. Build env — required for EVERY cargo command here:
export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
export LIBRARY_PATH="/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH"
export CPATH="/opt/homebrew/opt/sqlite/include:$CPATH"
cargo test -p sudo-secretspec-cli                                   # expect 229
cargo test -p secretspec --features sqlite --lib provider::sqlite   # expect 25
pytest tests/sudo_packaging -q                                      # expect 26

# 3. PROBE WITH THE RIGHT BINARY. `sqlite3` on PATH is the Android SDK build
#    (3.50.6) and reports secure_delete=1 — the OPPOSITE of what cargo links.
/opt/homebrew/opt/sqlite/bin/sqlite3 :memory: "PRAGMA secure_delete;"  # 0
#    sha256() is compiled into NEITHER build. Do digest comparisons in Python,
#    in memory, never writing plaintext to disk.

# 4. Where the next work goes:
grep -n "DB_NAME" sudo-secretspec-cli/src/history.rs sudo-secretspec-cli/src/audit.rs
grep -n "open_protected_db\|open_connection" sudo-secretspec-cli/src/audit.rs
```

**Hazards, repeated because they are the easiest ways to lose control:**

- **There is no migration path anymore, by design.** `check_history_version`
  *refuses* a database older or newer than `SCHEMA_VERSION`. If a future schema
  change needs a migration, it goes in that function, branching on
  `user_version` **before** the schema is created — and the index/triggers stay
  in `HISTORY_POST_MIGRATION`, after it.
- **Three production connection paths**, all now setting `secure_delete`:
  `secretspec/src/provider/sqlite.rs` `connection()`;
  `sudo-secretspec-cli/src/broker.rs` ~1162; `audit::open_protected_db` in
  `sudo-secretspec-cli/src/audit.rs` ~479, which serves **both** ledgers.
- **Never bare `cargo test -p secretspec` when mutating** — 21 pre-existing
  `sops` failures produce a false CAUGHT for every mutation. Always
  `--features sqlite --lib provider::sqlite`.
- **`AUTOINCREMENT` on `value_blobs` is load-bearing.** Do not "clean it up."
- **System change from 2026-08-18, recorded in no repo:**
  `/etc/sudoers.d/10-timestamp-global` containing
  `Defaults timestamp_type=global, timestamp_timeout=5`.
  Revert: `sudo rm /etc/sudoers.d/10-timestamp-global`.
