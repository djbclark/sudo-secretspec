---
schema_version: 1
handoff_id: cef6
parent_handoff_ids: [3e79]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 9138c96e9b617dc562de9bb466e64449502fc974
created_at: 2026-08-18T21:28:02-0400
writer: claude-code
---

# Handoff — the endpoint is proven, and the fork now sits on upstream HEAD

## THE NEXT ACTION

**Build the isolated test boundary from
[#3](https://github.com/frdminc/sudo-secretspec/issues/3), Part A.** Everything
else is blocked behind it. The endpoint's `--client` path is written but has
never been run against a real vault, and it must *not* be pointed at the
operational `/usr/local` boundary — this session crossed that boundary twice by
accident and the operator's standing rule is that it is load-bearing shared
infrastructure.

The single blocker is one line, `sudo-secretspec-cli/src/main.rs:20`:

```rust
const CONFIG_PATH: &str = "/usr/local/etc/sudo-secretspec.toml";
```

Gate an override behind a cargo feature (`test-boundary`, `default = []`) so a
release binary does not contain the code path at all. **Do not make it a plain
runtime environment variable** — redirecting the config redirects the vault path
and the service user, which is a privilege-escalation vector.

## The Goal

Resume `3e79`'s next action: write the minimal `ProviderHandler` in
`/Users/djbclark/src/ss-ipc-proto` and run it against upstream's
`provider-lifecycle.json`. Done. The operator then redirected mid-session to two
further goals: get the fork onto upstream HEAD, and stop testing against the
live boundary.

## Where We Are

`sudo-main` clean at `9138c96`, everything pushed. Three commits here this
session: `3e160f9` (design doc), `9138c96` (upstream merge), plus this handoff.
The prototype lives in a different worktree — `/Users/djbclark/src/ss-ipc-proto`
at `a204898`, branch `proto/privileged-endpoint`, **not pushed** (scratch).

**The fork is now 0 commits behind `upstream/main`.**

Test baseline: `cargo test -p sudo-secretspec-cli` = **229 passed / 0 failed**,
identical before and after the upstream merge. `cargo check -p secretspec
--all-targets` clean (3m30s cold).

Live vault never touched deliberately. See "What We Tried" for the one accidental
crossing.

New artifacts:

- `/Users/djbclark/src/ss-ipc-proto/privileged-endpoint-proto/` — the endpoint,
  its own workspace root, own README carrying all findings.
- `.../privileged-endpoint-proto/upstream-macos-sigtimedwait.patch` — a fix for
  upstream, deliberately **not applied**.
- Issue [#3](https://github.com/frdminc/sudo-secretspec/issues/3) — the isolated
  test boundary plan.
- Tag `pre-upstream-merge-2026-08-18` on the pre-merge `sudo-main`.

## What We Tried

**The conformance case passed for the wrong reason, and I nearly shipped that.**
The endpoint's first version used the installed client when given no arguments.
Upstream's `ipc-provider-conformance-driver` spawns an endpoint with *no
arguments* (`open_endpoint(endpoint, &[])`). So the first green run had executed
`/usr/local/bin/sudo-secretspec get --reason conformance __BLOCK__` against the
**live boundary, twice**, unannounced. It passed only because the subprocess was
slow enough to resemble a blocked request — the `__BLOCK__` branch I had written
was never reached. Two audit records exist with reason `conformance` for a
nonexistent name; no value was read or printed. Fixed by making no-arguments
mean no privileged access, then re-running *and* re-mutating.

**Mutation testing is what made the eventual pass trustworthy.** Replacing the
`__BLOCK__` branch with `if false` fails the case with *"provider cancellation
did not produce one cancelled terminal"* (exit 1). Without that check the green
result proved nothing, exactly as in the previous session's `busy_timeout`
episode.

**Assumed a big rebase; it was five commits.** I read `git rev-list --count
v0.19.1..sudo-main` = 286 and `v0.19.1..upstream/main` = 92 and started sizing a
massive merge — 129 overlapping files, 60 of them code, 12,661 insertions. All
of that was wrong framing: the actual merge base is `dfa4b10`, **87 commits past
`v0.19.1`**, because sudo-main already carried post-0.19.1 upstream work.
`dfa4b10..upstream/main` is **5 commits**, one conflict. *Compute the merge base
before sizing a merge; a tag is not a base.*

**`upstream`'s push URL was live in this checkout.** The sibling
`secretspec-sqlite` clone had it disabled; this one did not. Set to `DISABLED`
before doing anything that could push.

## Key Decisions

- **The prototype lives outside the upstream workspace.** It declares its own
  `[workspace]` table and depends on `secretspec-ipc` by path through public API
  only, so the no-patching claim is checkable by `git diff` rather than argued.
  *Rejected:* adding it as a workspace member (would have required editing
  upstream's root `Cargo.toml`, destroying the very claim being tested).
- **Kept the macOS C fix as a patch file rather than applying it**, for the same
  reason. Applying it would have left a non-empty `git diff` in the checkout.
- **Merge, not rebase, onto upstream.** A 199-commit fork delta replayed
  commit-by-commit is all downside; a merge is one conflict resolution and
  preserves history. `sudo-main` is the fork's trunk and upstream is a supplier.
- **Every non-zero broker exit collapses to one opaque `OperationFailed`.** The
  channel cannot distinguish "missing" from "denied", and reporting a denial as
  "this secret does not exist" is a security-relevant misreport. *Accepted cost:*
  the endpoint currently can never report a genuinely absent secret.
- **The endpoint advertises `resolve_address` + `get` and nothing else** — the
  smallest set upstream's `validate_capabilities` accepts. Writes are refused by
  upstream before reaching our code, so "read-only" needs no policy branch.
- **`initialize`'s caller-supplied `uri`, `base_dir` and `credentials` are
  dropped.** Deriving a vault path from them would hand the caller exactly the
  authority the boundary exists to withhold.
- **`project`/`profile` are not part of the address.** The privileged vault is
  one system-scoped namespace; `resolve_address` is the hook that makes the
  discard visible instead of silent.
- **Isolation: compile-time test prefix + ephemeral macOS CI. VM rejected** —
  operator does not own Parallels and runs out of disk weekly. *Also rejected:*
  chroot (macOS chroot isolates the filesystem but not `dscl`, `launchd`, or
  `/etc/sudoers.d`, which is precisely what this boundary is made of).
- **Did not merge `upstream/feat/ipc-v1`.** Six conflicts including
  `secretspec/src/config.rs` and `lib.rs`; belongs on a topic branch in a fresh
  session, since #362 is open and will be rebased.

## Evidence & Data

- Conformance transcript, `provider.lifecycle`, exit 0:
  `{"case":"provider.lifecycle","events":[{"kind":"initialized"},{"kind":"cancelled"},{"kind":"deadline_exceeded"},{"kind":"terminal"},{"kind":"closed"}]}`
- Upstream footprint after full build/test/conformance: `git diff --stat` empty;
  `git status --porcelain` = `?? privileged-endpoint-proto/`.
- **`sigtimedwait` does not exist on macOS.** `grep -rn sigtimedwait
  "$(xcrun --show-sdk-path)/usr/include/"` → no matches; `nm -g
  /usr/lib/libSystem.B.dylib | grep -i sigtimedwait` → no matches. So it is a
  *link* failure as well as a compile error, not a strict-mode nit. Site:
  `libsecretspec-ipc/src/process_posix.c:237`, under a plain `#ifndef _WIN32`.
  Blocks `cargo build -p secretspec-ipc-conformance` entirely on macOS, which
  takes out both conformance drivers and the `provider_cases`/`client_cases`
  tests. Reported: cachix/secretspec#362 comment `5336293027`.
- **Broker exit codes are ambiguous.** `source-get` exits 1 for *has no value*,
  *is not resolved*, and a resolve error alike (`broker.rs:960-978`); a `sudo -n`
  denial is also 1. Exit 2 covers argument validation, "cannot invoke broker",
  and version skew (`main.rs:731-744`).
- **Trailing-newline loss:** broker emits with `println!("{value}")`
  (`broker.rs:962`), inherited by the client through `cmd.status()`.
- **Upstream bounds an address key at 4096 bytes and nothing else**
  (`Address::validate`, `protocol.rs:737`), so `--help` is a legal key.
- Merge arithmetic: merge base `dfa4b10` = `secretspec-go/v0.19.1-87-gdfa4b10`;
  `dfa4b10..upstream/main` = 5 commits; only 18 non-merge fork commits touch
  `secretspec/src`. Conflict: `CHANGELOG.md` only (ours 39 lines, theirs 21,
  both kept).
- `upstream/feat/ipc-v1` is 6 ahead of / 2 behind `upstream/main`;
  `dfa4b10..feat/ipc-v1` = 9 commits. PR #362 open, 6 commits, base `main`.
- Machine: Apple M1, macOS 26.6.1, 98Gi free. Parallels **and** UTM installed,
  but Parallels is unlicensed.

## Operator Feedback

- **Work against basically HEAD.** "I think we will be more useful and get more
  accepted by upstream if we work against basically HEAD" — 0.20, a fork, or
  cherry-picks, whatever it takes.
- **Stop touching the operational boundary.** Testing must not disrupt the
  currently working, load-bearing `sudo-secretspec`.
- **No VMs.** No Parallels licence; weekly disk pressure. Chose the compile-time
  test prefix *plus* macOS CI, both.
- Standing from earlier chain: Opus 5 / high effort, never Fast mode; herdr panes
  for external reviews, never backgrounded `cursor-agent --print`; never print a
  retrieved secret value.

## Where We're Going

1. **THE NEXT ACTION** — #3 Part A, above.
2. #3 Part B: the `macos-latest` lifecycle workflow (install → declare → set →
   get → drift → rollback → uninstall → `--purge-vault`), synthetic secrets only.
3. Then, unblocked: wire the endpoint's `--client` path end to end against the
   *test* boundary and add a conformance-style case for it.
4. Merge `upstream/feat/ipc-v1` on a topic branch off `sudo-main` (6 conflicts:
   `.github/workflows/test.yml`, `CHANGELOG.md`, `Cargo.lock`, `Cargo.toml`,
   `secretspec/src/config.rs`, `secretspec/src/lib.rs`). Do not put it on
   `sudo-main` while #362 is open.
5. Watch cachix/secretspec#362 for a maintainer reply on the `sigtimedwait`
   report; offer it as a PR against `feat/ipc-v1` if that is preferred.
6. Still live, independent: `drift`/`doctor` never verifies the manifest chain at
   rest (`drift.rs:1036` is audit-only; `history.rs:387` covers capture).
7. Other track: `djbclark/secretspec-sqlite` issue #1 — confirm
   `secretspec/src/provider/sqlite.rs` even compiles against upstream `main`
   (**still unverified**, written against 0.19.1). Cheapest test of that track.
8. Standing: don't touch `PROMPT-REVIEW.md`, `PROMPT-SECREV.md`,
   `REVIEW_REPORT.md`, or branch `explore/pr-334-rust-first-spec` without asking.
   **DEFERRED, do not resurface:** cross-platform sudo/Linux port. Once upstream
   #377 resolves: `git worktree remove /Users/djbclark/src/ss-sigpipe && git
   branch -D fix/sigpipe-default-disposition`.

**SUPERSEDED, do not build:** the two-database `secrets.db`|`broker.sqlite3`
merge and its `transactions` FK. **RETRACTED, do not re-report:** "no
`busy_timeout` in the CLI crate" is FALSE — rusqlite 0.31.0 sets 5000ms on every
`Connection::open` (`inner_connection.rs:121`).

## Quick Start

```bash
# 0. READ FIRST, in this order:
gh issue view 3 --repo frdminc/sudo-secretspec     # the next action
gh issue view 2 --repo frdminc/sudo-secretspec     # the endpoint track
sed -n '1,120p' /Users/djbclark/src/ss-ipc-proto/privileged-endpoint-proto/README.md

# 1. EVERY cargo command needs this env (system SQLite, not bundled):
export PKG_CONFIG_PATH=/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH \
       LIBRARY_PATH=/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH \
       CPATH=/opt/homebrew/opt/sqlite/include:$CPATH
cargo test -p sudo-secretspec-cli      # expect 229 passed / 0 failed
# NEVER pipe cargo through `tail` without ${PIPESTATUS[0]} — it masks the exit code.

# 2. Rebuild the conformance driver (upstream is macOS-broken — apply, build, revert):
cd /Users/djbclark/src/ss-ipc-proto
git apply privileged-endpoint-proto/upstream-macos-sigtimedwait.patch
cargo build -p secretspec-ipc-conformance --bin ipc-provider-conformance-driver
git checkout -- libsecretspec-ipc/src/process_posix.c   # keep `git diff` empty

# 3. Re-run the passing case:
cargo build --manifest-path privileged-endpoint-proto/Cargo.toml
/Users/djbclark/.cargo/target-shared/debug/ipc-provider-conformance-driver \
  --implementation endpoint \
  --endpoint /Users/djbclark/.cargo/target-shared/debug/sudo-secretspec-endpoint \
  < conformance/ipc/cases/provider-lifecycle.json

# 4. Probe with the RIGHT sqlite. Bare `sqlite3` is the Android SDK build and lies.
/opt/homebrew/opt/sqlite/bin/sqlite3 --version   # expect 3.53.4

# 5. Undo the upstream merge if it ever needs undoing:
git -C /Users/djbclark/src/sudo-secretspec reset --hard pre-upstream-merge-2026-08-18
```

**Method rule this chain has now paid for three times:** a `grep` result is not a
behavioral fact, and a green test is not a passing test. Remove the fix, or
remove the branch under test — if it still passes, it proved nothing.

**NEVER print a retrieved secret value.** `V=$(sudo-secretspec get <NAME>
--reason '...')` then `echo ${#V}` only. Announce before any live-boundary work.
