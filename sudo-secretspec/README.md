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

Privileged boundary creation always requires an explicit `sudo-secretspec
install` (Touch ID/sudo); see [`../packaging/README.md`](../packaging/README.md)
for the Homebrew safety boundary.

## Security model

- Singular mediated path for credential CRUD/use
- Fail-closed value-free SQLite audit (`broker-audit.sqlite3`)
- No caller-selected manifests/providers/profiles
- Ownership, mode, and the resolved vault path are enforced before every
  operation, not only by `doctor`
- Doctor/drift is metadata-only and never repairs; advisory findings are
  reported without failing the check
- The NOPASSWD policy covers only mediated broker operations and `doctor`;
  `install` and `rollback` always require interactive authentication
- Rollback snapshots restore installed artifacts, not vault secret values, and
  are verified against the snapshot manifest before anything is written

See `AI-GUIDANCE.md` for AI/automation rules.

## Develop

Workspace member: `sudo-secretspec-cli`. Build/test commands and macOS build
caveats are in [`../FORK-AI.md`](../FORK-AI.md); the downstream version scheme
is in [`../CLAUDE.md`](../CLAUDE.md). All downstream work commits directly to
`sudo-main`; `main` is the upstream mirror only.
