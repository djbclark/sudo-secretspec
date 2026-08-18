---
title: SQLite Provider
description: Store secrets in a local SQLite database
---

The SQLite provider stores secrets as rows in a local SQLite database.

:::caution[Version compatibility]
The `sqlite` provider is added in SecretSpec 0.20.
:::

## At a glance

| | |
| --- | --- |
| Provider | `sqlite` (0.20+) |
| URI | `sqlite:PATH[?history=true]` |
| Access | Read, write, and delete |
| Best for | Local database storage, versioned history, and embedded secret stores |
| Authentication | Filesystem permissions |
| Availability | Built in (0.20+) |
| Default storage | Single SQLite database file (`0600` on Unix) |

## Quick start

Choose a database path, exclude it from version control, and route a declaration to it:

```text title=".gitignore"
/secrets.db*
```

```toml title="secretspec.toml"
[providers]
local_db = "sqlite:./secrets.db"

[profiles.development]
DATABASE_URL = { description = "Local database URL", providers = ["local_db"] }
```

```bash
$ secretspec set DATABASE_URL --profile development
Enter value for DATABASE_URL (profile: development): postgresql://localhost/dev

$ secretspec get DATABASE_URL --profile development

$ secretspec run --profile development -- npm start
```

## Setup

The provider requires no external daemon, service account, or network access. The SecretSpec process needs read and write access to the database file's location.

Relative database paths resolve from the directory containing `secretspec.toml`. On Unix, SecretSpec creates parent directories with mode `0700` and the database file with mode `0600`.

## Configuration

### URI format

```text
sqlite:PATH[?history=true]
```

`PATH` specifies the path to the SQLite database file. It can be relative (`sqlite:./secrets.db`), absolute (`sqlite:///var/lib/secrets.db`), or home-relative (`sqlite:~/.config/app/secrets.db`).

### Query parameters

- `history`: Optional boolean (`true`, `1`, `yes`, `on`). When enabled, mutations are tracked in a tamper-evident, hash-chained history log within the SQLite database.

### URI examples

```text
sqlite:./secrets.db                      # Relative to secretspec.toml
sqlite:///var/lib/myapp/secrets.db       # Absolute path
sqlite:~/.local/share/myapp/secrets.db   # Home-relative path
sqlite:./secrets.db?history=true         # With hash-chained history enabled
```

### Project configuration

Check in the provider alias, but never the database file:

```toml title="secretspec.toml"
[providers]
local_db = "sqlite:./secrets.db"
versioned_db = "sqlite:./secrets.db?history=true"

[profiles.default]
DATABASE_URL = { description = "Database connection string", providers = ["local_db"] }
```

## Storage model

Convention addresses store each secret in the `secrets` table under the primary key:

```text
{project}/{profile}/{key}
```

This isolates projects and profiles sharing a single SQLite database.

When history tracking is enabled via `?history=true`, SecretSpec records each mutation in a hash-chained entries table, preserving historical states and digests.

## Use existing secrets

A native `ref.item` specifies the exact primary key row name in the `secrets` table:

```toml title="secretspec.toml"
[providers]
local_db = "sqlite:./secrets.db"

[profiles.production]
DATABASE_PASSWORD = {
  description = "Database password",
  providers = ["local_db"],
  ref = { item = "custom/db_password" }
}
```

## Security considerations

:::danger[Filesystem permissions]
The SQLite provider stores values without cryptographic encryption. Confidentiality relies entirely on operating-system filesystem permissions.
:::

On Unix, SecretSpec ensures the database file is restricted to mode `0600` (readable and writable only by the owning user) and parent directories to mode `0700`. Ensure that database files are excluded from version control and backups of public assets.
