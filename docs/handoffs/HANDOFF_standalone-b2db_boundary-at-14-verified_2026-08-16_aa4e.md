---
schema_version: 1
handoff_id: aa4e
parent_handoff_ids: [f4a6]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 989f383f51d04fdc0cf1a3959f7650a56e0b3cb6
created_at: 2026-08-16T12:03:08-0400
writer: claude-code
---

# Handoff — boundary at 0.19.1-sudo.14, every claim verified live

Closing handoff for this work. **It exists primarily to correct `f4a6`.**

> `f4a6` states "The boundary is deliberately still at `.13`" and lists
> installing `.14` as THE next action. Both were true when written and are now
> false — the install ran and succeeded. A session resuming from `f4a6` alone
> would act on stale state.

Chain reading order: **`a651`** (why the install bug existed and how it was
fixed — the substantive document) → **`f4a6`** (release `.14` and the
pre-install proofs) → **this** (closure).

## The Goal

Operator invoked `/handoff` after the final verification step completed. The
session's own goal, from its opening instruction: *"Re: `sudo-secretspec install`
upgrading nothing because PATH resolves it to the old client — I think we need
to think about how to make this less tricky some more."*

That goal is met and proven, not merely implemented.

## Where We Are

`sudo-main` @ `989f383`, **clean, pushed, nothing outstanding.**

The boundary is installed at **0.19.1-sudo.14** and everything this session
built has now been observed working against the real installation:

| Check | Result |
| --- | --- |
| Installed client | `0.19.1-sudo.14` |
| Staged keg | `0.19.1-sudo.14` |
| Protected config stamp | `version = "0.19.1-sudo.14"` |
| `doctor` | `OK`, exit 0, **no advisories** |
| `audit-verify` | 484 events, chain intact |

Two releases were cut this session: **0.19.1-sudo.13** (tag `3f52487`) and
**0.19.1-sudo.14**, both with GitHub Release + tap published and `brew test`
passing.

### Files changed across the session

`sudo-secretspec-cli/src/{install,drift,config,main,lib}.rs`,
`sudo-secretspec-cli/tests/drift.rs`, `CHANGELOG.md`, `FORK-AI.md`,
`skills/sudo-secretspec/SKILL.md`, `sudo-secretspec/AI-GUIDANCE.md`,
`sudo-secretspec/README.md`, `Cargo.toml` (+2 member manifests, `Cargo.lock`),
`packaging/homebrew/sudo-secretspec.rb`, and three handoffs under
`docs/handoffs/`.

## What We Tried

**No failed approaches in this final segment** — the `.14` release and install
both worked first time. Stating that explicitly rather than padding the section.

The session's expensive lessons are recorded where they happened and are worth
not rediscovering:

- **`a651`** — the prior handoff's diagnosis was *wrong*. `6f50b74` recorded the
  no-op install as a `PATH` problem. It is not: `install` copies from the tree
  its own executable lives in, and `/usr/local` is shaped exactly like the
  distribution media, so the installed client's own `install` resolves every
  source path back onto the destination. `PATH` was one route in; alias,
  symlink, and hard link are others. Fixing the framing is what produced a fix
  that actually closes the class.
- **`a651`** — placing the guard after `require_root()` (the first
  implementation, `2efa03e`) meant paying a Touch ID prompt to be told no, and
  made the refusal unreachable by anything unattended. `f3a9629` moved it ahead
  of elevation.
- **`a651`** — a test that asserted against the *current* version string, so it
  silently took the wrong branch. Fixed to a deliberately-unreal `"0.0.0-older"`.
- **`a651`** — `cargo test --all` cannot build in this checkout at all (the PHP
  SDK crate needs `php` on `PATH`). Pre-existing, unrelated; `release.py` gates
  on a narrower scope by design.

## Key Decisions

**Wrote a third handoff rather than editing `f4a6`.** The chain is an
append-only DAG; the correction belongs in a child that supersedes, not in a
silent rewrite of a document another session may already have read.

**Updated the skill and AI-GUIDANCE *before* tagging `.14`** — both ship inside
the release, so a post-tag doc fix never reaches the installed copy. That is
precisely the trap the `.12` release hit (handoff `4752`).

**Left the boundary at `.13` while `.14` was staged** (operator-directed, and it
paid off). The gap is the *only* condition under which `UPGRADE_AVAILABLE` is
observable without fabricating state. Worth deliberately pausing there on future
releases rather than upgrading straight through.

**Rejected: a self-reference check in `rollback`.** Audited this session. It has
the same *shape* as the install bug — a `.prior` file that is its own
destination would restore onto itself and report success — but it is not
reachable: `capture_snapshot` always `fs::copy`s to a new inode, and crafting
one requires already being root plus writing a `0700` root-owned directory that
`rollback::run` re-validates. Adding ~4 lines that can never fire cuts against
this project's own `export-declarations` precedent
(`docs/design/template-check-resync.md`): unused code in a root-privileged
broker is a liability. Recommendation is to document the invariant instead.
`uninstall` has no instance — deleting its own executable is safe on Unix, and
sudoers ordering is already deliberate.

## Evidence & Data

### The final verification — version-transition reporting doing its real job

