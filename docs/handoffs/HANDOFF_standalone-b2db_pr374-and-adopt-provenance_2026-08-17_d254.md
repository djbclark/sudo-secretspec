---
schema_version: 1
handoff_id: d254
parent_handoff_ids: [e648]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: ccf72dd0ca78e06749a7102adc3660eda51389f5
created_at: 2026-08-17T12:16:22-0400
writer: claude-code
---

# Handoff — upstream PR #374 opened for #370, and `--adopt-existing` split on provenance

## The Goal

Resume `e648`, whose single next action was reimplementing `manifest-edit` on
the `Spec` shape for upstream #370 — **untouched across four prior sessions**
(`7c73` → `e439` → `90ab` → `e648`). The operator also asked, at the start,
for the pros and cons of making `--adopt-existing` the default for `install`
— the open question `90ab` and `e648` had both left for them — then chose the
recommended option, which added a second workstream.

Both landed. #370 is now upstream PR **#374**, open.

## Where We Are

Branch `sudo-main` at `ccf72dd`, **tree clean, pushed** (`523e889..ccf72dd`,
3 commits this session).

**Upstream PR #374 is OPEN**: https://github.com/cachix/secretspec/pull/374
— "Format-preserving single-declaration edits on `Spec`". 9 files,
+1035/−87, head `b3637e2`, base `main`. Zero CI checks had reported at the
time of writing (`statusCheckRollup` empty); no comments, no reviews yet.

A **second worktree is deliberately still present**:

```
/Users/djbclark/src/sudo-secretspec  ccf72dd [sudo-main]
/Users/djbclark/src/ss-370           b3637e2 [spec-manifest-edit]
```

`ss-370` is the PR branch, built off `upstream/main` (`dfa4b10`), **not**
cherry-picked from `sudo-main`. Keep it until #374 resolves — review changes
belong there, not on `sudo-main`.

**`0.19.1-sudo.17` is NOT cut.** The installed boundary is still
`0.19.1-sudo.16`. Both of this session's features are unreleased.

### Commits this session

| SHA | What |
|---|---|
| `6457339` | `feat(install)`: adopt the installed vault without `--adopt-existing` |
| `0483d3a` | `feat(spec)`: format-preserving single-declaration edits on `Spec` |
| `ccf72dd` | `docs`: record the #370 PR body as submitted |

### Files changed

**Fork (`sudo-main`), adopt-existing:** `sudo-secretspec-cli/src/install.rs`,
`sudo-secretspec-cli/src/main.rs`, `tests/sudo_postinstall/test_postinstall.py`,
`packaging/homebrew/sudo-secretspec.rb`, `sudo-secretspec/AI-GUIDANCE.md`,
`skills/sudo-secretspec/SKILL.md`, `sudo-secretspec/README.md`, `FORK-AI.md`,
`CHANGELOG.md`.

**Fork (`sudo-main`), #370:** `secretspec/src/spec.rs`,
`secretspec/src/config.rs`, `secretspec/src/manifest_edit.rs`, `Cargo.toml`,
`Cargo.lock`, `CHANGELOG.md`, and `docs/design/pr370-body.md` (new).

**PR branch (`ss-370`):** the same `spec.rs`/`config.rs` diff applied byte for
byte, plus a *newly created* `secretspec/src/manifest_edit.rs` (upstream has
no such module), `secretspec/src/cli/mod.rs` (helpers removed and imported),
`secretspec/src/lib.rs`, `Cargo.toml`, `secretspec/Cargo.toml`,
`Cargo.lock`, `CHANGELOG.md`.

## What We Tried

### 1. The `--adopt-existing` question — the framing in the handoffs was wrong

`90ab` and `e648` both recorded this as "make `--adopt-existing` the default:
ergonomics only." Reading the actual code first showed the question was
narrower than three handoffs had stated:

- **Interactive installs already adopted by default.** `main.rs:511-528`
  prompts `"Found existing vault at X. Adopt it?"` via
  `prompt_yes_no(..., true)` — bare Enter means yes. The flag was only
  load-bearing for `--non-interactive`, non-TTY, or an explicit "no".
