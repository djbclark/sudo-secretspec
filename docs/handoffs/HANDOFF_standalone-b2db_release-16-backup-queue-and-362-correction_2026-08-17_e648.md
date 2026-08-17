---
schema_version: 1
handoff_id: e648
parent_handoff_ids: [90ab]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: e0cffbcdc8002d6c949ae77e6e8206a907fb785e
created_at: 2026-08-17T11:47:00-0400
writer: claude-code
---

# Handoff — 0.19.1-sudo.16 released, backup/restore feature queued, #362 comment corrected and posted

## The Goal

Resume `90ab`, whose single next action was cutting `0.19.1-sudo.16` (the
release carrying the install-truncation fix). Along the way the operator
added three new asks: queue a new "infinite secret backup/restore" feature,
require prior-art search before designing it, and build deterministic
release tooling. All landed.

## Where We Are

Branch `sudo-main` at `e0cffbc`, **tree clean, pushed**
(`b97abf1..e0cffbc` to `frdminc/sudo-secretspec`, 12 commits this session).

**`0.19.1-sudo.16` is released AND installed.** Tag, GitHub Release, formula,
and tap all published; `brew reinstall` + `brew test` green; boundary
installed with `--adopt-existing`, printing the upgrade arrow
`0.19.1-sudo.15 -> 0.19.1-sudo.16`. Post-install verification:

```
sudo-secretspec --version   -> 0.19.1-sudo.16
sudo-secretspec doctor      -> OK
sudo-secretspec check       -> 46 found, 0 missing, 7 optional
pytest tests/sudo_postinstall -> 23 passed, 8 skipped
```

The truncation bug that motivated this release is now fixed **on this
machine** — plain `sudo-secretspec install` (no flag) is no longer
destructive here. It remains destructive on any machine still running
`.15` or earlier.

**Deterministic release tooling shipped and dogfooded this session** —
`justfile` (`just release 0.19.1-sudo.N`) + `packaging/stamp.py`. This
release was cut through it, live, not by hand.

**A new feature is queued** — "infinite secret backup/restore" — recorded in
`docs/design/infinite-secret-backup-restore.md`, explicitly sequenced
*after* #370's manifest-edit work. Not designed, not started.

**Upstream PR #362 has our comment**, posted after catching and correcting a
false claim from the *previous* session's analysis — see What We Tried.

### Files changed this session

- `justfile` (new) — `just release <version>` / `just release-dry <version>`
- `packaging/stamp.py` (new) — counted, idempotent version stamping,
  imports validation from `release.py` rather than duplicating it
- `docs/design/infinite-secret-backup-restore.md` (new, built up in 3
  commits) — queued-feature record: motivation, mandatory search-first
  step, prior-art findings, no-network-dependency constraint, local
  versioned-backend candidates
- `docs/design/pr362-comment.md` (new) — the posted PR comment's exact text,
  landed as a tracked file rather than left in scratch
- `docs/design/upstream-ipc-v1-and-the-fork.md` (modified, twice) —
  corrected the false ACL claim in "Next actions" item 1, then recorded the
  posted-comment URL
- Release-mechanical: `Cargo.toml`, `Cargo.lock`, `secretspec-derive/Cargo.toml`,
  `sudo-secretspec-cli/Cargo.toml` (stamped .15 → .16),
  `packaging/homebrew/sudo-secretspec.rb` (restamped by `release.py`)

Nothing else in the repo was touched. `#370` manifest-edit remains
completely untouched — now **four** sessions running.

## What We Tried

### 1. Cut `0.19.1-sudo.16` via the new justfile — worked end to end

`just release-dry 0.19.1-sudo.16` rehearsed cleanly (disk guard, stamp,
`release.py --dry-run --skip-tests` printed every mutating command).
`just release 0.19.1-sudo.16` then ran live: stamped, committed
(`3a9bc3e`), pushed, `release.py` ran the gating tests, created the tag,
published the GitHub Release, rewrote and committed the formula
(`c54beea`), synced the tap, `brew reinstall` (10 min cold build,
13 files, 46.6MB), `brew test` passed, readback verified. No manual
intervention needed — the whole point of building the tool this session.

