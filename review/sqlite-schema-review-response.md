# SQLite Schema Review: Consolidation, Three Hash Chains, and Declarative Invariants

**Date:** 2026-08-18
**Context:** Privilege-separated macOS secret vault, 39 live secrets, one-shot migration.
Target: SQLite 3.50.6, STRICT tables, foreign_keys=ON, trusted_schema=OFF,
journal_mode=DELETE, synchronous=FULL, busy_timeout=5s.

---

## Short Summary

All three proposals are **directionally correct** but none should be adopted
exactly as written. The core insight — pushing invariants into the schema — is
sound and undervalued. Specific answers below; TL;DR:

- **Proposal A** (declarative tombstone FK): Deploy, but with one CHANGE: the
  combination of `blob_sha256` + the two CHECKs over-constrains the state space.
  Simplify (detailed in Q1–Q5).
- **Proposal B** (consolidation): Merge vault + manifest history into one file;
  keep the audit ledger **separate**. Two files, not one or three. This is the
  coherent middle that Q11 asks about and it is correct.
- **Proposal C** (triggers for chain shape): Worth it for the broker-owned
  tables; marginal for the generic provider tables. Selective deployment
  (detailed in Q13–Q15).

---

## 1. Proposal A — Declarative Tombstone Enforcement

### Q1: Extra `blob_sha256` column vs. `BEFORE DELETE` trigger?

**The extra column is the right trade.** Your bias is correct.

A `BEFORE DELETE` trigger on `value_blobs` is a single point of enforcement.
Any connection that forgets to enable triggers, any future `PRAGMA
disable_triggers` used during bulk operations, any `sqlite3` CLI direct
modification — all bypass it. A foreign key constraint cannot be forgotten
per-connection; it is schema-resident and checked on every write regardless of
connection state.

The cost — 39 extra nullable TEXT columns in a table that currently holds
roughly 39×N rows — is in the tens of bytes. It is not measurable at this
scale.

One nuance you did not mention: the FK also provides `ON DELETE RESTRICT`
semantics for *free*. A trigger-based guard would need its own
`SELECT ... WHERE destroyed_by IS NULL` to decide whether to raise an error,
duplicating logic the FK gives you declaratively. The FK is strictly simpler
and less error-prone.

### Q2: Declarative enforcement of both directions without the extra column?

**No, not in SQLite.** SQLite does not support:

- Partial or filtered foreign keys (`WHERE destroyed_by IS NULL`)
- `MATCH FULL` on composite FKs (only `MATCH SIMPLE`, as you noted)
- Check constraints that reference other tables (only within-row checks)

The `(item, blob_sha256)` FK with `MATCH SIMPLE` semantics — where a NULL in
the child column makes the constraint vacuous — is the only declarative
mechanism available. Without the extra column, you cannot make the FK binding
on live rows and vacuous on tombstones; the two states would be
indistinguishable at the schema level. The extra column *creates* the
distinction the FK needs to act on.

There is one alternative worth naming for completeness: a *separate* live-rows
table and a tombstone table, with an FK only on the live table. This eliminates
the extra column at the cost of schema complexity (`UNION ALL` views for
historical queries, two tables to migrate). For 39 secrets, the extra column is
clearly better.

### Q3: Is `CHECK ((destroyed_by IS NULL) = (blob_sha256 IS NOT NULL))` sound under STRICT for all reachable states?

**Yes, with one caveat about mid-migration.**

The CHECK enforces an equivalence: either both are NULL (tombstone) or both are
non-NULL (live row). Under `STRICT`, the column type is enforced, so neither
can hold a type other than its declared affinity without rejection. The four
states are:

| `destroyed_by` | `blob_sha256` | CHECK | Semantics |
|---|---|---|---|
| NULL | NULL | ✓ | Tombstone (correct) |
| NULL | non-NULL | ✗ | Rejected |
| non-NULL | NULL | ✓ | Live row destroyed (correct) |
| non-NULL | non-NULL | ✗ | Rejected |

