# sudo-secretspec (downstream companion)

Privilege-separated SecretSpec companion for this fork
(`frdminc/sudo-secretspec`).

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

> **Which binary runs `install` matters.** `install` copies from the tree its
> own executable lives in, so it must be run from the copy your package manager
> ships — `"$(brew --prefix)"/opt/sudo-secretspec/libexec/sudo-secretspec`,
> which is kept off `PATH` so it cannot shadow the installed client. The short
> forms above are what you type for a *first* install, when nothing is installed
> yet and that keg copy is the only one present. Once a boundary is installed,
> `sudo-secretspec install` would reach the installed client, whose tree is the
> destination; since `0.19.1-sudo.13` that is refused with the correct command
> in the error, and on earlier versions it silently upgraded nothing.

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

## Removing it

```bash
sudo-secretspec uninstall --dry-run   # print the plan, change nothing
sudo-secretspec uninstall
```

Removes exactly the paths in the table above, sudo policy first. The vault and
the service identity are left alone: `--purge-vault` deletes the vault and every
secret in it, and `--remove-service-user` deletes the service user and group.
Each prompts for its own confirmation, and `--remove-service-user` refuses an
identity this installer did not create — an adopted account such as
`_secretspec` has other consumers.

The sudo policy is checked against the install manifest before it is unlinked.
If its bytes do not match, it is left in place with a warning: the path is
predictable and other vendors keep drop-ins in the same directory, so a file
sitting there is not proof this installer wrote it.

## Security model

- Singular mediated path for credential CRUD/use
- Fail-closed value-free SQLite audit (`broker-audit.sqlite3`)
- No caller-selected manifests/providers/profiles
- Ownership, mode, and the resolved vault path are enforced before every
  operation, not only by `doctor`
- Doctor/drift is metadata-only and never repairs; advisory findings are
  reported without failing the check
- The NOPASSWD policy covers only mediated broker operations and `doctor`;
  `install`, `rollback`, and `uninstall` always require interactive
  authentication
- Rollback snapshots restore installed artifacts, not vault secret values, and
  are verified against the snapshot manifest before anything is written

See `AI-GUIDANCE.md` for AI/automation rules.

## Develop

Workspace member: `sudo-secretspec-cli`. Build/test commands and macOS build
caveats are in [`../FORK-AI.md`](../FORK-AI.md); the downstream version scheme
is in [`../CLAUDE.md`](../CLAUDE.md). All downstream work commits directly to
`sudo-main`; `main` is the upstream mirror only.