### 2. Drafted a #362 comment reusing the prior session's analysis — caught a false claim before posting

The existing design doc (`docs/design/upstream-ipc-v1-and-the-fork.md`,
written the previous session) claimed `/Library/Application Support` is
"admin-group writable" on macOS, as the motivating example for why the
fork's ancestor-chain check exists. Before posting anything upstream, I
re-verified every claim against the PR's actual current head and this
machine's real state:

```
gh pr view 362 --json headRefOid   -> 337950c762a7892a847284e2c0402938cd10f067
/bin/ls -lde "/Library/Application Support"
  -> drwxr-xr-x  33 root  admin  1056 ... (no ACL line, no + suffix)
```

**The claim was false.** The directory is `root:admin 0755` with no ACL —
not writable by the admin group. Had this gone upstream unverified, it
would have been a factual error in a comment on someone else's open PR.

Rewrote the review points from what actually holds up, reading
`external.rs` at `337950c` directly:
- `check_file_security`/`check_parent_security` (~lines 307–360) gate only
  on `metadata.mode() & 0o022`, which is **blind to macOS ACLs** — an
  extended ACL can grant group write while POSIX bits read clean. The
  Windows path in the same file already validates ACLs
  (`path_acl_is_trusted`); the unix path has no equivalent.
- Both use `std::fs::metadata`, which **follows symlinks** — validation
  targets the resolved path, not the literal registered one.
- Trust does genuinely stop at the **immediate parent** — this part of the
  original claim held. Everything above it is unchecked; the default macOS
  chain being sound is assumed, not verified.

Cross-referenced against this fork's own `check_ancestor_chain`
(`sudo-secretspec-cli/src/drift.rs:330-368`), which walks the resolved
chain to `/`, rejecting non-root ownership, group/world-writable mode,
**any extended ACL** (`has_extended_acl`, shells out to `/bin/ls -lde`),
and symlinked components — this is what the comment offers as prior art.

### 3. Operator said "pause immediatly" mid-draft — stopped without posting

Caught mid-flow, before the comment was posted and before any further tool
call. Reported exact state: release fully done and verified; the corrected
comment drafted in scratch but **not yet landed as a tracked file, not
posted, and the design doc not yet corrected**. No destructive or
irreversible action had occurred at the pause point.

### 4. Resumed: corrected the doc, landed the draft, posted on explicit approval

On "resume": rewrote `docs/design/upstream-ipc-v1-and-the-fork.md`'s Next
Actions item 1 in place with a `**CORRECTED 2026-08-17**` marker stating
the original claim was false and must not be posted, replaced with the
verified points above (`37f187a`). Copied the draft from scratch into
`docs/design/pr362-comment.md` as a tracked file — a paused/resumed session
needs the exact wording durable, not sitting in `/tmp`. Only after the
operator explicitly said "Yes, post" did `gh pr comment 362` run.

## Key Decisions

- **Did not post the #362 comment without explicit operator approval**,
  even though the correction work was already done and the draft was
  ready. Commenting on someone else's open PR is a visible external action
  under this session's action-care policy; asked, got "Yes, post," then
  posted.
- **Corrected the record rather than silently fixing the output.** The
  false claim was in a committed design doc from a previous session, not
  just in my own scratch draft — leaving it uncorrected would let a future
  session (or a search hit) resurface the false claim even though this
  session's *comment* was accurate. Both are now consistent.
- **Landed the draft as a tracked file, not left in scratch.** The
  mid-draft pause is exactly the scenario a tracked file protects against
  — scratch is session-local and would not have survived a compaction or a
  fresh session picking this up.
- **Built `packaging/stamp.py` as a thin layer over `release.py`'s own
  validation** (imports `ANY_VERSION_RE`, `ReleaseError`, `parse_release`
  from it via `importlib`) rather than re-deriving the version regex or the
  downstream-suffix convention — one spelling of the rules, per the
  existing `release.py` docstring's own stated design principle.
