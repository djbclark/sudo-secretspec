# djbclark/sudo-secretspec

Downstream fork of [cachix/secretspec](https://github.com/cachix/secretspec)
with an optional privilege-separated companion for autonomous local AI/operator
use on macOS.

## Upstream SecretSpec

See [`secretspec/README.md`](secretspec/README.md) for the upstream product,
providers, and quick start.

## Downstream companion: `sudo-secretspec`

This fork adds **`sudo-secretspec`**: a single Rust binary that mediates all
credential CRUD/use through a root-owned broker, fail-closed value-free audit,
and metadata-only doctor checks.

Full companion docs: [`sudo-secretspec/README.md`](sudo-secretspec/README.md)

AI automation rules: [`sudo-secretspec/AI-GUIDANCE.md`](sudo-secretspec/AI-GUIDANCE.md)

Fork development notes for AIs: [`FORK-AI.md`](FORK-AI.md)

### Short install / use

```bash
# after packaging or local release build
sudo-secretspec install                 # prompts / auto-detects when possible
sudo-secretspec install --adopt-existing

sudo-secretspec check --reason "smoke"
sudo-secretspec run --reason "start app" -- your-command
sudo-secretspec doctor
```

### Branch policy

| Branch | Role |
|---|---|
| `main` | upstream mirror |
| `sudo-main` | downstream release line (default) |
| `feature/*` | work targeting `sudo-main` |

Downstream version scheme: `0.19.1-djbclark.1` (`v0.19.1-djbclark.1`).

### Packaging

See [`packaging/README.md`](packaging/README.md). Homebrew installs unprivileged
files only; privileged boundary creation is always an explicit
`sudo-secretspec install`.
