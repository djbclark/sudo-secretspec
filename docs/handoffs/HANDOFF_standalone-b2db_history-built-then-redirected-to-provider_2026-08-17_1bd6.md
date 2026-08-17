---
schema_version: 1
handoff_id: 1bd6
parent_handoff_ids: [3782]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: c124c0b2b770887864cce7333439e9474a91777a
created_at: 2026-08-17T16:33:46-0400
writer: claude-code
---

# Handoff — backup/restore designed and half built, then redirected onto a `sqlite://` provider

## The Goal

Resume `3782`, whose single next action was to design infinite backup/restore
properly (step 1, the prior-art re-search, was already done and recorded). The
operator then escalated scope twice, and the second escalation reversed the
architecture:

1. First: *"Do your plan"* — design the feature, decide the open questions.
2. Then: *"We def need to fix the gaps. Please view closing the gaps and
   implementing this feature as one project, and do all elements of the project
   in the most advantageous ordering."*
3. Then, mid-implementation: a question about whether storage belonged at the
   right level, escalating to *"switch our entire stack to instead be based on
   a normal-plugin-interface sqlite"* — which, after analysis, was **adopted**.

Net result: the feature is designed, four steps are shipped, and the storage
half of that design has been deliberately superseded. Nothing was reverted.

## Where We Are

Branch `sudo-main` at `c124c0b`, **tree clean, pushed** (`22e5178..c124c0b`,
5 commits this session).

**The installed boundary is still `0.19.1-sudo.18`.** None of this session's
code is released, and `CHANGELOG.md` has **no entry** for any of the three code
commits — they are code-only and still owe one.

A **second worktree is still deliberately present**:

```
/Users/djbclark/src/sudo-secretspec  c124c0b [sudo-main]
/Users/djbclark/src/ss-370           b3637e2 [spec-manifest-edit]
```

Keep `ss-370` until #374 resolves. Note it is based on `dfa4b10`, and
**`upstream/main` has since moved to `35791a2`** (PR #368 merged) — a rebase
will be wanted before any review round.

### Commits this session

| SHA | What |
|---|---|
| `c28960c` | `docs(design)`: decide the backup/restore design + template-check's retirement state |
| `5efb816` | `fix(broker)`: give `source-undeclare` the rollback copy its siblings have |
| `bd17934` | `feat(history)`: the chained history store behind the boundary |
| `dff3830` | `feat(history)`: keep the pre-mutation state instead of deleting it |
| `c124c0b` | `docs(design)`: redirect storage into a `sqlite://` provider |

### Files changed

**New:** `sudo-secretspec-cli/src/history.rs` (~950 lines with tests).

**Modified:** `sudo-secretspec-cli/src/broker.rs`,
`sudo-secretspec-cli/src/audit.rs`, `sudo-secretspec-cli/src/lib.rs`,
`sudo-secretspec-cli/Cargo.toml`,
`docs/design/infinite-secret-backup-restore.md`,
`docs/design/template-check-resync.md`.

## What We Tried

### 1. The finding that reframed the whole feature

Before designing anything, read the privileged paths. `broker.rs:235`
`Mutation` **already** copied both `secretspec.toml` and `.env` before every
`source-set`/`source-add`/`source-delete`, correctly chowned to the service
identity and refused on collision, keyed by the same `uuid::Uuid` the
hash-chained audit ledger records — and `commit()` then **deleted** them.

So "infinite backup/restore" was mostly *stop discarding a snapshot the
boundary already takes correctly*, not build a versioning store. That reframing
drove every subsequent decision and it is still true after the redirection.

### 2. Two gaps found while grounding the design

Both real, both independent of which storage wins:

- **`source-undeclare` was not wrapped in `Mutation`.** The `matches!` at
  `broker.rs:529` listed only `set`/`add`/`delete`, but `source-undeclare`
  performed a bare `std::fs::write(&manifest, updated)` at `broker.rs:866`. A
  write failing part way would corrupt the largest file in the vault (9735
  bytes live) with no rollback copy. **Fixed in `5efb816`.**
- **`install` has never had vault coverage.** `rollback.rs:4` states outright
  *"Never touches vault secret values"* and prints `"runtime vault preserved"`.
  Artifact rollback and secret history are disjoint by design — the exact seam
  the 2026-08-17 truncation fell through. **Not yet fixed.**

### 3. A storage decision that had to be revised mid-design

Told the operator whole-file snapshots would be the stored truth for both
files, then found that cannot support destroy-by-name: removing one name's
value from a `.env` blob changes that blob's digest and breaks the chain, and
NULLing the blob would destroy the other 45 names' history with it. Revised to
per-name rows for the dotenv plus a whole-file blob for the manifest, and said
so rather than quietly changing it.

### 4. The dotenv grammar — three approaches, only the third is sound

Byte-exact reassembly of a decomposed `.env` needs the exact renderer:

- *Rejected: hand-rolled `split('=')`.* A second implementation of the grammar
  is a second thing to keep in sync, and the one that drifts is the one that
  mis-parses a quoted or multi-line value.
- *Rejected: exposing `serialize_dotenv` from the `secretspec` crate.* It is
  `pub(crate)`; exposing it would have meant a new public API surface in the
  crate the fork keeps close to upstream.
- **Chosen:** `dotenv-ng` is already a **workspace dependency**
  (`Cargo.toml:34`, `dotenv = { package = "dotenv-ng", version = "1.0.0" }`),
  so `sudo-secretspec-cli` added `dotenv.workspace = true` and uses the *same
  parser and renderer the engine's provider does*. No change to `secretspec` at
  all. The wrapper mirrors `serialize_dotenv_pairs` in three lines, and the
  round-trip guard is what proves it still agrees.

### 5. Duplicating the hardened database opener — refused

`history.rs` needed the ledger's protected-open guarantees (directory
metadata, pre/post-open ownership, `0600`, pragmas, dev/ino re-check against a
swap during open). Copying ~180 lines would have created two copies that drift.
Instead `audit.rs`'s `open_connection` was parameterised on the filename and
exposed as `open_protected_db` / `require_protected_dir`. One hardened opener,
two databases.

