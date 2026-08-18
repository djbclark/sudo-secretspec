---
schema_version: 1
handoff_id: 1acd
parent_handoff_ids: [0adc]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 7d27d5dab832a0c5609063bdc0e7ff38d6ae5e99
created_at: 2026-08-18T17:06:58-0400
writer: claude-code
---

# Handoff — SQL invariants designed, and `destroy` never shredded

## The Goal

Resume from `0adc` and execute its stated next action: **move invariant
enforcement into SQLite/SQL** per the standing operator directive, bumping
`SCHEMA_VERSION` to 2 so the live vault migrates exactly once.

The session became a **design pass, not an implementation pass**, by operator
direction ("No let's talk more about the SQL stuff and 1 vs 2 files etc. not
done there yet"). The design is now settled and externally reviewed. **No code
was written. No commit other than this handoff. The live vault was not
touched.**

One finding displaced the whole agenda: **`destroy` has never actually shredded
plaintext.** See Evidence.

## Where We Are

`sudo-main` clean at `7d27d5d`, unchanged from session start. Working tree has
one untracked directory, `review/`, added by the operator — it holds one of the
four schema reviews. It is **not** committed by this handoff; ask before
committing it.

Live vault confirmed **still v0**: `secrets.db` has tables `secrets`,
`entries`, `captured_values`, `head` — **no `value_blobs`** — and
`user_version = 0`. The blob split from `0b66bdd` has never been applied to it.
Installed binary remains `0.19.1-sudo.21`, which predates `0b66bdd`, so nothing
run this session could have migrated it.

**Files changed this session:** none, other than this handoff document.

**Blockers / open questions.**

- `secure_delete` is OFF in the linked library and never set in code, so
  `destroy` leaves plaintext on freelist pages. This is a live gap in shipped
  `0.19.1-sudo.21`, independent of all schema work.
- Live vault migration still unapplied and still self-triggering: opening an
  old-shaped DB with a binary built from `0b66bdd` or later migrates it in
  place with no prompt.
- Whether to commit `review/`. Operator decision.
- Whether the manifest history is shareable with a third party — decides
  whether `audit export` must strip it. Unanswered.

## What We Tried

- **My first round of SQLite probes ran against the wrong binary.** `which
  sqlite3` on this machine resolves to
  `~/Library/Android/sdk/platform-tools/sqlite3` (3.50.6), **not** the Homebrew
  build the Rust code links (`/opt/homebrew/opt/sqlite`, 3.53.4). Every
  Proposal-A conclusion was re-verified against 3.53.4 and all held, but the
  provenance was wrong for several turns and I reported "verified on 3.50.6"
  when that was an unrelated Android platform-tools build. **Always probe with
  `/opt/homebrew/opt/sqlite/bin/sqlite3`, never bare `sqlite3`.**

- **I claimed "the CHECK alone blocks resurrection." Too strong, and it
  matters.** `CHECK ((destroyed_by IS NULL) = (blob_sha256 IS NOT NULL))`
  blocks `SET destroyed_by = NULL` on its own, but **not**
  `SET destroyed_by = NULL, blob_sha256 = value_sha256` once the blob exists
  again — which is exactly the destroy-then-re-set-same-value case that
  mutation (c) found. Verified by direct probe: with the generated-column
  variant the row **resurrected**. Consequence: the write-once trigger on
  `destroyed_by` is **load-bearing, not belt-and-braces**. Say so in the schema
  notes so nobody drops it believing a CHECK covers it.

- **Chased a suspected latent bug that was not one.** Two different `entries`
  tables exist with incompatible schemas, both created with
  `CREATE TABLE IF NOT EXISTS` — the provider's (in `secrets.db`) and the
  broker's manifest ledger (in `broker-history.sqlite3`). They are in
  **different files**, so there is no collision. Worth knowing before someone
  else finds the same thing and panics.

- **Started implementing before the design was settled**, kicking off a cargo
  baseline run. Operator stopped it. The baseline it produced is still valid
  and recorded below.

- **Considered and rejected merging all three databases into one file.** All
  four reviews independently said no. See Key Decisions.

## Key Decisions

- **Two files, not one and not three — and cut by OWNER.**
  `secrets.db` (generic provider, plaintext) stays alone;
  `broker.sqlite3` gets `transactions` + `manifest_entries` + `audit_events`
  and their heads. The reviews split **2–2** on where to cut; this breaks the
  tie. Rationale: the `transaction_id` FK is the strongest technical argument
  for merging anything, and it is a **broker↔broker** relationship — the
  competing `vault+manifest | audit` cut leaves those two tables in *different
  files* and therefore delivers none of it, while contaminating the generic
  provider's file with broker DDL and colliding on `user_version` (one integer,
  two schema owners).
  **Rejected:** all three in one file (destroys the audit ledger's
  survivability and its digest-only handable property); `vault+manifest |
  audit`; leaving all three separate (gives up the one FK worth having).

