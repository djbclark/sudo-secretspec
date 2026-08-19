# Upstream IPC v1 (PR #362) and what it means for this fork

Written 2026-08-16, against `cachix/secretspec` PR
[#362](https://github.com/cachix/secretspec/pull/362) (`feat/ipc-v1`, open,
authored by the maintainer) and `sudo-main` at `0a0e250`.

This is the most consequential upstream thread for the fork, and it went
untracked for a full session: it was visible only as a one-line pointer in a
comment on #64. Recorded here so a future session does not have to rediscover
the analysis, and so the ledger entry has something to point at.

---

## STATUS 2026-08-18 — READ THIS FIRST

Verified against `upstream/feat/ipc-v1` at **`a393a27`** (the branch has moved
since this doc was written). Three things a future session needs before
anything else here:

**1. One protocol was renamed. The gate was not.**
The northbound protocol is now **`secretspec.resolver/1`**, not
`secretspec.client/1` — that rename *is* the branch tip commit `a393a27`
("name the northbound protocol after the resolver, add a blocking client").
Evidence: `secretspec-ipc/src/blocking.rs:1`, `lifecycle.rs:249`.
**`secretspec.provider/1` is unchanged** (`lifecycle.rs:95`), so the gate this
fork depends on is intact. The table below has been corrected.

**2. The "small shim" claim is now verified, not asserted.**
The endpoint-author API is two items in `secretspec-ipc/src/provider.rs`:
`pub trait ProviderHandler` (line 43) and `pub async fn serve_provider` (line
357). **Only `resolve_address` is mandatory**, plus `capabilities` and
`initialize`. Every other method — `get`, `set`, `delete`, `exists`,
`get_many`, `set_expiring`, `clear`, `check_writable`, `check_deletable`,
`describe_write_target` — defaults to
`RpcError::new(ErrorKind::CapabilityRequired)`. A first endpoint is three
methods. `SecretValue` already wraps `Zeroizing<String>` (`provider.rs:20-27`).

**3. The endpoint shape is exactly what privilege separation needs.**
Upstream's reference endpoint
(`conformance/ipc/runner/src/bin/ipc-provider-endpoint-rust.rs`, 494 lines)
serves over **stdio**:

```rust
serve_provider(tokio::io::stdin(), tokio::io::stdout(), handler, config)
```

A stdio child process means **`sudo` launching that process as the service user
*is* the privilege mechanism**. The calling user never opens the vault files;
secretspec speaks JSON-RPC to a process that can. No upstream patching, and no
privilege code upstream. Upstream also ships conformance cases to test against:
`conformance/ipc/cases/provider-{lifecycle,operations,errors,reconnect,session-isolation}.json`.

**4. The prototype exists and passes. The claim is no longer an assertion.**
Worktree `/Users/djbclark/src/ss-ipc-proto`, branch `proto/privileged-endpoint`,
commit `a204898`, crate `privileged-endpoint-proto/` (its own README carries the
detail). Built 2026-08-18 against `a393a27`.

- **No upstream patching, demonstrated mechanically.** The crate declares its
  own `[workspace]` table, so it is not a member of the upstream workspace and
  depends on `secretspec-ipc` by path through public API only. After a full
  build, test, and conformance run, `git diff` in the checkout is **empty** and
  `git status` shows exactly one untracked directory.
- **`provider.lifecycle` passes**, driven by upstream's own checked-in
  `conformance/ipc/cases/provider-lifecycle.json` through
  `ipc-provider-conformance-driver`. Transcript: `initialized`, `cancelled`,
  `deadline_exceeded`, `terminal`, `closed`.
- **Verified by mutation, not by a green result.** Replacing the `__BLOCK__`
  branch in `get` with `if false` fails the case with *"provider cancellation
  did not produce one cancelled terminal"*, so the pass depends on the behavior
  it claims to test.
- **"A first endpoint is three methods" holds.** `capabilities`, `initialize`,
  `resolve_address`, plus `get`. Framing, version negotiation, capability
  gating, per-request deadlines, cancellation, and shutdown are all
  `serve_provider`'s.

Four findings came out of it, recorded in full in the crate's README:

1. **Upstream's conformance suite cannot be built on macOS at all.**
   `libsecretspec-ipc/src/process_posix.c:237` calls `sigtimedwait` under a
   plain `#ifndef _WIN32`. That function is absent from every Darwin SDK header
   *and* from `libSystem` — a link failure as well as a compile error, not a
   strict-mode warning. A portable `sigpending` + `sigwait` fix is kept as
   `upstream-macos-sigtimedwait.patch` rather than applied, so the no-patching
   claim stays literally true. **Worth offering on #362** — same lane as #377.
