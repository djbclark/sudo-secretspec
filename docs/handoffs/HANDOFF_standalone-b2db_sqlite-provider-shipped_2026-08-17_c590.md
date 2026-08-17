---
schema_version: 1
handoff_id: c590
parent_handoff_ids: [1bd6]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 239d1abbb8d112745a88981f74c93c23495270ca
created_at: 2026-08-17T18:16:33-04:00
writer: claude-code
---

# Handoff — the sqlite:// provider is shipped (step 1 of 8), a three-way architecture comparison settled it first

## The Goal

Resume `1bd6`, whose single "THE NEXT ACTION" was: build
`secretspec/src/provider/sqlite.rs` — a plain `get`/`set`/`delete` provider,
no boundary awareness. Mid-build the operator interrupted to ask whether an
existing local database backend could replace it, which became a real
three-way architecture comparison before the build resumed.

## Where We Are

Branch `sudo-main` at `239d1ab`, **tree clean, pushed**
(`f214155..239d1ab`, 1 commit this session).

**The installed boundary is still `0.19.1-sudo.18`.** This session's code is
not released.

The second worktree is still deliberately present:

```
/Users/djbclark/src/sudo-secretspec  239d1ab [sudo-main]
/Users/djbclark/src/ss-370           b3637e2 [spec-manifest-edit]
```

