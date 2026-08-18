---
schema_version: 1
handoff_id: bb66
parent_handoff_ids: [1446]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 0b66bdd2f4374d8511db1e8e46e6b073601c1add
created_at: 2026-08-18T15:23:31-0400
writer: claude-code
---

# Handoff — Content-addressed history blobs, plus the upstream SIGPIPE PR

## The Goal

Resume from handoff `1446` on the operator's instruction: **"Do all the
little things, then start on the big things."**

The little things were `1446`'s next-steps 2–6 (upstream watch, the SIGPIPE
follow-up PR, the `ss-370` rebase, the review-worktree teardown). The big
thing was its next-step 1: the `capture_history` content-addressed blob
migration, flagged **ANNOUNCE-FIRST** because it migrates the live vault.

## Where We Are

`sudo-main` is clean at `0b66bdd`, pushed. All the little things are done.
The big thing is **implemented, tested and pushed — but the live vault has
NOT been migrated**, by explicit operator choice (build-and-test-offline
first, ask again before writing).

Upstream `cachix/secretspec#377` (SIGPIPE) is open and awaiting review.

**Files changed this session.** In this repo, all in `0b66bdd`:

| file | change |
|---|---|
| `secretspec/src/provider/sqlite.rs` | `SCHEMA_VERSION`, `HISTORY_SCHEMA`, `migrate_history`, `has_legacy_value_blob`, `migrate_v0_to_v1`; `capture_history` made `pub` and its write path split into `value_blobs` + `captured_values`; FK test hardened; 5 new migration tests |
| `secretspec/src/lib.rs` | `pub use rusqlite` and the narrow `sqlite_history` module |
| `sudo-secretspec-cli/src/broker.rs` | tombstone UPDATE, blob DELETE, restore JOIN, finding-5 branch; 3 tests updated |
| `CHANGELOG.md` | superseded the old `source-destroy` refusal bullet; added the storage-change entry |

In the `ss-sigpipe` worktree (upstream-bound, commit `25b752a`):
`secretspec/src/bin/secretspec.rs`, `secretspec/tests/sigpipe.rs` (new),
`Cargo.toml`, `secretspec/Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`.

**Blockers / open questions.**

- The live vault migration is unapplied *and* self-triggering — see the
  hazard note at the end of this document. This is the one thing that can
  go wrong by accident rather than by decision.
- `0b66bdd` has had **no mutation testing**, which this project treats as
  the bar for believing a fix. Three specific mutations are listed in
  next-step 2.
- The migration has never run against real vault data — only against
  synthetic v0 fixtures built in `legacy_v0_database`. The rehearsal in
  next-step 1 is what closes that gap.
- Open question, not yet raised with the operator: whether the invariant
  "every live `captured_values` row has a `value_blobs` row" deserves a
  SQLite trigger. It is currently upheld by code and tests only; the old
  single-table `CHECK` that enforced the equivalent could not survive the
  split.

## What We Tried

Five things went wrong before they went right. Four are silent-failure
modes — the kind that look like success — so they are recorded in full.

- **The SIGPIPE repro exited 0 and looked like "no bug".** First attempt
  piped `export` (50 secrets, small values) into `head -c 5` and got exit
  0, no error. The pipe buffer on macOS is ~64KiB; the entire output fit
  inside it, so the write never blocked and the pipe never broke. Only
  after scaling to 200 secrets x 2000-byte values (402KB) did the real
  failure appear. **A small fixture makes this bug look absent.**

- **The `check --json` SIGPIPE test passed vacuously.** Having learned the
  size lesson, I padded both values *and* descriptions — and the test still
  passed with exit 0. `check --json`'s report contains neither: it emits
  ~200 bytes per secret of status metadata, ignoring value size and
  description entirely. Secret *count* is the only knob that pushes that
  path over the buffer. Fixed by raising `SECRET_COUNT` to 500 and dropping
  the useless `DESCRIPTION_LEN` padding.

- **`secretspec completion bash` is not a subcommand** (it is `completions`),
  which produced a 334KB flood of clap errors into the transcript before I
  noticed. Trivial, but it cost a turn and polluted one tool result.