2. **The client→broker text channel cannot distinguish "missing" from
   "denied".** The broker exits 1 for *has no value*, *is not resolved*, and a
   resolve error alike (`broker.rs:960-978`), and a `sudo -n` denial also
   surfaces as 1. The endpoint therefore collapses every non-zero exit to one
   opaque `OperationFailed`: mapping 1 to `Missing` would tell a caller a
   secret does not exist when it was actually denied. Consequence, accepted
   deliberately: the endpoint currently can never report a genuinely absent
   secret.
3. **The same channel loses a trailing newline** — the broker prints with
   `println!` (`broker.rs:962`), so exactly one `\n` is framing.
4. **Upstream bounds an address key at 4096 bytes and nothing else**
   (`Address::validate`, `protocol.rs:737`), so a caller may send `--help` or
   `-n` as a key. Nothing reaches a shell, but the fork's own `clap` would
   parse it. An endpoint author bridging to a CLI needs their own name guard.

Findings 2 and 3 are the sharpest argument yet for next action 3 below: on
`secretspec.provider/1` the failure is a structured `ErrorKind` and the value is
a byte-exact JSON string, so both defects *disappear* rather than being decoded.

Not yet done: wiring `--client` to the live vault end to end. It is implemented,
including killing the privileged child when a request is cancelled, but it has
not been run against the live boundary.