```
$ "$(brew --prefix)"/opt/sudo-secretspec/libexec/sudo-secretspec install --adopt-existing
installed sudo-secretspec 0.19.1-sudo.13 -> 0.19.1-sudo.14
source=/opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec
rollback_snapshot=/usr/local/libexec/sudo-secretspec-rollback-1786896107
rollback_artifacts=8
```

**The arrow is the point.** The `.13` upgrade could only print a bare version,
because `.12` wrote no `version` key to compare against. `.13` wrote one, so
`.14` could report an actual transition — the first live confirmation that this
feature does what it was built for. The `doctor` advisory then correctly went
*silent* once staged and installed matched again, which is the steady-state
branch passing.

Ledger moved 482 → 484 (the install's own audit records) with the hash chain
still verifying.

### Proofs carried from `f4a6`, all pre-install and unattended

1. **`UPGRADE_AVAILABLE` in its genuine window** — staged `.14` vs installed
   `.13`, real config: advisory printed *under* a passing `doctor: OK`, exit 0,
   naming both versions and the path to run.
2. **The pre-elevation guard in the shipped `.14` artifact** — the released keg
   binary refused a tree whose `libexec/sudo-secretspec` was a **hard link** to
   the real installed broker: `exit=2`, `elapsed=0s` with stdin closed (an auth
   prompt would have blocked). The hard link is why the comparison is
   `(dev, ino)` and not canonicalized paths.

### Test and lint baselines

| Check | Result |
| --- | --- |
| `cargo test -p sudo-secretspec-cli --locked` | 109 lib + 19 + 10 + 6 + 11 + 16, 0 failed |
| New tests added this session | 14 (8 install, 6 drift) |
| `pytest tests/sudo_packaging -q` | 21 passed |
| Clippy vs stashed clean-tree baseline | 12 vs 12 — zero new warnings |

### Upstream

- **#354, #355 merged.** #355's merge (`faa9645`) is what broke #334.
- **#334 fails to compile against current `main`** — verified by local merge,
  one `E0308` at `codegen.rs:573`; one-line fix (`build_ir` →
  `build_ir_from_config`) verified to compile and pass 11/11 codegen tests.
  Commented: `.../pull/334#issuecomment-5305816915`
- **#356 blocked on workflow approval**, all runs `action_required`, CI never
  executed. Commented: `.../pull/356#issuecomment-5305817471`
- **djbclark has pull-only access** on `cachix/secretspec`
  (`{"admin":false,"maintain":false,"pull":true,"push":false,"triage":false}`),
  so the approve button does not render for him. **There is no URL that works** —
  a maintainer must do it.

## Operator Feedback

- *"implement all of it, A-D. And yes rewrite doc."* — done, released, verified.
- *"do a new release, test, and then see if the skill, doc, or any issues or PRs
  need to be updated."* — done; fork has issues disabled and no PRs.
- Authorized posting both upstream comments unattended. Done.
- *"don't do tests that require me to be around, do that after the handoff"* —
  honoured; the Touch ID install was deferred past `f4a6` and then run.
- Ran the boundary upgrades personally (twice) so verification could proceed.
- Still deferred, **do not resurface**: item 12 cross-platform sudo/Linux port
  (`docs/design/privilege-boundary-and-packaging.md:487`).
- Still open, lowest priority: best way to keep files synced between computers
  (partial answer in `4752`, Where We're Going item 4).

## Where We're Going

**No blockers. Nothing is half-finished.** The work this session set out to do is
complete and verified.

1. **THE NEXT ACTION — watch the two upstream comments for replies.**
   `gh pr view 334 --repo cachix/secretspec --comments` and
   `gh pr view 356 --repo cachix/secretspec --comments`. Neither can be advanced
   from this side (see the access note above); both are waiting on a cachix
   maintainer.
2. **Do not delete `explore/pr-334-rust-first-spec`** — `bf0b25c` is linked by
   SHA from a public comment on upstream #357.
3. Optional, operator's call: the `rollback` invariant — document it rather than
   add the check, per the reasoning above.
4. If a `.15` is ever cut: pause before installing it and run `doctor` while the
   new build is merely staged. That is the only window in which
   `UPGRADE_AVAILABLE` can be exercised for real.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3              # expect 989f383 at HEAD, tree clean
sudo-secretspec --version         # expect 0.19.1-sudo.14
sudo-secretspec doctor            # expect OK, exit 0, no advisories
sudo-secretspec audit-verify      # expect 484+ events, chain intact

# the next action
gh pr view 334 --repo cachix/secretspec --comments
gh pr view 356 --repo cachix/secretspec --comments

# re-prove the install guard any time -- no root, no Touch ID, exits in 0s:
SB=$(mktemp -d); mkdir -p "$SB"/{bin,libexec,share/sudo-secretspec}
cp /opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec "$SB/bin/"
ln /usr/local/libexec/sudo-secretspec "$SB/libexec/sudo-secretspec"
cp /usr/local/share/sudo-secretspec/{AI-GUIDANCE.md,sudo-secretspec-retired.toml} \
   "$SB/share/sudo-secretspec/"
"$SB/bin/sudo-secretspec" install --adopt-existing --non-interactive </dev/null
rm -rf "$SB"                      # removes the link only; the original is untouched
```
