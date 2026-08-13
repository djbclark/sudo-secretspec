# sudo-secretspec (downstream companion)

Privilege-separated SecretSpec companion for this fork
(`djbclark/sudo-secretspec`).

Upstream SecretSpec remains the declaration/provider engine. This companion adds
a single mediated control plane for autonomous local AI/operator use on macOS.

## Quick start (short forms)

After the binary is available (`brew install` or local build):

```bash
# Preferred short install forms
sudo-secretspec install
sudo-secretspec install --adopt-existing

# Day-to-day use
sudo-secretspec check --reason "smoke"
sudo-secretspec run --reason "start gateway" -- your-command
sudo-secretspec doctor
```

`install` auto-detects common declaration paths and existing vaults, and
prompts on a TTY when needed. Use flags only to override defaults:

```bash
sudo-secretspec install \
  --declarations /path/to/secretspec.toml.example \
  --vault /var/db/stayturgid-secrets \
  --service-user _secretspec \
  --service-group staff \
  --operator "$USER" \
  --adopt-existing
```

## What gets installed

| Path | Role |
|---|---|
| `/usr/local/bin/sudo-secretspec` | public client |
| `/usr/local/libexec/sudo-secretspec` | privileged broker (same binary) |
| `/usr/local/etc/sudo-secretspec.toml` | protected config |
| `/etc/sudoers.d/sudo-secretspec` | NOPASSWD broker policy for operator |
| `/usr/local/share/sudo-secretspec/` | declarations pin, guidance, manifest |
| vault (e.g. `/var/db/stayturgid-secrets`) | `_secretspec`-owned runtime store |

Homebrew packaging installs files only. Privileged boundary creation always
requires an explicit `sudo-secretspec install` (Touch ID/sudo).

## Security model

- Singular mediated path for credential CRUD/use
- Fail-closed value-free SQLite audit (`broker-audit.sqlite3`)
- No caller-selected manifests/providers/profiles
- Doctor/drift is metadata-only and never repairs
- Rollback snapshots restore installed artifacts, not vault secret values

See `AI-GUIDANCE.md` for AI/automation rules.

## Develop

Workspace member: `sudo-secretspec-cli`.

```bash
export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
cargo test -p sudo-secretspec-cli
cargo build -p sudo-secretspec-cli --release
```

Branch policy for this fork:

- `main` mirrors upstream
- `sudo-main` is the downstream release line
- feature work PRs into `sudo-main`

Current downstream version: `0.19.1-djbclark.1`.
