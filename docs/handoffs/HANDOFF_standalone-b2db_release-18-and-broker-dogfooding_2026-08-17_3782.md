---
schema_version: 1
handoff_id: 3782
parent_handoff_ids: [d254]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: cb80ed67815993114d8e648f2192ac957056d20c
created_at: 2026-08-17T15:23:16-0400
writer: claude-code
---

# Handoff — releases `.17` and `.18`, broker dogfooded onto `Spec`, backup/restore step 1

## The Goal

Resumed `d254` via `/baton`. Its stated next action was "watch #374", which
turned out to be a no-op — the PR has had zero CI checks, comments and reviews
since it opened, and djbclark has pull-only access so a blocked run cannot be
unblocked from here. The real head of the queue was therefore `d254`'s items 3
and 4: cut `0.19.1-sudo.17` and get the first live verification of the
`--adopt-existing` provenance split.

Operator then directed, in order: the release; then (after a recommendation)
dogfooding the new `Spec` API in the broker; then the `CLINE_API_KEY` drift and
an upstream comment, followed by a second release; then, after a pause, the
backup/restore design.

**Two releases shipped, the broker is ported, and backup/restore step 1 is
done.** The design proper is not started.

## Where We Are

Branch `sudo-main` at `cb80ed6`, **tree clean, pushed**. 15 commits this
session (`d342215..cb80ed6`), 14 files, +437/−52.

- **`0.19.1-sudo.17`** — released and installed. Carried the `#370` `Spec`
  work and the `--adopt-existing` provenance split from `d254`.
- **`0.19.1-sudo.18`** — released and installed. Carries the broker `Spec`
  port and two message fixes. **This is the installed boundary now.**
- Vault verified `46 found / 0 missing / 7 optional` before and after *both*
  installs. `doctor: OK` throughout.
- Second worktree `/Users/djbclark/src/ss-370` (`spec-manifest-edit`,
  `b3637e2`) still present and still required — keep until #374 resolves.

### Commits this session

| SHA | What |
|---|---|
| `b2fef0c` | `fix(release)`: stop telling operators an upgrade needs `--adopt-existing` |
| `b6e0f75` / `7e80cc4` | `.17` stamp + formula |
| `c15e053` | `fix(install)`: drop the stale `--adopt-existing` from the self-install refusal |
| `404a90b` | `test(postinstall)`: drive install plans through the package broker |
| `c506a6f` | `refactor(broker)`: edit the runtime manifest through the public `Spec` API |
| `862c7be` | `docs(postinstall)`: the declare gate's "no inverse" claim is stale |
| `5384a27` | `docs`: record the #374 dogfooding comment as submitted |
| `5e3f771` / `ff8ad6b` | `.18` stamp + formula |
| `cb80ed6` | `docs(design)`: second prior-art pass for backup/restore, and the cache finding |

## What We Tried

### 1. The `.17` postinstall verification was not what `d254` said it was

`d254` recorded `pytest tests/sudo_postinstall` as "the only live verification
of the new adopt behaviour" and said the new test "fails against `.16` by
design". Both are half-true. The test sits behind
`SUDO_SECRETSPEC_POSTINSTALL_LIFECYCLE=1`; a plain run **skips** it. Running it
plain produces a green 23-passed result that verifies nothing about adoption.

Turning the gate on then exposed a second problem: **the two lifecycle install
tests could never pass at all.** They asked the *installed* client for an
install plan, but `install` refuses to source itself from the boundary it would
overwrite (`resolve_media`, `install.rs:710`), so both got a denial rather than
a plan. Invisible for however long, because the gate defaults off — this was
the first gated run since that guard landed.

Fixed in `404a90b` with `run_installer()` and an `installer` fixture resolving
the package's libexec copy (overridable via `SUDO_SECRETSPEC_INSTALLER`,
skipping when the media is absent).

### 2. A class of stale `--adopt-existing` instruction, in four places

The provenance split made the flag unnecessary for an upgrade, but four places
still said otherwise. Found the first one *before* cutting `.17`, which
mattered — it is the message `just release` prints at the end, i.e. the command
an operator copies straight out of a release:

1. `justfile` post-publish echo (`b2fef0c`, fixed before `.17` was cut)
2. `FORK-AI.md` reinstall note (`b2fef0c`)
3. `install.rs` self-install refusal — the product's own error text (`c15e053`)
4. `conftest.py`'s DECLARE gate, claiming declaring "has no inverse" when
   `undeclare` is exactly that (`862c7be`)

The Homebrew caveat was already correct from `d254`'s session.

### 3. Dogfooding found the thing it was supposed to find

Ported `source-add` and `source-undeclare` to the `Spec` text API (`c506a6f`).
Three results, in descending order of usefulness:

- **`Spec::from_toml` is the correct constructor for a privileged editor, and
  the reason is not obvious.** `Config::from_str` is a pure parse with no
  filesystem access; `from_toml` refuses `project.extends` and leaves
  `base_dir: None`, so `reparse` never reaches the extends-aware loader. Only
  `TryFrom<&Path>` sets `base_dir`. Pick the other constructor and a root
  process resolving `extends` would read whatever a parent path points at.
- **The tri-state was never an obstacle.** `Secret::new` / `required` /
  `optional` map exactly onto the fork's `Option<bool>`. `d254` treated the
  divergent signatures as a real gap between the trees; for this call site they
  are not.
- **One call site deliberately NOT ported.** The `undeclare` template guard
  asks whether a *different* document declares a name and must fail closed.
  `Spec::declares_secret_in_text` returns `bool` (unparseable reads as "not
  declared"), and routing the question through a `Spec` would also mean full
  validation plus an `extends` refusal on a document allowed to inherit. It
  stays on `manifest_edit::declares_secret`, which takes text and returns
  `Result`. Reasoning is recorded in-place at the call site.

### 4. Two of my own framings were wrong, and both were caught before acting

- **The `CLINE_API_KEY` "mirror it into the tracked template" plan was
  impossible.** I recommended a site-private PR. There is no tracked
  declarations file: `djbclark/site-private#87` ("Delete tracked secretspec
  declarations file and orphaned artifacts") and `djbclark/stayturgid#295` were
  both merged 2026-08-16, deliberately retiring that architecture. The handoff
  from *that* session even predicted `template-check` "will presumably still
  exit 1". Caught by looking for the file before opening the PR.
- **The #374 "genuine API defect" framing was too strong to post.** The PR
  promotes `manifest_edit` to a public module, so the fork's fail-closed guard
  has a supported upstream home, and the fail-open branch is near-unreachable
  for a validly-constructed `Spec` (the constructor already parsed the source).
  Rewrote it as an honest dogfooding report before sending.

### 5. The `.18` release "failed" at a step that had already succeeded

`just release 0.19.1-sudo.18` exited 1. The failure was the **last** call,
`gh repo view` — a GitHub **GraphQL** HTTP 503 outage, while REST worked fine.
Tag, Release, formula, tap, `brew reinstall` and `brew test` had all already
published. Per the justfile's own RESUME RULES that means finish by hand and
never re-run, so each artifact was verified individually instead. Nothing was
double-published.

### 6. Closing the risk the port actually introduced

`Spec::from_toml` validates the whole document, so a manifest it rejects would
break `add`/`undeclare` on the live boundary. Testing the tracked *template*
(52 declarations) was not sufficient — the runtime manifest has 53, including
`CLINE_API_KEY`. Probed the installed `.18` boundary with `undeclare` of an
undeclared name; it returned *"Secret 'ZZ_PROBE_NOT_DECLARED' is not declared in
profile 'default'"*, which only comes from `remove_secret_from_text` — proving
`from_toml` parsed and validated the real manifest first. A temporary in-tree
probe test was used for the template check and removed before committing.

## Key Decisions

- **Fix the release message before cutting the release.** A wrong command in
  the completion output is worse than a wrong command in a doc, because it is
  the one an operator copies.
- **Extract the manifest edits out of `execute`.** Same reasoning the
  truncation incident forced for `install`'s guards: that path only runs as
  root against a real vault, so anything inline there is untestable, and the
  last thing hidden behind that unreachability truncated a vault. Now
  `manifest_with_declaration` / `manifest_without_declaration` / `preserved`,
  with 7 tests.
  - *Rejected: port and rely on `secretspec`'s own `spec::tests::text_edits*`.*
    Those cover the API; they do not cover the broker's composition of it, and
    the byte-exact round trip is the property `Mutation::restore` depends on.
- **Keep the `undeclare` template guard on `manifest_edit`** (see What We Tried
  §3). A question about text is answered by the function that takes text and
  returns `Result`.
- **Post a dogfooding report to #374, not the approved defect claim.** The
  approved content had become inaccurate; posting it to someone else's PR would
  have been worse than deviating.
- **Do not open the site-private PR.** The target file was deliberately deleted
  two days ago; `CLINE_API_KEY` stays and the drift is by design.
- **`.18` was not re-run after its exit-1.** Publishing had completed; re-running
  is what the justfile explicitly forbids.

## Evidence & Data

- **Rust**: `cargo test --no-fail-fast -p sudo-secretspec-cli` — **177 passed,
  0 failed** (lib went 115 → 122 with the 7 new broker tests). `cargo fmt --all
  -- --check` clean. `cargo clippy -p sudo-secretspec-cli --all-targets` — **20
  warnings, unchanged** from the session's starting baseline.
- **Postinstall, `.17` and `.18`**: plain `pytest tests/sudo_postinstall` =
  **23 passed, 9 skipped**. With `SUDO_SECRETSPEC_POSTINSTALL_LIFECYCLE=1` =
  **27 passed, 5 skipped**, including
  `test_install_dry_run_adopts_the_installed_vault_without_the_flag` — the
  first live end-to-end proof of flagless adoption.
- **Adoption verified by hand too**: flagless dry run reported `adopt_existing=1`,
  `vault=/var/db/sudo-secretspec`, `service=_sudo_secretspec:_sudo_secretspec`,
  and announced the adoption on stderr naming both the vault and
  `/usr/local/etc/sudo-secretspec.toml`.
- **Vault**: `46 found / 0 missing / 7 optional` before and after both installs.
  `doctor: OK`. Rollback snapshots written both times (8 artifacts each).
- **Releases**: `v0.19.1-sudo.17` and `v0.19.1-sudo.18` both published
  non-draft; tap at `be8e288`; brew keg `.18`; `brew test` green both times.
  Disk 126Gi then 120Gi free against a 25Gi guard.
- **`template-check` drift, explained**: template declares 52 names, runtime 53.
  The single difference is `CLINE_API_KEY`. Not caused by either install.
- **Upstream at session end** (unchanged all session): **#374** OPEN, 0 checks,
  0 reviews, 1 comment — ours,
  https://github.com/cachix/secretspec/pull/374#issuecomment-5317955173.
  **#362** 2 comments, last still ours. **#372** 0 comments. **#373** 0
  comments, 0 reviews. `upstream/main` still `dfa4b10`.
- **Backup/restore step 1**: re-searched 8 terms upstream — no dedicated
  thread; all hits incidental (#370 ours, #64, #202, #176, #156, #11, #339).
  Web: no secretspec-specific discussion. Recorded in `cb80ed6`.

## Operator Feedback

- **"use your resume plan"** then **"the release"** — treated as the go for
  `.17`, which the plan had flagged as needing explicit approval because it
  publishes.
- **"Okay go for it"** — the broker dogfooding, after a recommendation that
  argued it was time-sensitive *because* #374 is open and unreviewed.
- **"1. I want to keep it, I just added it recently, probably confused due to
  the also recent restore from backup. 2. Yes do that all now — when that is
  done, cut the release"** — `CLINE_API_KEY` kept; #374 comment posted; `.18`
  cut.
- **"pause"** — stopped at a clean seam (released, installed, verified, pushed,
  Tier 1 written) rather than mid-operation.
- **"resume"** — continued to backup/restore step 1.
- Standing, from earlier in the chain: privileged installs and outward actions
  are confirmed, not assumed. Every `sudo` step this session raised a Touch ID
  prompt the operator answered; the gated lifecycle tests were flagged as
  raising ~4 prompts before being run.

## Where We're Going

1. **THE NEXT ACTION: design infinite backup/restore.** Step 1 (prior-art
   re-search) is DONE and recorded in `cb80ed6` — **do not redo it.** Decide
   the "Open questions" in `docs/design/infinite-secret-backup-restore.md`
   *with the operator*: per-key value history vs whole-file snapshots; whether
   `restore` needs authentication beyond the existing sudoers rules (it is
   itself a destructive write); dotenv-only vs provider-backed values;
   retention/compaction; fork-only vs proposing a shape upstream. Leading
   backend is the git-wrapped vault (infinite history, restore = checkout,
   `git fsck` integrity, no remote).
2. **Carry into that design:** `restore` MUST invalidate affected cache entries
   in the *same audited operation* — `secretspec/src/cache.rs` is a second
   store of values with `max_age` expiry, so a cached newer value can silently
   defeat a vault rewind. Upstream's `cache_refresh` is precedent for `restore`
   being its own audit verb rather than a `set`. Verified 2026-08-17: the
   privileged CLI does not use the cache yet, so nothing is at risk today.
3. **Operator decision not yet asked: what to do about `template-check`.** It
   can never pass on this host now that the tracked declarations file is gone.
   Either it detects a retired/missing tracked source and says so, or it is
   retired alongside the file. Right now it is pure noise.
4. **Watch #374** — `gh pr view 374 --repo cachix/secretspec --json
   state,comments,reviews,statusCheckRollup`. Still zero CI. A maintainer must
   act; pull-only access here.
5. **Keep `/Users/djbclark/src/ss-370`** (`spec-manifest-edit`, `b3637e2`)
   until #374 resolves. Review changes go THERE, then port to `sudo-main`
   separately — upstream's `add_secret_to_manifest` is description-only, the
   fork's takes `required: Option<bool>`.
6. **Watch #362, #372, #373** — all still unengaged.
7. **RESOLVED, do not re-investigate:** the `template-check` *drift* itself.
   `CLINE_API_KEY` is a real declaration the operator added and wants kept;
   `/usr/local/share/sudo-secretspec/secretspec.toml` is a frozen pre-deletion
   leftover nothing regenerates.
8. **Do NOT delete branch `explore/pr-334-rust-first-spec`** — `bf0b25c` is
   linked by SHA from a public comment on upstream #357.
9. **DEFERRED per explicit operator instruction, do NOT resurface:** item 12
   cross-platform sudo/Linux port
   (`docs/design/privilege-boundary-and-packaging.md:487`).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect cb80ed6 at tip, tree clean, pushed
git worktree list             # ss-370 must still be there until #374 resolves

# The next action starts here — step 1 is already done, read it and move on:
sed -n '/Second pass, 2026-08-17/,$p' docs/design/infinite-secret-backup-restore.md

# Confirm the boundary — do not assume:
sudo-secretspec --version                      # expect 0.19.1-sudo.18
sudo-secretspec doctor                         # expect OK
sudo-secretspec check --reason "orientation"   # expect 46 found / 0 missing / 7 optional
# `template-check` exits 1 BY DESIGN now — see Where We're Going item 7.

# Tests (--no-fail-fast REQUIRED or the CLI crate never runs):
cargo test --no-fail-fast -p sudo-secretspec-cli    # expect 177 passed
pytest tests/sudo_postinstall                       # 23 passed, 9 skipped
SUDO_SECRETSPEC_POSTINSTALL_LIFECYCLE=1 pytest tests/sudo_postinstall
# ^ 27 passed, 5 skipped; raises a Touch ID prompt PER lifecycle test.
# `cargo test --all` CANNOT run here: ext-php-rs needs a PHP toolchain.
# 21 provider::sops::* failures are EXPECTED in a full-workspace run.

# Upstream:
gh pr view 374 --repo cachix/secretspec --json state,comments,reviews,statusCheckRollup
cat docs/design/pr374-dogfooding-comment.md   # the comment as submitted

# A release, when there is one to cut (disk guard 25Gi):
just release-dry 0.19.1-sudo.19
just release 0.19.1-sudo.19
# If it exits 1 AFTER "git push origin <tag>": do NOT re-run. Finish by hand
# per the RESUME RULES at the top of the justfile.
sudo /opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec install --non-interactive
```