- **Since `.16`, not adopting is not destructive.** The guard at
  `install.rs:863` hard-refuses, and `create_fresh_runtime_files`
  (`install.rs:1167`) refuses again at the two writes that destroy data.

So the real question was only: *should a non-interactive install onto an
existing boundary adopt silently, or refuse?* Presented pros and cons on that
basis, with a recommendation. Operator: **"i'll go with your recommendation."**

### 2. First attempt at testing the refusal message was a tautology — caught and replaced

The initial test for the improved refusal text rebuilt the message inside the
test and asserted against its own copy. That asserts nothing. Replaced by
extracting `fresh_install_refusal(&Path) -> InstallError` and asserting the
real function's output. Same motive as the extraction the truncation incident
forced: the guard only fires as root against a populated `/var/db`, so
anything left inline there is untestable, and the last thing hidden behind
that unreachability truncated a vault.

### 3. A test fixture was wrong, and validation caught it

`a_declaration_survives_every_field_it_was_given` first used
`.at_least_one(["auth"])`, which failed with *"at_least_one group 'auth' must
contain at least two secrets"*. The validation was right; the fixture was
wrong. Rewritten around `ref` — `NativeAddress { item, field }` — which is a
better test anyway: it serializes as a **nested table**, which is precisely
where a writer that only knows how to place scalars into an inline table
breaks.

### 4. `toml_edit`'s serde support is feature-gated — needed enabling

`toml_edit::ser::to_document` did not resolve: `pub mod ser` is
`#[cfg(feature = "serde")]` and the workspace had `toml_edit = "0.23"` with
default features (`parse`, `display`) only. Enabled `features = ["serde"]`.
Verified the cost: `Cargo.lock` gains only `serde_core` and `serde_spanned`,
**both already in the tree via `toml`** — no new crates fetched.

Rejected the alternative of serializing through `toml::Value` and
hand-writing a `toml::Value` → `toml_edit::Value` mapper. That is a second
representation to keep in sync forever, which is exactly the failure mode the
whole design is trying to avoid.

### 5. `toml_edit`'s *value* serializer refuses nested tables

`ValueSerializer` cannot emit a nested table, and `ref`, `refs`, `extract`,
`generate`, and a presence group's `required` all produce one. Resolution:
serialize the whole `Secret` with `to_document`, then flatten the resulting
table's items into an `InlineTable`. Each item's `into_value()` is checked
rather than unwrapped, so a future field with no inline form is a named error
instead of a panic.

### 6. Deliberately did NOT port the fork's tri-state `required` upstream

The fork's `add_secret_to_manifest` takes `required: Option<bool>`; upstream's
takes only a description. The issue text explicitly says #334 settled
requiredness and it is not being re-raised, so the PR keeps upstream's
signature and routes everything richer through `add_secret_value_to_manifest`
and the `Secret` that `add_secret_to_text` already takes. **The two trees now
differ here** — see Where We're Going item 2.

### 7. Briefly wrote the PR body into the PR branch, then moved it

`PR_BODY.md` was created inside `ss-370`, which would have shipped it in the
diff. Removed it from the branch and re-landed it as a tracked file on
`sudo-main` at `docs/design/pr370-body.md` — the same reasoning `e648` applied
to `pr362-comment.md`: scratch does not survive a session, and a review round
has to be read against the exact submitted wording.

## Key Decisions

- **Adoption defaults on provenance, not convenience.** `detect_existing_vault`
  now returns `ExistingVault { vault, service_user, service_group, origin }`
  with `VaultOrigin::{InstalledConfig, PathScan}`. `InstalledConfig` adopts
  with no flag; `PathScan` still requires it.
  - *Rejected: adopt on any detected vault.* The scan's candidate list
    includes `LEGACY_VAULT`, which migration leaves on disk on purpose.
    Silently binding a new boundary to retired secrets is exactly what
    `detect_existing_vault`'s own doc comment says it exists to prevent.
  - *Rejected: keep the flag mandatory everywhere.* The Homebrew caveat's
    copy-pasteable line then fails on every upgrade, and a refusal invites a
    script to "fix" it by deleting the vault — worse than an adopt.