The CHECK is sound. The mid-migration concern is real: during `INSERT INTO ...
SELECT ...` from the old `captured_values`, rows are constructed before the
CHECK fires. If your derivation of `blob_sha256` from legacy data produces a
state that violates the CHECK, the INSERT fails and the migration rolls back.
That is exactly what you want (fail-closed), so no problem.

The one edge case: if you ever `UPDATE` a row to change only one of the two
columns and the intermediate state (between the SET clauses) violates the
CHECK. In SQLite, a single UPDATE statement's CHECK evaluation happens at
statement end, not per-column assignment — the intermediate state is
invisible. So this is safe.

### Q4: Declarative write-once for `destroyed_by`?

**No.** SQLite has no `IMMUTABLE` column constraint, no `ON UPDATE` semantics
that can reject a change, and no TRIGGER-free way to enforce "set once, never
change."

Your current plan — a `BEFORE UPDATE` trigger that rejects changing
`destroyed_by` from one non-NULL value to another — is the correct, minimal
mechanism. Specifically:

```sql
CREATE TRIGGER captured_values_destroyed_by_write_once
BEFORE UPDATE ON captured_values
WHEN OLD.destroyed_by IS NOT NULL AND NEW.destroyed_by IS NOT OLD.destroyed_by
BEGIN
    SELECT RAISE(ABORT, 'captured_values.destroyed_by is write-once');
END;
```

This cannot be done declaratively. Accept the trigger for this one constraint
and keep it narrow.

One alternative you could consider: instead of `destroyed_by INTEGER`, use a
boolean column `destroyed INTEGER NOT NULL DEFAULT 0` plus a separate
`destroyed_at_sequence INTEGER` stored in a different table keyed by
`(sequence, item)`. This makes the boolean purely declarative (`CHECK (destroyed
IN (0, 1))`), but trades away the single-row convenience of seeing which
operation did the destroying. I'd stick with the trigger.

### Q5: Should `captured_values` be fully append-only (no DELETE at all)?

**No, and the cost you're not seeing is a correctness-adjacent one: audit
clarity.**

If tombstones are created by INSERT of a new row (`destroyed_by = X,
blob_sha256 = NULL`) rather than UPDATE of an existing row, then for a given
`(sequence, item)` there could be *both* a live row and a tombstone row. This
introduces temporal ambiguity: was the item alive or dead at that point in the
sequence? You'd need an additional rule (e.g., "the row with the highest
sequence ≤ X governs") to disambiguate, which is precisely the kind of
logic-in-code the schema push is trying to eliminate.

The current UPDATE approach — one row per `(sequence, item)`, its state
determined by the nullity of `destroyed_by` and `blob_sha256` — is
unambiguous. The delete-then-reinsert resurrection path is already closed by
the FK on `blob_sha256` plus `CHECK (blob_sha256 IS NULL OR blob_sha256 =
value_sha256)`. The FK ensures the blob must exist; the CHECK ensures
`blob_sha256` matches `value_sha256`; and `destroy` deletes the blob. A
re-insert after destroy would need the blob to exist, which it cannot unless
re-created by a new `set` — at which point it's a genuinely new value (or
re-set, which is a valid operation).

**Bottom line on Proposal A:** Deploy it, with the trigger for Q4 and the
acceptance that Q2/Q4 have no pure-declarative answer in SQLite.
---

## 2. Proposal B — Consolidation

### Q6: Three chains, or one?

**Three chains, kept independent.** Your instinct is correct and here is why it
is not an accident you are defending:

The provider is generic library code — it should not import "broker" or
"transaction_id" into its namespace. If the provider chain somehow becomes
*the* chain, then every `sqlite://` user — including non-broker users of the
library — inherits broker coupling. That is a layering violation that will
haunt you in every future refactor.

The consolidation win is real, but it is a *physical* win (one file, one
connection, one backup target) not a *logical* win (one chain). The three
chains model different things:

- **Provider chain:** Mutations to the secret store. "The vault changed."
- **Manifest chain:** Mutations to the declaration. "The spec changed."
- **Audit chain:** Privileged operations. "Someone did something."

These are related temporally (they happen together) but not semantically. A
`secretspec set` that fails validation mutates nothing — the audit chain
records the attempt, but the provider chain does not advance. A
`secretspec config init` mutates the manifest but not the vault. The chains
*diverge* in normal operation; forcing them into one chain would require NULL
sentinels for "this chain didn't change" rows, which is worse than three clean
chains.