- **The migration SQL failed with `near "IS": syntax error`.** Rust's `\`
  line-continuation inside a string literal strips the newline *and the
  next line's leading whitespace*, so
  `... FROM captured_values\` + `WHERE value_blob IS NOT NULL` became
  `FROM captured_valuesWHERE value_blob IS NOT NULL`. The parser then read
  `captured_valuesWHERE` as a table alias and choked on `IS`. Rewrote
  `migrate_v0_to_v1`'s batch as a raw string with real newlines rather than
  patching spaces in, because the same trap recurs on every future edit.
  **Other SQL literals in this codebase survive only because they happen to
  put a space before the backslash.**

- **`convention("APP_SECRET").item()` does not exist** on `Address`. Used
  the literal `"project/production/APP_SECRET"` instead (the shape
  `convention_address` builds: `{project}/{profile}/{key}`).

And one thing that was *already* broken and would have stayed hidden:

- **`foreign_keys_are_enforced_on_every_connection_this_provider_hands_out`
  passed for the wrong reason.** Its orphan-row `INSERT` named `value_blob`
  — a column that no longer exists after the migration — so it failed with
  "no such column" rather than on the foreign key, and the bare
  `assert!(orphan.is_err())` stayed green. The test had silently stopped
  testing foreign keys. It now asserts
  `rusqlite::ErrorCode::ConstraintViolation` specifically. This is the
  exact class of rot the operator's mutation-testing discipline exists to
  catch, found here only because the column moved.

## Key Decisions

- **Blobs are keyed by `(item, value_sha256)`, NOT by digest alone.**
  This is the decision the whole change turns on, and it went against the
  obvious reading of "content-addressed". Measured against the real vault,
  global dedup saves **76 more bytes** (1921 vs 1997, a 3.8% edge) — and it
  breaks `destroy --name` on secrets that exist *today*. The vault holds
  `CLINE_API_KEY` and `HERMES_CLINE_API_KEY` with one identical 67-byte
  value, and `TELEGRAM_ALLOWED_USERS` / `TELEGRAM_HOME_CHANNEL` with one
  identical 9-byte value. With a shared blob, `destroy CLINE_API_KEY` must
  either keep the blob (its plaintext survives destruction — the one
  outcome the verb exists to prevent) or delete it (erasing
  `HERMES_CLINE_API_KEY`'s live history). Per-item keying makes the
  dilemma structurally impossible and needs no reference counting.
  **Rejected:** global digest keying with a refcount/GC — more machinery,
  worse semantics, for 76 bytes.

- **Restore now filters on `destroyed_by IS NULL`, not on blob presence.**
  The two conditions agreed when every row carried its own blob. With blobs
  shared across an item's snapshots they can diverge: destroy a name, then
  re-set it to the same value, and the old tombstoned row's `(item,digest)`
  would resolve to live bytes again. Making the tombstone authoritative
  keeps a destroyed row unrestorable regardless.

- **Finding 5 closed by exposing chain-append, not by relaxing the schema.**
  `destroy` on an already-source-deleted name refused because tombstones
  need a real entry to attribute to and the broker would not restate the
  hash-chain logic. Added a deliberately narrow `secretspec::sqlite_history`
  module exporting only `capture_history`, so the broker appends a genuine
  entry on its own connection. Hashing stays in exactly one place — which
  was the original objection to synthesising an entry.
  **Rejected:** relaxing the `CHECK` to allow a null `destroyed_by`
  (destroys auditability), and duplicating chain logic in the broker
  (two implementations of one chain is how a ledger stops being provable).

- **The migration introduces `PRAGMA user_version` because nothing existed.**
  The provider had *no* migration machinery: schema is `CREATE TABLE IF NOT
  EXISTS`, so an old-shaped database was silently accepted and never
  upgraded, forever. Version stamping had to be built as a prerequisite.

- **Re-exported `rusqlite` from `secretspec`.** Both crates independently
  pin 0.31 and the lock resolves one copy, so passing a `Connection` across
  the boundary type-checks — but only by luck. The re-export makes the
  coupling explicit so a future version skew fails loudly.

- **Did not migrate the live vault.** Operator picked
  "build + test offline first, then ask again" from a three-way choice.

- **Upstream commits carry no `Co-Authored-By` trailer**, matching the
  existing convention on `1b1783c` in this same PR series. Flagged to the
  operator rather than silently deciding; not overridden.

- **Declined to trust the Hindsight page.** Handoff `1446` pointed at
  `kp-4591bfacb8f64679a468c647a5dc617f` as the source for this work. The
  page is **empty** — it explicitly says it has no information about the
  initiative. Derived the design from `sqlite.rs` and
  `docs/design/infinite-secret-backup-restore.md` instead, and overwrote
  the page with a real summary via `hindsight_capture_initiative`.

## Evidence & Data

**Live vault baseline** (read from a `sudo cp` copy at `/tmp`, then
`rm -P` shredded — it held all 39 plaintext secrets):

| metric | value |
|---|---|
| `captured_values` rows | 158 |
| `entries` | 4 |
| live `secrets` | 39 |
| distinct `(item, digest)` | 40 |
| distinct digest alone | 38 |
| tombstoned rows | 2 |
| total blob bytes | 7988 |
| dedup by `(item, digest)` | **1997** (4.0x) |
| dedup by digest alone | 1921 (4.2x) |

The 40-vs-38 gap is the whole design argument: two digests appear under
more than one name.

**Tests** (all with the system-SQLite env exported):
- `cargo test -p sudo-secretspec-cli` → **228 passed, 0 failed** (baseline 228)
- `cargo test -p secretspec --features sqlite --lib provider::sqlite` → **21 passed** (baseline 16; +5 new migration tests)
- `pytest tests/sudo_packaging -q` → **26 passed**
- `cargo fmt --all -- --check` → clean
- Pre-existing and unrelated: `cargo test -p secretspec --lib` → 1352 passed / **21 failed**, every one `The 'sops' CLI is not installed`. Confirmed `which sops` is empty. Do not chase.

**Three broker tests changed**, two mechanically and one by intent:
`a_non_utf8_capture_is_refused_...` now seeds `value_blobs`;
`destroy_tombstones_every_captured_copy_of_the_name` joins `value_blobs` to
mean "readable"; and
`destroying_an_already_deleted_name_refuses_rather_than_misattributing_it`
was renamed to `..._attributes_it_to_a_new_entry` and now asserts exit 0,
`tip_after == tip_before + 1`, and that **no** tombstone names an entry
other than the new one — preserving its original anti-misattribution point.

**Upstream SIGPIPE, `cachix/secretspec#377`** (branch
`fix/sigpipe-default-disposition`, worktree `/Users/djbclark/src/ss-sigpipe`,
commit `25b752a`, based on upstream `cf48a7a`):

