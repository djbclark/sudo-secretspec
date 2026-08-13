---
title: Docker credentials
description: Let Docker retrieve registry credentials through SecretSpec providers
---

The Docker credential integration is available in SecretSpec 0.20+. It lets
`docker pull`, `docker push`, `docker build`, and Docker Compose retrieve
registry credentials from any SecretSpec provider without copying the
password or token into Docker's `config.json`.

## Quick start

Configure the registry with its non-secret username:

::::danger[This changes your Docker configuration]
Docker has no repository-local configuration. `configure` updates
`$DOCKER_CONFIG/config.json` when `DOCKER_CONFIG` is set, or the user-level
`~/.docker/config.json` (`%USERPROFILE%\.docker\config.json` on Windows)
otherwise. The change applies to every Docker command using that configuration.
SecretSpec preserves unrelated settings and refuses to replace another helper.
Undo it with `secretspec docker unconfigure --registry ghcr.io`.
::::

```bash
$ secretspec docker configure --registry ghcr.io --username YOUR_USERNAME
```

After confirmation, the command prints the matching login command:

```console
Configured Docker credential for ghcr.io.
Docker configuration: /home/you/.docker/config.json
Store the credential with: secretspec docker login 'ghcr.io'
Undo with: secretspec docker unconfigure --registry 'ghcr.io'
```

Store the password or access token in SecretSpec's embedded, registry-isolated
credential store:

```bash
$ secretspec docker login ghcr.io
```

Docker now invokes `docker-credential-secretspec get` automatically:

```bash
$ docker pull ghcr.io/OWNER/IMAGE:TAG
$ docker push ghcr.io/OWNER/IMAGE:TAG
```

`configure` does not retrieve or store the credential. It adds the registry's
`credHelpers` entry and records only the registry, username, provider selection,
and other value-free metadata. `login` prompts for the secret and stores it
through the selected provider. Each registry has a separate SecretSpec project
identity, so credentials cannot collide between registries.

To use a provider other than your default, pass the same override to both
commands. The follow-up command printed by `configure` includes it automatically:

```bash
$ secretspec docker configure \
  --registry ghcr.io \
  --username YOUR_USERNAME \
  --provider onepassword
$ secretspec docker login ghcr.io --provider onepassword
```

## Docker Hub

Docker uses the historical key `https://index.docker.io/v1/` for Docker Hub.
SecretSpec 0.20+ normalizes the familiar Docker Hub hostnames and URL forms to
that key:

```bash
$ secretspec docker configure \
  --registry docker.io \
  --username YOUR_DOCKER_ID
$ secretspec docker login docker.io
```

Registry addresses may contain a port, such as
`registry.example.com:5000`, but not a repository path. Credentials are scoped
to the registry rather than an image namespace.

## Use a project manifest

For a credential already declared by a project, pass `--file` to select the
advanced custom-manifest mode. In this mode, `--token-secret` and either
`--username` or `--username-secret` are required:

```toml
[project]
name = "docker-credentials"
revision = "1.0"

[profiles.default]
GHCR_TOKEN = { description = "GitHub Container Registry token" }
```

```bash
$ secretspec --file secretspec.toml docker configure \
  --registry ghcr.io \
  --token-secret GHCR_TOKEN \
  --username YOUR_USERNAME
```

To resolve the username from SecretSpec too, declare it and replace
`--username` with `--username-secret GHCR_USERNAME`. Custom-manifest mode also
accepts `--profile` and `--provider`.

The managed state records the manifest's absolute path and resolved profile,
but never resolved secret values. If the manifest moves, rerun `configure` for
the affected registry. Manage custom-manifest values with `secretspec set` and
`secretspec delete`; `secretspec docker login` and `logout` intentionally manage
only the embedded store.

## Alternate Docker configuration directory

SecretSpec honors `DOCKER_CONFIG` when selecting `config.json`, just like the
Docker CLI:

```bash
$ DOCKER_CONFIG="$HOME/.config/docker-work" \
  secretspec docker configure \
    --registry registry.example.com \
    --username YOUR_USERNAME
```

Use the same `DOCKER_CONFIG` value when unconfiguring entries from that file.

## Remove credentials and configuration

Remove an embedded secret without changing Docker's helper configuration:

```bash
$ secretspec docker logout ghcr.io
```

Pass the same `--provider` used for login when it was explicitly overridden.

Remove one helper registration from the active Docker configuration:

```bash
$ secretspec docker unconfigure --registry ghcr.io
```

Remove every Docker credential helper registration that SecretSpec owns in
that file:

```bash
$ secretspec docker unconfigure --all
```

Configuration changes prompt with a default of **No**. Pass `--yes` for
non-interactive setup or removal. SecretSpec preserves the default credential
store, other registry helpers, existing `auths`, and unrelated Docker options.
If a managed entry changes outside SecretSpec, `unconfigure` refuses to modify
it.

`logout` and `unconfigure` are independent: logout deletes the embedded secret,
while unconfigure removes Docker's reference to the helper. This matches the
separation between `login` and `configure`.

## Read-only helper behavior

In SecretSpec 0.20+, `docker-credential-secretspec` answers Docker's `get`
operation. It rejects `store`, `erase`, and `list`, so Docker's own
`docker login` and `docker logout` cannot overwrite or delete values in a shared
provider. Use `secretspec docker login` and `secretspec docker logout` for the
embedded store, or normal SecretSpec commands for a custom manifest.

When no matching configuration or stored value exists, the helper returns
Docker's standard credential-not-found response.
