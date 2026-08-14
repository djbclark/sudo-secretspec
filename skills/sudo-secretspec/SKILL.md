---
name: sudo-secretspec
description: Use managed credentials without backing-store access.
version: 0.1.0
author: Dan Clark (djbclark), Hermes Agent
license: Apache-2.0
platforms: [macos]
metadata:
  hermes:
    tags: [secrets, privilege-separation, audit, macos]
    related_skills: []
---

# sudo-secretspec Skill

Use the installed privilege-separated client for autonomous credential CRUD and consumer execution. Do not access SecretSpec’s provider or protected backing files directly.

## When to Use

- A task needs an API key, token, password, or other declared credential.
- A credential must be initialized, rotated, read, deleted, checked, exported, or injected into a child process.
- The managed boundary or drift checker reports an error.

Do not use for boundary installation, adoption, rollback, or repair without explicit operator authorization.

## Prerequisites

- `/usr/local/bin/sudo-secretspec` is installed.
- The credential name is present in the released declarations before `add`.
- The consumer can read its credentials from environment variables when using `run`.

## How to Run

Invoke through `terminal` with a short non-secret reason:

```bash
sudo-secretspec run --reason "query provider API" -- command args...
sudo-secretspec get NAME --reason "inspect managed credential"
sudo-secretspec set NAME --reason "rotate managed credential"
sudo-secretspec check --reason "validate required credentials"
```

## Procedure

1. Confirm the name already exists in released declarations. If absent, stop and request the declaration PR/release/deploy lifecycle; do not create an alternate manifest.
2. Prefer `run` for consumers so values do not enter command arguments, source files, or shell history. Completion: the child receives its environment without value output in the agent transcript.
3. Use `add`, `set`, `delete`, or `get` only through `sudo-secretspec`, with a non-secret reason. Completion: the command returns success and the audit ledger records a terminal outcome.
4. On any broker, audit, authorization, or drift failure, stop and report only the non-secret error. Completion: no fallback store, manifest, provider, symlink, copy, permission change, or repair was attempted.

## Pitfalls

- Permission denied while inspecting a `0700` store is expected and does not mean files are missing.
- Reasons are hashed in the protected broker ledger but may reach SecretSpec’s native reason interface; never put values in them.
- Client labels are correlation hints, not authenticated AI identities.
- An alert-only watchdog must never repair state.
- `install` and `rollback` are not available through the NOPASSWD broker path; they require interactive operator authentication through `/usr/local/bin/sudo-secretspec`.
- `run` intentionally exposes the declared environment to its child; this deployment model authorizes every local caller for every declared credential.

## Verification

Run through `terminal`:

```bash
sudo-secretspec doctor
```

Success requires a zero exit status. Findings marked `[advisory]` — vault clutter and a leftover mutation backup — do not fail the check; report them and continue. Any `[error]` finding is a hard stop. Do not run `install` or `rollback` merely to verify a normal credential operation.

See `sudo-secretspec/AI-GUIDANCE.md` in the distribution for the complete policy contract.
