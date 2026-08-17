# Infinite secret backup/restore (queued feature)

Written 2026-08-17, on operator request, with `sudo-main` at `b97abf1`.

**Status: PARTLY BUILT, THEN REDIRECTED 2026-08-17.** Steps 1–4 of the original
plan shipped (`5efb816`, `bd17934`, `dff3830`). The operator then raised a
better architecture, and the storage half of this design was **superseded the
same day** — see
[Redirection: storage moves into a `sqlite://` provider](#redirection-storage-moves-into-a-sqlite-provider)
below, which is the current plan and takes precedence over the storage
decisions in [The design](#the-design-decided-2026-08-17-with-sudo-main-at-22e5178).

Read the redirection section **first**. The design section below it is retained
because its reasoning about chaining, destroy/tombstones, authorisation and
scope still holds and transfers; only the question of *where the values live*
changed.

## What the operator asked for

"Infinite secret backup/restore": every change to the managed secrets is
retained, and any prior state is restorable — by the boundary itself, without
external tooling.

## Why — the incident that motivates it

On 2026-08-17 a plain `sudo-secretspec install` (without `--adopt-existing`)
truncated `/var/db/sudo-secretspec/.env` to 0 bytes, destroying every stored
value. The install bug is fixed (`ca2b6ab`), but recovery exposed the deeper
gap: the boundary keeps **no history of its own**. Restoring required Arq —
and only Arq's GUI at that: `arq_restore` cannot parse Arq 7.47's tree format,
and CLI reads of the CloudStorage backup set fail with EDEADLK regardless of
Full Disk Access. Full record:
`docs/handoffs/HANDOFF_standalone-b2db_vault-restored-truncation-fixed_2026-08-17_90ab.md`.

With built-in history, that incident would have been
`sudo-secretspec restore --to <timestamp>` instead of a multi-hour,
operator-driven GUI recovery. Off-machine backup (Arq) remains necessary for
disk loss; this feature removes the dependency on it for logical loss.

## Search for prior discussion FIRST (operator directive, 2026-08-17)

Design work on this feature must begin by searching for existing discussion —
upstream issues/PRs and the web generally — and must repeat the search even
though a first pass was done, because the queue ahead of this is long and
upstream moves.

First pass, 2026-08-17:

- **Upstream (`cachix/secretspec`) issues and PRs**: searched `backup`,
  `restore`, `versioning`, `snapshot`, `rollback`, `history`, `undo`,
  `revert`. **No dedicated thread exists.** Every hit was an incidental
  keyword match (#64 out-of-tree providers, our own #370, #202 cache
  clearing, #176 file-shaped secrets, #156 property tests).
- **Web**: no secretspec-specific discussion of backup/restore found. What
  exists is prior art in other managers, worth studying at design time:
  - [Vault KV v2 version history](https://oneuptime.com/blog/post/2026-02-09-vault-secret-versioning-rollback/view)
    — every write creates a version; distinct delete / destroy / undelete
    verbs; any historical version retrievable.
  - [AWS Secrets Manager restore-secret](https://docs.aws.amazon.com/secretsmanager/latest/userguide/manage_restore-secret.html)
    — deletion is a scheduled recovery window, not an immediate destroy.
  - [Azure Key Vault backup/restore](https://learn.microsoft.com/en-za/dotnet/api/azure.security.keyvault.secrets.secretclient.restoresecretbackupasync?view=azure-dotnet)
    — per-secret encrypted backup blobs restorable through the API.
  - Also in the same family: 1Password item history, `pass` (git history is
    the vault history), [Velero for Kubernetes secrets](https://oneuptime.com/blog/post/2026-02-09-secrets-backup-restore-velero/view).
- Before building fork-only, ask whether upstream wants a shape of this —
  the delete/versioning surface touches the provider trait (PR #354 lineage).

Second pass, 2026-08-17 (re-run per the directive above, with the queue ahead
now clear through `0.19.1-sudo.18`):

- **Upstream issues and PRs**: re-searched the same eight terms. **Still no
  dedicated thread.** The hits are unchanged in kind and all incidental —
  #370 (ours), #64 (out-of-tree providers via gRPC), #202 (orphaned cache
  entries), #176 (file-shaped secrets), #156 (property tests), #11 (dynamic
  secrets), #339 (a provider-registry test). Nothing to join or defer to.
- **Web**: still no secretspec-specific discussion of vault backup/restore.
  The prior-art list above stands as written; no newer local-versioning
  design surfaced that changes the candidate set.

### What the second pass *did* find: the cache is a second value store

Not a backup/restore thread, but a direct input this design has to absorb —
`secretspec/src/cache.rs` is in-tree, with `max_age` expiry, `cache clear`,
and writes audited as `cache_refresh` rather than `set`. Consequences:

- **A restore that only rewinds the vault is not a restore.** A cached copy of
  a *newer* value can outlive the rewind and still resolve, silently defeating
  it. Restore must invalidate affected cache entries, and the invalidation has
  to be part of the same audited operation, not a follow-up the operator is
  told to run.
- **`cache_refresh` is useful precedent.** Upstream already models a
  value-touching mutation that is deliberately *not* `set`. `restore` wants
  the same treatment — a distinct verb in the ledger, so a rewind never reads
  back as an ordinary write.
- **Not urgent, but do not discover it later.** Verified 2026-08-17: the
  privileged CLI does not reference the cache at all, so the boundary serves
  uncached today and nothing is currently at risk. This becomes live the
  moment caching is enabled inside the boundary, which is exactly when it
  would be most expensive to notice.

## Operator constraints and backend candidates (2026-08-17)

Constraint from the operator: the history store must **not be a network
dependency**, and should ideally be a backend that *already implements
versioning* rather than versioning written from scratch.

No existing secretspec provider gives local versioning out of the box: the
versioned providers (Vault KV v2, AWS Secrets Manager, Azure, Keeper, …) are
all network services; keyring/Keychain has no history API; dotenv and file
keep none. Local candidates to evaluate at design time:

- **git as the history layer** (the `pass` model, applied to our vault) —
  wrap the existing root-owned dotenv vault in a local git repo inside the
  boundary. Infinite history, diffs, restore = checkout, integrity via
  `git fsck`; no remote ever needed. Minimal change to the current vault
  shape, and git is already on every machine this runs on.
- **`pass` / `gopass` providers** (both already in-tree) — GPG-encrypted
  files in a local git store; gopass auto-commits every change. Gets
  versioning "for free" but brings a GPG toolchain dependency the boundary
  doesn't otherwise need.
- **`kdbx` provider** (in-tree) — the KeePass format keeps per-entry history
  inside one encrypted local file. But history depth is capped/configurable,
  not infinite, and whether the Rust kdbx stack preserves history on write
  needs verification.
- **SQLite append-only history table** — `rusqlite` is already a dependency
  of the privileged CLI crate (the audit ledger), so this is versioning we'd
  write ourselves, but as trivial schema on a substrate the boundary already
  trusts, and it could share the ledger's integrity design.

## Redirection: storage moves into a `sqlite://` provider

Decided 2026-08-17, after steps 1–4 had shipped. **This section is the current
plan.**

### What changed

The original design put value history in a boundary-owned sidecar
(`broker-history.sqlite3`) beside a dotenv vault that remained the source of
truth. The operator asked whether the whole stack should instead sit on a
**normal-plugin-interface `sqlite://` provider** — storage as an ordinary
secretspec provider like `keyring` or `onepassword`, rather than a private
store bolted beside one.

It should. Three things decided it:

1. **The provider is boundary-unaware, and that is what makes it reusable.**
   Point it at a `0600` file owned by the service user and an unprivileged
   caller gets `EACCES` while the mediated broker works; point it at a file
   owned by the running user and it is an ordinary local provider with no
   privilege story at all. *Identical code in both cases.* The privilege model
   stays entirely external to it.
2. **It fills a gap this document already identified.** "No existing secretspec
   provider gives local versioning out of the box" — every versioned provider
   upstream ships (Vault KV v2, AWS Secrets Manager, Azure, Keeper) is a
   network service, and the operator's binding constraint was no network
   dependency. A local versioned provider is the missing row in upstream's own
   matrix, so it is a far better upstream candidate than a fork-only sidecar
   could ever be.
3. **Switching now wastes less than finishing first.** Six further steps built
   on the sidecar would each need reworking. The recommendation to finish the
   original plan first was momentum rather than analysis, and is withdrawn.

### Two corrections to the original design's reasoning

- **The trait surface is not a prerequisite.** This document previously treated
  upstream's unsettled provider `delete`/versioning surface (the PR #354
  lineage) as a gate. It is not. A `sqlite://` provider can be upstreamed as a
  plain `get`/`set`/`delete` provider that *retains* history internally, with
  history and restore reached through concrete methods the fork's broker calls.
  A generic trait surface becomes a later improvement, not a blocker.
- **A provider store would not have survived the incident either.** The
  argument that "a provider cannot cover the install truncation" was true but
  not a point of distinction: had `install` blindly `File::create`'d a
  `vault.sqlite3`, it would have destroyed the values *and* their history
  together. Install-time capture is required under **both** designs. It counts
  against neither.

### What does not move into the provider

Smaller than first claimed, but real, and it is what remains of the boundary
piece:

1. **Declarations.** `secretspec.toml` is not provider data. Manifest history
   stays boundary-level — and that is the file the `undeclare` gap was about.
2. **Install-time capture.** Outside the provider under either design, per the
   correction above.
3. **Authorisation.** Forward-safe-for-agents versus operator-only `--force`,
   `--all` and `destroy` is boundary policy wherever the bytes live.

### What survives from steps 1–4

Nothing is reverted; all three commits stay.

- `5efb816` — the `undeclare` rollback-copy fix is independent of storage
  entirely and stands as shipped.
- `bd17934` / `dff3830` — the chain design, the digest-not-bytes rule that
  makes destroy auditable, the `destroyed_by` schema `CHECK`, the shared
  hardened opener, and the "failed archive keeps the copies" contract all
  transfer into the provider. `capture` / `list` / `restore` were deliberately
  built as the seam the value half moves behind, so the surface does not
  change shape when it does.
- The verified fact that the live vault's dotenv round-trips byte-exactly
  (39 names, 2858 bytes, identical digest) is what makes the **migration**
  safe to attempt: the values can be read out through the same grammar that
  wrote them.

### Companion change: drop the broker from root to the service user

Independent of storage, and worth taking separately. The broker requires root
today (`ALL=(root)` in the installed sudoers policy, `install.rs:825`), but
that requirement looks **largely incidental** — root is there to `chown` things
*to* the service user:

- `require_boundary` reads a `0700` directory owned by `_sudo_secretspec` —
  being that user suffices
- `Mutation::begin` chowns its copies to `uid:gid` — unnecessary if the process
  already is that user
- `audit::open_connection` chowns the ledger `if geteuid() == 0` — already
  correct as that user
- the config is `0444`; the vault is service-user-owned

sudo's `Runas_Spec` already supports a non-root target, so `ALL=(_sudo_secretspec)`
would drop the privileged identity from root to a service user while keeping
**sudoers as the policy engine**. `install` stays root; it is already a
separate privileged step.

*Rejected: a setuid binary.* It reaches the same privilege reduction by
hand-writing the hardening `sudo` performs for us — environment sanitisation,
`closefrom`, saved-set-uid ordering, supplementary-group dropping, argv, fd,
umask and cwd hygiene — in a binary guarding real credentials. It also moves
authorisation out of an externally auditable policy file and into `getuid()`
checks in our own code, which is the wrong direction for this system: the
operator-only-versus-agent-callable split for `restore` and `destroy` is
naturally expressed as sudoers rules. Estimated at 3–4× the cost of the whole
remaining plan, with the failure mode surfacing late.

### Design points to settle early

- **History retention must be opt-in** (`sqlite://path?history=…` or similar).
  An upstream user pointing the provider at their own store would be surprised
  to find every prior value retained by default. The fork turns it on
  explicitly.
- **Migration of the live vault** — 39 secrets, currently in
  `/var/db/sudo-secretspec/.env`, on a host where that file has already been
  truncated once. Needs a rehearsal and a rollback, not a one-shot import.

### Honest caveat on the upstream justification

This decision leans on upstream acceptance, so the current evidence belongs
next to it: **upstream is not merging our work right now.** #362, #372, #373
and #374 are all open with zero maintainer engagement, and #374 has not had CI
run at all — which djbclark cannot trigger, having pull-only access. #354 and
#355 merged earlier, so the channel is not dead. Build the better upstream
candidate, but do not price the plan on acceptance arriving soon.

### Cost

Roughly **560–750k tokens** across the provider (~250–350k), the runas
reduction (~60–100k), the reduced boundary piece (~150–200k), and migration
plus release and live verification (~100k). That is **more** than the
~250–350k needed to finish the original plan. The justification is
architecture and a reusable artifact, not cost — recorded plainly so nobody
later reads "switching wastes less" as "switching is cheaper".

## The design (decided 2026-08-17, with `sudo-main` at `22e5178`)

> **Storage decisions in this section are superseded** by the redirection
> above. Its reasoning about chaining, destroy/tombstones, authorisation,
> scope, retention and upstream posture still holds and transfers.

Status of this section: **designed, not implemented.** The open questions
below it are answered, not pending.

### The spine: the capture path already exists

`broker.rs:235` `Mutation` already copies **both** `secretspec.toml` and
`.env` before every `source-set` / `source-add` / `source-delete`, into
`.secretspec.{toml,env}.rollback.<transaction>` inside the vault, chowned to
the service identity and refused on collision. The transaction is the same
`uuid::Uuid` the hash-chained audit ledger records for that operation
(`broker.rs:492`).

On success, `Mutation::commit` **deletes** those copies (`broker.rs:306`).

So this feature is mostly *stop discarding a snapshot the boundary already
takes correctly*. It is not a new capture mechanism, and it does not need
one. What is genuinely new is durable storage, a read surface, and a restore
path — plus closing two gaps the review found (below).

Two gaps found while grounding this design, both worth fixing regardless:

1. **`source-undeclare` is not wrapped in `Mutation`.** The `matches!` at
   `broker.rs:529` lists only `set`/`add`/`delete`, but `source-undeclare`
   performs a bare `std::fs::write(&manifest, updated)` at `broker.rs:866`.
   A failed write there corrupts the manifest with no rollback copy, unlike
   its three siblings. It is the largest file in the vault (9735 bytes live).
2. **`install` has never had vault coverage.** `rollback.rs:4` states
   *"Never touches vault secret values"* and `run` prints
   `"runtime vault preserved"`. Artifact rollback and secret history are
   disjoint by design — which is precisely the seam the 2026-08-17
   truncation fell through.

### Backend: SQLite beside the audit ledger

`/var/db/sudo-secretspec/broker-history.sqlite3`, root-owned `0600`, beside
`broker-audit.sqlite3`, hash-chained with the same design as `audit.rs`
(`previous_hash` / `entry_hash`, verified end to end by a `history-verify`
counterpart to `audit-verify`).

*Chosen over a git-wrapped vault*, which was the favoured candidate above.
Git gives history, diffs and `git fsck` for free, but it gets them by running
`git` **from a root process**, which inherits `/etc/gitconfig`, hooks,
`.gitattributes` filters and `core.fsmonitor` — every one a code-execution
vector. The vault directory is service-user-writable (`_sudo_secretspec`,
`0700`), so a repo placed there would be outright root RCE; a root-owned
history directory elsewhere closes that specific hole but still makes a
privilege boundary depend on an external binary's behaviour at uid 0. The
operator's constraint was "no network dependency, ideally a backend that
already implements versioning" — the *purpose* of that constraint is fewer
bugs and no remote, and on this substrate the versioning we would write is a
single table, while the surface git imports is not small. `rusqlite` is
already a dependency of this crate for the ledger.

*Rejected: `pass`/`gopass`* (GPG toolchain the boundary does not otherwise
need) and *`kdbx`* (history depth capped, not infinite).

### Storage shape: per-name values, whole-file manifest

This **revises** the earlier working assumption that whole-file snapshots
would be the stored truth for both files. Whole-file cannot support
destroy-by-name: removing one name's value from a `.env` blob changes that
blob's digest and breaks the hash chain, and NULLing the whole blob would
destroy the other 45 names' history with it.

- **`.env` → per-name value rows.** A dotenv is flat `KEY=VALUE`; decomposing
  it loses nothing. Each captured transaction stores one row per name, plus
  the digest of the whole original file.
- **`secretspec.toml` → whole-file blob.** Structured TOML where formatting,
  comment placement and ordering all matter, and which holds *declarations,
  not values* — so destroying a value never needs to touch it.

Consistency is preserved by storing `file_sha256` for the reconstructed
`.env`: a restore reassembles the rows and must reproduce that digest exactly
or it refuses. Capture applies the same guard in reverse — if reassembling
what it just parsed does not reproduce the bytes on disk, capture fails
closed rather than storing a snapshot that cannot be restored.

```sql
CREATE TABLE entries (            -- one row per captured transaction
  sequence      INTEGER PRIMARY KEY AUTOINCREMENT,
  transaction   TEXT NOT NULL,    -- the SAME uuid as the audit event
  timestamp_ns  INTEGER NOT NULL,
  operation     TEXT NOT NULL,    -- source-set | source-add | source-delete | ...
  manifest_blob BLOB NOT NULL,    -- secretspec.toml, whole file
  manifest_sha  TEXT NOT NULL,
  env_file_sha  TEXT NOT NULL,    -- digest of the whole pre-mutation .env
  previous_hash TEXT NOT NULL,
  entry_hash    TEXT NOT NULL     -- covers the metadata above, incl. digests
);

CREATE TABLE values_ (            -- one row per name per entry
  sequence      INTEGER NOT NULL REFERENCES entries(sequence),
  name          TEXT NOT NULL,
  value_blob    BLOB,             -- NULL once destroyed
  value_sha256  TEXT NOT NULL,    -- RETAINED after destroy
  destroyed_by  INTEGER REFERENCES entries(sequence),
  PRIMARY KEY (sequence, name)
);
```

`entry_hash` covers the *original* digests, never the mutable blob. That is
what makes destruction auditable: a destroyed row still proves that bytes
with digest X existed at that point and were destroyed by entry N, without
retaining the value. Same property Vault KV v2's `destroy` needs, reached the
same way — chain the metadata, not the data.

### Verb surface and the authorisation model

Restore is **agent-callable but forward-safe only**. Agents may recover state;
they may not overwrite live state.

| Verb | Who | Rule |
|---|---|---|
| `history [--name NAME]` | agent | Lists transactions, names touched, timestamps. **Never prints values.** |
| `restore --name NAME --to <txn>` | agent | Permitted **only if `NAME` currently holds no value and carries no tombstone.** Otherwise refuses and names the operator-only form. |
| `restore --name NAME --to <txn> --force` | operator | Overwrites a live value. Separate sudoers verb, excluded from the agent rule. |
| `restore --all --to <txn>` | operator | Whole-vault rewind. Necessarily overwrites; never agent-callable. |
| `destroy --name NAME` | operator | Deletes *and* tombstones every historical value for `NAME`. The rotation-safe path. |

The dynamic half of that rule cannot live in sudoers, which knows nothing
about whether a value currently exists — so sudoers separates
`source-restore` from `source-restore-force`, and the broker enforces the
"currently holds no value" condition at runtime. This is the same shape as
the guard already at `broker.rs:841`, which refuses to undeclare a name that
still holds a value and tells the caller to `delete` it first.

### `delete` stays soft; `destroy` is the rotation-safe verb

Forward-safe restore has one hole if `delete` is the only removal verb: a
credential rotated *because it leaked* is deleted, and an agent may then
restore it — reviving the leak. Closing it does not require making the agent
path useless, because the prior art already recorded above solves exactly
this:

- **`delete` is soft.** Snapshots retained, agent-recoverable. This is the
  ordinary case and stays ergonomic.
- **`destroy` is permanent and operator-only.** It is what a leak response
  uses. Mirrors Vault KV v2's `delete` vs `destroy` and AWS Secrets Manager's
  recovery window vs force-delete-without-recovery.

### Auditing, and the cache precondition

Every one of `history`, `restore` and `destroy` is **its own operation name**
in the protected ledger, following the `cache_refresh` precedent identified
in the second prior-art pass: a rewind must never read back as an ordinary
`set`. They go through the existing attempt/terminal funnel (`broker.rs:528`)
with a mandatory `--reason` digest, like every other source verb.

**Cache invalidation is a precondition, not a follow-up.** Verified again
2026-08-17: the privileged CLI does not reference `secretspec/src/cache.rs`
at all, so nothing is at risk today. The design's requirement is therefore
enforced as a *guard against a future change*: restore must invalidate
affected cache entries inside the same audited operation, and the
implementation carries a test that fails if the privileged CLI gains a
reference to the cache without restore handling it. Discovering this after
caching is enabled inside the boundary is exactly when it is most expensive.

### Retention: genuinely infinite, observed rather than assumed

Live sizes, 2026-08-17: `.env` 2858 bytes, `secretspec.toml` 9735 bytes —
~12.6 KB per full snapshot, and the manifest blob is identical across the
long runs of transactions that do not touch declarations. The ledger holds
1268 events to date across the fork's whole life, of which mutations are a
fraction. No compaction, no retention window, no expiry.

What the design adds instead is a `doctor` line reporting history entry count
and file size, so growth is *observed*. If it ever becomes a problem, that is
a decision made against real numbers rather than a policy guessed at now.

### Scope, stated as a limit

History covers **the vault dotenv and the runtime manifest only** — the
boundary's own store. It does **not** cover provider-backed values (keyring
and friends), which have no enumeration or history API; a partial guarantee
that reads as complete is worse than a stated limit. The CLI help, the
`history` output header, and `sudo-secretspec/README.md` all say so
explicitly.

### Upstream posture: fork-only

Two independent prior-art passes (above) found no upstream thread on
backup/restore/versioning across eight search terms. More decisively, the
design's central property — history living inside a privilege boundary,
outside every caller's trust domain — has no counterpart upstream, which has
no privileged broker at all. There is no shape here to contribute without
first contributing the boundary.

Revisit only if upstream settles a provider `delete`/versioning surface
(the PR #354 lineage), which would give the storage half somewhere to land.

## Implementation plan (current, post-redirection)

Steps 1–4 of the superseded plan below already shipped. What follows replaces
its steps 5–10.

1. **`sqlite://` provider in the `secretspec` crate.** A plain
   `get`/`set`/`delete` provider following the ~35 sibling conventions:
   module in `secretspec/src/provider/sqlite.rs`, `#[provider]` registration,
   URI parsing, profile-aware storage, and cases in
   `secretspec/src/provider/tests.rs`. No boundary awareness whatsoever.
2. **Internal versioning, opt-in.** Retention behind a URI option, with the
   chain design carried over from `history.rs`: entries hash-chained over
   metadata and value *digests*, `destroyed_by` outside the hash and kept
   honest by the schema `CHECK`.
3. **Provider documentation**, all seven locations in `CLAUDE.md`'s checklist
   plus the credentials catalog and `npm --prefix docs run
   check:provider-credentials`. Version-label everything unreleased.
4. **Reduced boundary piece**: manifest history and install-time capture, which
   is what `history.rs` becomes once values live in the provider.
5. **Restore and destroy verbs**, with the authorisation split unchanged from
   the superseded design — forward-safe for agents, `--force` / `--all` /
   `destroy` operator-only behind a separate sudoers verb.
6. **Migration** of the live 39-secret vault off dotenv, rehearsed with a
   rollback before it is run for real.
7. **Runas reduction** (`ALL=(root)` → `ALL=(_sudo_secretspec)`), independent
   of all of the above and takeable at any point.
8. **Docs, sudoers, CHANGELOG, release, postinstall verification.**

Still owed from steps 1–4 regardless: `CHANGELOG.md` has **no** entry for any
of `5efb816`, `bd17934`, `dff3830` — those commits are code-only.

## Superseded implementation plan

Retained for the reasoning; steps 1–4 shipped, steps 5–10 are replaced above.

1. **Wrap `source-undeclare` in `Mutation`** (`broker.rs:529`). One-line
   `matches!` change plus a test; independent of everything below.
2. **`history.rs`**: schema, chained append, `history-verify`. Modelled
   directly on `audit.rs`'s chain code, which is already tested against
   tampering.
3. **Divert `Mutation::commit`** from `remove_file` to an archiving append.
   `restore()` keeps deleting, since a rolled-back mutation is state that
   never took effect. This is the point at which history starts accruing.
4. **`history` verb**, agent-callable, values never printed.
5. **`restore`**, forward-safe path first, then `--force` and `--all` behind
   the separate sudoers verb.
6. **`destroy`**, with the tombstone write and the retained-digest property
   verified by `history-verify` over destroyed rows.
7. **`doctor`** history size/count line.
8. **Docs**: `sudo-secretspec/README.md`, `AI-GUIDANCE.md`,
   `skills/sudo-secretspec/SKILL.md`, and the stated scope limit in each.

Steps 1–3 alone deliver the incident's actual lesson: after them, the bytes
exist. Steps 4–6 are what make them reachable without a database client.
