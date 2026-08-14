# PROMPT-SECREV.md — Rust Privilege Boundary Security Review

## Target

**`sudo-secretspec`** — a single Rust binary acting as:
- **Public CLI** (`/usr/local/bin/sudo-secretspec`) — unprivileged operator
- **Privileged Broker** (`/usr/local/libexec/sudo-secretspec`) — root via NOPASSWD sudoers

The same binary dispatches via hidden `__broker` subcommand.

---

## Threat Model

| Asset | Threat | Mitigation |
|---|---|---|
| Vault secret values | Leak via audit/logs/CLI | Value-free audit: only SHA-256(reason) stored; no argv/env values |
| Vault integrity | Mutation by compromised operator | Mutation backed by `.rollback.<uuid>` + atomic commit/restore |
| Control plane hijack | Alternate manifest/provider/binary | Fixed paths in broker; `execute()` purges ambient `SECRETSPEC_*` env |
| Audit tampering | Ledger rewrite / deletion | Hash-chained events + atomic head; `verify()` integrity check |
| Privilege escalation | Broker runs as root | `require_root()` + fixed allowlist; no shell; `execvpe` only |
| Vault access by unauthorized UID | File perm/ACL bypass | `require_boundary()` validates ownership/mode/symlinks/ACL |

---

## Code to Audit

### `sudo-secretspec-cli/src/broker.rs`

**Entry**: `run()` → `dispatch()` → `execute()`

**Key checks**:
- [ ] `require_root()` uses `libc::geteuid() == 0`
- [ ] `require_boundary()` validates:
  - Vault exists, not symlink, mode 0700, owned by service user
  - Manifest/dotenv exist, not symlinks, mode 0600, correct owner
  - Binary/audit helper exist, not symlinks, root:wheel:755
- [ ] Allowlist: only `source-*` + `audit-verify` + `source-template-check`
- [ ] Empty reason rejected before any operation
- [ ] **Audit attempt** before ANY secret operation (fail-closed)
- [ ] `Mutation` struct:
  - Backs up manifest + dotenv to `.rollback.<uuid>`
  - `restore()` returns `bool` (not void!)
  - `commit()` removes backups
  - On failure: `restore()` called; if fails → `unknown` terminal
- [ ] `execute()`:
  - Purges `SECRETSPEC_PROVIDER/FILE/PROFILE/SCOPE` env
  - Pins dotenv provider: `dotenv:///var/db/.../.env`
  - Uses `secretspec::Secrets` library directly (no subprocess)
  - `ExportFormat::Json` → stdout → parsed → injected via `execvpe`
- [ ] **Terminal audit** also fail-closed (maps failure to exit 125 if unknown)

### `sudo-secretspec-cli/src/audit.rs`

- [ ] `AppendEventRequest` → `AuditEvent` with SHA-256(reason)
- [ ] Hash chain: `event_hash = H(previous_hash || event_without_hash)`
- [ ] Head table: `singleton=1` row with `(sequence, event_hash)`
- [ ] `verify()` checks chain + head + `PRAGMA integrity_check`
- [ ] `chown` ledger to vault owner when broker is root
- [ ] No secret values in any field (verify `reason_sha256` only)
- [ ] `open_connection()` validates dir metadata before opening DB

### `sudo-secretspec-cli/src/drift.rs`

- [ ] `Report.ok` ignores `LEGACY_VAULT_CLUTTER` / `PENDING_ROLLBACK`
- [ ] Vault allowlist includes `.local`, `.ansible`, `.cache` → `LEGACY_VAULT_CLUTTER` (advisory)
- [ ] Source manifest hash verification vs `MANIFEST.sha256`
- [ ] Sudoers syntax check via `visudo -c`
- [ ] Ancestor chain validation (root-owned, non-writable, no ACLs)
- [ ] Vault realpath validation (`/private/var/db/...`)

### `sudo-secretspec-cli/src/install.rs`

- [ ] `--adopt-existing`: **never chown/chmod vault** — only verify metadata
- [ ] Fresh install: creates service user/group (400-499), vault 0700
- [ ] Dry-run still requires root (metadata access)
- [ ] Source root derived from binary path (`libexec/../..`)