- **The operator's "entire state in a single file" is overruled by the
  reviews, and was relayed as such.** The reachable goal is "one database per
  privilege domain," not one file — `secretspec.toml` stays a file (it is
  human-authored, diffed by the drift checker, and has rollback copies) and the
  XDG `audit.log` belongs to upstream.

- **`journal_mode=DELETE` stays. Settled, and not on performance grounds.**
  In WAL mode `cp secrets.db` is not a valid snapshot — recent commits can live
  only in `-wal` — and the entire rehearsal procedure and backup story are
  `cp`-based. Operator explicitly discounted "minor performance issues," which
  removes WAL's only benefit here.
  **Rejected:** WAL (adds `-wal`/`-shm` sidecars, breaks `cp`).

- **`secure_delete` fix ships FIRST and ALONE**, ahead of all schema work.
  It is a pragma plus tests with no schema risk; it closes a live gap in
  shipped software; and it must be in place *before* the migration regardless,
  since the v0→v2 rebuild frees every page of the old plaintext table. Follows
  `1446`'s precedent of releasing the proven tree before the unproven
  migration. Operator delegated the ordering call explicitly.

- **Skip v1 on the live vault; migrate v0→v2 directly**, one transaction,
  `user_version` last. The vault has never had the blob split applied, so there
  is no reason to take two irreversible rewrites.

- **Prefer the GENERATED column over a writable one** for `blob_sha256` — but
  only alongside the write-once trigger. Verified working as an FK child on
  3.53.4. `destroy` collapses to a single-column write and the column cannot be
  desynchronized by anyone. One reviewer preferred an identity `blob_id` on
  `value_blobs` instead, on the grounds that it makes the buggy
  `JOIN USING (item, value_sha256)` a type error rather than a live hazard —
  **this is a genuinely open call and is the first thing to settle when
  implementation starts.**

- **Keep three hash chains and three `head` tables.** Unanimous across
  reviews. The provider is generic `sqlite://` code that must not learn what a
  broker transaction is.
  **Rejected:** one merged chain; a unified `chain_heads` table.

- **Forbid `DELETE` on `captured_values`**; keep the tombstone `UPDATE`.
  Unanimous, except one review that wanted full append-only — rejected because
  liveness would become a historical predicate, the exact class of predicate
  mutation testing showed nobody notices disappearing.

- **Fail closed on legacy inconsistency during migration.** Unanimous.

- **Did not commit `review/`** — not mine to stage, and the handoff commit is
  handoff-only by protocol.

## Evidence & Data

**THE finding — `destroy` does not shred.** Confirmed against the exact library
the binary links:

| binary | version | `PRAGMA secure_delete` |
|---|---|---|
| `/opt/homebrew/opt/sqlite/bin/sqlite3` (**what cargo links**) | 3.53.4 | **0** |
| `/usr/bin/sqlite3` (system) | 3.51.0 | 2 |
| `~/Library/Android/.../sqlite3` (**what is on `PATH`**) | 3.50.6 | 1 |

`grep -rn secure_delete --include=*.rs .` → **zero hits**. So
`DELETE FROM value_blobs` and v0's `UPDATE … SET value_blob = NULL` leave the
bytes on freelist pages until incidentally overwritten. Note `secure_delete=ON`
does **not** retroactively zero already-freed pages — clearing existing residue
requires `VACUUM`.

**Proposal A, verified on 3.53.4 (the linked build):**

| probe | result |
|---|---|
| composite FK with one NULL child column | vacuous for tombstones, binding for live rows |
| live row whose blob is absent | `FOREIGN KEY constraint failed` |
| `ON DELETE RESTRICT` vs a live reference | refused |
| GENERATED column as FK child | **accepted** |
| `RESTRICT` through the generated column | refused |
| `destroy` = single-column `UPDATE destroyed_by` | generated col auto-NULLs, blob delete then succeeds |
| clear `destroyed_by` after same-value re-set | **RESURRECTED** — trigger is the only guard |
| `DROP TABLE` firing `BEFORE DELETE` triggers | does **not** fire |
| rolled-back INSERT burning an AUTOINCREMENT value | **does not** — sequences `1,2`, `sqlite_sequence`=2 |