- **`justfile`'s `min_free_gi := "25"` disk guard** is a new invariant, not
  present in `release.py` itself — added because the `.15` release died at
  100% disk *after* publishing (recorded in `90ab`/`7c73`). Guarding in the
  justfile keeps `release.py` itself simple and keeps the guard visible at
  the call site the operator actually runs.
- **No network-dependent backend for infinite backup/restore.** Operator
  constraint, recorded verbatim in the design doc. Ruled out every
  in-tree networked provider (Vault, AWS, Azure, Keeper, etc.) as history
  *sources* for this feature, even though several of them already implement
  versioning — the versioning has to happen locally.

## Evidence & Data

- Commits this session, chronological: `3ac21c8` (backup/restore stub),
  `98d07e0` (search-first directive + prior art), `ca8c611` (no-network
  constraint + backend candidates), `0ccaca7` (justfile + stamp.py),
  `3a9bc3e` (stamp commit, via `just release`), `c54beea` (Homebrew
  formula, via `release.py`), `37f187a` (ACL-claim correction),
  `e0cffbc` (posted-comment record).
- Release artifacts: tag `v0.19.1-sudo.16` on `frdminc/sudo-secretspec`;
  GitHub Release published (not draft); formula and tap both restamped;
  install rollback snapshot `/usr/local/libexec/sudo-secretspec-rollback-1786975465`,
  `rollback_artifacts=8`, `pruned_snapshots=1`.
- Vault health across the session: 46 found / 0 missing / 7 optional both
  before and after the release+install (up from the `90ab` handoff's
  recorded 45/0/8 — still net positive, no regression from the install).
- Disk: 70Gi free at session start, never approached the threshold that
  killed `.15`.
- Upstream check (this session): `cachix/secretspec` issues/PRs searched
  for `backup`, `restore`, `versioning`, `snapshot`, `rollback`, `history`,
  `undo`, `revert` — **no dedicated thread exists** for the queued backup
  feature. Web search likewise found no secretspec-specific discussion;
  recorded prior art from Vault KV v2, AWS Secrets Manager, Azure Key
  Vault, `pass`/git.
- `#372`/`#373`: still open, **zero comments, zero reviews** as of this
  session's check (`gh issue view 372` / `gh pr view 373`).
- **Tests, precisely what ran:** `just release` invoked `release.py`'s
  default gate — `pytest tests/sudo_packaging -q`, `cargo test -p
  sudo-secretspec-cli --locked`, `cargo build -p secretspec --locked` — all
  three with `check=True`, so a failure would have aborted before any
  publish step ran; exact pass counts were not captured (tail-truncated in
  this session's tool output), but the release publishing at all is proof
  they passed. Explicitly captured this session: `pytest
  tests/sudo_postinstall -q` → **23 passed, 8 skipped**. **NOT run this
  session:** the full workspace suite
  (`cargo test --no-fail-fast -p secretspec -p secretspec-derive -p
  sudo-secretspec-cli`) — last run in `90ab`'s session as
  `install::` only (34/34); run it fresh before trusting broader coverage.
- `#362` comment posted:
  https://github.com/cachix/secretspec/pull/362#issuecomment-5316998149
  (PR head at post time: `337950c762a7892a847284e2c0402938cd10f067`).
- `upstream/main` unchanged at `dfa4b10` throughout the session.
- In-tree provider survey for the backup feature (`secretspec/src/provider/`):
  no local provider implements versioning — `kdbx`, `pass`, `gopass` are the
  closest local candidates but none is currently wired for history-as-a-feature
  in this codebase; `rusqlite` is already a dependency of
  `sudo-secretspec-cli` (backs the audit ledger).

## Operator Feedback

- **"We want to add infinite secret backup/restore as a feature after all
  of this other stuff"** — accepted as queued, sequenced explicitly after
  the existing work. Recorded rather than discussed at length, per the
  standing instruction not to over-elaborate on exploratory asks.
- **"Also be sure we search issues and the web generally for any discussion
  of that feature."** — done immediately (no upstream thread, no web
  discussion), and turned into a **mandatory repeatable step** in the
  design doc so a future design session re-checks rather than trusting a
  stale first pass.
