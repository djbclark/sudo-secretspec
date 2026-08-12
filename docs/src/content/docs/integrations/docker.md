---
title: Docker credentials
description: Let Docker retrieve registry credentials through SecretSpec providers
---

The Docker credential helper is available in SecretSpec 0.20+. It lets ordinary
`docker pull`, `docker push`, `docker build`, and Docker Compose operations
retrieve registry credentials from any SecretSpec provider.

Use it when a registry token already lives in a provider such as 1Password,
Bitwarden, or Vault and you do not want Docker to copy it into its own
credential store or `config.json`.

## Prerequisites

- Docker CLI or another client that supports Docker credential helpers
- SecretSpec 0.20 or newer, including `docker-credential-secretspec` on `PATH`
- A SecretSpec manifest that declares the registry token

For example:

```toml
[project]
name = "docker-credentials"
revision = "1.0"

[profiles.default]
GHCR_TOKEN = { description = "GitHub Container Registry token" }
```

Store the token through the manifest's configured provider:

```bash
$ secretspec set GHCR_TOKEN
```

## Configure a registry

Keep a fixed username in SecretSpec's integration configuration and resolve the
token from the provider:

::::danger[This changes your Docker configuration]
The command below updates the active Docker `config.json`, normally
`~/.docker/config.json` (or `%USERPROFILE%\.docker\config.json` on Windows), for
every Docker command run by your user. Review the registry and manifest path
before confirming. To roll it back, run
`secretspec docker unconfigure --registry ghcr.io`; see
[Remove the configuration](#remove-the-configuration) for all removal options.
::::

```bash
$ secretspec docker configure \
  --registry ghcr.io \
  --token-secret GHCR_TOKEN \
  --username YOUR_USERNAME
```

The command validates the registry and secret declaration without retrieving
the token. It records the manifest's absolute path and resolved profile, then
sets the registry's Docker `credHelpers` entry to `secretspec`. The default
credential store, other registry helpers, existing `auths`, and unrelated
Docker settings remain untouched.

Docker configuration stores only the helper name. In its user configuration
directory, SecretSpec separately stores the manifest path, profile, provider
override, reason, secret names, and any literal username needed to invoke the
helper, but never resolved secret values. If you move or delete the manifest,
run `secretspec docker configure` again for the affected registry.

To retrieve the username from a SecretSpec provider as well, declare it and use
`--username-secret` instead of `--username`:

```bash
$ secretspec docker configure \
  --registry ghcr.io \
  --token-secret GHCR_TOKEN \
  --username-secret GHCR_USERNAME
```

Configuration changes prompt with a default of **No**. Pass `--yes` only for
non-interactive setup. You can also select a profile or provider with
`--profile` or `--provider`; the corresponding SecretSpec environment variables
are supported.

Docker invokes `docker-credential-secretspec get` automatically when it needs
the credential:

```bash
$ docker pull ghcr.io/OWNER/IMAGE:TAG
$ docker push ghcr.io/OWNER/IMAGE:TAG
```

You do not need to run `docker login`: configuring the helper replaces the need
to copy a credential into Docker's store.

## Docker Hub

Docker uses the historical key `https://index.docker.io/v1/` for Docker Hub
credentials. SecretSpec accepts the familiar aliases and stores the canonical
key, so this is sufficient:

```bash
$ secretspec docker configure \
  --registry docker.io \
  --token-secret DOCKER_HUB_TOKEN \
  --username YOUR_DOCKER_ID
```

Registry addresses may contain a port, such as `registry.example.com:5000`, but
not a repository path. Configure credentials per registry rather than per image
namespace.

## Alternate Docker configuration directory

SecretSpec honors `DOCKER_CONFIG` when selecting `config.json`, just like the
Docker CLI:

```bash
$ DOCKER_CONFIG="$HOME/.config/docker-work" \
  secretspec docker configure \
    --registry registry.example.com \
    --token-secret REGISTRY_TOKEN \
    --username YOUR_USERNAME
```

Use the same `DOCKER_CONFIG` value when removing entries from that file.

## Remove the configuration

Remove one registry from the active Docker configuration:

```bash
$ secretspec docker unconfigure --registry ghcr.io
```

Remove every Docker credential that SecretSpec configured in that file:

```bash
$ secretspec docker unconfigure --all
```

Removal also prompts with a default of **No** and accepts `--yes` for
non-interactive use. SecretSpec removes only entries it recorded. If a managed
entry was changed or removed outside SecretSpec, the command refuses to modify
its state and asks you to inspect the files manually.

## Read-only behavior

In SecretSpec 0.20+, the helper answers only Docker's `get` operation. It rejects
`store` and `erase`, so `docker login` and `docker logout` cannot overwrite or
delete a value in a shared provider. Manage values explicitly with
`secretspec set` and `secretspec delete`, and manage helper registration with
`secretspec docker configure` and `unconfigure`.

When no matching configuration or stored value exists, the helper returns
Docker's standard credential-not-found response.