- **The trust rule is `install::adopts_without_flag(VaultOrigin) -> bool`**,
  named and unit-tested, not a comparison inline in `main.rs`'s `run_install`,
  which no test can reach.
- **The automatic adoption is announced on stderr**, naming both the vault and
  `CONFIG_PATH`. An operator who believed they were installing clean finds out
  at the install, not from a later surprise.
- **`preserved_text()` returns `Option`, `to_toml()` returns `Result<String>`;
  they stay two methods.** A single renderer whose exactness depended on hidden
  state would hand a silently regenerated document to exactly the callers who
  need the original.
- **Edits reparse rather than hand-mutate.** `config`/`compiled` are re-derived
  from the edited text through the same validated path every other `Spec` uses.
  One synchronization point instead of one per method; the two cannot disagree,
  and an invalid edit fails at the edit.
- **`Spec`'s retained text is the ROOT document only.** `Config::try_from`
  folds parents into the child; retaining the merged text would silently inline
  inherited declarations into a file that had merely referenced them.
- **`SpecBuilder` stays non-format-preserving**, per the issue. `into_builder()`
  / `to_builder()` remains a hard boundary: only `Spec` carries source text.
- **PR built on a worktree off `upstream/main`, not cherry-picked** — same
  method as #373, so the diff carries no fork-local noise.

## Evidence & Data

- **Full workspace suite** (`cargo test --no-fail-fast -p secretspec
  -p secretspec-derive -p sudo-secretspec-cli`): **1367 passed, 21 failed,
  4 ignored.** All 21 failures are `provider::sops::*`, every one reporting
  *"The 'sops' CLI is not installed"*; `which sops` confirms it is absent.
  Environmental. This is the first full-suite run since `90ab`.
- **PR branch, clean `dfa4b10` base**: 1363 passed, same 21 `sops` failures.
  All new `spec::tests::text_edits*` and `manifest_edit::tests` pass there.
- `cargo clippy -p secretspec --all-targets`: **20 warnings before and after**
  on the PR branch — verified by stashing. Fork side: 21 → 20 for
  `sudo-secretspec-cli`. No new warnings in any touched file.
- `cargo fmt --all -- --check` clean in both trees.
- Feature isolation verified: `secretspec` builds with `--no-default-features`,
  with `--no-default-features --features manifest-edit`, and with defaults.
- **PR #374**: OPEN, 9 files, +1035/−87, head `b3637e2`, base `main`,
  `statusCheckRollup` empty (0 checks reported).
- **Upstream state at session end**: `upstream/main` still `dfa4b10`
  throughout. **#362** — no maintainer reply; the last human comment is still
  ours (2026-08-17 14:16Z). **#372** — 0 comments. **#373** — 0 comments,
  0 reviews. **#370** — 0 comments, still OPEN.
- **Environment**: installed boundary `0.19.1-sudo.16`, `doctor: OK`, disk
  65Gi free (the `justfile` guard is 25Gi).
- **NOT run this session**: `pytest tests/sudo_postinstall`. The new test
  `test_install_dry_run_adopts_the_installed_vault_without_the_flag` fails
  against `.16` **by design** — this suite's stated convention is that a
  version-dependent gate fails rather than skips (see the comment at
  `test_postinstall.py:168`).
- **Live behaviour of the adopt change is UNVERIFIED end to end.** Only unit
  tests cover it. It cannot be exercised until `.17` is installed.

## Operator Feedback

- **"resume - and give me pros and cons of making it the default for install"**
  — answered from the code rather than from the handoffs, which had the
  framing wrong (see What We Tried §1).
- **"i'll go with your recommendation."** — implemented the provenance split
  exactly as recommended, including both companion fixes (runnable refusal
  command; Homebrew caveat corrected).
- **"pause whenever there is a good moment to do so"** — paused after the #370
  fork-side commit, at a clean seam: tree clean, pushed, suite run. Resolved
  a 21-vs-22 failure-count discrepancy before stopping rather than leaving it
  dangling; all failures were `sops`.