That last row **refutes** one reviewer's argument against a contiguity trigger
("benign rollbacks will permanently brick the chain"). `AUTOINCREMENT` state is
transactional. The objection does not survive.

**Live vault inventory** (`/var/db/sudo-secretspec/`, all `0600`, owner
`_sudo_secretspec`, directory `0700`):

| thing | size | tables / note |
|---|---|---|
| `secrets.db` | 73 K | `secrets`, `entries`, `captured_values`, `head` — **v0**, `user_version=0` |
| `broker-history.sqlite3` | 745 K | `entries`, `head` — manifest chain |
| `broker-audit.sqlite3` | 688 K | `events`, `head` — digest-only chain |
| `secretspec.toml` | 9.8 K | declarations, drift-checked, has rollback copies |
| `.env` | 2.8 K | pre-migration plaintext copy of all 39 values |
| `.state/secretspec/audit.log` | 58 K | **upstream** XDG per-user log, not ours |

Three reviewers independently flagged `.env` as the largest plaintext exposure
in the system, and noted it is gated on nothing.

**Tests** (system-SQLite env exported):
- `cargo test -p secretspec --features sqlite --lib provider::sqlite` →
  **21 passed, 0 failed** (baseline unchanged, pre-change).
- No other suites run this session; no code changed.

**Availability check, start of session:** `0.19.1-sudo.21`, `doctor: OK`,
`check` → 47 found / 0 missing / 7 optional.

**Upstream, unchanged from `0adc`:** nothing to action. `#377` open, `#373` /
`#374` mergeable, `#372` is an issue not a PR.

**Design artifact:** the 17 KB / 18-question review request is at
`<scratchpad>/schema-questions.md` (session-scoped, will not survive). Its
content is reproduced in substance by this handoff plus `review/`.

## Operator Feedback

- **"From a sysadmin POV it is awesome to have entire state in a single file
  rather than 3."** Stated twice. Answered: all four reviews said two files;
  relayed plainly rather than softened. The audit ledger must not share a file
  with the vault.
- **"If we don't care about minor performance issues, does the answer here
  become clear?"** — yes, and the deciding reason was `cp` validity, not speed.
- **"No let's talk more about the SQL stuff and 1 vs 2 files etc. not done
  there yet"** — stopped implementation mid-command. Design first.
- **"Add the ledger question to the fable query as well as everything else."**
  Done; became section 3b.
- **"You can do the secure_delete work in whatever order or combined you think
  would be more efficient."** — ordering delegated. Chose first-and-alone.
- Asked what the use case for extracting the digest-only ledger even is. Answer
  that survived review: backup asymmetry, forensics, compliance — all
  recoverable via an `audit export` verb — plus **tamper evidence surviving
  vault loss**, which is *not* recoverable by any export and is the real reason
  to keep the files separate.
- Standing from `1446`/`bb66`/`0adc`: 2 free ultra reviews remain, saved for
  other projects. `sudo-main` only, no branches, no PRs, push directly.

## Where We're Going

1. **THE NEXT ACTION — fix `secure_delete`, then release `0.19.1-sudo.22`.**
   Set `PRAGMA secure_delete=ON` in the provider's `connection()` init batch
   (alongside the existing `foreign_keys` / `journal_mode` / `synchronous` /
   `trusted_schema` pragmas, `secretspec/src/provider/sqlite.rs` ~line 164) and
   on the broker's own connection (`sudo-secretspec-cli/src/broker.rs` ~line
   1162, where `foreign_keys=ON` is already set). Add a test that proves it
   bites. Changelog entry required (user-facing: destroy now zeroes freed
   pages). Note the pragma does not clean existing residue — decide whether
   `.22` also runs a one-time `VACUUM`.

2. **Then the v2 schema.** First decision to settle: **generated
   `blob_sha256` vs identity `blob_id`** on `value_blobs`. The `blob_id`
   argument is that it makes `JOIN USING (item, value_sha256)` — the exact
   mutation-(c) bug — impossible to write, rather than merely wrong. Then:
   composite FK with `ON DELETE RESTRICT`; `CHECK ((destroyed_by IS NULL) =
   (blob_sha256 IS NOT NULL))`; `CHECK (destroyed_by IS NULL OR destroyed_by >
   sequence)`; write-once trigger (**load-bearing**); no-`DELETE` trigger; add
   `CREATE INDEX ON captured_values(item, blob_sha256)` — the `(sequence,item)`
   PK does not serve the parent-side delete check.