### 6. An ordering call I made and then withdrew

Sequenced the template-check config change second, arguing a permanently-red
`doctor` was useless as a verification signal for everything after. On reading
the code that was wrong twice over: `declarations` is **also** the install-time
seed for a fresh vault manifest (`install.rs:1053`), a required CLI argument, a
`0444` installed artifact and a `drift` Layout field — a config-schema
migration across four source files and three test files, not one field. And the
rationale was weak anyway: a red `template-check` line does not stop the rest
of `doctor`'s output being readable. **Deferred it**, and said why.

### 7. The redirection — the operator was right and my recommendation was momentum

The operator asked whether a `sqlite://` provider, registered like any other
plugin, was the better level; then whether the whole stack should move to
setuid + sqlite; then whether runas and the provider compose.

My first answer recommended finishing the current plan. **That was wrong**, and
withdrawn: if the provider is the better architecture, finishing six more steps
on the sidecar means building more that then needs reworking. Switching now
wastes less.

Two of my own arguments were also overstated and are corrected in the design
doc rather than dropped:

- **"It needs upstream's provider versioning trait settled (the #354
  lineage)."** Not a blocker. A `sqlite://` provider can be upstreamed as a
  plain `get`/`set`/`delete` provider that *retains* history internally, with
  history/restore reached through concrete methods the broker calls.
- **"A provider can't cover the install truncation."** True but not
  distinguishing — a blindly recreated `vault.sqlite3` would have destroyed
  values *and* history together. Install-time capture is required under both
  designs; it counts against neither.

## Key Decisions

### Original design (four operator decisions, 2026-08-17)

- **Backend: SQLite beside the ledger.** *Rejected: git-wrapped vault*, the
  previously favoured candidate — it buys history by running `git` from a root
  process, inheriting `/etc/gitconfig`, hooks, `.gitattributes` filters and
  `core.fsmonitor`, every one a code-execution vector. The vault directory is
  service-user-writable, so a repo placed there would be outright root RCE.
  *Rejected: `pass`/`gopass`* (GPG toolchain) and *`kdbx`* (capped history).
- **Restore is agent-callable but forward-safe only** — agents may recover a
  name holding no value; overwriting a live value and whole-vault rewind are
  operator-only. Operator chose this over both "agent-callable like set" and
  "operator-only".
- **Scope: vault dotenv + manifest only**, stated as a limit, since keyring has
  no enumeration or history API.
- **`template-check` reports a retired source** as a distinct non-failing
  state, detected from an **explicit absent config key** — *rejected: inferring
  retirement from a missing file*, because a file that will not open is equally
  consistent with a broken install. Same reasoning the `undeclare` guard at
  `broker.rs:809` already documents.

### Derived design decisions

- **`delete` stays soft, a new operator-only `destroy` tombstones.** Closes the
  hole forward-safe restore would otherwise leave: a credential rotated
  *because it leaked* could be revived by an agent. Maps Vault KV v2's
  delete/destroy and AWS Secrets Manager's recovery window onto the fork's
  verbs.
- **The chain binds each value's DIGEST, never its bytes.** That is what makes
  destruction auditable — a destroyed row still proves bytes with digest X were
  captured then and destroyed by entry N, without retaining the value.
  `destroyed_by` is deliberately **outside** the hash (a destroy must not
  invalidate the chain) and kept honest instead by
  `CHECK ((value_blob IS NULL) = (destroyed_by IS NOT NULL))`.