**How to make them mutually verifiable:** A `transactions` parent table is the
right answer:

```sql
CREATE TABLE transactions (
    transaction_id TEXT PRIMARY KEY,
    started_at_ns  INTEGER NOT NULL,
    committed_at_ns INTEGER
) STRICT;
```

Both broker chains reference `transaction_id` via FK. The provider chain is
**deliberately unlinked** — it does not know what a transaction is. Instead,
the broker's verification process cross-references: "for every provider
`entries.sequence` in range [M..N], there must exist a broker transaction with
`committed_at_ns ≥ entries.timestamp_ns` that covers it." The broker-side code
already has the responsibility of orchestrating the provider; this verification
is a natural extension of that responsibility.

### Q7: Cross-chain proof: "no vault mutation without a corresponding audit event"

**Yes, there is a coherent scheme.** The broker is the gatekeeper for all vault
mutations (it holds the `sudo` boundary). The scheme is:

1. Before any vault mutation, the broker writes an audit event with
   `phase='attempt'` and records the event's `sequence`.
2. The broker performs the vault mutation via the provider.
3. The broker reads the provider's new `head.sequence` and `head.entry_hash`.
4. The broker writes a manifest entry recording the new state.
5. The broker updates the audit event to `phase='success'` and stores a
   cross-reference: `provider_sequence INTEGER` and
   `provider_entry_hash TEXT` in the audit events row.

Step 5 gives you the cross-chain link. The verification query:

```sql
-- Every provider mutation must have a corresponding success audit event
SELECT e.sequence, e.entry_hash
FROM entries e
WHERE e.sequence > 0  -- skip genesis
  AND NOT EXISTS (
    SELECT 1 FROM audit_events a
    WHERE a.provider_sequence = e.sequence
      AND a.phase = 'success'
  );
```

An empty result set means the invariant holds. This does not require the
provider to know about the audit chain — the broker stores the cross-reference
in its own tables, using values the provider already exposes.


### Q8: Three `head` tables vs. one `chain_heads` table?

**Keep them separate.** A single `chain_heads(chain TEXT PRIMARY KEY, ...)`
table is appealing for schema compactness, but it confuses two things:

1. The `head` table is not just a record — it is a *concurrency mechanism*.
   The broker reads `head.sequence`, computes `NEW.sequence = head.sequence +
   1`, and writes the new entry. This is effectively a compare-and-swap on one
   row. If all three chains share one `chain_heads` table, a read-modify-write
   on any chain's head would need to filter by `chain` — still correct under
   SQLite's serialized writes, but the intent is less clear to a reader.

2. The provider's `head` table is generic library code. The broker's
   `manifest_head` and `audit_head` are broker code. Sharing a table between
   library and application code creates a dependency in both directions: the
   library must not accidentally read/write the broker's rows, and vice versa.
   Separate tables enforce this by namespace.

Keep three `head` tables. The schema has 5 tables now (plus `secrets`); adding
a few more `head` tables and `transactions` is not a complexity cost worth
optimizing.

### Q9: Lock contention under `journal_mode=DELETE`?

**Agree.** With `journal_mode=DELETE`, every write holds an exclusive lock for
the duration of the transaction. Ledger appends (audit events, manifest entries,
provider entries) all happen within the same broker transaction, so they
contend with *reads* (vault lookups) but not with each other. Hold time is
dominated by fsync (synchronous=FULL), which is typically < 2ms on modern macOS
with an SSD. Against a 5s `busy_timeout`, a reader blocked by a writer for 2ms
will not even notice. A writer blocked by a reader will wait at most
busy_timeout — but readers hold no locks in WAL-journal-mode, and in
DELETE-journal-mode they hold a shared lock that a writer can upgrade. With
DELETE journal, the contention model is: writer blocks readers for the duration
of the write; readers block writers only briefly during the lock-upgrade
window. This is acceptable for a single-user, single-process pattern.

