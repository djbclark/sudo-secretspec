---
schema_version: 1
handoff_id: 4752
parent_handoff_ids: [0e34]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 6f50b740ad78f0f86bbb08ecb40620cd3b0349a9
created_at: 2026-08-15T23:58:18-0400
writer: claude-code
---

# Handoff — `add` requiredness, upstream fold-in, and release 0.19.1-sudo.12

## The Goal

Resumed from handoff `0e34` to watch four upstream PRs. The session then took
on an operator gap report (four numbered items from consuming the boundary in
the djbclark-ops suite), which became the bulk of the work: `add` could only
ever declare a *required* secret, and the Claude skill symlink pointed into a
worktree that could be on any branch.

Explicit operator constraint, stated up front and honoured: **any code proposed
upstream gets two outside AI opinions first.**

## Where We Are

Everything shipped, verified, and pushed. Tree clean.

- `sudo-main` @ `6f50b74`, pushed. No uncommitted changes.
- Release **0.19.1-sudo.12** cut and installed. Verified three ways:
  `sudo-secretspec --version` → `0.19.1-sudo.12`; `add --help` lists
  `--optional` and `--required`; `sudo-secretspec doctor` → `OK`, exit 0.
- Upstream PR **#356** now carries both the `manifest_edit` refactor
  (`8025fbb`) and the requiredness feature (`e917888`), retitled to cover the
  expanded scope. `MERGEABLE`.

Commits this session, oldest first:

| SHA | What |
| --- | --- |
| `1139f1a` | `add --optional` (first, flawed design — superseded) |
| `13dbd54` | tri-state requiredness after the two reviews |
| `1d343e4` | skill v0.3.0 + AI-GUIDANCE refresh for .12 |
| `89bee84` | workspace version stamp 0.19.1-sudo.12 |
| `143ec2d` | Homebrew formula for v0.19.1-sudo.12 |
| `6f50b74` | document the silent-no-op upgrade path |

Files changed: `secretspec/src/manifest_edit.rs`, `secretspec/src/cli/mod.rs`,
`sudo-secretspec-cli/src/{main,broker}.rs`, `docs/src/content/docs/reference/cli.md`,
`skills/sudo-secretspec/SKILL.md`, `sudo-secretspec/AI-GUIDANCE.md`,
`CHANGELOG.md`, `Cargo.toml` (+ two member manifests, `Cargo.lock`),
`packaging/homebrew/sudo-secretspec.rb`, and a new
`docs/design/template-check-resync.md`.

Also pushed, deliberately kept alive: branch `explore/pr-334-rust-first-spec`
(commit `bf0b25c`). **Do not delete it** — it is linked by SHA from a public
comment on upstream #357.

## What We Tried

### The `--optional`-only design was wrong (most expensive lesson here)

The first implementation (`1139f1a`) added a single `--optional` bool. The
stated rationale — *"omitting the flag already means required"* — is **false**.

`config.rs` `merge_secret` falls back to `defaults.and_then(|d| d.required)`
when a secret carries no explicit `required`. So a profile with
`defaults = { required = false }` makes an omitted key resolve to **optional**.
With only `--optional`, both paths produced an optional secret, and declaring a
*required* secret in such a profile was unreachable.

Caught by both reviewers independently, from opposite directions. Verified
empirically rather than by reading — built a throwaway manifest with
defaults-optional and ran the real `check`:

```
before fix:  ○ NEEDED - must be set (optional)     <- wanted required
after fix:   ✗ NEEDED - must be set (required)     <- correct
             ○ ALSO_OPT - only some hosts (optional)
```

Fix (`13dbd54`): requiredness is tri-state. Library takes `Option<bool>`
(`None` omits the key, `Some(v)` writes it); CLI gets `--optional` and
`--required` marked `conflicts_with`.

### `sudo-secretspec install --adopt-existing` silently upgraded nothing

After releasing .12 and `brew reinstall`, I told the operator to run
`sudo-secretspec install --adopt-existing`. It printed `installed
sudo-secretspec`, wrote a rollback snapshot, exited 0 — and the version stayed
at `.11`.

Cause: `install` resolves through `PATH` to `/usr/local/bin/sudo-secretspec`,
which **is** the old client. It reinstalls itself. The upgrade must be driven
by the Homebrew `libexec` copy, kept off `PATH` precisely so it cannot shadow
the installed client:

```bash
/opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec install --adopt-existing
```

