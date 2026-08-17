# Infinite secret backup/restore (queued feature)

Written 2026-08-17, on operator request, with `sudo-main` at `b97abf1`.

**Status: QUEUED, not designed, not started.** Explicitly sequenced *after* the
work already in flight: the `0.19.1-sudo.16` release, the `Spec`-shaped
manifest-edit reference implementation for upstream
[#370](https://github.com/cachix/secretspec/issues/370), and the comment on
upstream [PR #362](https://github.com/cachix/secretspec/pull/362). Do not start
this before those are done.

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

## Non-binding sketch (to be designed properly when the queue clears)

- History lives **inside the privilege boundary** (root-owned, e.g.
  `/var/db/sudo-secretspec/history/`), outside every caller's trust domain —
  a compromised or careless caller can no more delete history than read the
  vault.
- Every mutating verb (`set`, `delete`, `import`, `install`, and `restore`
  itself) preserves prior state before writing. The whole vault was 2852 bytes
  at the last restore, so "infinite" retention is plausible at face value.
- New verbs: list history, and restore a single key or the whole vault to a
  chosen point.
- Restores and history reads are audited in the protected ledger like any
  other privileged action.

## Open questions (decide at design time, not now)

- Per-key value history vs whole-file snapshots (or both).
- Whether restore requires operator authentication beyond existing sudoers
  rules — a restore overwrites current values, so it is itself a destructive
  write.
- Scope: the dotenv vault only, or also provider-backed values (keyring) —
  provider backends may not support enumeration.
- Retention/compaction policy if "infinite" ever becomes a size problem
  (secret *values* are tiny; audit-grade metadata may not be).
- Upstream posture: fork-only, or is there a shape upstream would want
  (relates to the provider `delete`/versioning surface from PR #354)?