- **"resume"** — continued to the upstream PR.
- **Approval for the outward action was asked and given** ("Yes — push and open
  the PR"). Consistent with `e648`'s precedent for the #362 comment: publishing
  to someone else's repo is confirmed, not assumed.

## Where We're Going

1. **THE NEXT ACTION: watch #374.** No CI checks had reported when it was
   opened. `gh pr view 374 --repo cachix/secretspec --json
   state,comments,reviews,statusCheckRollup`. **djbclark has pull-only access
   and cannot approve a blocked CI run** — a maintainer must.
2. **If #374 needs changes, make them in `/Users/djbclark/src/ss-370`**, then
   port back to `sudo-main` *separately*. The two trees diverge on purpose:
   upstream's `add_secret_to_manifest` is description-only, the fork's takes
   `required: Option<bool>`. A blind copy either way breaks one of them.
3. **Cut `0.19.1-sudo.17`**: `just release-dry 0.19.1-sudo.17`, then
   `just release 0.19.1-sudo.17`. It carries both this session's features.
4. **After installing `.17`, run `pytest tests/sudo_postinstall`.** This is the
   only live verification of the new adopt behaviour; nothing has exercised it
   end to end.
5. **Watch #362** for a maintainer reply —
   https://github.com/cachix/secretspec/pull/362#issuecomment-5316998149
6. **Watch #372 and #373** — still zero engagement across three sessions.
   Branch `fix/check-report-to-stdout` is pushed.
7. **Optional, not started:** dogfood the new API — the fork's broker still
   calls `secretspec::manifest_edit` directly rather than
   `Spec::add_secret_to_text` / `remove_secret_from_text`. Doing so would
   exercise it on the fork's own provable-undo path.
8. **Design the infinite backup/restore feature** after #374 settles.
   Mandatory step 1 per `docs/design/infinite-secret-backup-restore.md`:
   re-search upstream issues and the web; do not trust the 2026-08-17 pass as
   current. Constraint: no network dependency.
9. **Do NOT delete branch `explore/pr-334-rust-first-spec`** — `bf0b25c` is
   linked by SHA from a public comment on upstream #357.
10. **DEFERRED per explicit operator instruction, do NOT resurface:** item 12
    cross-platform sudo/Linux port
    (`docs/design/privilege-boundary-and-packaging.md:487`).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect ccf72dd at tip, tree clean, pushed
git worktree list             # ss-370 must still be there until #374 resolves

# THE next action:
gh pr view 374 --repo cachix/secretspec --json state,comments,reviews,statusCheckRollup

# Other upstream items:
gh pr view 362 --repo cachix/secretspec --json comments --jq '.comments[-2:]'
gh issue view 372 --repo cachix/secretspec --json comments
gh pr view 373 --repo cachix/secretspec --json comments,reviews
git fetch upstream main && git log --oneline -1 upstream/main   # was dfa4b10

# Confirm the boundary — do not assume:
sudo-secretspec --version                      # expect 0.19.1-sudo.16 until .17 is cut
sudo-secretspec doctor                         # expect OK
sudo-secretspec check --reason "orientation"

# Tests (--no-fail-fast REQUIRED or the CLI crate never runs):
cargo test --no-fail-fast -p secretspec -p secretspec-derive -p sudo-secretspec-cli
# `cargo test --all` CANNOT run here: ext-php-rs needs a PHP toolchain.
# 21 provider::sops::* failures are EXPECTED — `sops` is not installed.

# The release, when ready (disk guard is 25Gi):
just release-dry 0.19.1-sudo.17
just release 0.19.1-sudo.17
# THEN, and only then, the live check of the new adopt behaviour:
pytest tests/sudo_postinstall

# The PR branch:
cd /Users/djbclark/src/ss-370 && git log --oneline -2   # b3637e2 on dfa4b10
cat /Users/djbclark/src/sudo-secretspec/docs/design/pr370-body.md   # as submitted
```
