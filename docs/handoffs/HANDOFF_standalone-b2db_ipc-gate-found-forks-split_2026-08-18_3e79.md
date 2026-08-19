---
schema_version: 1
handoff_id: 3e79
parent_handoff_ids: [7a1c]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 917cc9472acf8c9d68b8bc973c7c5530ebb41cde
created_at: 2026-08-18T20:20:34-0400
writer: claude-code
---

# Handoff — the gate was already upstream; the project split in two

## THE NEXT ACTION

**Write the minimal `ProviderHandler` and get it compiling**, in the existing
worktree `/Users/djbclark/src/ss-ipc-proto` (branch `proto/privileged-endpoint`,
from `upstream/feat/ipc-v1` at `a393a27`). Three methods: `capabilities`,
`initialize`, `resolve_address` — then `get`. Model it on upstream's reference
endpoint at `conformance/ipc/runner/src/bin/ipc-provider-endpoint-rust.rs`
(494 lines). Then run it against
`conformance/ipc/cases/provider-lifecycle.json`. Only after that, wire it to a
real vault path and `sudo`.

Everything you need to know before starting is in
**[frdminc/sudo-secretspec#2](https://github.com/frdminc/sudo-secretspec/issues/2)**
and `docs/design/upstream-ipc-v1-and-the-fork.md` (STATUS section at the top,
added this session).

## The Goal

Resume `ccbb`'s stated next action — the two-database file merge. That work is
now **superseded**, and the session instead established the project's actual
shape. Nothing was merged, and that is the correct outcome.

## Where We Are

`sudo-main` clean at `917cc94`, everything pushed. Three commits, all docs:
`ffdf68e` (handoff 7a1c), `d0bf406` (its retraction), `917cc94` (IPC doc
front-loaded + protocol-name correction). **No Rust changed all session.**

Live vault never touched. Test baseline `cargo test -p sudo-secretspec-cli` =
**229 passed / 0 failed** (verified before any edits).

New this session:

- **`djbclark/secretspec-sqlite`** — public, a true fork of `cachix/secretspec`,
  cloned at `/Users/djbclark/src/secretspec-sqlite`, `upstream` remote added
  with **pushes to upstream disabled**. Its only job: upstream the sqlite
  provider. Issue **#1** carries the scope and the rejected alternatives.
  (Forked to `djbclark`, not `frdminc`, because GitHub allows one fork per
  account and `frdminc/sudo-secretspec` already holds that slot.)
- **Worktree `/Users/djbclark/src/ss-ipc-proto`** on `proto/privileged-endpoint`
  for the endpoint prototype.
- Issues **secretspec-sqlite#1** and **sudo-secretspec#2**, cross-linked both
  ways. Issues had to be *enabled* on both repos first (off by default).

## What We Tried

Three of this session's headline findings were **wrong and were retracted**.
That is the most useful content here.

- **"No `busy_timeout` in the CLI crate" (both external reviewers' P0).**
  FALSE. rusqlite 0.31.0 calls `sqlite3_busy_timeout(db, 5000)` unconditionally
  on every `Connection::open`
  (`rusqlite-0.31.0/src/inner_connection.rs:121`). I wrote the fix, wrote a
  contention test, went green — **then the mutation survived**, which is what
  exposed the line as a no-op. Reverted. *A `grep` returning nothing is not
  evidence the behavior is absent when a library sets a default.*
- **"The manifest chain is never verified in production."** Overstated.
  `capture` verifies it on every append (`history.rs:387`), symmetric with
  audit (`audit.rs:932`). Only the at-rest `drift`/`doctor` check is missing.
- **"The `head` collision fails loudly."** Wrong the other way — it is
  operationally **silent**: `broker.rs:691` makes audit always win the creation
  race deterministically, and `Mutation::commit` swallows the resulting archive
  failure into a warning (`broker.rs:428-443`).
- **Reported a false green baseline** by piping `cargo test` through `tail`,
  which masked a build failure. Then blamed `CLAUDE.md` for what was actually a
  missing `devenv` on this machine. `devenv 2.2.1` is now installed.
- **Nearly ran `brew install php`** to fix that; `devenv.nix:61,87` already
  pins `pkgs.php.unwrapped.dev` for `ext-php-rs`. Checking the file first is
  what avoided a duplicate, version-skewed dependency.
- **Misread the architecture entirely** — treated "upstream's generic provider"
  and "our broker's storage" as two permanent owners, and built a whole
  objection on it. The operator corrected: the plan is **one** sqlite provider,
  upstreamed. That reframing is what led to everything below.

## Key Decisions

- **The project splits in two, and the gate between them already exists
  upstream.** `cachix/secretspec` PR
  [#362](https://github.com/cachix/secretspec/pull/362) defines
  **`secretspec.provider/1`**: an external process serving as a provider
  endpoint. This fork becomes an out-of-tree *privileged* endpoint. **This was
  worked out on 2026-08-16, recorded in
  `docs/design/upstream-ipc-v1-and-the-fork.md`, and then went untracked** —
  hence issue #2.
- **Verified this session, not assumed:** the endpoint-author API is
  `ProviderHandler` (`secretspec-ipc/src/provider.rs:43`) + `serve_provider`
  (line 357); **only `resolve_address` is mandatory**; the reference endpoint
  serves over **stdio**, so `sudo` launching it as the service user *is* the
  privilege mechanism. No upstream patching, no privilege code upstream.
- **Protocol rename caught:** northbound is now `secretspec.resolver/1`, not
  `secretspec.client/1` (branch tip `a393a27`). `secretspec.provider/1` is
  unchanged.
- **Upstream should get the sqlite provider *with* opt-in history.** Evidence:
  `NativeAddress` already carries `pub version: Option<String>` — *"the secret
  version to read on stores that support version-pinned reads"*
  (`config.rs:1639`) — honored by openbao/AWS PS/AKV/Scaleway and rejected by
  1Password (`onepassword.rs:1178`). Versioning is an existing upstream
  abstraction, not our invention. And `onepassword.rs` is **2,939 lines**, so
  upstream tolerates rich providers.
  **Rejected:** upstreaming a *minimal* provider and patching history back in
  (shared-file trap — worse than keeping all of sqlite in the fork, since a file
  upstream lacks can never conflict; and `file.rs:33-37` rejects any query
  string, which would break the load-bearing `?history=true`); a separate
  provider crate (**impossible** — `pub(crate) mod provider`, `lib.rs:62`, plus
  `linkme` registration); asking for a public registry or sealed trait (wrong
  ask — the maintainer's answer to out-of-tree providers is IPC).
- **Privilege separation is NOT a provider concern** and stays downstream. The
  fork's own `sqlite.rs:7-11` already says the privilege model is external to
  the provider.
- **SUPERSEDED: the two-database merge and its `transactions` FK.** "Cut by
  owner" assumed a permanent provider/broker split that this plan dissolves.
  If ever revisited: SQLite cannot add an FK to an existing table, and FKs
  cannot span files (**both verified by probe**), so splitting the FK from the
  merge means rewriting the audit trail twice.

## Evidence & Data

- Baseline: **229/0** (`cargo test -p sudo-secretspec-cli`). `cargo test --all`
  needs `devenv shell` first.
- **Disk: 44Gi → 104Gi free.** Deleted `ss-370/target`, `ss-sigpipe/target`,
  `sudo-secretspec/target` (45G, needed operator `sudo` — root-owned artifacts),
  `brew cleanup --prune=all`. **`~/.cargo/config.toml` now sets
  `build.target-dir = ~/.cargo/target-shared`** (verified via `cargo metadata`),
  marked `.metadata_never_index` once — `cargo clean` destroys a per-repo
  marker, this one survives. Spotlight had **9,267** indexed items under one
  `target/`.
- **External review yield was poor and is worth planning around.** Four
  `cursor-agent --print` runs: **two returned a single byte at rc=0**. Two herdr
  panes: codex silently swallowed its first prompt, then hit a usage limit
  (quota returns Aug 20); its offer to downgrade to a cheaper model was
  declined per standing rule. **Always `wc -c` a review before reading it.**
- **Model agreement is not corroboration.** The two reviewers that answered the
  first brief agreed with each other on the false `busy_timeout` P0. On the
  second brief they **disagreed**, and the more specific one (Cursor Grok 4.6
  Extra High) was right; the other's separate-crate recommendation is refuted by
  `pub(crate) mod provider`.
- Full reviews: `review-grok-upstream.md`, `review-gemini-upstream.md` in the
  session scratchpad (ephemeral — the durable conclusions are in issues #1/#2).

## Operator Feedback

- **Use herdr panes for other TUIs**, not backgrounded `cursor-agent --print`.
  Vindicated: every herdr failure was visible and diagnosable; the `--print`
  failures were invisible.
- Wants the sqlite provider feature-rich upstream (1Password comparison) — and
  the evidence supports it.
- **Upstream acceptance of everything is not required.** What matters is that
  upstream keeps it *not hard* to fork and add the few things needed. This is
  the objective function; optimise for small, stable, low-conflict delta.
- Opus 5 / high effort, never Fast mode; never `-fast` model variants.

## Where We're Going

1. **THE NEXT ACTION** — see the top of this document.
2. Then: run the endpoint against the upstream conformance cases before wiring
   anything real.
3. In `secretspec-sqlite`: confirm `provider/sqlite.rs` even compiles against
   upstream `main` (**still unverified**; written against 0.19.1). Cheapest test
   of that whole track.
4. Remove the `pub use rusqlite` seam (`lib.rs`) — path-based API or a
   `SqliteConn` newtype; `capture_history` is
   `pub fn capture_history(conn: &Connection, operation: &str)`
   (`sqlite.rs:618`), so either is small.
5. Still live and independent: make `drift`/`doctor` verify the manifest chain
   at rest (`drift.rs:1036` is audit-only).
6. Watch for a maintainer reply on #362.
7. Standing: don't touch `PROMPT-REVIEW.md`, `PROMPT-SECREV.md`,
   `REVIEW_REPORT.md`, or branch `explore/pr-334-rust-first-spec` without
   asking. **DEFERRED, do not resurface:** cross-platform sudo/Linux port.
   Once upstream #377 resolves:
   `git worktree remove /Users/djbclark/src/ss-sigpipe && git branch -D fix/sigpipe-default-disposition`.

## Quick Start

```bash
# 1. READ FIRST — the whole plan, front-loaded:
gh issue view 2 --repo frdminc/sudo-secretspec
sed -n '1,60p' /Users/djbclark/src/sudo-secretspec/docs/design/upstream-ipc-v1-and-the-fork.md

# 2. The prototype worktree is already created:
cd /Users/djbclark/src/ss-ipc-proto && git log --oneline -1   # a393a27
sed -n '43,120p' secretspec-ipc/src/provider.rs               # the trait to implement
wc -l conformance/ipc/runner/src/bin/ipc-provider-endpoint-rust.rs  # 494, model on this

# 3. EVERY cargo command needs this env (system SQLite, not bundled):
export PKG_CONFIG_PATH=/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH \
       LIBRARY_PATH=/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH \
       CPATH=/opt/homebrew/opt/sqlite/include:$CPATH
# Baseline in sudo-secretspec: expect 229 passed / 0 failed.
cargo test -p sudo-secretspec-cli
# NEVER pipe cargo through `tail` without ${PIPESTATUS[0]} — it masks the exit code.
# First build will be slow: ~/.cargo/target-shared is empty (target/ was deleted).

# 4. Probe with the RIGHT sqlite. Bare `sqlite3` is the Android SDK build and lies.
/opt/homebrew/opt/sqlite/bin/sqlite3 --version   # expect 3.53.4
```

**Method rule this session paid for twice:** a `grep` result is not a behavioral
fact. Write the test, then *remove the fix* — if it still passes, the fix
changed nothing.

**NEVER print a retrieved secret value.** `V=$(sudo-secretspec get <NAME>
--reason '...')` then `echo ${#V}` only. Announce before any live-boundary
work — the installed broker is shared infrastructure other agents call
constantly.
