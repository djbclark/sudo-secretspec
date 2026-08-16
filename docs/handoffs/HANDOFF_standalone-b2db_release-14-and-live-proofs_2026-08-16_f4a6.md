---
schema_version: 1
handoff_id: f4a6
parent_handoff_ids: [a651]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: a5a1c8dc5bd45050fee35c95fd1a2b9178a67164
created_at: 2026-08-16T01:31:19-0400
writer: claude-code
---

# Handoff — release 0.19.1-sudo.14 and the live proofs it made possible

Short continuation of `a651`, which holds the full account of the install
self-source bug, its diagnosis, and the A–D implementation. **Read `a651` first
for the why.** This document covers only what happened after it: the `.14`
release and the verification that release unlocked.

## The Goal

Operator: *"push and release everything, do a new cut, then test it"*, followed
by *"actually don't do tests that require me to be around, do that after the
handoff."*

So: ship `.14`, run every test that does not need the operator present, write
this, and leave the one test that does need them queued.

## Where We Are

`sudo-main` @ `a5a1c8d`, clean, pushed. Release **0.19.1-sudo.14** cut (GitHub
Release + tap live, `brew test` passed, built in 13 minutes).

**The boundary is deliberately still at `.13`.** Installing `.14` needs an
interactive Touch ID prompt, which is the operator-present test that was
explicitly deferred. This is not drift and not an oversight — it is the queued
next action, and it is also *why* the strongest proof below was possible.

| SHA | What |
| --- | --- |
| `1d2a7db` | skill v0.5.0 + AI-GUIDANCE to `.14` (advice changed, not just the stamp) |
| `49bd13a` | workspace version stamp 0.19.1-sudo.14 |
| `a5a1c8d` | Homebrew formula for v0.19.1-sudo.14 |

`.14` releases two commits that `a651` recorded as pushed-but-unreleased:
`f3a9629` (pre-elevation guard) and `5bb76df` (README).

## Evidence & Data

Three verifications, all run with no operator present and none simulated.

### 1. `UPGRADE_AVAILABLE` in its real window — the proof `a651` could not get

`a651` could only demonstrate this advisory against a throwaway `--config`
carrying a fake version, because staged and installed were both `.13`. Cutting
`.14` while the boundary stayed at `.13` created the genuine window. Real
config, no fakery:

```
installed: sudo-secretspec 0.19.1-sudo.13
staged   : sudo-secretspec 0.19.1-sudo.14

$ sudo-secretspec doctor
doctor: OK
- [advisory] UPGRADE_AVAILABLE (/opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec):
  a different build is staged here (0.19.1-sudo.14) than the one installed
  (0.19.1-sudo.13); install it by running that path directly -- it is kept off
  PATH so it cannot shadow the installed client
exit=0
```

Every design property held in production: the advisory prints *under* a passing
check, `exit=0` (it is not a stop condition for automated callers), it names
both versions, and it names the exact path to run.

### 2. The pre-elevation guard, in the shipped `.14` artifact

Built a tree whose `bin/sudo-secretspec` is the **released `.14` keg binary**
and whose `libexec/sudo-secretspec` is a **hard link** to the real installed
broker, then ran install against it with stdin closed:

```
binary under test: sudo-secretspec 0.19.1-sudo.14
install denied: refusing to install from the installed boundary itself (...)
exit=2 elapsed=0s
```

`elapsed=0s` with stdin closed is the load-bearing detail: an authentication
prompt would have blocked or failed. The refusal now happens entirely before
elevation. The hard link is also why `(dev, ino)` is the right comparison — a
path-based check would have passed this through.

Cleanup verified safe both times: broker link count returned to 1.

### 3. Nothing was mutated

`sudo-secretspec --version` still `0.19.1-sudo.13`; `doctor` exit 0;
`audit-verify` → 482 events, tip
`dbb67e2f9d34821b612c40b4f088bacaa83c56f29412b28944bb99b7387b7310`.
Working tree clean.

## Key Decisions

**Updated the skill and AI-GUIDANCE *before* cutting the tag, not after.** Both
ship inside the release, so a post-tag doc fix would not reach the installed
copy — which is exactly the trap the `.12` release hit (see `a651` and handoff
`4752`). The `.14` doc change is substantive, not a version bump: the refusal
moving ahead of elevation means a wrong invocation now costs no authentication
prompt and is reachable unattended, which is advice an agent acts on.

**Left the boundary at `.13` rather than installing `.14`.** Directed by the
operator, and it turned out to be what enabled proof #1. Worth remembering as a
technique: the pre-upgrade window is the only time `UPGRADE_AVAILABLE` can be
observed without fabricating state.

## Operator Feedback

- *"push and release everything, do a new cut, then test it"* — done.
- *"don't do tests that require me to be around, do that after the handoff"* —
  honoured; the only such test is installing `.14`, queued as item 1.

## Where We're Going

1. **THE NEXT ACTION — install `.14` and confirm the version transition.** This
   is the operator-present test that was deferred. One Touch ID prompt:

   ```bash
   "$(brew --prefix)"/opt/sudo-secretspec/libexec/sudo-secretspec install --adopt-existing
   ```

   Expect `installed sudo-secretspec 0.19.1-sudo.13 -> 0.19.1-sudo.14` — **with
   the arrow this time**, unlike the `.13` upgrade, because `.13` wrote the
   `version` key into the protected config. That arrow is the first live
   confirmation of the version-transition reporting doing its actual job.
   Afterwards `doctor` should return to `OK` with **no** `UPGRADE_AVAILABLE`,
   since staged and installed match again.
2. **Watch the two upstream comments for replies** —
   `gh pr view 334 --repo cachix/secretspec --comments` (compile break against
   current `main`, one-line fix supplied) and
   `gh pr view 356 --repo cachix/secretspec --comments` (asked a maintainer to
   approve workflow runs that have never executed). Neither can progress from
   this side: djbclark has pull-only access on `cachix/secretspec`, so the
   approve button does not render for him. Do not go hunting for a URL.
3. **Do not delete `explore/pr-334-rust-first-spec`** — `bf0b25c` is linked by
   SHA from a public comment on upstream #357.
4. Optional, operator's call and argued against in `a651`: a self-reference
   check in `rollback`. Not reachable without root; the project's own
   `export-declarations` precedent says unused code in a root-privileged broker
   is a liability. Recommendation stands: document the invariant, don't add code.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3            # expect a5a1c8d at HEAD, tree clean
sudo-secretspec --version       # expect 0.19.1-sudo.13 until item 1 is done
sudo-secretspec doctor          # expect OK + the UPGRADE_AVAILABLE advisory

# item 1 (needs the operator; one Touch ID):
"$(brew --prefix)"/opt/sudo-secretspec/libexec/sudo-secretspec install --adopt-existing
sudo-secretspec --version       # expect 0.19.1-sudo.14
sudo-secretspec doctor          # expect OK, advisory now gone

# re-prove the guard any time, no root and no Touch ID:
SB=$(mktemp -d); mkdir -p "$SB"/{bin,libexec,share/sudo-secretspec}
cp /opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec "$SB/bin/"
ln /usr/local/libexec/sudo-secretspec "$SB/libexec/sudo-secretspec"
cp /usr/local/share/sudo-secretspec/{AI-GUIDANCE.md,sudo-secretspec-retired.toml} \
   "$SB/share/sudo-secretspec/"
"$SB/bin/sudo-secretspec" install --adopt-existing --non-interactive </dev/null
rm -rf "$SB"                    # removes the link only; the original is untouched
```