One concern you did not raise: with three files under `journal_mode=DELETE`,
write ordering across files is not atomic. If the broker writes to
`broker-audit.sqlite3` and then `secrets.db` and the process crashes between
them, the audit record exists but the vault mutation does not. Merging into one
file makes the broker's writes transactional — all or nothing. This is a
**stronger argument for consolidation than convenience.** It is worth calling
out explicitly: cross-file write atomicity is a correctness property you get
for free by merging.

---

## 3b — Ledger Separability

### Q10: Is "the audit ledger must survive destruction of the thing it audits" a real design principle?

**It is real, but it is narrower than you've framed it.**

The design principle is: *an append-only ledger of privileged operations should
be append-only to the thing doing the operations, not just to the operations
themselves.* Put differently: the broker should not be able to corrupt or
delete its own audit trail as a side effect of a bug — even a bug that
truncates the vault.

The four use cases you listed:

1. **Backup asymmetry** — valid and practical. A regular off-machine backup of
   a tiny (688K) digest-only file is operationally trivial; backing up a file
   containing 39 plaintext secrets requires more care.

2. **Forensics / second opinion** — valid. The ability to hand the ledger to a
   colleague or AI without exfiltration risk is the kind of property that
   enables review that would otherwise not happen.

3. **Tamper evidence surviving vault loss** — this is the one that survives
   only with separate files. And it is **not hypothetical**:

   > *This system has already had one vault-truncation incident.*

   That line in your query is doing a lot of work. You have empirical evidence
   that the vault can be corrupted or truncated by your own code. When that
   happened, `broker-audit.sqlite3` preserved the record of every operation
   that led up to and potentially caused the incident. Merging the files would
   have destroyed that evidence along with the vault.

   The threat model you described — "accident and our own bugs, not a
   root-level adversary" — makes this case *stronger*, not weaker. A root
   adversary can destroy anything; that's uninteresting. What's interesting is
   that your own broker binary, running with its legitimate privileges, can
   accidentally corrupt its data. Separate files mean "corrupt the vault" and
   "corrupt the evidence" are separate failure modes requiring separate bugs.

   That is defense-in-depth worth preserving.

4. **Compliance / attestation** — valid. Handing over a file that *cannot*
   contain the secret values is a stronger attestation than handing over a file
   that *does not appear to* contain them.

**Are you inventing a requirement?** No. The vault-truncation incident is proof
that separability already provided value you would have lost under a single
file. This is not speculative design; it is a property you already benefitted
from.

### Q11: Two files (vault + manifest merged, audit separate) — coherent middle or worst of both?

**Coherent middle, and the right answer.** Here's why:

**What two files gives you:**

- **Audit survives vault loss** (the property you already used).
- **Cross-database FK gap closed** for the one relationship that crosses the
  boundary today: manifest-to-audit. These are both broker tables and belong
  together. The provider chain remains deliberately unlinked.
- **Write atomicity** for vault + manifest mutations (they are always
  co-mutated, so this eliminates a crash-consistency gap).
- **Simpler backup story**: one "hot" file (contains plaintext, treat
  carefully) and one "cold" file (digests only, safe to replicate
  aggressively).
- **The `cp secrets.db` backup invariant** is preserved — WAL was rejected for
  this reason, and a single-file design would need WAL to get the write
  concurrency you'd otherwise have across files. With two files, journal_mode=
  DELETE continues to work, and `cp` continues to produce consistent snapshots.

**What two files gives up vs. one file:**

- One additional file descriptor (negligible).
- Two `cp` commands instead of one for a full backup snapshot.
- Two journal files instead of one (negligible).

**What two files avoids vs. three files:**

- The cross-FK gap between manifest and audit tables.
- The cross-file atomicity gap for broker operations.
- One fewer file to manage and document.

**What two files inappropriately avoids vs. one file:**

- Nothing meaningful. The "entire state in a single file" requirement from the
  owner is better re-stated as "entire *vault-relevant* state in a single file,
  audit ledger as a separable companion." The owner's operational goal —
  simpler backup and restore — is fully met by two files with one of them safe
  to copy anywhere. The one-file purity is aesthetic, not operational.

### Q12: Is an `audit export` verb the right shape?

**Yes, and it should produce a standalone SQLite file, not JSON.**