3. **Trigger lifecycle:** do **not** rely on `CREATE TRIGGER IF NOT EXISTS` to
   refresh a body — it matches by name and silently keeps a stale body forever.
   Use `DROP TRIGGER IF EXISTS` + `CREATE`, applied *after* `migrate_history`.
   Branch on `user_version` **before** running `CREATE TABLE IF NOT EXISTS`;
   that ordering is what dropped triggers in the v0→v1 design.

4. **Then the file merge** (`secrets.db` | `broker.sqlite3`), with the
   `transaction_id` FK and a `vault_entry_hash` citation from audit events —
   the latter is what actually proves "no vault mutation without an audit
   event," and it works across files, so it does not depend on the merge.

5. **Then rehearse, then migrate.** Needs operator go-ahead; the live migration
   needs a **second, separate** one.
   ```bash
   sudo cp /var/db/sudo-secretspec/secrets.db /tmp/probe.db
   sudo chown "$(id -un)" /tmp/probe.db
   # open with the new binary. v1 baseline was 158 captured_values rows,
   # 40 value_blobs rows, 7988 -> 1997 blob bytes, 2 tombstones. Those numbers
   # are for v1 and MUST be re-derived for v2. user_version will be 2.
   rm -P /tmp/probe.db   # MUST shred: holds all 39 plaintext secrets
   ```
   Migration must run `PRAGMA foreign_key_check` **and** `PRAGMA
   integrity_check` **and** the Rust hash walker before being considered good.
   39 rows is a full walk, not a sample. `VACUUM` after commit.

6. **Fail-closed migration table** — abort and print `(sequence, item)` for
   either disagreement, do not "repair":

   | `destroyed_by` | `value_blob` | action |
   |---|---|---|
   | NULL | non-NULL | live: migrate bytes, bind FK |
   | non-NULL | NULL | tombstone: retain digest, FK NULL |
   | non-NULL | non-NULL | **abort** — destroy failed to shred |
   | NULL | NULL | **abort** — live row with no bytes |

7. **Delete `.env`.** Three reviewers flagged it. It can be retired by
   SHA-256-matching its values against `secrets.db` rather than by waiting for
   the migration.

8. **Ask the operator** whether to commit `review/`.

9. Standing don't-touch list, unchanged: `PROMPT-REVIEW.md`,
   `PROMPT-SECREV.md`, `REVIEW_REPORT.md` (ask before removing); branch
   `explore/pr-334-rust-first-spec`. **DEFERRED, do not resurface:**
   cross-platform sudo/Linux port.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -2          # expect this handoff at tip, clean but for review/

# 1. IS IT UP? Other agents depend on this. Installed binary is 0.19.1-sudo.21,
#    which PREDATES 0b66bdd, so none of this migrates the vault.
sudo-secretspec --version                      # expect 0.19.1-sudo.21
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
cargo test -p secretspec --features sqlite --lib provider::sqlite   # expect 21
pytest tests/sudo_packaging -q                                      # expect 26

# 3. PROBE WITH THE RIGHT BINARY. `sqlite3` on PATH is the Android SDK build.
/opt/homebrew/opt/sqlite/bin/sqlite3 :memory: "PRAGMA secure_delete;"  # 0

# 4. Where the work goes:
grep -n "PRAGMA foreign_keys=ON" secretspec/src/provider/sqlite.rs   # ~164
grep -n "PRAGMA foreign_keys=ON" sudo-secretspec-cli/src/broker.rs   # ~1162
grep -n "HISTORY_SCHEMA\|SCHEMA_VERSION\|migrate_v0_to_v1" \
  secretspec/src/provider/sqlite.rs

# 5. The four schema reviews: review/sqlite-schema-review-response.md (untracked)
```

**Hazards, repeated because they are the easiest ways to lose control:**
opening the live vault with a binary built from `0b66bdd` or later migrates it
in place with no prompt; installing `0.19.1-sudo.22` would do the same on the
next privileged verb. If mutating anything in the `secretspec` package, always
pass an explicit filter (`--features sqlite --lib provider::sqlite`) — a bare
`cargo test -p secretspec` includes 21 pre-existing `sops` failures and reports
a false CAUGHT for every mutation.