Keep `ss-370` until #374 resolves. It is still based on `dfa4b10`;
`upstream/main` has since moved to `35791a2` (PR #368 merged) — a rebase
will be wanted before any review round.

### Commit this session

| SHA | What |
|---|---|
| `239d1ab` | `feat(provider)`: add a local `sqlite://` provider |

### Files changed

**New:** `secretspec/src/provider/sqlite.rs` (~450 lines with tests).

**Modified:** `CHANGELOG.md`, `Cargo.lock`, `secretspec/Cargo.toml`,
`secretspec/src/provider/mod.rs`, `secretspec/src/provider/tests.rs`.

## What We Tried

### 1. Resumed correctly, then got interrupted mid-build

Read `1bd6` in full, then the design doc's "Redirection" and "Implementation
plan (current, post-redirection)" sections. Began building
`sqlite.rs` directly from `kdbx.rs`/`file.rs` as templates — examined the
`register_provider!` macro, `mod.rs` registration, the full `Provider` trait
in `traits.rs`, `Cargo.toml`/`rusqlite` conventions, and the
`SECRETSPEC_TEST_PROVIDERS` integration hookup in `tests.rs` before writing
any code.

Mid-build, the operator interrupted: *"Wait, is there already a local
database backend? If so then maybe we only need to do the sudo runas bit
with it."* Answered directly rather than continuing to build: no `sqlite://`
provider existed yet; `kdbx` is the only local database with native
per-entry history, but needs an interactive unlock secret (mismatch with a
headless runas path); `sudo-secretspec-cli`'s own `rusqlite` usage
(`history.rs`/`audit.rs`) is private boundary code, not a reusable provider
plugin.

### 2. The three-way comparison the operator then asked for

*"I don't care which local database we use, as long as we can use it with
sudo runas and it can keep history. So look into existing backends we may
not have found (do a search for 3rd party ones) vs the possibility of
modifying kdbx to do what we want vs ur new sqlite plus sudo runas idea."*

Researched all three before recommending:

- **Third-party/overlooked backends**: checked `origin/main` (upstream
  mirror, ahead of `sudo-main`) for provider work not yet merged down —
  nothing new. Web-searched for a `secretspec` community-provider ecosystem
  — none exists; it is a young single-repo project. Web-searched Rust
  embedded versioned KV stores — `redb` has MVCC/savepoints but no history
  API (same amount of work as building it ourselves, on a less-proven
  dependency); `CrepeDB` (a versioned wrapper over redb/rocksdb/mdbx) exists
  but is obscure and unaudited — wrong tradeoff for a store guarding real
  credentials.
- **Modifying `kdbx`**: checked `pass.rs`/`gopass.rs` too — both shell out to
  their CLIs with zero history handling coded, and both would need
  `gpg-agent`/a passphrase, the same headless-unlock mismatch as `kdbx`.
  `kdbx` does retain entry history natively in the KDBX format (confirmed in
  its own existing test), but nothing in this fork reads it back out — new
  `list`/`restore` methods would be needed regardless of which backend wins.
  Rejected: the master-password-or-keyfile requirement just relocates the
  secret onto another file guarded by the same filesystem permissions the
  boundary already relies on, buying nothing, while KeePass's history has no
  hash chain or tamper-evidence — its audit properties would have to be
  rebuilt on a foreign binary format not designed for it.
- **New `sqlite://` provider (the original plan)**: `rusqlite` is already a
  proven, workspace-vetted dependency used identically elsewhere in this
  exact codebase (`audit.rs`, `history.rs`), the chain/versioning design is
  already built and tested, and confidentiality is filesystem-permissions
  only — same model as everything else the boundary already protects.

Recommended keeping the original plan. Operator confirmed: *"yes."*

### 3. Built the provider, found and fixed a real ordering bug

Wrote `secretspec/src/provider/sqlite.rs`. First test run: 13/14 passed, one
failure — `unsupported_native_coordinate_is_rejected`. Root cause: `get()`
and `delete()` checked `self.config.path.exists()` **before** resolving the
address through `flat_item` (which is what triggers coordinate validation).
An invalid native coordinate (`field` on this flat store) was silently
misreported as "not found" instead of erroring, but **only** when the
database file did not yet exist — the same invalid address correctly errored
once the file existed. Fixed the ordering in `get`, `delete`, and
`get_many` (the last needed restructuring: resolve every request's address
into a `Vec` up front, before the batch existence check). Added a targeted
regression test
(`unsupported_native_coordinate_is_rejected_even_before_the_database_exists`).

### 4. Full verification pass

- `provider::sqlite`: 14/14 pass.
- Full `provider::` module: 794 passed, 21 failed — all 21 are the
  pre-existing `sops`-CLI-not-installed baseline (`FORK-AI.md`-documented),
  zero unexpected failures, zero regressions elsewhere.
- `cargo clippy -p secretspec --lib --all-features`: 10 warnings, all
  pre-existing in `sops/config.rs`, `sops/pattern.rs`, `sops/mod.rs`,
  `cli/mod.rs` — **none** in `sqlite.rs`.
- `cargo fmt -p secretspec -- --check`: found diffs in how this fork's
  `rustfmt` wraps long `map_err`/`execute` call chains versus what I first
  wrote; ran `cargo fmt -p secretspec`, re-checked clean.

### 5. Tier 1 write required a schema fix on the first attempt

`session_log.py write` rejected the first payload: `"payload blockers must
be a list of strings"` — the tool's schema is stricter than the file-format
doc's prose example suggests (a bare string is not accepted). Fixed by
wrapping `blockers` in a single-element list; second call succeeded.

## Key Decisions

- **Kept the new `sqlite://` provider plan** over third-party backends
  (none exist) and modifying `kdbx` (rejected — see "What We Tried" #2 for
  the full reasoning).
- **Addressing model**: `convention_address` produces
  `"{project}/{profile}/{key}"` as a flat SQLite primary-key string
  (`item TEXT PRIMARY KEY`), deliberately **without** `file.rs`'s
  `validate_convention_component` path-safety checks — there is no
  filesystem-escape risk since the string is just an opaque DB key, not a
  path.
- **Schema/pragma style mirrors this fork's existing SQLite usage exactly**
  (`STRICT` table; `PRAGMA journal_mode=DELETE`, `synchronous=FULL`,
  `trusted_schema=OFF`) — checked against `audit.rs`/`history.rs` before
  writing, rather than inventing a different convention for a third SQLite
  consumer in the same repo.
- **Every `connection()` open re-asserts `0600` permissions** on the DB
  file, in case it pre-existed with looser ones — mirrors `file.rs`'s
  `restrict_temporary_file` defense-in-depth pattern, applied
  unconditionally rather than only on first-create.
- **No in-process mutex** (unlike `kdbx`'s `KDBX_IO_LOCK`). SQLite's own
  file-level locking plus a 5s `busy_timeout` is sufficient here because
  every operation is one atomic SQL statement, not `kdbx`'s
  load-modify-save-whole-file cycle.
- **Query parameters are rejected outright for now** (`has_query()` check)
  rather than silently accepted-and-ignored — the future `?history=...` opt-in
  (step 2) does not exist yet, and accepting-then-ignoring an unimplemented
  flag would be worse than erroring.
- **CHANGELOG entry scoped strictly to this session's diff** — did not fold
  in the separately-owed debt for `5efb816`/`bd17934`/`dff3830` (prior
  session's code-only commits), to keep authorship/dates honest.
- **Committed and pushed immediately** at this clean, fully green seam
  (matches the chain's established pattern), rather than batching with
  step 2.

## Evidence & Data

- `provider::sqlite`: **14/14 pass** (config parsing incl. rejecting query
  params/missing path; convention round-trip + profile isolation; native
  item addressing; unsupported-coordinate rejection incl. the regression
  case; set-updates-existing-row; missing-database-returns-none
  -without-creating-a-file; delete idempotence without creating a missing
  database; URI round-trip; base-dir rebasing; `0600` permission check
  (unix); `get_many`; concurrent writes from two threads).
- Full `provider::` module: **794 passed, 21 failed, 4 ignored** — the 21
  are all `provider::sops::tests::*`, all failing with `"The 'sops' CLI is
  not installed"` (pre-existing baseline, not a regression).
- `cargo clippy -p secretspec --lib --all-features`: **10 warnings total,
  zero in `secretspec/src/provider/sqlite.rs`**.
- `cargo fmt -p secretspec -- --check`: clean after `cargo fmt -p secretspec`.
- `git diff --stat` for `239d1ab`: **6 files changed, 565 insertions(+), 1
  deletion(-)**.
- `rusqlite v0.31.0` built and linked against **system SQLite** via the
  `FORK-AI.md`-documented `PKG_CONFIG_PATH`/`LIBRARY_PATH`/`CPATH` pointed at
  `/opt/homebrew/opt/sqlite` — confirms no bundled-`libsqlite3-sys` hang on
  this Mac.
- Pushed: `origin/sudo-main` `f214155..239d1ab`.
- Tier 1 canonical log (`chains/standalone-b2db/SESSION_LOG.md`) updated via
  `session_log.py write` — `blockers` must be a JSON list of strings, not a
  bare string (see "What We Tried" #5).

## Operator Feedback

- **"Which claude model and thinking level are sufficient for the sqlite
  provider and sudo runas work?"** — wanted a costed, concrete
  model/effort recommendation split by sub-task, not one blanket answer.
  Gave a table; recommended Sonnet-high for the templated provider plumbing,
  Opus-high for the hash-chain versioning half, and Opus-**xhigh**
  specifically for the runas privilege *audit* (not the mechanical edit
  after) — reasoning from actual code surface (`require_root()` defined
  4×, 82 `chown`/`uid` hits) rather than the task's one-line description.
- **Switched session model to Sonnet 5 (high) via `/model`**, then *"okay
  continue as planned"* — did not ask me to match the per-subtask split
  just recommended; proceeded under the session's active model rather than
  re-litigating it.
- **Mid-build interruption**: *"Wait, is there already a local database
  backend?..."* — this operator watches for scope-narrowing opportunities
  and interrupts build work directly (not queue the question) when one
  occurs to them.
- **Explicit three-way comparison request** with concrete evaluation
  criteria stated up front (runas-compatible, history-capable). Wanted real
  research (web search, checking unmerged upstream) before a
  recommendation, not a reassertion of the existing plan.
- **"yes"** — confirmed the redirected plan a second time after the
  comparison; no pushback on the recommendation.
- **"continue as you think best"** (after commit/step-2/pause were offered
  as explicit options) — delegated the immediate judgment call. This did
  not extend to unilaterally ending the session: the context-size hook
  warning was surfaced as information and a recommendation, not acted on
  by itself.
- **Ran `/handoff`** — confirms the recommendation to pause here rather
  than start step 2 (the versioning port) in a filling context.

## Where We're Going

1. **THE NEXT ACTION — STEP 2 of 8** (see
   `docs/design/infinite-secret-backup-restore.md` §"Implementation plan
   (current, post-redirection)"): add opt-in internal versioning to
   `secretspec/src/provider/sqlite.rs` behind a URI option
   (`sqlite://path?history=...` or similar — the exact parameter name is not
   yet fixed; decide it as part of this step). Carry the chain design over
   from `sudo-secretspec-cli/src/history.rs` (1091 lines, 15 tests, already
   built and tested) **verbatim**: entries hash-chained over metadata and
   value **digests** (never raw bytes), `destroyed_by` kept **outside** the
   hash and enforced honest by a schema `CHECK ((value_blob IS NULL) =
   (destroyed_by IS NOT NULL))`. Read `history.rs`'s schema/chain code
   first — port the design, do not re-derive it.
2. **THEN step 3**: provider documentation in the **seven** locations
   `CLAUDE.md`'s "Adding Provider Documentation" checklist names, all
   version-labeled `(0.20+)`.
3. **THEN step 4**: reduce `sudo-secretspec-cli/src/history.rs` to manifest
   history + install-time capture only (gap 2: `rollback.rs:4` states
   plainly it never covers vault values — the seam the 2026-08-17 truncation
   incident fell through).
4. **THEN step 5**: restore and destroy verbs, authorization split
   unchanged from the original design — forward-safe for agents,
   `--force`/`--all`/`destroy` operator-only behind a separate sudoers verb.
5. **THEN step 6**: migrate the live vault — 39 secrets in
   `/var/db/sudo-secretspec/.env`, on a host where that file has already
   been truncated once. Rehearse with a rollback first; do NOT one-shot it.
6. **INDEPENDENT, takeable any time — step 7**: runas reduction, sudoers
   `ALL=(root)` → `ALL=(_sudo_secretspec)` at
   `sudo-secretspec-cli/src/install.rs:825`. `require_root()` is defined
   **four** separate times with different error types
   (`rollback.rs:30`, `broker.rs:105`, `uninstall.rs:168`,
   `install.rs:277`) across 5 call sites — audit every one before changing
   any; install/uninstall almost certainly still need real root. 82 hits for
   `chown`/`geteuid`/`getuid`/`Uid::` across the crate is the surface the
   "root is largely incidental" claim needs verifying against, not assumed.
   Scoped in this session's model discussion as an xhigh-effort audit, then
   a Sonnet-tier edit — worth deliberately budgeting for that split.
7. **STEP 8**: docs, sudoers, CHANGELOG, release, postinstall verification —
   the wrap-up pass after all of the above land.
8. **SEPARATE, still owed**: `CHANGELOG.md` has no entry for `5efb816`,
   `bd17934`, or `dff3830` (prior session's code-only commits — `239d1ab`
   already has its own entry). Does not block step 2.
9. **DEFERRED but DECIDED, not forgotten**: `declarations` →
   `Option<PathBuf>` so `template-check` can report a retired source. Shape
   and reasoning in `docs/design/template-check-resync.md` §"Decision,
   2026-08-17".
10. **Watch #374** (upstream still zero maintainer engagement, along with
    #362/#372/#373); changes go in `/Users/djbclark/src/ss-370` (branch
    `spec-manifest-edit`, still on `dfa4b10` — a rebase will be wanted
    before any review round) and port to `sudo-main` separately.
11. **Do NOT delete** branch `explore/pr-334-rust-first-spec` — `bf0b25c`
    is linked by SHA from a public comment on upstream #357.
12. **DEFERRED per explicit operator instruction, do NOT resurface**:
    cross-platform sudo/Linux port
    (`docs/design/privilege-boundary-and-packaging.md:487`).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect 239d1ab at tip, tree clean, pushed
git worktree list             # ss-370 must still be there until #374 resolves

# READ FIRST — the exact next step:
sed -n '/Implementation plan (current, post-redirection)/,/Superseded implementation plan/p' \
  docs/design/infinite-secret-backup-restore.md

# What ships today, to build on:
sed -n '1,90p' secretspec/src/provider/sqlite.rs

# What transfers verbatim for step 2 (chain design):
sed -n '1,240p' sudo-secretspec-cli/src/history.rs
cargo test -p sudo-secretspec-cli --lib history     # 15 tests, all passing

# Build/test the sqlite provider (system SQLite, not bundled):
export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
export LIBRARY_PATH="/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH"
export CPATH="/opt/homebrew/opt/sqlite/include:$CPATH"
cargo test -p secretspec --lib provider::sqlite     # expect 14 passed
cargo test -p secretspec --lib provider::           # expect ~794 passed, 21 sops failures (expected)
cargo clippy -p secretspec --lib --all-features     # expect 10 pre-existing warnings, none in sqlite.rs
cargo fmt -p secretspec -- --check                  # expect clean

# Confirm the boundary — do not assume:
sudo-secretspec --version      # expect 0.19.1-sudo.18; NOTHING here is released
sudo-secretspec doctor
```