Reasons for SQLite:

1. **The existing `audit-verify` already understands SQLite.** Exporting a
   different format means the recipient needs a different verification tool. A
   SQLite export is verified by the same `audit-verify` command, which is a
   strong property.

2. **SQLite is self-describing.** A `.schema` dump reveals the structure; the
   hash chain is in the data. A JSON dump would need a separate schema
   definition and a separate verifier that re-derives hashes from JSON fields
   — that's a second code path to audit.

3. **The export is a point-in-time copy.** `ATTACH DATABASE 'export.db' AS
   export; INSERT INTO export.audit_events SELECT * FROM main.audit_events
   ORDER BY sequence;` — this is a few lines of SQL and produces a file
   identical in schema to the original, minus any future rows. No serialization
   ambiguity.

4. **You can add signing later.** A `VACUUM INTO 'export.db'` plus a detached
   Ed25519 signature over the file is a clean separation of export and
   authentication. A signed JSON blob conflates the two.

The shape of `audit export`:

```
sudo-secretspec audit export [--output audit-export.db]
  → ATTACH 'audit-export.db' AS export;
  → CREATE TABLE export.audit_events AS SELECT * FROM main.audit_events;
  → CREATE TABLE export.head AS SELECT * FROM main.audit_head;
  → DETACH export;
  → [Optionally: VACUUM 'audit-export.db';]
```

This is simple, verifiable with existing tooling, and recoverable without
custom tooling.

---

## 4. Proposal C — Hash-Chain Enforcement in SQL

### Q13: Are append-only-by-trigger chains worth having?

**Yes for the broker-owned tables; marginal for the generic provider tables.**
Here's the breakdown:

**Broker-owned tables (manifest_entries, audit_events):**

These are tables the broker writes in a controlled, transactional manner.
Triggers that enforce:

- `NEW.sequence = (SELECT MAX(sequence) + 1 FROM <table>)` (contiguity)
- `NEW.previous_hash = (SELECT entry_hash FROM <table> WHERE sequence = NEW.sequence - 1)` (linkage)
- `BEFORE UPDATE` / `BEFORE DELETE` reject (append-only)

These triggers protect against **our own broker's SQL being wrong** — exactly
the threat you identified. A bug that writes `sequence = 5` when it should be
`sequence = 6` is caught immediately, not discovered days later when chain
verification fails. A bug that writes `previous_hash = (SELECT entry_hash FROM
audit_events WHERE sequence = NEW.sequence)` (self-loop) is a real class of
bug; the trigger catches it.

The cost is nil: one trigger per table, fired once per INSERT, evaluating a
simple subquery against an indexed primary key. This is microseconds.

**Provider tables (entries, captured_values):**

The provider is generic library code. Adding triggers to its tables from the
broker is a layering violation — the broker should not modify the schema of a
table owned by generic code. If the provider wants chain-continuity triggers,
it should add them itself.

That said, the provider's chain *is* append-only in practice (the broker never
DELETEs or UPDATEs provider entries). If you want to enforce that in SQL
without layering issues, you can add triggers to the provider's schema
definition in the library itself. The question is whether the library's other
users want that constraint — it may be broker-specific.

**Does the answer differ for `captured_values`?** Yes, and for a stronger
reason. The `captured_values` invariants (tombstoning, blob FK) are enforced by
declarative constraints (Proposal A). The chain contiguity of `captured_values`
is enforced by its FK to `entries(sequence)`, which already has contiguity
enforcement. Redundant triggers on `captured_values` for chaining add no new
property.

### Q14: Is linkage-without-hash-verification actively harmful?

**Marginally.** The harm is real but small, and it is avoidable by naming.

If you write:

```sql
CREATE TRIGGER entries_chain_is_continuous
BEFORE INSERT ON entries
WHEN NEW.previous_hash IS NOT COALESCE(
    (SELECT entry_hash FROM entries WHERE sequence = NEW.sequence - 1),
    '0000...0')
BEGIN
    SELECT RAISE(ABORT, 'entries: previous_hash does not chain');
END;
```

A future maintainer *might* read this and incorrectly conclude "the chain is
validated in SQL." The mitigation is:

1. **Name the trigger accurately.** `entries_chain_linkage_matches` not
   `entries_chain_is_valid` or `entries_chain_is_integral`.

2. **Comment the trigger.** A one-line comment: `-- Enforces linkage, NOT hash
   correctness; full verification is in application code.`

3. **Separate the concerns in the codebase.** The chain verification code
   (which computes SHA-256 and walks the full chain) should live in a module
   with a name like `chain::verify`, not `chain::sql_guard`. The trigger lives
   in `schema.rs` as a migration/applied DDL constant.

With these three habits, the risk is low. Without them, yes — a future
maintainer could mistake linkage guard for integrity verification. But
naming discipline is a standard mitigation for this class of problem.

### Q15: Is enforcing `NEW.sequence = max+1` contiguity wise?

**Yes, with one caveat.** The caveat is concurrency.

With `journal_mode=DELETE` and a single writer (the broker), contiguity
enforcement is straightforward: read `head.sequence`, compute `NEW.sequence =
head.sequence + 1`, write. The trigger catches the case where the application
code is wrong — e.g., a race between reading `head` and writing the entry that
is impossible in a single-process model but could be introduced by a future
refactor.

The risk is if you ever move to a multi-writer model. Then the trigger fires
on a genuine race: writer A reads `head.sequence = 5`, writer B reads
`head.sequence = 5`, both try to write `sequence = 6` — the second one gets
rejected by the trigger even though it is "correct" from its perspective. The
fix in that model is `INSERT ... SELECT MAX(sequence) + 1 ...` inside the same
transaction, which atomically reads and writes. But that's a future problem,
and for now single-writer is settled.

So: yes, enforce it. The cost is zero and it catches a real class of bug.

---

## 5. Migration Mechanics

### Q16: "Apply trigger DDL after migration, idempotently, every connection" — right idiom?

**Yes, and the cost is negligible.**

`CREATE TRIGGER IF NOT EXISTS` on connection open compiles and stores the
trigger body once. For a handful of triggers (3–5), the compilation cost is
microseconds. The `sqlite_master` lookup for `IF NOT EXISTS` is an index
lookup on `name`. On a warm connection (the common case, since the broker is
long-lived or invoked per-command), this is invisible.

The alternative — baking triggers into the schema version migration — is
fragile for exactly the reason you already hit: `DROP TABLE ... RENAME` loses
triggers. Keeping trigger DDL separate from table DDL means you can re-apply
triggers after any table rebuild without coupling the two.

One refinement: apply triggers inside a transaction and verify with:

```sql
SELECT name FROM sqlite_master WHERE type = 'trigger' ORDER BY name;
```

Compare against an expected set. If a trigger is missing despite `CREATE
TRIGGER IF NOT EXISTS` having run, something is wrong (e.g., a migration
dropped it). This is a cheap sanity check that prevents silent drift.

### Q17: Non-idempotent or non-restartable v0→v2 migration?

**No, the design is restartable by construction.** Specifically:

- **`pragma_foreign_key_check` before migration** catches pre-existing
  corruption and aborts early, leaving v0 intact.
- **Migration runs in a transaction.** If any step fails, the entire migration
  rolls back. SQLite's `journal_mode=DELETE` provides crash-safe rollback —
  if the process crashes mid-migration, on next open the journal file is
  replayed or rolled back, restoring v0.
- **The `blob_sha256` derivation from `destroyed_by` (fail-closed)** is
  correct. A legacy row where `destroyed_by IS NOT NULL` but `value_blob` is
  also non-NULL is an inconsistency — the row claims to be destroyed but still
  has plaintext. Fail-closed means the migration refuses to proceed until the
  inconsistency is resolved. The alternative (silently treating it as live)
  would produce a restorable tombstone, violating the destroy guarantee.
- **No `DROP TABLE` without `IF EXISTS`.** All destructive steps should be
  guarded so that re-running the migration on a partially-migrated database
  does not fail.
- **The `user_version` pragma is set at the end of the successful migration.**
  If the migration fails, `user_version` stays at the old value, and the next
  open retries. Standard and correct.