Tracking issue:
[frdminc/sudo-secretspec#2](https://github.com/frdminc/sudo-secretspec/issues/2).
Companion (upstreaming the sqlite provider):
[djbclark/secretspec-sqlite#1](https://github.com/djbclark/secretspec-sqlite/issues/1).

---

## What #362 actually is

"SecretSpec IPC v1 for 0.20+": two application protocols over one framed
JSON-RPC wire/session layer.

| Boundary | Protocol | Purpose |
|---|---|---|
| Application / SDK → SecretSpec broker | `secretspec.resolver/1` (renamed from `secretspec.client/1` in `a393a27`) | Resolve one exact declared name as a value or leased file |
| SecretSpec → external provider endpoint | `secretspec.provider/1` | Naming, reads, presence, writes, expiry, deletion, preflight, reflection |

Plus `secretspec broker --stdio`, broker-owned file leases, trusted
external-provider discovery, a Rust endpoint-author API, a pure-C11 client,
conformance suites, and the `secretspec-ffi` → `libsecretspec` rename.

## The load-bearing distinction: it is not a privilege boundary

Upstream's "broker" is a **private child process of the caller, in the caller's
own trust domain**. This is not an inference — the PR's own architecture doc
says so:

- "possession of the inherited pipe handles is the session authority"
  (`ipc-architecture.md:150-152`)
- no caller identity or delegation (`:191`)
- persistent socket transport explicitly deferred (`:163-167`)
- forwarding secret authority is listed as a **non-goal** (`:211-223`)

Decisively, upstream's `initialize` accepts a **caller-supplied** manifest —
even inline TOML — provider, and profile (`broker.rs:67-87`). That is precisely
what this fork's control plane exists to remove: the whole point of the
privilege boundary is that the caller does not get to name the manifest or the
provider.

The audit story differs the same way. Upstream's is fail-open, size-capped,
truncate-in-place JSONL — "Auditing never blocks secret access"
(`audit.rs:8-19`). This fork's is fail-closed and hash-chained in SQLite
(`sudo-secretspec-cli/src/audit.rs:29-31,510-533`).

So: **#362 subsumes the fork's transport-and-contract layer, and none of its
boundary.** The fork's client↔broker hop is argv-over-`sudo`
(`sudo-secretspec-cli/src/main.rs:9-17`) with no wire contract; upstream now has
a considerably better one. But nothing in #362 touches sudo policy, the
root-owned vault, `template-check`, or install/uninstall/doctor/rollback.

## It is the `exec://` mechanism #345 asked for

`secretspec.provider/1` matches our closed #345 proposal almost clause for
clause: JSON-over-stdio provider endpoints, with `sudo-secretspec` named as the
flagship reference implementation.

Shape of a fork endpoint under it:

- Register scheme `sudosecretspec.json` in the **root-owned system**
  directory (`/Library/Application Support/SecretSpec/providers.d`,
  `external.rs:473-483`; System scope requires uid 0 and a
  non-group/world-writable path, `external.rs:317-326`).
- The executable is a thin **unprivileged shim** implementing `ProviderHandler`
  that internally crosses the existing NOPASSWD `sudo` path to
  `/usr/local/libexec/sudo-secretspec`.
- Capability negotiation lets it advertise exactly the mediated surface and
  nothing more.

**This requires no upstream patching.** `ProviderHandler`/`ProviderApplication`
are public API in `secretspec-ipc`, and registration is data. The fork's
installer already knows how to write root-owned artifacts. The payoff is real:
stock `secretspec` and every SDK would reach secrets *through* the boundary,
which the fork has never had a way to offer.

## What it does not change

**#370 and #371 are unaffected.** No `spec.rs`, `codegen.rs`, or manifest-edit
changes across #362's 216 files; the client protocol is five methods
(`client.openrpc.json:8-37`) with no manifest-shape reflection, and
`provider.reflect` describes the *endpoint*, not the manifest. The fork's
`codegen-schema` shape debt is **not** retired by the 0.20 IPC work. Both issues
remain open and remain the right venue — but expect them to queue behind this
PR.

## Strategic read

This makes the fork's architecture **more** defensible, not less. Upstream built
the plumbing and explicitly fenced off the fork's territory — no caller
identity, no privileged transport, forwarding secret authority a stated
non-goal, and even a nod that "privileged integrations should construct an
allowlisted environment" (`ipc-architecture.md:158-160`). Read plainly, #362
*grants* the out-of-tree mechanism #345 requested.

Two risks worth naming rather than dismissing:

1. **Perception.** "SecretSpec has a broker, with audit and reasons" will sound
   to the ecosystem like it already covers the local-AI-agent threat model. The
   fork has to state the trust-domain distinction loudly and early, including in
   its own README, which currently does not address it at all.
2. **Convergence.** If upstream later specifies the deferred socket transport
   *with peer credentials*, it genuinely enters this fork's space. The counter is
   to be the incumbent reference privileged deployment shaping that spec — which
   argues for engaging now rather than after 0.20 ships.

## Next actions

1. Comment on #362 — supportive, announcing intent to ship the first
   out-of-tree **privileged** `secretspec.provider/1` endpoint, closing the loop
   on #345. **CORRECTED 2026-08-17** (verified against PR head `337950c` and
   this machine, macOS 26.6.1 — the original claim below was false and must
   not be posted): `/Library/Application Support` is `root:admin 0755`, no
   ACL, **not** group-writable. The real gaps, verified in
   `external.rs` `check_file_security`/`check_parent_security`
   (lines ~307-360 as of `337950c`):
   - Both check only `metadata.mode() & 0o022` via `std::fs::metadata`, which
     is **blind to macOS ACLs** — a third-party installer can grant
     `add_file`/`write` to a group via an extended ACL while POSIX mode reads
     clean. The Windows path in the same file already validates ACLs
     (`path_acl_is_trusted`); the unix path has no equivalent.
   - `std::fs::metadata` **follows symlinks**, so a symlinked path component
     anywhere in the chain is validated at its resolved target, not the
     literal registered path.
   - Trust genuinely does stop at the **immediate parent** — everything above
     it is unchecked. Soundness of the macOS default chain is assumed, not
     verified, and one loosened ancestor (by mode or by ACL) upstream of the
     parent defeats both checks below it.
   This fork's `drift.rs` `check_ancestor_chain` walks the resolved chain to
   `/`, rejects non-root ownership, group/world-writable mode, **any extended
   ACL**, and symlinked components, at every level — not just the parent.
   Offer the corrected points as review feedback plus conformance cases
   (ancestor-writable-by-ACL, symlinked-component), not a demand.
   **POSTED 2026-08-17**: https://github.com/cachix/secretspec/pull/362#issuecomment-5316998149
   (text preserved at `docs/design/pr362-comment.md`). Watch for a maintainer
   reply.
2. ~~Prototype the provider endpoint against `feat/ipc-v1` in a scratch
   worktree.~~ **DONE 2026-08-18** — see STATUS item 4 above. It is a small
   shim, and it now proves the "no upstream patching" claim rather than
   asserting it. Remaining: wire `--client` to the live vault end to end, and
   offer the macOS `sigtimedwait` fix upstream on #362.
3. Plan the 0.20 rebase: adopt `secretspec-ipc` types for the fork's own
   client↔broker hop over time, keeping `sudo` as the authority mechanism.
4. Leave #370/#371 as filed; offer the PRs once #362 settles.
5. Position the README against the distinction above.

## Provenance

Produced by a Fable 5 analysis pass reading both codebases directly (upstream
files from `feat/ipc-v1`, fork files from `sudo-main`), with file:line citations
verified in the source rather than taken from the PR description. Claims about
upstream line numbers are as of the PR's state on 2026-08-16 and will drift as
it is revised — re-check before quoting any of them upstream.
