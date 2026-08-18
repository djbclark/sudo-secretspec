---
schema_version: 1
handoff_id: 74fa
parent_handoff_ids: [c590]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: c31896949c1b068dd07f4ff8d3ff1f6efe8fd679
created_at: 2026-08-17T21:39:26-04:00
writer: claude-code
---

# Handoff — all remaining work, distributed across the available AI fleet

**This document is different from the other 25 in this directory.** Those are
single-session recovery documents for the next Claude Code session. This one is
a *distribution* document: it hands the entire remaining backlog to whichever
AI picks up each piece, and says which AI that should be and at what effort
level. Read "The Fleet" and "Assignments" together — the assignment table is
meaningless without the capacity table underneath it.

Every agent taking any item from this document must read **"Rules that bind
every agent"** first. Those rules are not style preferences; two of them exist
because this host has already lost a live secret vault once.

---

## The Goal

### The project goal (unchanged since 2026-08-17 15:37)

`sudo-secretspec` is a privilege-separated secrets broker: a root-owned vault
at `/var/db/sudo-secretspec/` holding a `secretspec.toml` manifest and a `.env`
value store, reachable by unprivileged agents only through a narrow sudoers-
gated CLI. The feature under construction is **infinite secret backup and
restore** — every mutation of a secret retains its prior state, tamper-evidently,
forever, with restore available to agents in a forward-safe form and destructive
verbs reserved to the operator.

The design is settled and committed (`docs/design/infinite-secret-backup-restore.md`).
It was redirected mid-implementation (`c124c0b`) away from a boundary-private
history store and into a **normal `secretspec` provider plugin** — `sqlite://` —
so that versioning is a property of the storage backend, not of the privileged
CLI. That redirection is why `sudo-secretspec-cli/src/history.rs` (1091 lines,
built and passing) is now *source material to port from* rather than the
shipping implementation.

Eight steps. **Step 1 shipped.** Seven remain, plus independent debt.

### This session's goal

The operator asked for exactly one thing: *"Create a handoff document to give
all current and future work to other AIs. Suggest which AIs to use based on
`aiuse --json` plus your knowledge of what each of those has available, include
thinking level etc. Include context and goals not only steps, but include steps
as well."*

No code was written this session. The session resumed the chain, verified state,
surveyed fleet capacity, and produced this document.

---

## Where We Are

Branch `sudo-main` at `c318969`, **tree clean, in sync with `origin/sudo-main`**.

```
/Users/djbclark/src/sudo-secretspec  c318969 [sudo-main]           <- primary
/Users/djbclark/src/ss-370           b3637e2 [spec-manifest-edit]  <- keep until #374 resolves
```

**The installed boundary is `0.19.1-sudo.18`. Nothing in this tree is released.**
Three commits of shipped code (`5efb816`, `bd17934`, `dff3830`) plus the new
provider (`239d1ab`) are all sitting unreleased. Verify with
`sudo-secretspec --version` — do not assume.

### Recent commits (the state any agent inherits)