One thing to verify: the `CREATE TABLE IF NOT EXISTS` that runs *before*
migration must not interfere with migration steps. If the migration does `DROP
TABLE captured_values`, but `CREATE TABLE IF NOT EXISTS` has already created a
new `captured_values` with the v2 shape, the DROP would destroy a table the
pre-open code just created. The fix is simple: the migration DROP should be
`DROP TABLE IF EXISTS captured_values_v0` and the pre-open CREATE should create
`captured_values_new` (or nothing — just let migration create the tables). The
pattern:

```
1. Pre-open: CREATE TABLE IF NOT EXISTS ... (only CREATE, no ALTER/DROP)
2. Check user_version
3. If old: BEGIN; run migration steps; PRAGMA user_version = 2; COMMIT;
4. Apply triggers (idempotently)
5. pragma_foreign_key_check; pragma_integrity_check;
```

### Q18: Fail-closed vs. forgiving migration for legacy inconsistency?

**Fail-closed is the right choice.** Here's why:

You have a live vault with 39 secrets. The migration is one-shot and
irreversible. You have exactly one chance to get it right. If the migration
encounters an inconsistency and proceeds anyway, you have permanently baked
that inconsistency into the new schema — and the new schema's constraints will
either reject future operations on those rows (worse: mysterious runtime
failures) or silently accept them (worse: the tombstoning guarantee is broken
and you don't know).

A fail-closed migration says: "I will not proceed until the data is consistent.
Fix the inconsistency manually, then re-run." For a vault with 39 items, manual
inspection and fix is tractable — it's minutes of work, not days. Contrast with
a forgiving migration that silently produces a subtly broken vault discovered
six months later when a `restore` resurrects a tombstoned value.

The principle: **migrations should be strict about invariants, because the
schema after migration will be strict about them too.** A migration that is
more permissive than its target schema is a migration that can produce
un-queryable data.

---

## Overall Architecture Verdict

| Proposal | Verdict | Action |
|---|---|---|
| A — Declarative tombstone FK | **Deploy** | Adopt the v2 schema as designed; add the write-once trigger for `destroyed_by` (Q4); accept that pure-declarative is not fully achievable in SQLite |
| B — Consolidation | **Deploy 2/3** | Merge `secrets.db` + `broker-history.sqlite3`; keep `broker-audit.sqlite3` separate; add `transactions` table; add `provider_sequence` cross-reference in audit events; add `audit export` command |
| C — SQL chain enforcement | **Selective deploy** | Full chain-shape triggers on broker-owned tables (manifest_entries, audit_events); contiguity-only on provider entries (if the library is willing); no triggers on captured_values (FK already covers it); clear naming and comments to prevent false-confidence hazard |

### Recommended file layout after consolidation:

```
vault/
├── secrets.db          # Vault + manifest history (plaintext, 0600)
├── broker-audit.db     # Audit ledger (digest-only, 0600)
├── secretspec.toml     # Declarations (human-authored, 0644)
└── .env                # DELETE after migration verified
```

Two files, clear separation of concerns, audit survives vault loss, write
atomicity for co-mutated tables, and `cp secrets.db` continues to work as a
backup snapshot.

### Two things you didn't ask but should consider:

**A. The `.env` file containing all 39 plaintext secrets.** This is a
higher-risk artifact than any schema decision. Delete it the moment the
migration is verified, and consider whether the migration verification itself
(which requires comparing old and new values) can be done with a hash
comparison rather than keeping `.env` around. If the values in `secrets.db` are
already the same plaintext (v0 stores them), you can verify by SHA-256
matching, then delete `.env` immediately after migration rather than "once
verified."

**B. The XDG audit log (58K, upstream).** You note it "belongs to the upstream
library, not us." But it contains records of every `secretspec` operation —
including `set`, `get`, `run` — that the broker delegated to the library. If
the broker's audit chain is the authoritative record, the upstream log is at
best redundant and at worst a second plaintext-adjacent log you must manage.
Consider whether the broker should suppress the upstream audit log (if the
library supports it) or redirect it to `/dev/null` for broker-initiated
operations, leaving it only for non-broker library use. A silent second audit
trail is a liability, not a feature.

---

*End of review.*

