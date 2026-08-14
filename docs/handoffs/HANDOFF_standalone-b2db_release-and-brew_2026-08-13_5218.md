---
schema_version: 1
handoff_id: 5218
parent_handoff_ids: [f3eb]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 5f5ff9a3a97cb3444b22f55595fea17e4659576f
created_at: 2026-08-13T22:13:34-04:00
writer: claude-code
---

# Handoff — v0.19.1-djbclark.1 released; Homebrew keg built but unlinked

## The Goal

Finish what the hermes session set out to do this morning ("review, then
release v0.19.1-djbclark.1"). The parent handoff `f3eb` closed the review
half. This session was to close the release half, then get the release
installable via the Homebrew tap on this machine.

**The release is published.** What remains is Homebrew packaging polish and
one operator decision about which `secretspec` wins on PATH.

## Where We Are

Branch `sudo-main` at `5f5ff9a`, **clean tree**, pushed to origin.

Published and irreversible (all verified by readback):

| Artifact | State |
|---|---|
| Tag `v0.19.1-djbclark.1` | pushed; annotated `668f173` → commit `9ccbcd7` |
| GitHub Release | live, `isDraft=false`, "SecretSpec 0.19.1 — sudo-secretspec downstream 1" |
| Formula sha256 | real digest `29f25c0274ad8db6fd5e14b5c6db5ba2906c906e470e5504134f71a16e540200`, commit `5f5ff9a` |
| Tap `djbclark/homebrew-sudo-secretspec` | `Formula/sudo-secretspec.rb` at `afd2520`, public, default branch `main` |
| Local brew tap | tapped; clone at `/opt/homebrew/Library/Taps/djbclark/homebrew-sudo-secretspec` |

Built but **not linked**: `/opt/homebrew/Cellar/sudo-secretspec/1` (12 files,
44.5MB, 6 min build). Both binaries in the keg work and report
`0.19.1-djbclark.1`. `share/sudo-secretspec/AI-GUIDANCE.md` and the skill
are installed.

Two packaging defects remain (neither needs a re-tag — the formula and tap
are fixable independently of the published release):

1. **Homebrew parsed the version as `1`.** It cannot extract
   `0.19.1-djbclark.1` from `.../tags/v0.19.1-djbclark.1.tar.gz`, so the
   Cellar path is `.../sudo-secretspec/1` and `brew list --versions` shows
   `sudo-secretspec 1`. Fix: explicit `version "0.19.1-djbclark.1"` in the
   formula. Affects upgrade detection, not just cosmetics.
2. **`brew link` conflict** — this is what made `brew reinstall` exit 1. A
   separate upstream `secretspec` 0.19.1 formula is installed and owns
   `/opt/homebrew/bin/secretspec`; our formula ships the same binary name,
   so linking refused and left the keg unlinked.

## What We Tried

Chronological; the failures are the expensive part to rediscover.

1. **`sudo -l <command>` to decide whether install needs a password.**
   Failed — `sudo -l` reports only whether a command is *permitted*, not
   whether it requires authentication. All four probes printed the command
   because the blanket `(ALL) ALL` rule matches everything. Superseded by
   `sudo -n <command>`, which does report the auth decision and, with `-n`,
   never executes when a password is required.
2. **Hypothesis: Touch ID was never enabled, so the missing prompt was a
   non-event.** Wrong. `pam_tid.so` is enabled *twice* — `/etc/pam.d/sudo`
   line 2 and an uncommented line in `/etc/pam.d/sudo_local`. Killing this
   hypothesis made the question sharper, not softer.
3. **`release.py --dry-run` as a release rehearsal.** Near-worthless as
   found: `main()` skipped `preflight()` entirely on the dry-run path, so
   every invariant most likely to be wrong went unchecked. Fixed this
   session; hand-checking preflight is what found the release blocker.
4. **Removing the single corrupted cargo cache entry (`addr2line-0.25.1`).**
   Too narrow. The next build failed identically on `adler2-2.0.1`. A scan
   showed **464 of 465** extracted crates were corrupt. Per-crate removal
   would have taken hundreds of iterations.
5. **Piping `brew` output through `tail` (twice).** The pipeline's exit
   status is `tail`'s, so a failed build was reported as exit 0. Run brew
   unpiped when the exit code matters.

## Key Decisions

- **Skip `PROMPT-SECREV.md` for now** (operator: a separate AI will run it).
  It has still never been executed against the hardened crate. Rejected:
  blocking the release on it.
- **`parent_slug()` tolerates both gh JSON shapes** rather than pinning a gh
  version. Rejected: requiring gh ≥ some version, which would break silently
  on other machines.
- **`--dry-run` now runs `preflight()`.** Rejected: leaving it skipped. A
  rehearsal that skips validation hides exactly the failures it exists to
  surface — this bug among them.
- **Wiped all of `registry/src`** rather than per-crate surgery. The 2.0MB
  extraction layer is fully regenerable from the 45MB of `.crate` tarballs,
  which were verified intact (`tar -tzf` shows `Cargo.toml` present).
- **Leave `/usr/local/bin/sudo-secretspec` shadowing brew's copy.** This is
  by design: `AI-GUIDANCE.md` says "Use only /usr/local/bin/sudo-secretspec"
  and the sudoers policy is written against
  `/usr/local/libexec/sudo-secretspec`. Letting the brew copy win could
  weaken the privilege boundary hardened in the parent session.
- **PENDING — which `secretspec` wins on PATH.** Recommended
  `brew unlink secretspec && brew link sudo-secretspec` (reversible, keeps
  upstream installed). Alternative: `brew uninstall secretspec` (cleaner,
  discards upstream). Rejected: `brew link --overwrite`, which clobbers
  another formula's symlink and leaves it installed-but-broken.

## Evidence & Data

**The release blocker that was found and fixed (commit `9ccbcd7`).**
`gh repo view --json parent` on gh 2.97.0 returns
`{id, name, owner{login}}` with **no `nameWithOwner` key inside parent**.
`packaging/release.py:134` read that missing key, so `preflight()` would have
aborted with `fork parent must be cachix/secretspec, got None`. The identical
pattern at `packaging/release.py:296` in `verify_readback()` would have fired
**after** the tag and GitHub Release were already published. Reproduced
directly:

```
parent.nameWithOwner = None
preflight _require passes? False
```

The existing tests missed it because their mock asserted
`parent: {"nameWithOwner": "cachix/secretspec"}` — a shape gh does not
return here. Test and code encoded the same wrong assumption and agreed with
each other. New tests pin the real shape.

**CI — `sudo-release.yml` had never executed before this session** (it
triggers only on PR paths, tag push, or `workflow_dispatch`, and all fork
work goes straight to `sudo-main` with no PR).

| Run | Commit | Result |
|---|---|---|
| 31760888868 | `f2713ca` | success, all steps |
| 31761326194 | `9ccbcd7` | success, all steps |

`Verify pinned tag identity` shows `skipped` in both — correct, it is guarded
by `if: github.ref_type == 'tag'` and these were dispatches.

Test counts: companion crate **68** (`26+19+3+3+8+9`, matching the parent
session's local number, so CI reproduces the local suite); release helper
**9 → 14**; ruff check and format clean.

**Touch ID question — RESOLVED, boundary intact.** `install --dry-run`
returns early at `install.rs:515` after read-only checks (`id`,
`dscl -read`), before any writes, which made `sudo -n` probing safe:

| Command | Result |
|---|---|
| `doctor` (libexec) | runs passwordless — by design, NOPASSWD |
| `install --dry-run` (libexec) | `sudo: a password is required` |
| `install --dry-run --adopt-existing` (dev-tree binary) | `sudo: a password is required` |
| `rollback --list` (libexec) | `sudo: a password is required` |

`install` self-elevates at `main.rs:394` via `Command::new(SUDO)`
deliberately **without** `-n` ("Install itself needs interactive sudo/Touch
ID, not NOPASSWD -n"). The only passwordless root surface, `__broker *`,
dispatches a closed allowlist — `source-get/set/add/delete/check/export`,
`source-template-check`, `audit-verify` — with no install, rollback, or
sudoers verb. Conclusion: the originally-observed missing prompt was a cached
sudo timestamp (no explicit `timestamp_timeout` in policy, so the 5-minute
default applied). **Limit: the original event cannot be replayed** — that
timestamp expired — so this is inference from the currently enforced policy,
not a reproduction. Also unproven: that `pam_tid` engages rather than falling
through to a password prompt when one *is* supplied interactively (a UX
question, not a boundary one).

**Homebrew cargo cache corruption.** 464 of 465 extracted crates under
`~/Library/Caches/Homebrew/cargo_cache/registry/src/` had the `.cargo-ok`
completion marker but no `Cargo.toml`. That marker is why it could not
self-heal: cargo treats the extraction as complete and never retries, so each
build failed on a different crate as it walked the dependency graph. Not disk
space (62Gi free, no inode pressure). **Cause unknown and predates this
session's builds — if it recurs, it is a real host problem worth chasing.**

**Versions verified.** `secretspec 0.19.1-djbclark.1` and
`sudo-secretspec 0.19.1-djbclark.1` from the Cellar keg — both `brew test`
assertions would pass. Engine built clean in 2m27s before the cut, removing a
compile failure from the post-tag window.

**Files changed this session:** `packaging/release.py`,
`tests/sudo_packaging/test_release.py` (commit `9ccbcd7`);
`packaging/homebrew/sudo-secretspec.rb` (commit `5f5ff9a`, sha256 rewrite by
`release.py`). No Rust source touched — no CHANGELOG entry, per `CLAUDE.md`
scoping that to Rust changes and user-facing content.

## Operator Feedback

- **Skip the security review for now** — "I'll probably get a separate AI to
  do it." `PROMPT-SECREV.md` remains unexecuted by design, not oversight.
- **Wanted the brew path working end to end**: cut the release, get a brew
  release working, subscribe to the tap, and install from it on this machine.
- Mentioned `~/src/aiuse` as a source of brew packaging code to copy. **Not
  needed** — this project already has `packaging/homebrew/sudo-secretspec.rb`,
  a tap repo, and full tap automation in `release.py`.
- Wants the Tier 1 session log kept current as work lands.
- Prefers a summary restated after a handoff is written.

## Where We're Going

1. **THE NEXT ACTION — decide which `secretspec` owns
   `/opt/homebrew/bin/secretspec`, then link the keg.** Recommended:
   `brew unlink secretspec && brew link sudo-secretspec`. This is an operator
   decision because it changes what their shell runs.
2. Add `version "0.19.1-djbclark.1"` to
   `packaging/homebrew/sudo-secretspec.rb`, sync it to the tap, commit both.
   Homebrew currently reads the version as `1`.
3. Run `brew test djbclark/sudo-secretspec/sudo-secretspec` and the
   `verify_readback` equivalent — neither ran, because `release.py` died at
   `brew reinstall`.
4. Consider splitting `release.py`'s publish steps from its local-Homebrew
   verification. A purely local cache fault returned exit 1 for a release that
   had fully succeeded; `--skip-homebrew` already exists, so the fix is mostly
   about not conflating the two exit statuses.
5. Hand `PROMPT-SECREV.md` to the separate AI when ready.
6. Unrelated host hygiene: `sudo chmod 0440 /etc/sudoers.d/yabai` (mode is
   wrong, so sudo silently ignores that file; pre-existing, untouched).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect 5f5ff9a at HEAD, clean tree

# 1. Confirm the release really is published (all should succeed)
gh release view v0.19.1-djbclark.1 --repo djbclark/sudo-secretspec
brew tap | grep sudo-secretspec

# 2. THE next action — link the keg (operator decision first)
brew unlink secretspec && brew link sudo-secretspec
secretspec --version          # expect 0.19.1-djbclark.1

# 3. Then the formula version fix
$EDITOR packaging/homebrew/sudo-secretspec.rb    # add: version "0.19.1-djbclark.1"
cp packaging/homebrew/sudo-secretspec.rb ~/src/homebrew-sudo-secretspec/Formula/
git -C ~/src/homebrew-sudo-secretspec commit -am "sudo-secretspec 0.19.1-djbclark.1 explicit version" && git -C ~/src/homebrew-sudo-secretspec push

# 4. Verification that never ran
brew test djbclark/sudo-secretspec/sudo-secretspec

# If the cargo cache corruption recurs (464/465 crates had .cargo-ok but no Cargo.toml):
rm -rf ~/Library/Caches/Homebrew/cargo_cache/registry/src   # .crate tarballs are retained
```

Do **not** disturb `/usr/local/bin/sudo-secretspec` or
`/usr/local/libexec/sudo-secretspec` — those are the privileged install's
client and policy target, and the sudoers rules are written against the
libexec path.