### `sudo-secretspec-cli/src/rollback.rs`

- [ ] Snapshot path validated under `/usr/local/libexec/sudo-secretspec-rollback-*`
- [ ] `MANIFEST.sha256` verified before any restore
- [ ] Restores **only** installed artifacts (bin/libexec/share/config/sudoers)
- [ ] **Never touches vault**
- [ ] `visudo -c` after restore

### `sudo-secretspec-cli/src/main.rs`

- [ ] `run` → `execvpe` (no shell)
- [ ] `--reason` required for all lifecycle ops
- [ ] `--reason` SHA-256 hashed before crossing broker boundary
- [ ] Install/rollback/doctor elevate via `sudo` (interactive) not `-n`

### `sudo-secretspec-cli/src/config.rs`

- [ ] `deny_unknown_fields` on TOML
- [ ] Path traversal rejection (`..` in managed paths)
- [ ] Service identity pattern: `^_[a-z_][a-z0-9_]{0,30}$` / `^_?[a-z_][a-z0-9_]{0,30}$`

### `sudo-secretspec-cli/Cargo.toml`

- [ ] `rusqlite = { version = "0.31" }` **no `bundled`** (system SQLite via Homebrew)
- [ ] `secretspec = { path = "../secretspec", version = "0.19.1-djbclark.1", default-features = false }`

---

## Packaging Security

### `packaging/homebrew/sudo-secretspec.rb`

- [ ] Installs `secretspec` + `sudo-secretspec-cli` from source
- [ ] **No** `sudo`, **no** user/group creation, **no** sudoers edit, **no** `/var/db` mutation
- [ ] Caveat instructs explicit `sudo-secretspec install`

### `packaging/release.py`

- [ ] Tag validation: `v0.19.1-djbclark.1`
- [ ] SHA-256 of tarball → formula rewrite
- [ ] Homebrew tap sync + `brew test` + version readback
- [ ] Readback: repo/tag/release/fork lineage verified

---

## macOS-Specific Checks

- [ ] `/var` → `/private/var` alias handled (canonicalize + validate `/private` chain)
- [ ] `rusqlite` **no `bundled`** — uses Homebrew SQLite (`PKG_CONFIG_PATH` etc.)
- [ ] Touch ID / `sudo` for install/rollback (interactive), `-n` for broker
- [ ] System SQLite 3.53.4+ available via Homebrew
- [ ] Service user `_name` pattern (hidden, no shell, uid 400-499)

---

## Test Coverage to Verify

```bash
cargo test -p sudo-secretspec-cli
# 58 tests: config(3), drift(8), install_rollback(2), cli(3), audit(21 lib + 19 integration)
```

---

## Files to Read

| Path | Why |
|---|---|
| `sudo-secretspec-cli/src/broker.rs` | Core privilege boundary |
| `sudo-secretspec-cli/src/audit.rs` | Value-free audit ledger |
| `sudo-secretspec-cli/src/drift.rs` | Non-repairing drift checker |
| `sudo-secretspec-cli/src/install.rs` | Adopt-existing + fresh install |
| `sudo-secretspec-cli/src/rollback.rs` | Artifact restore |
| `sudo-secretspec-cli/src/audit.rs` | Fail-closed SQLite audit |
| `sudo-secretspec-cli/src/config.rs` | Typed TOML validation |
| `sudo-secretspec-cli/src/main.rs` | Public CLI + elevation |
| `sudo-secretspec-cli/src/rollback.rs` | Artifact restore |
| `sudo-secretspec-cli/Cargo.toml` | Deps (no bundled SQLite) |
| `packaging/homebrew/sudo-secretspec.rb` | Formula (no privileged install) |
| `packaging/release.py` | Release automation |

---

## Review Output

Post findings as PR comments on https://github.com/djbclark/sudo-secretspec/pull/1

Tag:
- `���� SECURITY` — exploitable / invariant violation
- `���� ADVISORY` — hardening / defense-in-depth
- `���� OK` — invariant holds

---

*Reference: `FORK-AI.md` (Rust build caveats), `AI-GUIDANCE.md` (AI rules), `CLAUDE.md` (fork notes).*