| case | before | after |
|---|---|---|
| `export \| head` | exit 1, `IO error: Broken pipe` | exit 141, **empty stderr** |
| `check --json \| head` | exit 101, **panic** in `println!` | exit 141, **empty stderr** |
| `export` untruncated | 1005892 bytes, exit 0 | unchanged |

Fix is `libc::signal(libc::SIGPIPE, libc::SIG_DFL)` under `#[cfg(unix)]` at
the top of `secretspec/src/bin/secretspec.rs`'s `main()`; `libc` added under
`[target.'cfg(unix)'.dependencies]` only. Regression tests in
`secretspec/tests/sigpipe.rs`, **mutation-verified 2/2** (reverted the fix,
both failed with exactly the quoted errors). `check` (non-JSON) writes to
stderr on `upstream/main` and only benefits once `#373` lands.

**Other upstream state:** `#374` rebased onto `cf48a7a` (upstream had moved
past `1446`'s recorded `35791a2`), force-with-lease pushed as `d96f9b6`, now
`MERGEABLE`. `#373` has no new maintainer response since `1b1783c`; I
commented linking `#377` (`issuecomment-5332584991`). `#374` and `#372`
still have zero maintainer engagement — `#374`'s only comment is our own
dogfooding report. No CI checks are reported on fork PRs.

**Housekeeping:** removed the `sudo-secretspec-review` worktree and its four
`review/*` branches.

## Operator Feedback

- **"Do all the little things, then start on the big things."** — the
  session's whole shape.
- **Approved all three of my recommendations** on the migration: `(item,
  digest)` keying, scope = migration + finding 5 together (one vault
  migration rather than two), and build-and-test-offline-first.
- **"Let's aim to pause around 3:21pm to change back to the MIT account."**
  — arrived mid-turn during the broker test failures; drove the decision to
  fix the three tests, commit, push and write Tier 1 rather than start
  mutation testing.
- Standing from `1446`, still in force: 2 free ultra reviews remain, saved
  for other projects — prefer local review + mutation testing here.

## Where We're Going

1. **THE NEXT ACTION — rehearse the vault migration against a copy, then
   ask before touching the real one.** The migration runs **automatically
   on first open** of an old-shaped database (`migrate_history`, called from
   `connection()`), so *any* privileged `sudo-secretspec` verb against
   `/var/db/sudo-secretspec/secrets.db` will migrate it as a side effect.
   Rehearse first:
   ```bash
   sudo cp /var/db/sudo-secretspec/secrets.db /tmp/probe.db
   sudo chown "$(id -un)" /tmp/probe.db
   # open with the new binary, then compare against the baseline table above:
   #   expect 158 captured_values rows preserved, 40 value_blobs rows,
   #   blob bytes 7988 -> 1997, user_version = 1, 2 tombstones intact
   rm -P /tmp/probe.db   # MUST shred: it holds all 39 plaintext secrets
   ```
   Then get explicit operator go-ahead before the live migration.

2. **Mutation-verify `0b66bdd`. Nothing in it has been mutation-tested yet**,
   which is a standing requirement in this project. Highest-value mutations:
   (a) change the `value_blobs` primary key to `(value_sha256)` alone →
   `migration_keeps_identical_values_separate_per_name` must fail;
   (b) drop `WHERE value_blob IS NOT NULL` from `migrate_v0_to_v1` →
   `migration_dedups_blobs_without_resurrecting_destroyed_bytes` must fail;
   (c) revert restore's filter to blob-presence → a destroyed row must
   become restorable. Helper:
   `~/.local/state/handoffs/chains/standalone-b2db/mutate.sh <file> <label> <python-expr> <pkg>`
   — it backs up with `cp` not `git checkout`, and **verify the mutation
   actually landed before trusting a MISSED result**.

3. **Watch `cachix/secretspec#377`.** Its body offers to swap the new
   `cfg(unix)` `libc` dependency for a hand-rolled `extern "C"` declaration
   if domenkozar would rather not add the direct dep — that is the likely
   review ask. `gh pr view 377 --repo cachix/secretspec --json state,reviews,comments`

4. `#373`, `#374`, `#372`: nothing to action, keep watching each time
   upstream state is re-checked. `#374` is `MERGEABLE` at `d96f9b6`.

5. Once `#377` resolves, remove the worktree:
   `git worktree remove /Users/djbclark/src/ss-sigpipe && git branch -D fix/sigpipe-default-disposition`.

6. Consider whether a release (`0.19.1-sudo.22`) should follow the vault
   migration, and in which order. `1446`'s precedent was to release the
   proven tree *before* the unproven migration.

7. Standing don't-touch list: `PROMPT-REVIEW.md`, `PROMPT-SECREV.md`,
   `REVIEW_REPORT.md` (ask before removing); `/var/db/sudo-secretspec/.env`
   (last pre-migration reference copy of all 39 values — **keep until the
   vault migration is done and verified**); branch
   `explore/pr-334-rust-first-spec` (do not delete — `bf0b25c` is linked by
   SHA from a public comment on upstream `#357`).

8. **DEFERRED per explicit operator instruction, do not resurface:**
   cross-platform sudo/Linux port.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -2          # expect 0b66bdd at tip, clean, pushed

# 1. IS IT UP? Do this first — other agents depend on it.
#    NOTE: the installed binary is still 0.19.1-sudo.21, which PREDATES
#    0b66bdd, so these commands do NOT migrate the vault yet.
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
cargo test -p sudo-secretspec-cli                            # expect 228
cargo test -p secretspec --features sqlite --lib provider::sqlite  # expect 21
pytest tests/sudo_packaging -q                               # expect 26

# 3. The migration code:
grep -n "migrate_history\|migrate_v0_to_v1\|HISTORY_SCHEMA" \
  secretspec/src/provider/sqlite.rs
grep -n "sqlite_history" secretspec/src/lib.rs sudo-secretspec-cli/src/broker.rs

# 4. Upstream SIGPIPE PR:
gh pr view 377 --repo cachix/secretspec --json state,reviews,comments
```

**Hazard, repeated because it is the easiest way to lose control of this:**
opening the live vault with a binary built from `0b66bdd` migrates it in
place, with no prompt. Installing `0.19.1-sudo.22` would do the same on the
next privileged verb. Rehearse against a copy first (next-step 1).
