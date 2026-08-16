---
schema_version: 1
handoff_id: a651
parent_handoff_ids: [4752]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: f3a96299c477a0422cc3ef3dcaf9b3e096ee52e0
created_at: 2026-08-16T01:04:18-0400
writer: claude-code
---

# Handoff — install self-source guard, release 0.19.1-sudo.13

## The Goal

Resumed from handoff `4752` via `/baton` to watch upstream PRs. The operator
immediately redirected: *"Re: `sudo-secretspec install` upgrading nothing
because PATH resolves it to the old client — I think we need to think about how
to make this less tricky some more."*

That became the session. The prior fix (`6f50b74`) had documented the trap; this
one removes it, and corrects the diagnosis the prior fix was built on.

## Where We Are

Everything shipped, released, verified live, and pushed. Tree clean.

- `sudo-main` @ `f3a9629`, pushed.
- Release **0.19.1-sudo.13** cut (tag `3f52487`), GitHub Release + tap live,
  `brew test` passed.
- Boundary **upgraded and verified live**: `sudo-secretspec --version` →
  `0.19.1-sudo.13`, `doctor: OK` exit 0, config now carries
  `version = "0.19.1-sudo.13"`.
- Two commits are **committed but unreleased**: `5bb76df` (README) and
  `f3a9629` (pre-elevation guard). See Where We're Going item 1.

| SHA | What |
| --- | --- |
| `2efa03e` | A+B+C: self-source guard, version reporting, dry-run parity, D advisory, docs |
| `5ebdccc` | workspace version stamp 0.19.1-sudo.13 |
| `5180143` | Homebrew formula for v0.19.1-sudo.13 |
| `5bb76df` | README: which binary must run `install` (post-tag, unreleased) |
| `f3a9629` | refuse a self-sourced install *before* elevating (post-tag, unreleased) |

## The Actual Bug (the prior handoff's diagnosis was wrong)

`6f50b74` recorded this as a `PATH` problem whose fix is "drive the upgrade from
the libexec copy." That is a correct workaround and a misleading diagnosis, and
the wrong framing is why the trap still felt unresolved.

`install.rs` infers its source from `current_exe()`:

```rust
let source_root = self_exe.parent().and_then(Path::parent)?;   // two levels up
let broker_src  = source_root.join("libexec/sudo-secretspec");
```

Run the installed client at `/usr/local/bin/sudo-secretspec` and `source_root`
becomes `/usr/local`, so `broker_src` resolves to the **already-installed
broker**. Every artifact is copied onto itself.

The deep reason it fails *silently*: **the destination tree is shaped exactly
like the source tree** — install itself puts `libexec/sudo-secretspec` and
`share/sudo-secretspec/` under `/usr/local`. The destination is therefore always
a structurally valid source. Nothing is malformed, so no pre-existing check
could fail. `PATH` is merely the most common way to reach the wrong binary; an
alias, a symlink, a hard link, or a stale shell hash all produce the same no-op.

This is a bug, not a preference: `main.rs:235` already refuses `install` through
the *broker* path on exactly the principle "the installed copy must not be the
installer." The installed *client* path slipped through because
`invoked_as_privileged_broker()` returns false for it by construction. The fix
completes an existing rule.

Every safety net missed it: exit 0 was honest, the rollback snapshot was
faithfully written, `--dry-run` returned ~110 lines before the media was ever
resolved, `doctor` passed (the install genuinely *was* consistent — consistently
old), and the brew caveats said the right thing but went unread.

## What We Tried

### Placing the guard after `require_root()` (fixed same session)

The first implementation (`2efa03e`) put the check inside `install::run()`,
which begins with `require_root()`. Correct, but it means the operator pays an
interactive Touch ID prompt to be told the install would do nothing.

Worse, and the reason it actually mattered: a refusal reachable only after
authentication cannot be exercised unattended — not by CI, not by an agent, not
by a test. `f3a9629` adds `install::preflight_media()`, called from
`run_install` before elevation. `install::run` still repeats every check as root
and remains authoritative; the preflight is explicitly documented as *not* a
security boundary.

This is what then made the live proof possible at all (see Evidence).

### A test that asserted against the current version

`the_version_line_distinguishes_an_upgrade_from_a_reinstall` initially used
`"0.19.1-sudo.12"` as the *old* version — which was `VERSION` at the time, so it
took the reinstall branch and failed. Fixed to `"0.0.0-older"`, deliberately not
a real predecessor, so the test cannot rot against whatever `VERSION` becomes.

### `cargo test --all` cannot build in this checkout

Not a regression — the PHP SDK crate's `ext-php-rs` build script requires `php`
on `PATH`, which is not installed here. `packaging/release.py` deliberately
gates on a narrower scope (`pytest tests/sudo_packaging`, `cargo test -p
sudo-secretspec-cli --locked`, `cargo build -p secretspec --locked`), which is
what was used to validate the release.