The brew caveats say this; I hadn't read them. Only caught because I checked
`--version` afterwards instead of trusting the success message. Cost: one
wasted Touch ID prompt. Documented in both files (`6f50b74`) — neither had
covered upgrading at all.

### Rejected: an options struct for `add_secret_to_manifest`

Both reviewers wanted one, agy calling the signature change *"a hard SemVer
break for external consumers."* **That premise is false for upstream.**
`add_secret_to_manifest` is a *private* `fn` on `upstream/main`
(`cli/mod.rs:619`); it only becomes `pub` in our own unmerged PR #356. No
published signature exists to break. Cursor stated the correct conditional
("acceptable *if* this lands in the same PR that introduces `manifest_edit`"),
which is satisfiable — so no struct is needed, provided the two land together.

Verifying that premise is what turned "add an options struct" into the much
cheaper "fold into #356".

### Process waste worth not repeating

Polled long builds with chained `sleep` background tasks. Many were killed and
re-spawned; `ps -eo etime` readings didn't reconcile with wall time. Checking
child processes (`ps | grep rustc`) to see the actual build stage was what
resolved "is this hung?" — do that first next time. The release build alone
took 11 minutes; the full `cargo test --all-features` ~7 minutes because
`trybuild` recompiles from scratch.

## Key Decisions

**Tri-state `Option<bool>` over `bool`** — chosen. Both reviewers independently
converged on this shape for the library. Rejected: plain `bool`, because it
cannot express "inherit the profile default", which is the pre-existing and
correct behaviour.

