# `sudo-secretspec` Code Review Report

## Executive Summary
This report combines the architectural review (`PROMPT-REVIEW.md`) and the privilege boundary security review (`PROMPT-SECREV.md`) into a single, comprehensive analysis of the `sudo-secretspec-cli` codebase.

Overall, the design holds its primary security invariants—most notably the fail-closed value-free audit, the `execvpe` execution model, and the rollback-capable atomic mutations. However, two critical security violations were identified where the implementation deviates from the required specification.

---

## Critical Findings (🔒 SECURITY)

### 1. Missing Binary/Audit Metadata Validation in `broker.rs`
**Location:** `sudo-secretspec-cli/src/broker.rs` -> `require_boundary()`
**Issue:** The `require_boundary()` function accurately verifies the `vault` (mode 0700, correct ownership, canonical path) and `manifest/dotenv` (mode 0600, correct owner). However, it **fails to validate the binary and audit helper paths**. 
**Violation:** The function terminates without checking that the binary/audit helper exist, are not symlinks, and are owned by `root:wheel` with mode `755`. This means a compromised operator might be able to manipulate the binaries if they somehow gain write access to the binary directories, bypassing the boundary checks.
**Remediation:** Add checks within `require_boundary()` to `stat` the executed binaries (e.g., `/usr/local/libexec/sudo-secretspec` and `/usr/local/bin/sudo-secretspec`) ensuring they are regular files, not symlinks, and are strictly `root:wheel:755`.

### 2. Incorrect Source Root Derivation in `install.rs`
**Location:** `sudo-secretspec-cli/src/install.rs`
**Issue:** The installer does *not* derive the source root from the fixed `libexec/../..` path structure as specified.
**Violation:** Instead, it relies on `std::env::current_exe()` and dynamically evaluates the manifest logic against itself using inline strings for auxiliary artifacts. This breaks the expected static release bundle structure, relying on the runtime path of the installer instead of the deterministic layout of the installation media.
**Remediation:** Update `install.rs` to derive the source root deterministically from the known installation target layout (`libexec/../..`) as per the specification.

---

## Invariant Verifications (✅ OK)

### Core Privilege Boundary (`broker.rs`)
- **`require_root()`**: Correctly uses `libc::geteuid() == 0` to ensure root execution.
- **Operation Allowlist**: Strictly limited to `source-*`, `audit-verify`, and `source-template-check`.
- **Reason Digestion**: Empty reasons (`SHA-256("")`) are explicitly rejected by `is_valid_reason_digest()`.
- **Fail-Closed Audit**: Audit attempt is always written before ANY secret operation is performed.
- **Mutation Safety**: The `Mutation` struct correctly backs up manifest/dotenv to `.rollback.<uuid>`. Rollback failure forces the terminal audit to `Outcome::Unknown`.
- **Environment Isolation**: `purge_ambient_env()` safely purges `SECRETSPEC_*` and `XDG_*` variables, and sets `HOME` to `/var/root` before `execute()` runs.
- **Provider Pinning**: The dotenv provider is strictly pinned to `dotenv:///.../.env`.

### Value-Free Audit Ledger (`audit.rs`)
- **Value-Free Guarantees**: `AppendEventRequest` maps to `AuditEvent` carrying only `SHA-256(reason)`. No secret values are ever logged or transported as plain text into the ledger.
- **Hash Chaining**: `event_hash = H(previous_hash || event_without_hash)` is properly implemented.
- **Head Tracking**: The `singleton=1` row correctly tracks the `(sequence, event_hash)` tip.
- **Integrity Checking**: `verify()` correctly asserts the hash chain, the head match, and executes `PRAGMA integrity_check`.
- **Ownership**: The ledger is properly `chown`ed to the vault owner when the broker runs as root.

### Drift Detection (`drift.rs`)
- **Report Safety**: `Report.ok` correctly ignores advisory findings like `LEGACY_VAULT_CLUTTER` and `PENDING_ROLLBACK`.
- **Vault Clutter**: `.local`, `.ansible`, and `.cache` are properly categorized as advisory clutter.
- **Validation**: Source manifest hashes are correctly verified against `MANIFEST.sha256`. Sudoers syntax is validated using `visudo -c`.
- **Ancestor Chain**: Validated to ensure root ownership, non-writability, and absence of ACLs.
- **Realpath Checks**: Vault is validated to resolve strictly under `/private/var/db/...`.

### Installation and Rollback (`install.rs` & `rollback.rs`)
- **Existing Adoption**: `--adopt-existing` safely reads metadata but *never* chowns or chmods the vault.
- **Fresh Install**: Correctly searches for an unused UID/GID in the 400-499 range and creates the vault at `0700`.
- **Dry-Run Permissions**: `require_root()` is enforced *before* checking the `dry_run` flag.
- **Snapshot Scoping**: Snapshots are strictly validated under `/usr/local/libexec/sudo-secretspec-rollback-*`.
- **Restore Safety**: `MANIFEST.sha256` is validated before restore. Restores never touch the vault data.
- **Policy Safety**: Sudoers policy changes are verified with `visudo -c` post-restore.

### Public CLI & Execution (`main.rs`)
- **No Shell Evaluation**: Execution relies on `execvpe` securely without invoking a shell.
- **Reason Enforcement**: `--reason` is required by Clap, and the plain-text reason is hashed by the client before being sent across the broker boundary.
- **Elevation**: Install, rollback, and doctor commands elevate interactively using `sudo` rather than `-n`.

### Configuration and Packaging (`config.rs`, `Cargo.toml`, Homebrew)
- **TOML Parsing**: Uses `deny_unknown_fields` and properly validates the service identity pattern (`^_[a-z_][a-z0-9_]{0,30}$`).
- **Path Traversal**: Rejects `..` path traversal in managed paths.
- **Dependencies**: `rusqlite` relies on the system SQLite via Homebrew (no `bundled` feature).
- **Homebrew Formula**: Safe bootstrap process. The formula does not run `sudo`, mutate `/var/db`, or edit sudoers.

---

## Conclusion
The codebase exhibits a strong security posture, heavily leveraging Rust's type system to enforce privilege boundaries. The use of SQLite for an append-only hash-chained ledger is robust, and the execution funnel guarantees operations and audit records remain consistent. By addressing the missing binary metadata validations in `broker.rs` and fixing the source root path derivation in `install.rs`, this boundary application will fully meet its target design invariants.