## Key Decisions

**Compare by `(dev, ino)`, not canonicalized paths** — chosen, and this was not
theoretical. A hard link has no link to follow, so path comparison would call
the two files distinct and wave the no-op through. Proven live: the guard caught
a tree whose `libexec/sudo-secretspec` was a hard link to the real installed
broker.

**Version stamp as `Option<String>` in the protected config** — rejected a
defaulted string. A boundary installed before the stamp existed has no honest
value; `None` means "not recorded", which is different from any version. This is
why the live upgrade printed `installed sudo-secretspec 0.19.1-sudo.13` with no
arrow, exactly as predicted. Follows the precedent `config.rs` already documents
for `profile`.

**`MEDIA_PREFIXES` as a fixed list, never `brew --prefix`** — `doctor` reaches
that code as root, and asking an operator-writable tool where a root process
should look is the wrong shape.

**The staged version is probed by the *unprivileged* client** and passed across
the elevation boundary via hidden `--available-path` / `--available-version`
flags, mirroring the existing `--caller-path`. The Homebrew prefix is routinely
operator-writable, so it is not code the root broker may execute. Advisory-only:
a doctored value can only make the caller nag itself.

**`UPGRADE_AVAILABLE` reports "differs", never "newer"** — ordering these
strings needs a version parser this project does not own, and a *downgrade* is
worth surfacing too (after a rollback, that is exactly the state to see).

**Rejected: adding a self-reference check to `rollback`.** See the audit below.

## Evidence & Data

Tests, measured not asserted:

| Check | Result |
| --- | --- |
| `cargo test -p sudo-secretspec-cli --locked` | 109 lib + 19 + 10 + 6 + 11 + 16, 0 failed |
| New tests added | 14 (8 install, 6 drift) |
| `pytest tests/sudo_packaging -q` | 21 passed |
| Clippy vs stashed clean-tree baseline | 12 vs 12 — **zero new warnings** |

Live verifications (all real, none simulated):

1. **Upgrade** (operator ran, one Touch ID):
   `installed sudo-secretspec 0.19.1-sudo.13` /
   `source=/opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec`.
   No arrow — `.12`'s config predated the stamp, as designed.
2. **Post-upgrade state:** client and keg both `0.19.1-sudo.13`; config carries
   `version = "0.19.1-sudo.13"`; `doctor: OK` exit 0 with **no**
   `UPGRADE_AVAILABLE` — confirming D's steady-state (staged == installed) stays
   silent.
3. **The guard, live against the real boundary, no root and no Touch ID:** built
   a tree whose `libexec/sudo-secretspec` was a **hard link** to
   `/usr/local/libexec/sudo-secretspec`. Refused, exit 2, with the correct
   remedy in the message. Cleanup verified safe: link count 2 → 1, installed
   broker intact.
4. **`UPGRADE_AVAILABLE` end-to-end** against a throwaway `--config` carrying a
   fake version — exercised the whole chain (unprivileged probe → hidden flags →
   sudo boundary → `.13` broker), not just the classifier:
   `advisory: True`, path `/opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec`,
   detail naming both versions.

### Upstream

**#355 merged** (`faa9645`, 01:45Z) — that is 2 of our 4 PRs landed (#354, #355).

**#334 fails to compile against current `main`, and #355 merging is what caused
it.** Verified, not inferred: merged `pull/334/head` into `main` (`e9004eb`)
locally, `cargo check -p secretspec --tests` →

```
error[E0308]: mismatched types
   --> secretspec/src/codegen.rs:573:27
    | expected `&Spec`, found `&Config`
```

Exactly one error. #355's new test `schema_emits_description_when_declared`
calls `build_ir(...)`; #334 re-signatures `build_ir` to take `&Spec` and
migrated the *then-existing* tests to a `build_ir_from_config` helper. The new
test landed in a region #334 never touched, so GitHub reports `MERGEABLE` /
`CLEAN` correctly — clean-merging and compiling are different things. One-line
fix verified: `build_ir` → `build_ir_from_config` at `codegen.rs:573`, after
which `cargo test -p secretspec --lib codegen::` passes 11/11.
Posted: https://github.com/cachix/secretspec/pull/334#issuecomment-5305816915

**#356 is blocked on a button, not a review.** All 6 workflow runs on head
commit `e917888` are `action_required`, at `0s`, across both pushes — CI has
never executed. That is what makes it `UNSTABLE`; nothing is failing. Critically,
**djbclark cannot approve these**: `gh api repos/cachix/secretspec` returns
`{"admin":false,"maintain":false,"pull":true,"push":false,"triage":false}`, so
the "Approve and run workflows" button does not render on his view at all. Only
a cachix maintainer can. A comment is therefore the only available lever.
Posted: https://github.com/cachix/secretspec/pull/356#issuecomment-5305817471