- **"Re: backups, is there another backend we could use that already
  supports versioning but also that isn't a network dependency?"** —
  answered by surveying in-tree providers; none qualify locally, so
  recorded candidates to build on top of instead (git-wrapped vault, `pass`/
  `gopass`, `kdbx`, a SQLite table beside the audit ledger) rather than
  a ready answer.
- **"Re: cutting releases we should make a just/justfile or other way of
  cutting a release that is deterministic."** — built same session and
  used it for the live `.16` cut, not left as an unused artifact.
- **"pause immediatly"** — stopped instantly with zero further tool calls;
  reported exact state including what was and wasn't yet durable.
- **"Yes, post, and then handoff."** — both executed in order.

## Where We're Going

1. **THE NEXT ACTION: reimplement manifest-edit on the `Spec` shape** for
   upstream #370's reference implementation — untouched across **four**
   sessions now (`7c73` → `e439` → `90ab` → this one). Wrinkle carried
   forward unchanged: `Spec::from_toml` rejects non-empty `project.extends`,
   so the reparse needs an extends-aware variant seeded from `self.base_dir`,
   and the retained text must be the ROOT file only.
2. **Watch #362 for a maintainer reply** to the corrected comment posted
   this session.
3. **Watch #372 and #373** — still zero engagement after two sessions.
   Branch `fix/check-report-to-stdout` is pushed; recreate its worktree
   with `git worktree add <dir> fix/check-report-to-stdout` if changes are
   requested.
4. **Design the infinite backup/restore feature — after #370, not before.**
   Mandatory first step per `docs/design/infinite-secret-backup-restore.md`:
   re-search upstream issues and the web (do not trust this session's first
   pass as still current). Constraint: no network dependency. Candidates
   already recorded: git-wrapped vault (the `pass` model — likely leading
   candidate, smallest diff from the current vault shape), `pass`/`gopass`
   providers (already in-tree, brings a GPG dependency), `kdbx` per-entry
   history (depth capped, needs verification the Rust stack preserves it),
   SQLite append-only table beside the existing audit ledger.
5. **Open question, operator's call:** make `--adopt-existing` the default.
   The guard removes the danger either way; ergonomics only.
6. **Do NOT delete branch `explore/pr-334-rust-first-spec`** —
   `bf0b25c` is linked by SHA from a public comment on upstream #357.
7. **DEFERRED per explicit operator instruction, do NOT resurface:** item 12
   cross-platform sudo/Linux port
   (`docs/design/privilege-boundary-and-packaging.md:487`).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect e0cffbc at tip, tree clean, pushed

# Confirm the release landed and the boundary is current — don't assume:
sudo-secretspec --version                       # expect 0.19.1-sudo.16
sudo-secretspec check --reason "orientation"     # expect ~46 found, 0 missing
sudo-secretspec doctor                           # expect OK

# Re-check upstream contact FIRST, every session:
cat sudo-secretspec/UPSTREAM-CONTACT.md
git fetch upstream main && git log --oneline -1 upstream/main   # was dfa4b10

# Check for replies to the posted comment:
gh pr view 362 --repo cachix/secretspec --json comments --jq '.comments[-3:]'
gh issue view 372 --repo cachix/secretspec --json comments
gh pr view 373 --repo cachix/secretspec --json comments,reviews

# The deterministic release tool, for the NEXT release after #370 lands:
just --list
just release-dry 0.19.1-sudo.17     # rehearse first, always

# Tests (--no-fail-fast REQUIRED or the CLI crate never runs):
cargo test --no-fail-fast -p secretspec -p secretspec-derive -p sudo-secretspec-cli
# `cargo test --all` CANNOT run here: ext-php-rs needs a PHP toolchain

# Backup/restore design entry point when the queue clears:
cat docs/design/infinite-secret-backup-restore.md
# STEP 1 IS MANDATORY: re-search upstream issues + the web before designing.
```
