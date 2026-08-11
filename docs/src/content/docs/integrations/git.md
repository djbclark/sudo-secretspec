---
title: Git credentials
description: Let Git retrieve HTTPS credentials through SecretSpec providers
---

The Git credential helper is added in SecretSpec 0.19. It lets ordinary
`git clone`, `git fetch`, `git pull`, and `git push` commands retrieve HTTPS
credentials from any SecretSpec provider.

Use it when your Git token already lives in a provider such as 1Password,
Bitwarden, or Vault and you do not want to copy it into a separate Git
credential store. The integration does not manage SSH keys or inject secrets
into repositories.

## Prerequisites

- Git
- SecretSpec 0.20 or newer, including `git-credential-secretspec` on `PATH`
- A SecretSpec manifest that declares the token

For example:

```toml
[project]
name = "git-credentials"
revision = "1.0"

[profiles.default]
GITHUB_TOKEN = { description = "GitHub token for HTTPS authentication" }
```

Store the token through the manifest's configured provider:

```bash
secretspec set GITHUB_TOKEN
```

## Configure Git

Keep a fixed username in Git and let SecretSpec supply the token:

```bash
git config --local credential.https://github.com.username YOUR_USERNAME
git config --local credential.https://github.com.helper \
  'secretspec --url https://github.com --password-secret GITHUB_TOKEN'
```

This registers `git-credential-secretspec` for HTTPS authentication to
`github.com`. Other configured helpers remain part of Git's helper chain.

To load the username from SecretSpec too, declare it and configure both keys:

```bash
git config --local credential.https://github.com.helper \
  'secretspec --url https://github.com --username-secret GITHUB_USERNAME --password-secret GITHUB_TOKEN'
```

The helper checks `--url` independently before returning credentials. A token
configured for `https://github.com` is therefore not returned for another host
or for an HTTP remote.

To limit a credential to part of a host, include the path in both settings and
tell Git to preserve HTTP paths in credential requests:

```bash
git config --local credential.https://github.com/cachix.useHttpPath true
git config --local credential.https://github.com/cachix.helper \
  'secretspec --url https://github.com/cachix --password-secret GITHUB_TOKEN'
```

This example answers for repositories below `https://github.com/cachix/`, but
not for another GitHub organization.

## Clone private repositories

During the initial clone, the destination repository and its manifest do not
exist yet. Put the declaration in a separate manifest and configure its path:

```bash
git config --global credential.https://github.com.helper \
  'secretspec --url https://github.com --file ~/.config/secretspec/git/secretspec.toml --password-secret GITHUB_TOKEN'
```

You can also select a profile with `--profile`. The corresponding
`SECRETSPEC_FILE`, `SECRETSPEC_PROFILE`, `SECRETSPEC_PROVIDER`, and
`SECRETSPEC_REASON` environment variables are supported.

## Read-only behavior

SecretSpec 0.19 only answers Git's `get` operation. It safely ignores Git's
automatic `store` and `erase` requests, so a rejected credential cannot delete
or overwrite a value in a shared provider. Manage the value explicitly with
`secretspec set` or `secretspec delete`.

Git can continue to try another configured helper or prompt when SecretSpec has
no stored value for the declared key.