**Two mutually-exclusive flags over `--required <bool>`** — a synthesis, since
the reviewers disagreed. agy wanted `--required <bool>` for tri-state; cursor
wanted to keep bare `--optional` on UX grounds ("`--required false` makes the
common case noisier") and argued the gap was pre-existing. Both were right
about something: `--optional` + `--required` gives tri-state *and* keeps the
common case clean. Rejected a presence-group flag
(`at_least_one`/`exactly_one`) — it spans several secrets and doesn't fit a
single-secret declare.

**Fold into PR #356 rather than file a new upstream PR** — operator approved.
Landing the refactor (which makes the function `pub`) together with the feature
(which changes its signature) means no published signature ever changes. Kept
as two commits so the refactor stays reviewable alone.

**Rejected making `template-check` semantic** (operator's gap item 2a). Byte
exactness is load-bearing: it is what makes green mean "the running file is the
exact bytes reviewed", and what makes `add` → `undeclare` a *provable* undo
(two tests assert it). Recommended `export-declarations` (2b) instead but
**deliberately did not build it** — the operator deleted the tracked-file
workflow, so it has zero users, and unused code in a root-privileged broker is
a liability. Reasoning recorded in `docs/design/template-check-resync.md` at
the operator's request.

**Skill symlink → Homebrew `opt` path**, not a pinned worktree. `brew`
re-points `opt` on every upgrade, so it never goes stale, and it cannot be
knocked over by a `git checkout` — which had *actually* broken it during this
session while on `explore/pr-334-rust-first-spec` (no `skills/` dir there).

**Cut a release rather than hand-edit the Cellar.** Consequence of the symlink
change: the live skill is now the brew-installed copy, which brew overwrites on
upgrade. Releasing is the only durable way to update it.

## Evidence & Data

Test baselines, measured by stashing rather than asserted:

| Run | Passed | Failed |
| --- | --- | --- |
| `sudo-main` clean baseline | 1206 | 21 (all `provider::sops::*`) |
| `sudo-main` with changes | 1214 | 21 (identical set) |
| PR356 branch, no changes | — | 26 = 21 sops + 5 `cli::completion` |
| PR356 branch, with changes | 1256 | 26 (identical set) |

+8 passing = the new tests. All failures pre-existing: sops CLI not installed;
`cli::completion` tests are working-directory dependent. The PR356-branch stash
check mattered — I edited the clap `Add` variant and completion reads the CLI
definition, so "pre-existing" needed proving, not assuming.

Test counts: `manifest_edit` 13 on the fork, 7 upstream (the fork's byte-exact
round-trip tests depend on `remove_secret_from_manifest`, which is **fork-only**;
adapted upstream into a preservation assertion over a document mixing a presence
group, a full table and a quoted dotted key).

Release: built in 11 minutes, `brew test` passed, tag + GitHub Release + tap all
verified by `release.py` before exit 0. `doctor: OK` exit 0 afterwards.

Upstream PR state at handoff time: **#354 merged**; #355, #356, #357 open.
Comment posted on #357:
`https://github.com/cachix/secretspec/pull/357#issuecomment-5305260964`.

PR #334 impact assessment (branch `explore/pr-334-rust-first-spec`, `bf0b25c`):
two breaks only — `broker.rs::emit_schema` took `&secretspec::Config` (now
`__private`), switched to `Spec::load_from`; and our own PR #355 added a
`codegen.rs` test using the pre-#334 `build_ir(&Config)` signature, renamed to
`build_ir_from_config`. That second one *is* the conflict the maintainer flagged.

## Operator Feedback

- **"Item 2 is specific to our site"** — the template-check drift
  (`FIRERPA_MCP_TOKEN`, `ATLASSIAN_CFENGINE_API_TOKEN`) is operator-config, not
  a repo concern; handed to another agent. Removed from next steps. Do not
  re-add.
- **"We want to get 2 other AI opinions on any code that we are going to
  propose go upstream."** Standing constraint. Used `agy` (gemini) and
  `cursor-agent` (grok), matching the pattern from handoff `75ef`.
- **"Wait have we done a release with the new stuff yet?"** — the highest-value
  intervention of the session. It had not. Documenting unreleased flags in the
  *installed* skill would have shipped instructions that error, a trap created
  by the same-session symlink change. Prompted cutting .12.
- **"Update both our local skill and the skill in the repo"** — satisfied via
  the release, not by editing the Cellar.
- **"fold it in"** — explicit approval to amend open PR #356.
- **"record it in docs/design/ so the reasoning survives"** — done.
- Paused the session twice mid-turn; resumed cleanly both times.
- Deferred, do not resurface: item 12, cross-platform sudo/Linux port
  (`docs/design/privilege-boundary-and-packaging.md:487`) — operator will raise
  it when working on their Linux VPS.

## Where We're Going

1. **THE NEXT ACTION — watch the three open upstream PRs.**
   `gh pr view <355|356|357> --repo cachix/secretspec --comments`. #354 is
   merged. #356 now holds both commits and is `MERGEABLE`. #357 awaits a
   maintainer response to our #334 comment; if #334 looks close to merging, the
   fix is already worked out on `explore/pr-334-rust-first-spec` (`bf0b25c`).
2. **Do not delete `explore/pr-334-rust-first-spec`** — `bf0b25c` is linked from
   a public comment on upstream #357; deleting it breaks that link.
3. `6f50b74` (upgrade-path docs) is committed but **not in any release**, so the
   brew-installed skill still lacks it. It ships with whatever release comes
   next; not worth cutting .13 for a doc-only change.
4. **Unanswered operator question, lowest priority:** best way to keep files
   synced between computers. Partial answer given — git for repo content, the
   site-private memory convention for cross-session facts, and machine-local
   state like the skill symlink deliberately *not* synced because its correct
   target is platform-dependent (`$(brew --prefix)` differs). Offered a
   `just`/script target using `$(brew --prefix)` if it becomes annoying.
5. Never live-tested `add --optional` through the real boundary — that would
   declare a real name in the production manifest and need `delete` +
   `undeclare` cleanup. Flags are proven present in the installed binary and
   tested against throwaway manifests. A deliberate live check is the operator's
   call (this chain has a prior incident of a "probe" mutating real state).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -6                    # expect 6f50b74 at HEAD, tree clean
sudo-secretspec --version               # expect 0.19.1-sudo.12
sudo-secretspec doctor                  # expect OK, exit 0

# the next action
gh pr view 356 --repo cachix/secretspec --comments
gh pr view 357 --repo cachix/secretspec --comments
gh pr view 355 --repo cachix/secretspec --comments

# if #334 merges, the prepared fix:
git log --oneline -1 explore/pr-334-rust-first-spec   # bf0b25c
git show bf0b25c

# upgrading the boundary later (NOT plain `sudo-secretspec install`):
/opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec install --adopt-existing
sudo-secretspec --version               # confirm it actually moved
```

Re-running the two outside reviews (prompt is ephemeral in `/tmp`):
`agy -p "$(cat /tmp/review_prompt.md)"` and
`cursor-agent --print --output-format text "$(cat /tmp/review_prompt.md)"`.
Rebuild the prompt from the upstream-bound diff if `/tmp` is gone:
`git show 13dbd54 -- secretspec/src/manifest_edit.rs secretspec/src/cli/mod.rs docs/src/content/docs/reference/cli.md`.