- **A failed archive does not fail the operation.** The mutation has already
  committed and cannot be undone, so reporting failure would misdescribe it.
  `commit()` keeps the rollback copies on disk instead: the bytes survive,
  `drift` reports them as `PENDING_ROLLBACK`, and the degraded state is exactly
  the pre-history behaviour rather than a loss.
- **A rolled-back mutation is not archived.** It never took effect, so
  recording it would fill history with entries identical to the state beside
  them.
- **A listing verifies the chain in the same transaction as the read**, so a
  tampered store is refused rather than reported as fact.
- **`mutates_vault()` + `SOURCE_OPS` replace an inline `matches!`.** `run()`
  only executes as root against a real vault, so nothing inline there is
  test-reachable — the same reasoning that moved the adoption rule into
  `install::adopts_without_flag`. A new verb now breaks a test that pins the
  mutating set, so whether it writes must be stated rather than defaulted.

### Redirection decisions (current plan)

- **Storage moves into a `sqlite://` provider**, boundary-unaware, so identical
  code serves a `0600` service-user file behind the broker and an ordinary
  user-owned store. That is what makes it upstreamable, and it fills a gap this
  design already named: every versioned provider upstream ships is a network
  service, while the operator's binding constraint is no network dependency.
- **Provider history retention must be opt-in** (`sqlite://path?history=…`).
  An upstream user would be surprised to find every prior value retained.
- **Drop the broker from root to the service user via sudo's `Runas_Spec`**
  (`ALL=(root)` → `ALL=(_sudo_secretspec)`, `install.rs:825`). Root looks
  *largely incidental* — it is there to `chown` things **to** the service user,
  unnecessary if the process already is that user. **Verify that claim against
  every privileged path before relying on it**; the paths read so far are
  `require_boundary`, `Mutation::begin`, `audit::open_connection`, the `0444`
  config, and the service-user-owned vault.
- ***Rejected: a setuid binary.*** Same privilege reduction, reached by
  hand-writing the hardening `sudo` performs — env sanitisation, `closefrom`,
  saved-set-uid ordering, supplementary-group dropping, argv/fd/umask/cwd
  hygiene — in a binary guarding real credentials. It also moves authorisation
  out of an externally auditable policy file into `getuid()` checks in our own
  code, when the operator-only-vs-agent-callable split is naturally a sudoers
  rule. Estimated 3–4× the whole remaining plan, failing late.
- **The new plan costs MORE, not less** (~560–750k vs ~250–350k tokens). The
  justification is architecture and a reusable artifact. Recorded explicitly so
  nobody later reads "switching wastes less" as "switching is cheaper".

## Evidence & Data

- **Live vault round-trip, verified under `sudo` against
  `/var/db/sudo-secretspec/.env`**: 39 names, 2858 bytes in and out,
  `orig_sha == rend_sha` (`41fd54fd…dccb`), `ROUND_TRIPS=true`. This is what
  makes the migration safe to attempt — the values can be read out through the
  same grammar that wrote them. The probe was temporary and **has been
  deleted**; it printed digests and lengths only, never a value.
- **Live vault sizes**: `.env` 2858 bytes, `secretspec.toml` 9735 bytes
  (~12.6 KB per full snapshot). Vault dir `0700 _sudo_secretspec`.
- **Audit ledger**: `audit-verify` → **1268 events**, chain intact, tip
  `5c94d040…d87e`, 524288 bytes.
- **Tests**: `cargo test -p sudo-secretspec-cli` → **149 lib** (up from 126) +
  19 + 10 + 6 + 11 + 16 integration, **0 failures**. 15 new in `history.rs`,
  7 new in `broker.rs`.
- **Clippy**: `cargo clippy -p sudo-secretspec-cli --all-targets` → **20
  warnings, unchanged from baseline** before and after.
- **`cargo fmt --all -- --check`** clean.
- **NOT run this session**: the full workspace suite
  (`-p secretspec -p secretspec-derive`), and `pytest tests/sudo_postinstall`.
  Note `cargo test --all` **cannot** run here — `ext-php-rs` needs a PHP
  toolchain — and 21 `provider::sops::*` failures are expected, `sops` is not
  installed.
