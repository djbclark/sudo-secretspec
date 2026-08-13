# Fork-only notes for AI agents (`djbclark/sudo-secretspec`)

This file is **downstream-only**. Do not port it upstream to `cachix/secretspec`.

## Layout that matters

| Path | Purpose |
|---|---|
| `sudo-secretspec-cli/` | Rust companion binary/library (the product) |
| `sudo-secretspec/` | Guidance, config example, companion README |
| `packaging/` | Release script + Homebrew formula |
| `skills/sudo-secretspec/` | Distributable AI skill |
| `main` | upstream mirror |
| `sudo-main` | downstream default/release branch |

## High-value lessons (save time)

1. **Do not write complex shell for the boundary.** Runtime policy is Rust.
2. **Avoid `rusqlite` `bundled` on this Mac.** It can hang in `libsqlite3-sys`
   build scripts. Prefer system SQLite:
   ```bash
   export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
   export LIBRARY_PATH="/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH"
   export CPATH="/opt/homebrew/opt/sqlite/include:$CPATH"
   ```
   and `rusqlite = { version = "0.31" }` (no `bundled`).
3. **Cargo registry extract cache can be incomplete** after interrupted builds.
   Incomplete dirs under `~/.cargo/registry/src/index.crates.io-*` missing
   `Cargo.toml` can be deleted when the matching `.crate` archive exists.
4. **Toolchain is pinned** by `rust-toolchain.toml` (currently `1.92.0`). Ensure
   `PATH` includes `~/.cargo/bin`.
5. **macOS path aliases:** `/var` → `/private/var`, `/etc` → `/private/etc`.
   Validate resolved protected chains; accept public spellings.
6. **Dotenv provider must be pinned** to the vault file:
   `dotenv:///var/db/.../.env` — never bare `dotenv` (cwd-relative).
7. **Audit must fail closed.** Do not ignore `append_event` errors.
8. **Public client elevates with:**
   - lifecycle: `sudo -n /usr/local/libexec/sudo-secretspec __broker ...`
   - install/rollback: interactive `sudo` (Touch ID)
   - doctor: prefer `sudo -n` libexec elevation
9. **Adopt-existing must not chown/chmod the vault.** Only verify metadata.
10. **Install UX goal:** short forms (`install`, `install --adopt-existing`) with
    auto-detection/prompts; long flags are overrides only.

## Useful commands

```bash
# focused tests
cargo test -p sudo-secretspec-cli

# release binary
cargo build -p sudo-secretspec-cli --release
cp target/release/sudo-secretspec ~/bin/sudo-secretspec

# live smoke after install
sudo -n /usr/local/libexec/sudo-secretspec doctor
sudo-secretspec check --reason "smoke"
```

## Delivery conventions

- PRs target `sudo-main` on `djbclark/sudo-secretspec` (not upstream).
- Never open upstream issues/PRs without explicit permission.
- Keep `CHANGELOG.md` Unreleased entries user-facing.
- After privileged install, verify paths under `/usr/local` and vault ownership
  without reading secret values.

## Known residual findings on this host

After adopt-existing install of the stayturgid vault, doctor may report:

- `LEGACY_VAULT_CLUTTER` / previously `UNEXPECTED_RUNTIME_ENTRY` for `.local` /
  `.ansible` under the vault (non-secret tool state; safe to clean later)
- audit ledger ownership should be `_secretspec:staff` (fixed in current tree by
  chown-after-open when broker is root)

## Related human/AI guidance

- `sudo-secretspec/AI-GUIDANCE.md`
- `skills/sudo-secretspec/SKILL.md`
- `packaging/README.md`
- Local reusable Rust agent notes: `~/src/agent-guidance/rust/AGENTS.md`
