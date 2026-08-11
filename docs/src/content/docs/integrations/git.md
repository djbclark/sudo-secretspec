---
title: Git credentials
description: Let Git retrieve HTTPS credentials through SecretSpec providers
---

The Git credential helper is available in SecretSpec 0.20+. It lets ordinary
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
$ secretspec set GITHUB_TOKEN
```

## Configure Git

Keep a fixed username in Git and let SecretSpec supply the token:

```bash
$ secretspec git configure \
  --url https://github.com \
  --token-secret GITHUB_TOKEN \
  --username YOUR_USERNAME
```

The command validates the URL and secret declaration without retrieving the
token. It records the manifest's absolute path and resolved profile, then
registers `git-credential-secretspec` for this repository. Other configured
helpers and usernames remain untouched.

To retrieve the username from a SecretSpec provider as well, declare it and use
`--username-secret` instead of `--username`:

```bash
$ secretspec git configure \
  --url https://github.com \
  --token-secret GITHUB_TOKEN \
  --username-secret GITHUB_USERNAME
```

The helper checks `--url` independently before returning credentials. A token
configured for `https://github.com` is therefore not returned for another host
or for an HTTP remote.

To limit a credential to part of a host, include the path in the URL:

```bash
$ secretspec git configure \
  --url https://github.com/cachix \
  --token-secret GITHUB_TOKEN \
  --username YOUR_USERNAME
```

SecretSpec also enables Git's `useHttpPath` setting for that URL. This example
answers for repositories below `https://github.com/cachix/`, but not for
another GitHub organization.

## Clone private repositories

During the initial clone, the destination repository and its manifest do not
exist yet. Put the declaration in a separate manifest and configure it
globally:

```bash
$ secretspec \
  --file ~/.config/secretspec/git/secretspec.toml \
  git configure \
  --url https://github.com \
  --token-secret GITHUB_TOKEN \
  --username YOUR_USERNAME \
  --global
```

Then clone normally:

```bash
$ git clone https://github.com/OWNER/REPOSITORY.git
```

Git invokes the SecretSpec credential helper automatically. The token does not
need to appear in the clone URL or your shell history.

Global changes require a confirmation that defaults to **No**. Pass `--yes`
only for non-interactive setup. You can also select a profile or provider with
`--profile` or `--provider`; the corresponding SecretSpec environment variables
are supported.

## Remove the configuration

Remove one credential from the current repository:

```bash
$ secretspec git unconfigure --url https://github.com
```

Remove every Git credential that SecretSpec configured in the current
repository:

```bash
$ secretspec git unconfigure --all
```

Add `--global` to operate on global configuration. Global removal also defaults
to **No** and accepts `--yes` for non-interactive use:

```bash
$ secretspec git unconfigure --all --global
```

SecretSpec stores generated entries in its own included Git configuration
file. Configure and unconfigure never replace existing credential helpers,
usernames, or unrelated includes. Removing the final managed credential removes
the SecretSpec include and its file. If that file contains anything SecretSpec
does not recognize, the command refuses to modify it and asks you to inspect it
manually.

## Manual configuration

The convenience command is equivalent to registering the helper yourself. For
example:

```bash
$ git config --local credential.https://github.com.username YOUR_USERNAME
$ git config --local credential.https://github.com.helper \
  'secretspec --url https://github.com --password-secret GITHUB_TOKEN'
```

When configuring a path manually, set `useHttpPath` and use the same URL in the
helper:

```bash
$ git config --local credential.https://github.com/cachix.useHttpPath true
$ git config --local credential.https://github.com/cachix.helper \
  'secretspec --url https://github.com/cachix --password-secret GITHUB_TOKEN'
```

These entries are not recorded in SecretSpec's managed file, so
`secretspec git unconfigure` does not remove them. Remove manually configured
entries with `git config` as well.

## Read-only behavior

In SecretSpec 0.20+, the helper only answers Git's `get` operation. It safely
ignores automatic `store` and `erase` requests, so a rejected credential cannot
delete or overwrite a value in a shared provider. Manage the value explicitly
with `secretspec set` or `secretspec delete`.

Git can continue to try another configured helper or prompt when SecretSpec has
no stored value for the declared key.