- **Upstream state, checked 2026-08-17**: `upstream/main` moved
  `dfa4b10` → **`35791a2`** (PR #368 merged). **#374** OPEN, 0 CI checks,
  0 reviews, 1 comment (ours). **#362** OPEN, last comment ours. **#372** 0
  comments. **#373** 0 comments, 0 reviews. Zero maintainer engagement across
  all four.
- **`/usr/local/share/sudo-secretspec/secretspec.toml`**: 9663 bytes,
  `root:wheel`, `0444`, frozen 2026-08-15 — the pre-deletion leftover nothing
  regenerates, which is why `template-check` can never converge.

## Operator Feedback

- **"Do your plan"** — executed all four steps of the stated resume plan.
- **"We def need to fix the gaps. Please view closing the gaps and implementing
  this feature as one project, and do all elements of the project in the most
  advantageous ordering."** — built a 10-step ordering with two deliberate
  departures from the design doc's plan, both explained at the time.
- **"pause"** (mid-turn) — stopped at a clean seam: committed, pushed, all
  suites green, Tier 1 updated. Did not leave work dangling.
- **Rejected an `AskUserQuestion` with "the user wants to clarify"** — take the
  signal: this operator prefers discussing an architectural tradeoff in prose
  over picking from prepared options. Subsequent turns were handled as
  discussion and that worked.
- **Asked for a token estimate explicitly** before deciding. Provide costed
  options for architectural choices with this operator; they price decisions.
- **"Okay, we are going to go with switch. Do the handoff."**

## Where We're Going

1. **THE NEXT ACTION: build the `sqlite://` provider** in the `secretspec`
   crate — a plain `get`/`set`/`delete` provider with **no boundary
   awareness**. `secretspec/src/provider/sqlite.rs`, `#[provider]`
   registration, URI parsing, profile-aware storage, cases in
   `secretspec/src/provider/tests.rs`. Follow the ~35 siblings in that
   directory for convention.
2. **Opt-in internal versioning** behind a URI option. Carry the chain design
   over from `history.rs` verbatim — it is built and tested.
3. **Provider documentation in all seven locations** from `CLAUDE.md`'s
   checklist, plus the credentials catalog and
   `npm --prefix docs run check:provider-credentials`. Version-label everything
   unreleased.
4. **Reduce `history.rs`** to manifest history + install-time capture (gap 2),
   which is what the boundary piece becomes once values live in the provider.
5. **Restore and destroy verbs** with the authorisation split unchanged.
6. **Migrate the live vault** — 39 secrets, on a host where that file has
   already been truncated once. Rehearse with a rollback; do not one-shot it.
7. **Runas reduction** — independent of all the above, takeable any time.
8. **Docs, sudoers, CHANGELOG, release, postinstall verification.**
9. **DEFERRED but decided, not forgotten:** `declarations` → `Option<PathBuf>`
   so `template-check` can report a retired source. Shape and reasoning in
   `docs/design/template-check-resync.md` § "Decision, 2026-08-17".
10. **Watch #374**; if it needs changes make them in `/Users/djbclark/src/ss-370`
    and port to `sudo-main` separately — upstream's `add_secret_to_manifest` is
    description-only, the fork's takes `required: Option<bool>`.
11. **Do NOT delete branch `explore/pr-334-rust-first-spec`** — `bf0b25c` is
    linked by SHA from a public comment on upstream #357.
12. **DEFERRED per explicit operator instruction, do NOT resurface:** item 12
    cross-platform sudo/Linux port
    (`docs/design/privilege-boundary-and-packaging.md:487`).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -5          # expect c124c0b at tip, tree clean, pushed
git worktree list             # ss-370 must still be there until #374 resolves

# READ FIRST — the redirection section is the current plan, and takes
# precedence over the design section below it in the same file:
#   docs/design/infinite-secret-backup-restore.md
#     § "Redirection: storage moves into a sqlite:// provider"
#     § "Implementation plan (current, post-redirection)"

# The conventions the new provider must follow:
ls secretspec/src/provider/            # ~35 siblings
sed -n '/Adding Provider Documentation/,/adding-providers.md/p' CLAUDE.md

# What already exists and transfers:
sed -n '1,60p' sudo-secretspec-cli/src/history.rs   # chain + shape rationale
cargo test -p sudo-secretspec-cli --lib history     # 15 tests, all passing

# Tests (--no-fail-fast REQUIRED or the CLI crate never runs):
cargo test --no-fail-fast -p secretspec -p secretspec-derive -p sudo-secretspec-cli
# `cargo test --all` CANNOT run here: ext-php-rs needs a PHP toolchain.
# 21 provider::sops::* failures are EXPECTED — `sops` is not installed.
cargo clippy -p sudo-secretspec-cli --all-targets   # baseline is 20 warnings

# Confirm the boundary — do not assume:
sudo-secretspec --version      # expect 0.19.1-sudo.18; NOTHING here is released
sudo-secretspec doctor
sudo-secretspec audit-verify   # was 1268 events

# Upstream:
gh pr view 374 --repo cachix/secretspec --json state,comments,reviews,statusCheckRollup
git fetch upstream main && git log --oneline -1 upstream/main   # was 35791a2
```
