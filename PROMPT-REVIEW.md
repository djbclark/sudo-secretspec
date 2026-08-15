# PROMPT-REVIEW.md — Complete Code Review Request for `frdminc/sudo-secretspec`

## Scope

This is a **downstream fork** of `cachix/secretspec` adding a privilege-separated companion
`sudo-secretspec` for macOS. Review all **new/changed** code in this fork against the
upstream `cachix/secretspec` 0.19.1 baseline.

**PR**: https://github.com/frdminc/sudo-secretspec/pull/1  
**Base**: `sudo-main` (downstream release branch)  
**Head**: `feature/sudo-privilege-boundary`  
**Version**: `0.19.1-sudo.9` (`v0.19.1-sudo.9`)

---

## What to Review

### 1. New Rust crate: `sudo-secretspec-cli`

**Path**: `sudo-secretspec-cli/`  
**Binary**: `sudo-secretspec` (single binary, dual role: public CLI + privileged broker via hidden `__broker` subcommand)

| Module | Purpose | Key Review Points |
|---|---|---|
| `main.rs` | Public CLI parser, short-install UX, broker dispatch | - Short install UX with auto-detect/prompt<br>- `execvpe` for `run` (no shell)<br>- `--reason` required for all ops<br>- `exec` elevation for install/rollback/doctor |
| `config.rs` | Typed TOML config (`deny_unknown_fields`) | - Rejects `..` path traversal<br>- Validates service identity (`_name` pattern)<br>- `direct_child` ensures single `/var/db` child |
| `audit.rs` | Fail-closed SQLite audit ledger | - Hash-chained events + atomic head<br>- Value-free: only SHA-256(reason), no argv/secrets<br>- `chown` ledger to vault owner when broker is root<br>- `verify()` integrity check via lib |
| `broker.rs` | Root-owned privilege boundary | - `require_root()` + `require_boundary()`<br>- Fixed ops allowlist (`source-*`, `audit-verify`)<br>- Pin dotenv provider to vault (`dotenv:///var/db/.../.env`)<br>- Mutation rollback (`.rollback.<uuid>` backup/commit/restore)<br>- **Fail-closed**: audit attempt/terminal must succeed |
| `drift.rs` | Metadata-only, non-repairing drift check | - Validates installed artifact hashes vs `MANIFEST.sha256`<br>- Vault allowlist (`.local`, `.ansible`, `.cache` → advisory `LEGACY_VAULT_CLUTTER`)<br>- `LEGACY_VAULT_CLUTTER` / `PENDING_ROLLBACK` → advisory only<br>- `Report.ok` ignores advisory codes |
| `install.rs` | Adopt-existing + fresh install | - Dry-run still requires root (metadata access)<br>- Adopt-existing: **no chown/chmod** of vault<br>- Fresh: creates `_service_user` + vault 0700<br>- Source-root/layout autodetection from binary path |
| `rollback.rs` | Artifact restore from snapshot | - Only restores installed binaries/config/sudoers<br>- **Never touches vault**<br>- Verifies `MANIFEST.sha256` before restore |

### 2. Configuration & Packaging

| File | Review Focus |
|---|---|
| `sudo-secretspec-cli/Cargo.toml` | `rusqlite = { version = "0.31" }` (no `bundled` — uses system SQLite via Homebrew) |
| `packaging/release.py` | Downstream release: tag `v0.19.1-sudo.9`, GitHub Release, Homebrew formula rewrite |
| `packaging/homebrew/sudo-secretspec.rb` | Formula installs `secretspec` + `sudo-secretspec-cli` companion; **no sudo/users/sudoers** |
| `sudo-secretspec.conf.example` | 9-key TOML: engine, audit, vault, vault_realpath, declarations, service_user/group |
| `.github/workflows/sudo-release.yml` | CI: cargo test, ruff, shellcheck, formula style, tag check |

### 3. Documentation (review for accuracy)

- `sudo-secretspec/README.md` — companion quick-start
- `README.downstream.md` — fork overview + branch policy
- `FORK-AI.md` — AI time-savers (system SQLite, registry cache, macOS path aliases, etc.)
- `sudo-secretspec/AI-GUIDANCE.md` — AI automation rules
- `CHANGELOG.md` — Unreleased entries

---

## Threat Model & Security Invariants

| Invariant | Where Enforced |
|---|---|
| No caller-selected manifest/provider/profile | `broker.rs` fixed paths, `execute()` removes ambient env |
| Value-free audit | `audit.rs` hashes reason, stores only hash + basename |
| Fail-closed audit | `broker.rs` returns error if `append_event`/`verify` fails |
| No secret values in audit/CLI/logs | `audit.rs` only hashes reason; CLI never logs values |
| Vault protected | `require_boundary()` checks ownership/mode/symlinks |
| Mutation atomic + rollback | `Mutation` struct in `broker.rs` |
| Vault never mutated by watchdog | `drift.rs` `Report.ok` ignores advisory codes |

---

## How to Test

```bash
# Build
export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
export LIBRARY_PATH="/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH"
cargo test -p sudo-secretspec-cli   # 152 tests
cargo build -p sudo-secretspec-cli --release

# Live install (Touch ID)
~/bin/sudo-secretspec install --adopt-existing

# Doctor
sudo -n /usr/local/libexec/sudo-secretspec doctor

# Smoke
/usr/local/bin/sudo-secretspec check --reason "smoke"
```

---

## Files Changed (vs upstream 0.19.1)

See PR diff. Key new files:
- `sudo-secretspec-cli/` (entire crate)
- `sudo-secretspec/` (README, AI-GUIDANCE, conf.example, retired.toml)
- `packaging/` (release, Homebrew, tests)
- `FORK-AI.md`, `README.downstream.md`, `CHANGELOG.md`, `CLAUDE.md` updates

---

## Skills / Context for Reviewer

- **Rust**: See `~/src/agent-guidance/rust/AGENTS.md` (prefer system SQLite, `rusqlite` non-bundled, registry cache repair)
- **macOS privilege boundaries**: `FORK-AI.md` (system SQLite, `/var` → `/private/var`, Touch ID for install)
- **SecretSpec upstream**: `secretspec/` crate (provider system, `Secrets::load_from`, `ExportFormat::Json`)
- **Homebrew**: `packaging/homebrew/sudo-secretspec.rb` (no privileged install)

---

## Reviewer Checklist

- [ ] No caller-selected control plane escape
- [ ] Audit is truly value-free (no argv, no secret in reason hash)
- [ ] Audit fails closed on ledger corruption / missing ledger
- [ ] Broker mutation rollback restores exact pre-state
- [ ] Vault `.local`/`.ansible`/`cache` are advisory, not errors
- [ ] Install `--adopt-existing` never chowns vault
- [ ] `execvpe` in `run` — no shell evaluation
- [ ] System SQLite, no `bundled` feature
- [ ] Homebrew formula: no sudo/users/sudoers/vault mutation
- [ ] Doctor elevation via NOPASSWD libexec path works
- [ ] `Report.ok` ignores `LEGACY_VAULT_CLUTTER` / `PENDING_ROLLBACK`

---

## How to Deliver Review

Post findings as PR comments on https://github.com/frdminc/sudo-secretspec/pull/1  
Tag security-critical items with `[SECURITY]`, advisory with `[ADVISORY]`.

---

*Generated for autonomous AI review. Context files: `FORK-AI.md`, `AI-GUIDANCE.md`, `CLAUDE.md`.*