### Audit: does the same bug class exist elsewhere?

Operator-requested sweep of the analogous "destination is a valid source" shape.

- **`rollback`** — has the same *shape*: if `<index>.prior` were the same file
  as its destination, restore would copy a file onto itself and report success.
  **Not reachable.** `capture_snapshot` uses `fs::copy` (`install.rs:156`), which
  always produces a new inode, and crafting a self-referential snapshot requires
  already being root *and* writing a `0700` root-owned directory that
  `rollback::run` re-validates (uid 0, mode 0700, path prefix). Unlike install,
  no operator can walk into it by typing the obvious command.
  **Deliberately not fixed.** A defensive check would be ~4 lines that can never
  fire, and this project's own recorded precedent — the `export-declarations`
  decision in `docs/design/template-check-resync.md` — is that unused code in a
  root-privileged broker is a liability. Flagged for the operator rather than
  added unilaterally.
- **`uninstall`** — no instance. Its self-reference is deleting its own
  executable, which Unix handles safely (the inode survives until the last fd
  closes), and sudoers ordering is already deliberate (test:
  `the_sudo_policy_is_always_planned_first`).

## Operator Feedback

- **"I want you to implement all of it, A-D. And yes rewrite doc."** — done, all
  four plus the docs rewrite.
- **"When all this is done, do a new release, test, and then see if the skill,
  doc, or any issues or PRs need to be updated."** — done. Fork issues are
  disabled and it has no open PRs; the two upstream items are above.
- **Authorized posting both upstream comments unattended.** Done.
- **Ran the boundary upgrade before sleeping** so the live verification could
  proceed overnight.
- **"Keep busy without me to the extent you think you can usefully."** — scoped
  to read-only verification, the rollback/uninstall audit, and this handoff.
  Deliberately did **not** cut a second release unattended: a tag, a GitHub
  Release and a tap push are outward-facing, and the authorization was for the
  A–D release specifically.
- Still deferred, do not resurface: item 12 cross-platform sudo/Linux port
  (`docs/design/privilege-boundary-and-packaging.md:487`).
- Still live, lowest priority: how best to keep files synced between computers
  (partial answer in handoff `4752`, Where We're Going item 4).

## Where We're Going

1. **THE NEXT ACTION — decide whether to cut `0.19.1-sudo.14`.** Two commits are
   pushed but in no release: `f3a9629` (pre-elevation guard) and `5bb76df`
   (README). Neither is urgent — `.13` already refuses the bad install, just
   after authenticating instead of before. Cutting `.14` needs no Touch ID, but
   *installing* it does, so it wants the operator present. Cutting it would also
   naturally demonstrate `UPGRADE_AVAILABLE` in its real window (staged `.14`
   vs installed `.13`).
2. **Watch the two upstream comments for replies** —
   `gh pr view 334 --repo cachix/secretspec --comments` and
   `gh pr view 356 --repo cachix/secretspec --comments`. #356 cannot progress
   until a maintainer approves its workflow runs.
3. **Do not delete `explore/pr-334-rust-first-spec`** — `bf0b25c` is linked by
   SHA from a public comment on #357. (Separate throwaway branch
   `pr334-merge-check` and its worktree under the session scratchpad are safe to
   delete; they were only for the compile verification.)
4. Optional, operator's call: the `rollback` defensive check argued against
   above. Recommendation is to document the invariant rather than add code.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -5                 # expect f3a9629 at HEAD, tree clean
sudo-secretspec --version            # expect 0.19.1-sudo.13
sudo-secretspec doctor               # expect OK, exit 0, no advisories

# prove the guard again, no root and no Touch ID needed:
SB=$(mktemp -d); mkdir -p "$SB"/{bin,libexec,share/sudo-secretspec}
cargo build -p sudo-secretspec-cli && cp target/debug/sudo-secretspec "$SB/bin/"
ln /usr/local/libexec/sudo-secretspec "$SB/libexec/sudo-secretspec"
cp /usr/local/share/sudo-secretspec/{AI-GUIDANCE.md,sudo-secretspec-retired.toml} \
   "$SB/share/sudo-secretspec/"
"$SB/bin/sudo-secretspec" install --adopt-existing --non-interactive  # expect exit 2
rm -rf "$SB"                         # removes the link only; original is untouched

# upstream
gh pr view 334 --repo cachix/secretspec --comments
gh pr view 356 --repo cachix/secretspec --comments

# upgrading the boundary (needs the operator; one Touch ID):
"$(brew --prefix)"/opt/sudo-secretspec/libexec/sudo-secretspec install --adopt-existing
sudo-secretspec --version
```

Cleanup still pending from this session (harmless, in the scratchpad):
`git worktree remove <scratchpad>/pr334 && git branch -D pr334-merge-check pr334-verify`