| SHA | What |
|---|---|
| `c318969` | handoff `c590` (previous session's Tier 2) |
| `239d1ab` | **step 1**: `secretspec/src/provider/sqlite.rs`, plain get/set/delete provider, 541 lines, 14 tests |
| `f214155` | handoff `1bd6` |
| `c124c0b` | **the redirection**: design moved from boundary-private history to a `sqlite://` provider |
| `dff3830` | `Mutation::commit` archives instead of deleting — history now accrues on every mutation |
| `bd17934` | the hash-chained history store (`sudo-secretspec-cli/src/history.rs`) |
| `5efb816` | `source-undeclare` got the rollback copy its three siblings already had |

### Code surface, measured

```
  541  secretspec/src/provider/sqlite.rs        <- step 1's output, step 2's target
 1091  sudo-secretspec-cli/src/history.rs       <- port FROM this; step 4 reduces it
 1550  sudo-secretspec-cli/src/broker.rs        <- Mutation lives at :235
 1717  sudo-secretspec-cli/src/install.rs       <- sudoers written at :825
  201  sudo-secretspec-cli/src/rollback.rs      <- :4 states it never covers vault values
```

### Upstream, verified 2026-08-17 21:39 ET (not assumed)

| # | Kind | State | Maintainer engagement |
|---|---|---|---|
| 374 | PR — format-preserving single-declaration edits on `Spec` | OPEN | **none** (1 comment, djbclark's own dogfooding report) |
| 373 | PR — `check` writes report to stdout | OPEN | **none** (0 comments) |
| 372 | Issue — `check` report goes to stderr, breaks piping | OPEN | **none** (0 comments) |
| 362 | PR — versioned client and provider IPC | OPEN | **none** (2 comments: a Cloudflare bot, and djbclark) |

`djbclark` has pull-only access to `cachix/secretspec`, so blocked CI runs
cannot be approved from this side. `upstream/main` has moved `dfa4b10` →
`35791a2` (PR #368 merged); the `ss-370` worktree is still on `dfa4b10` and
wants a rebase before any review round.

---

## Context & Goals — what each agent needs to understand before touching anything

An agent that reads only the step list will do the wrong thing. The four facts
below are what make this codebase's decisions make sense.

**1. The privilege boundary is the whole point.** Unprivileged coding agents
(including whichever AI reads this) can *use* secrets through the broker but
must never be able to read the vault directly, silently overwrite it, or destroy
history. Every design choice — the forward-safe restore, the separate
operator-only `destroy` verb, the hash chain — exists to keep an agent's worst
case bounded. When in doubt about an authorization split, the answer is "the
agent can move history forward, only the operator can erase it."

**2. Confidentiality here is filesystem permissions, not cryptography.** The
`sqlite://` provider stores values in a `0600` file. That is deliberate and
consistent with the rest of the boundary — it is the reason `kdbx` was rejected
(a master password merely relocates the secret onto another file behind the same
permissions). Do not "improve" this by adding an encryption layer with a key
that lives next to the data.

**3. The hash chain binds digests, never bytes.** Each history entry hashes the
entry metadata and the *digest* of the value, not the value itself. That is what
lets a later `destroy` null out a value blob while the entry still proves what
was destroyed and by whom. `destroyed_by` sits **outside** the hash and is kept
honest by a schema `CHECK ((value_blob IS NULL) = (destroyed_by IS NOT NULL))`.
This design is already built and tested in `history.rs`. **Port it. Do not
re-derive it.** Re-deriving it is how the tamper-evidence property gets
accidentally dropped.

**4. This host has already lost the vault once.** On 2026-08-17,
`/var/db/sudo-secretspec/.env` was truncated during an install. It currently
holds **39 real secrets in use**. `rollback.rs:4` says plainly that rollback has
never covered vault values — that is the exact seam the incident fell through,
and closing it is step 4. Until step 4 and step 5 land, there is no undo for a
bad write to that file.

---

## The Fleet — measured capacity, 2026-08-17 21:37 ET

From `aiuse --json` (snapshot `2026-08-18T013719Z`), cross-checked against
`cswap list`. Reset times converted to local ET.

| Surface | CLI | Capacity right now | Verdict |
|---|---|---|---|
| **Claude Code** — `cswap` slot 1 | `claude` | 5h **0% used** (fresh). Weekly **67% used**, resets Aug 22 04:59 | **Best Claude slot.** Untouched 5h window |
| **Claude Code** — `cswap` slot 2 (active) | `claude` | 5h **3% used**, resets Aug 18 02:20. Weekly **72% used**, resets Aug 21 06:00. Fable weekly **82% used** | Usable, but it has carried this whole project. Spare it |
| **Antigravity** (Google AI Pro) | IDE / `gemini` | Gemini 5h **~0%** (just reset). Gemini weekly **43% used**, resets Aug 20 13:09. **Claude/GPT 5h 0% used**, Claude/GPT weekly **9% used**, resets Aug 20 13:21 | **The biggest untapped pool.** `aiuse` explicitly flags 91% of Claude/GPT weekly as likely to go unused |
| **Cursor Pro** | `cursor-agent` | included **11% used**, Auto **11%**, other models **8%** — all reset Sep 2. On-demand budget untouched | Large headroom, ~2 weeks of runway |
| **GitHub Copilot** (Individual Pro) | `copilot` | premium requests **29% used**, resets Sep 1 | Large headroom; flagged as likely to go unused |
| **ClinePass** | `cline` | 5h **50% used**, resets Aug 17 22:24 (~45 min). Weekly **20%**, monthly **10%** | Half a 5h window now, effectively full again within the hour |
| **OpenCode Go** | `opencode` | 5h **0%**, weekly **0%**, monthly **1.9% used** | Essentially untouched |
| **Grok** | — | **15% used**, resets Aug 20 02:41 | Headroom, no local agent CLI wired up |
| **OpenRouter** | `aider` | small positive prepaid balance | Small but real; pay-per-token, no window |
| **Codex** (`plus`) | `codex` | weekly **100% EXHAUSTED**, 0 reset-credits, resets **Aug 20 05:00** | ❌ **Unusable until Aug 20 05:00 ET.** Do not route work here |
| **OpenCode Zen** | `opencode` | balance slightly **negative** | ❌ Overdrawn |
| **DeepSeek** | — | prepaid balance **exhausted** | ❌ Exhausted |

**A caveat about `aiuse` alerts, from this machine's own operating notes:** the
`kind: "conserve"` alerts are *pace projections*, not measurements. `aiuse`
currently emits high-urgency "pace yourself" alerts for both Claude accounts and
for Cursor — for Cursor it is doing so with **89% of the budget still unspent**,
which shows the projection firing on a fast recent hour rather than on real
scarcity. Use the raw `used_percent` and `resets_at` numbers in the table above
for go/no-go decisions. The one alert worth acting on is the Codex exhaustion,
which is `used_percent: 100.0` — a measurement, not a projection.

**Honesty about what these ratings are worth:** the capacity numbers are
measured. The *suitability* judgments below are reasoning from general knowledge
of each tool plus the shape of each task — **all 25 prior handoffs in this
directory were written by `claude-code`, so no other agent has ever touched this
repository.** None of them are known-good here. That is a concrete reason to
start any new agent on a low-stakes item (the CHANGELOG debt) before trusting it
with the hash chain or the vault.

### Claude Code effort levels, since the operator asked for thinking level

Claude Code exposes effort as `low | medium | high | xhigh | max` and models
Opus 5 / Sonnet 5 / Haiku 4.5 / Fable 5. Two standing constraints:

- **Do not use `-fast` model variants or the `/fast` toggle** on this work.
  There is no deadline pressure here, and the plain tier is the operator's
  standing preference.
- Fable's weekly is at **82% used** — the tightest Claude budget on the machine.
  Do not route bulk work to Fable.

The previous session already produced a costed split for this exact backlog and
the operator did not dispute it: **Sonnet-high for templated provider plumbing,
Opus-high for the hash-chain half, Opus-xhigh specifically for the runas
privilege audit** (not for the mechanical edit that follows it). That split is
carried forward below rather than re-derived.

---

## Assignments

Ordered by dependency, not by priority. Steps 2 → 3 → 4 → 5 → 6 are a chain;
step 7 and the debt items are independent and takeable in parallel *by a
different agent in a different worktree*.

| # | Work | Recommended AI | Effort / model | Why this one |
|---|---|---|---|---|
| **2** | Opt-in versioning in `sqlite.rs` | **Claude Code, `cswap` slot 1** | **Opus 5, high** | Highest-stakes correctness in the queue; needs 541 + 1091 lines held at once and a security invariant preserved verbatim. Fresh 5h window on that slot |
| **3** | Provider docs, 7 locations | **Cursor** (`cursor-agent`); fallback **Copilot CLI** | agent mode, default model | Mechanical, checklist-driven, machine-verifiable by `npm --prefix docs run check:provider-credentials`. Wastes Claude weekly budget. Multi-file coordinated edits are Cursor's strength |
| **4** | Reduce `history.rs` to manifest history + install capture | **Antigravity** (Gemini 3 Pro), or Claude Sonnet 5 high | high | Large-context deletion refactor across 1091 lines; Antigravity's Gemini weekly is only 43% used and its 5h just reset |
| **5** | `restore` / `destroy` verbs + authorization split | **Claude Code** | **Opus 5, high** | Authorization design on a privilege boundary. Getting the agent/operator split wrong is the failure mode the whole feature exists to prevent |
| **6** | Migrate the live 39-secret vault | **Claude Code + operator watching** | **Opus 5, xhigh** | ⚠️ Real credentials on a host that has already been truncated once. **Do not delegate. Do not run unattended.** Rehearse with a rollback first |
| **7a** | Runas reduction — **the audit** | **Claude Code** | **Opus 5, xhigh** | `require_root()` is defined **4×** with different error types across 5 call sites; 82 `chown`/`geteuid`/`getuid`/`Uid::` hits to verify the "root is largely incidental" claim against. This is judgment, not editing |
| **7b** | Runas reduction — **the edit** | Claude Sonnet 5, or Cursor | high / default | Once 7a says which call sites genuinely need root, the sudoers change at `install.rs:825` is mechanical |
| **8** | Docs, sudoers, CHANGELOG, release, postinstall | **Claude Code + operator** | Sonnet 5 high; **operator present for the release** | The `/release` skill lives in Claude Code. See the release hazard under "Rules" — a failed local brew step leaves a *fully published* release |
| **D1** | CHANGELOG debt for `5efb816`, `bd17934`, `dff3830` | **OpenCode Go** or **ClinePass** | default | Small, self-contained, verifiable. **Use this to smoke-test any new agent** before trusting it with anything else |
| **D2** | Rebase `ss-370` onto `35791a2`; watch #362/#372/#373/#374 | **Copilot CLI** | default | Low compute, GitHub-shaped. Copilot's premium budget is 71% unused and expires Sep 1 |
| **D3** | `declarations` → `Option<PathBuf>` (deferred but **decided**) | Claude Sonnet 5, when scheduled | high | Config-schema migration, not a one-field change: `declarations` is also the install-time seed, a required CLI arg, a `0444` artifact and a drift field. Shape in `docs/design/template-check-resync.md` §"Decision, 2026-08-17" |

**Do not route anything to Codex before Aug 20 05:00 ET** — its weekly quota is
measured at 100% with zero reset credits. **Do not route anything to OpenCode
Zen or DeepSeek** — both are at or below zero balance.

**Parallelism advice:** steps 2 and 7a are the two genuinely independent
high-value items. Running them concurrently — 2 on Claude mit.edu, 7a on
Antigravity or the gmail slot — is the best use of two fresh 5h windows. Steps
3 and D1/D2 can run alongside on the non-Claude surfaces without touching the
same files. **Anything concurrent must use a separate git worktree**; two agents
committing to `sudo-main` in the same checkout will collide.

---

## The steps, in detail

### Step 2 — THE NEXT ACTION: opt-in versioning in the `sqlite://` provider

**Goal.** Make the provider retain history, without changing its behaviour for
callers who did not ask for it.

**Context.** `sqlite.rs` today is a flat `item TEXT PRIMARY KEY` store — one row
per `{project}/{profile}/{key}`. It currently **rejects query parameters
outright** via a `has_query()` check, deliberately, because accepting-and-
ignoring an unimplemented flag would be worse than erroring. Step 2 is where
that guard changes on purpose.

**Steps.**
1. Read `docs/design/infinite-secret-backup-restore.md` §"Implementation plan
   (current, post-redirection)" step 2 — first, before any code.
2. Read `sudo-secretspec-cli/src/history.rs` schema and chain code. **Port it
   verbatim.** Entries hash-chained over metadata and value *digests*;
   `destroyed_by` outside the hash; the schema `CHECK` enforcing it.
3. Decide the URI parameter name (`sqlite://path?history=…` is a placeholder —
   the name is genuinely undecided and choosing it is part of this step). Relax
   `has_query()` to accept exactly that parameter and still reject unknown ones.
4. Match the existing SQLite conventions in this repo rather than inventing a
   third style: `STRICT` tables, `PRAGMA journal_mode=DELETE`,
   `synchronous=FULL`, `trusted_schema=OFF` (checked against `audit.rs` and
   `history.rs`).
5. Tests in `secretspec/src/provider/tests.rs`; keep the existing 14 green.

**Done when.** `cargo test -p secretspec --lib provider::sqlite` passes, the
full `provider::` module still shows ~794 passed with only the 21 known `sops`
failures, clippy has zero warnings in `sqlite.rs`, and `cargo fmt` is clean.

### Step 3 — provider documentation, seven locations

**Goal.** The `sqlite://` provider appears everywhere providers are listed, so
none of the listings drift.

**Context.** `CLAUDE.md` §"Adding Provider Documentation" enumerates the
locations because they *do* drift when one is missed. There is a checker for
part of it. The provider is unreleased, so **every entry needs a `(0.20+)`
version label** — the docs site publishes from `main` and users land directly on
provider pages, so a changelog note is not sufficient.

**Steps.** Work the `CLAUDE.md` checklist literally: the provider page under
`docs/src/content/docs/providers/`, the sidebar *and* the `starlightLlmsTxt`
providers sentence in `docs/astro.config.ts`, the table in
`concepts/providers.mdx`, the section *and* the Security Considerations row in
`reference/providers.md`, `providerMetadata` *and* the hero mini-terminal in
`docs/src/pages/index.astro`, the `config init` output in `quick-start.mdx`, and
the bullet list *and* `config init` output in `README.md`. Then the credentials
catalog if the provider registers credential names.

**Done when.** `npm --prefix docs run check:provider-credentials` passes and
every new mention carries `(0.20+)`.

### Step 4 — reduce `history.rs` to manifest history + install capture

**Goal.** Once values live in the provider, the boundary-private store keeps
only what the provider cannot: manifest history and the install-time capture.

**Context.** This closes gap 2 — `rollback.rs:4` states plainly that rollback
has never covered vault values, which is the seam the truncation incident fell
through. Note the behaviour `dff3830` established and must be preserved: a
failed archive does **not** fail the operation. The mutation has already
committed; a failed archive leaves the rollback copies on disk, so the degraded
state is the pre-history behaviour rather than a loss.

### Step 5 — `restore` and `destroy` verbs

**Goal.** Rewind a secret, with an authorization split that cannot be abused by
an agent.

**Context.** The split is settled and must not be renegotiated: **restore is
agent-callable but forward-safe only** (it moves history forward, creating a new
entry, never erasing one). `--force`, `--all`, and `destroy` are **operator-only,
behind a separate sudoers verb**. This derives from Vault KV v2 and AWS Secrets
Manager prior art mapped onto this fork's verbs, and it closes the
revive-a-leaked-credential hole a naive restore would open.

**A trap already found:** `secretspec/src/cache.rs` is a **second store of
secret values** with `max_age` expiry. A restore that only rewinds the vault can
be silently defeated by a cached newer value, so **cache invalidation belongs
inside the audited restore operation**. The privileged CLI does not use the
cache today, so nothing is at risk yet — but step 5 is where it would become
one. Upstream's `cache_refresh` audit event is precedent for restore being its
own audited verb.

### Step 6 — migrate the live vault ⚠️

**Goal.** Move 39 real secrets from `/var/db/sudo-secretspec/.env` onto the
versioned provider.

**This is the one item in this document that must not be delegated to an
unattended agent, and must not be run by an AI the operator is not watching.**
The file has been truncated once already on this host. Rehearse the whole
migration with a rollback first. Do **not** one-shot it. Check the vault
before and after (`46 found / 0 missing / 7 optional` was the last verified
reading at release .18).

### Step 7 — runas reduction (independent, takeable now)

**Goal.** `sudoers` `ALL=(root)` → `ALL=(_sudo_secretspec)` at
`install.rs:825`, so the broker stops running as full root.

**Context and the trap.** The claim "root is largely incidental" is **not
verified**. `require_root()` is defined **four separate times with different
error types** — `rollback.rs:30`, `broker.rs:105`, `uninstall.rs:168`,
`install.rs:277` — across 5 call sites, and there are **82 hits** for
`chown`/`geteuid`/`getuid`/`Uid::` across the crate. `install` and `uninstall`
almost certainly still need real root. Audit every one of those before changing
any of them. This is why 7a is scoped at xhigh: the audit is the hard part, the
edit afterwards is not.

Also rejected already, do not revisit: **setuid**. Sudo's `Runas_Spec` gives the
same privilege reduction without hand-writing sudo's hardening or moving
authorization out of a policy file and into our code.

### Step 8 — wrap-up and release

Docs, sudoers, CHANGELOG, release, postinstall verification, in that order,
after everything above lands. See the release hazard in the rules below.

---

## Rules that bind every agent

Violating any of these has already caused a real incident or a real cleanup.

1. **All downstream work commits directly to `sudo-main` and pushes.** No
   feature branches, no PRs. If asked to work on "main" or "master", that means
   `sudo-main`. `main` is the upstream mirror only — do not develop on it and
   do not merge it into `sudo-main` without being asked; it carries post-0.19.1
   upstream work meant for a future release.
2. **Never touch `/var/db/sudo-secretspec/.env` or `secretspec.toml` directly.**
   Go through the broker. That file holds 39 live secrets and has been
   truncated once.
3. **Build with system SQLite, not bundled.** On macOS, bundled
   `libsqlite3-sys` hangs. Export before building:
   `PKG_CONFIG_PATH`, `LIBRARY_PATH`, `CPATH` pointed at
   `/opt/homebrew/opt/sqlite` (exact lines in Quick Start; rationale in
   `FORK-AI.md`).
4. **21 `sops` test failures are the expected baseline** — the `sops` CLI is not
   installed on this host. Do not "fix" them. Ten clippy warnings in
   `sops/*.rs` and `cli/mod.rs` are likewise pre-existing. A change is clean
   when it adds none of its own.
5. **`CHANGELOG.md` gets one user-facing entry per Rust change**, under the
   existing Unreleased section. Never create a new release subsection. Leave out
   development and testing notes.
6. **Version-label everything unreleased** as `(0.20+)` at each point of use.
   The docs site publishes from `main`, ahead of the latest release, and users
   land directly on provider and reference pages.
7. **Release hazard:** the release helper **publishes before the Homebrew step**.
   A failed local brew step therefore leaves a fully published release behind
   it. Check free disk before releasing — `target/debug` has reached 58G on this
   host. If `just release` exits non-zero *after* publishing, finish it by hand
   per the justfile's RESUME RULES; do not re-run it.
8. **Do not delete branch `explore/pr-334-rust-first-spec`.** Commit `bf0b25c`
   is linked by SHA from a public comment on upstream #357.
9. **Do not resurface the cross-platform sudo/Linux port**
   (`docs/design/privilege-boundary-and-packaging.md:487`). Deferred by explicit
   operator instruction.
10. **Concurrent agents need separate worktrees.** Two agents committing to
    `sudo-main` in the same checkout will collide.
11. **Handoff protocol is Claude-Code-specific.** The Tier 1 pointer/canonical
    log under `~/.local/state/handoffs/chains/standalone-b2db/` is written by a
    Claude Code skill (`session_log.py`) — its `blockers` field must be a JSON
    **list of strings**, not a bare string. **Non-Claude agents should not try to
    write it.** Instead, leave a plain markdown report in `docs/handoffs/` and
    say in your final message that Tier 1 still needs updating; the next Claude
    Code session will fold it in.

---

## What We Tried

### This session

Only one thing was tried and it worked: the reader protocol resolved cleanly.
`cwd` (`/Users/djbclark/src/sudo-secretspec`) did not match either existing
pointer directory name (`privilege-boundary`, `ss-370`), but both pointers
redirect to the same chain, so the ambiguity was moot. Both workspaces' recorded
`head_sha` matched actual `HEAD`; no `precompact-*` sidecars existed; nothing was
stale.

Upstream state was **checked rather than inherited** — the Tier 1 log listed
"#374/#362/#372/#373" without kinds, and `gh pr view 372` fails because **372 is
an issue, not a PR**. The corrected kinds are in the table above.

### Carried forward — the failed and rejected approaches that cost real time

These are the expensive ones to rediscover. Do not re-attempt them.

- **`kdbx` as the versioned backend — rejected.** It is the only local database
  with native per-entry history, but it needs an interactive unlock secret
  (master password or keyfile), which is a mismatch for a headless runas path
  and merely relocates the secret behind the same filesystem permissions the
  boundary already relies on. KeePass history also has no hash chain or
  tamper-evidence, so those properties would have to be rebuilt on a foreign
  binary format not designed for them.
- **Third-party / overlooked providers — none exist.** `secretspec` has no
  community provider ecosystem; it is a young single-repo project. `origin/main`
  (the upstream mirror) was checked for unmerged provider work — nothing.
- **`redb` / `CrepeDB` — rejected.** `redb` has MVCC and savepoints but no
  history API, so it is the same amount of work on a less-proven dependency.
  `CrepeDB` is obscure and unaudited — wrong tradeoff for a store guarding real
  credentials.
- **`pass` / `gopass` — rejected.** Both shell out to their CLIs with zero
  history handling, and both need `gpg-agent` or a passphrase: the same headless
  unlock mismatch as `kdbx`.
- **setuid for privilege reduction — rejected** in favour of sudo's
  `Runas_Spec`. Same reduction, without hand-writing sudo's hardening or moving
  authorization out of a policy file into our code.
- **Whole-file blobs for both vault files — abandoned mid-design.** They cannot
  support destroy-by-name without breaking the hash chain. `.env` is therefore
  stored as **per-name rows plus a whole-file digest** for exact reassembly.
- **An ordering bug worth knowing about** (found and fixed in `239d1ab`):
  `get`/`delete`/`get_many` checked file existence *before* resolving the
  address, so an invalid native coordinate was misreported as "not found"
  instead of erroring — but only while the database did not yet exist. If step 2
  restructures those paths, keep the regression test
  `unsupported_native_coordinate_is_rejected_even_before_the_database_exists`.

---

## Key Decisions

### Made this session

- **This document lives in `docs/handoffs/` like the other 25**, not in a new
  location, and carries the same chain tag and lineage. A distribution document
  that falls outside the chain would be invisible to the next `/resume`.
- **Capacity read from raw `used_percent`, not from `aiuse` alerts.** The
  `kind: "conserve"` alerts are pace projections; the Cursor alert fires with
  89% of the budget unspent, which demonstrates the failure mode. Only the Codex
  exhaustion (a measurement at 100%) was treated as a hard gate.
- **Claude reserved for the four judgment-heavy items** (2, 5, 6, 7a) and
  everything mechanical routed elsewhere. Both Claude weeklies are past two
  thirds spent while Antigravity, Cursor, Copilot and OpenCode Go are barely
  touched and several expire unused.
- **`cswap` slot 1 recommended over the active slot 2** for step 2:
  its 5h window is completely untouched and its weekly is 5 points less spent.
- **New agents get the CHANGELOG debt as a smoke test**, because no agent other
  than Claude Code has ever touched this repository and the assignment ratings
  are reasoning, not measurement.
- **Rejected: recommending a single AI for everything.** The operator asked for
  a distribution; a single-vendor answer would also strand four budgets that
  expire without rolling over.
- **Rejected: assigning step 6 at all.** The live-vault migration is written up
  as explicitly non-delegable rather than given an owner.

### Carried forward from the design (do not renegotiate)

- SQLite beside the ledger, **over git-wrapped storage** — git at root would
  import hook/config/filter code-execution surface.
- Restore is **agent-callable but forward-safe only**; `destroy` is operator-only
  behind a separate sudoers verb.
- Scope is the **vault dotenv + manifest only**.
- `template-check` reports a retired source as a **distinct non-failing state**.
- The chain binds **digests, not bytes**; `destroyed_by` outside the hash.
- A **failed archive does not fail the operation**.
- Provider addressing is a flat opaque DB key — deliberately **without**
  `file.rs`'s `validate_convention_component` path-safety checks, since there is
  no filesystem-escape risk for a string that is never a path.
- **No in-process mutex** in the provider (unlike `kdbx`'s `KDBX_IO_LOCK`):
  SQLite's file locking plus a 5s `busy_timeout` suffices because every
  operation is one atomic statement.

---

## Evidence & Data

**Git.** `sudo-main` at `c31896949c1b068dd07f4ff8d3ff1f6efe8fd679`, `git status
-s` empty, `git diff --stat` empty, in sync with `origin/sudo-main`. Second
worktree `/Users/djbclark/src/ss-370` at `b3637e2` on `spec-manifest-edit`.

**Tier 1.** `session_log.py read` returned `"stale": false` for both workspaces,
`legacy_unmigrated: false`, no `precompact-*.md` sidecars. Canonical log:
`~/.local/state/handoffs/chains/standalone-b2db/SESSION_LOG.md`.

**Handoff history.** 26 files in `docs/handoffs/`, **all** `writer: claude-code`
— no other agent has worked this repository.

**Last measured test results** (from `239d1ab`, step 1, unchanged since — **no
tests were run this session**):
- `provider::sqlite` — **14 passed / 14**
- full `provider::` module — **794 passed, 21 failed, 4 ignored**; all 21 are
  `provider::sops::tests::*` failing with "The 'sops' CLI is not installed"
- `cargo clippy -p secretspec --lib --all-features` — 10 warnings, **zero in
  `sqlite.rs`**
- `cargo fmt -p secretspec -- --check` — clean
- `git diff --stat` for `239d1ab` — 6 files changed, 565 insertions(+), 1
  deletion(-)
- `sudo-secretspec-cli` history tests — 15 passing
  (`cargo test -p sudo-secretspec-cli --lib history`)

**Fleet snapshot.** `aiuse --json`, collected `2026-08-18T01:37:19Z`, saved to
`~/.cache/aiuse/snapshots/2026-08-18T013719.534324Z.json`; 12 accounts, **zero
collector errors**; Antigravity cross-check between CodexBar and OpenUsage.ai
reported `consistent` across 4 overlapping measurements. Claude cross-checks
returned `warning` only for the known account-matching limitation (single-session
tools vs multi-account `cswap`) plus a 10-point disagreement on Claude Fable —
which is why the Fable figure above is quoted as ~82% and not treated as precise.

**Installed agent CLIs, verified present:** `codex`, `gemini`, `cursor-agent`,
`cline`, `copilot`, `opencode`, `aider`, `herdr` (with `HERDR_ENV=1`).

**Upstream, verified via `gh` this session:** #374 OPEN / 1 comment (own), #373
OPEN / 0 comments, #372 OPEN **issue** / 0 comments, #362 OPEN / 2 comments (a
Cloudflare bot + own). Zero maintainer engagement across all four.

---

## Operator Feedback

- **"Create a handoff document to give all current and future work to other
  AIs."** — the deliverable is a distribution document, not another
  single-successor recovery document. "All current *and future* work" was read
  as the complete 7-step remainder plus the independent debt, not just the next
  action.
- **"Suggest which AIs to use based on `aiuse --json` plus your knowledge of what
  each of those has available."** — explicitly wanted the live quota data joined
  to capability judgment, not one or the other. Both halves are labelled above so
  the measured part and the reasoned part are distinguishable.
- **"include thinking level etc."** — wanted concrete effort settings, not just
  vendor names. This matches the previous session, where the same operator asked
  *"which claude model and thinking level are sufficient?"* and wanted a costed
  per-sub-task table rather than one blanket answer.
- **"Include context and goals not only steps, but include steps as well."** —
  an explicit correction of the default handoff shape, which leans procedural.
  Hence the "Context & Goals" section and the per-step goal/context/steps
  structure.
- **Standing pattern from prior sessions:** this operator interrupts mid-build to
  ask scope-narrowing questions rather than queueing them, asks for real research
  (web search, checking unmerged upstream) before accepting a recommendation, and
  delegates judgment calls with "continue as you think best" — which does **not**
  extend to unilaterally ending a session.

---

## Where We're Going

1. **THE NEXT ACTION — step 2**: opt-in versioning in
   `secretspec/src/provider/sqlite.rs`, porting the chain design verbatim from
   `sudo-secretspec-cli/src/history.rs`. **Claude Code, `cswap` slot 1
   (`cswap switch 1`), Opus 5 at high effort.** Read
   `docs/design/infinite-secret-backup-restore.md` §"Implementation plan
   (current, post-redirection)" step 2 before writing code.
2. **In parallel, on a separate worktree — step 7a**: the runas privilege audit.
   Claude Code Opus 5 at **xhigh**, or Antigravity/Gemini 3 Pro for the
   82-hit sweep with Claude for the judgment. Do not let it edit anything.
3. **In parallel, non-Claude — step 3**: provider docs in seven locations,
   `(0.20+)`-labelled. Cursor (`cursor-agent`), fallback Copilot CLI.
4. **In parallel, non-Claude — D1**: the CHANGELOG debt for `5efb816`,
   `bd17934`, `dff3830`. OpenCode Go or ClinePass. Use it to smoke-test the
   agent.
5. **Then step 4** (reduce `history.rs`), **then step 5** (restore/destroy),
   **then step 6** (live vault migration — operator present, rehearsed with a
   rollback, never one-shot), **then step 8** (release).
6. **D2, any time**: rebase `ss-370` (`dfa4b10` → `35791a2`) and keep watching
   #362 / #372 / #373 / #374. Copilot CLI.
7. **D3, when scheduled**: `declarations` → `Option<PathBuf>`; shape in
   `docs/design/template-check-resync.md` §"Decision, 2026-08-17".
8. **Codex becomes available again Aug 20 05:00 ET.** Nothing before then.

---

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3        # expect c318969 at tip, tree clean, pushed
git worktree list           # ss-370 must still be there until #374 resolves

# READ FIRST, in this order:
#   1. this file
#   2. the immediate predecessor:
#      docs/handoffs/HANDOFF_standalone-b2db_sqlite-provider-shipped_2026-08-17_c590.md
#   3. the exact next step:
sed -n '/Implementation plan (current, post-redirection)/,/Superseded implementation plan/p' \
  docs/design/infinite-secret-backup-restore.md

# What ships today, to build on:
sed -n '1,90p' secretspec/src/provider/sqlite.rs

# What transfers VERBATIM for step 2 (the chain design):
sed -n '1,240p' sudo-secretspec-cli/src/history.rs
cargo test -p sudo-secretspec-cli --lib history     # expect 15 passing

# REQUIRED before any build — system SQLite, not bundled (bundled hangs on macOS):
export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
export LIBRARY_PATH="/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH"
export CPATH="/opt/homebrew/opt/sqlite/include:$CPATH"

cargo test -p secretspec --lib provider::sqlite     # expect 14 passed
cargo test -p secretspec --lib provider::           # expect ~794 passed, 21 sops failures (EXPECTED)
cargo clippy -p secretspec --lib --all-features     # expect 10 pre-existing warnings, none in sqlite.rs
cargo fmt -p secretspec -- --check                  # expect clean

# Confirm the boundary — do NOT assume:
sudo-secretspec --version      # expect 0.19.1-sudo.18; nothing in this tree is released
sudo-secretspec doctor

# Re-check fleet capacity before choosing an agent (these numbers age fast):
aiuse --json | python3 -c "import json,sys; d=json.load(sys.stdin); [print(a['provider'], a.get('account'), [(w['label'], round(w['used_percent'],1)) for w in a['windows'] or []]) for a in d['snapshot']['accounts']]"
cswap list                      # Claude accounts specifically; cswap switch 1 -> mit.edu
```

**For a non-Claude agent starting cold:** read this file, then `CLAUDE.md`
(fork conventions, provider-doc checklist), then `FORK-AI.md` (build tricks),
then `sudo-secretspec/AI-GUIDANCE.md` (the automation contract for the broker
itself). Do not attempt to write the Tier 1 session log — leave a plain report
in `docs/handoffs/` and say Tier 1 still needs updating.
