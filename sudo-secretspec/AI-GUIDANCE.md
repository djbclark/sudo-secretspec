# AI and automation contract for sudo-secretspec

Use this policy in `AGENTS.md`, skills, runbooks, and autonomous-agent prompts for deployments of sudo-secretspec.

## Required behavior

- Use only the installed `/usr/local/bin/sudo-secretspec` client for credential CRUD and use.
- Supply a short operational `--reason` for every `add`, `set`, `delete`, `get`, `check`, `export`, `template-check`, or `run` operation. The protected audit ledger stores only its SHA-256 digest.
- Run a consumer as `sudo-secretspec run --reason "purpose" -- <command>` instead of extracting values into shell history or command arguments.
- Treat an unavailable broker, failed audit append/verification, declaration mismatch, authorization failure, or non-advisory drift finding as a hard stop. `doctor` marks advisory findings — `LEGACY_VAULT_CLUTTER`, `PENDING_ROLLBACK`, and `CLIENT_DUPLICATE` — and still exits zero; report them to the operator and continue. Read the `advisory` field rather than matching code names, which grow over releases.
- Treat `CLIENT_SHADOWED` as a hard stop and do not work around it: it means a different `sudo-secretspec` would run instead of the installed client, so no operation you perform can be trusted to have reached the boundary. Report the reported path to the operator.
- Report only the non-secret error and request operator action for boundary installation or repair.
- Normal broker-mediated credential operations are autonomous when the installed sudoers policy allows them.

## Forbidden behavior

Never:

- invoke `secretspec` directly for a managed deployment;
- select or create another manifest, provider, profile, binary, dotenv file, or credential store;
- set `SECRETSPEC_FILE`, `SECRETSPEC_PROVIDER`, `SECRETSPEC_PROFILE`, or a direct-mode bypass;
- read, write, copy, symlink, regenerate, relocate, chmod, chown, rename, unlink, or repair protected backing files;
- weaken permissions or ACLs to inspect the store;
- place secret values in reasons, logs, audit metadata, command arguments, issue text, commits, tests, or documentation;
- treat inability to stat a `0700` store as evidence that files are absent;
- use a watchdog or drift check to mutate or repair state.

## Declaration lifecycle

Tracked Git content contains declarations only. A new name must first be reviewed and released in the declaration file. After installation/deployment pins that release, `sudo-secretspec add NAME --reason "purpose"` may create its value. Remove a declaration through review/release before `delete` removes its value. The broker rolls manifest/value mutations back on failure and records an explicit `unknown` terminal state if restoration cannot be proven.

## Security scope

The public client may export the full declared environment to a requested child process; this is intentional for a deployment where every authorized local AI/user may use every managed credential. The boundary provides integrity, singular control-plane enforcement, least-privilege backing-file access, and value-free audit—not per-secret confidentiality among callers sharing the authorized operator identity.

Client-family labels are correlation metadata, not authenticated principals. Deployments requiring per-agent or per-secret authorization need a stronger principal model and policy layer rather than trusting environment markers.

The audit ledger is hash-chained, so altering or removing any event other than the last one is detectable. Truncating the tail is **not**, and neither is deleting the ledger outright: the `head` row that names the tip lives in the same database as the events, so whoever can delete the last N events can rewrite `head` to match, and verification then passes. Deleting everything is the same move taken to its limit — an empty ledger is indistinguishable from a fresh install. Reaching either state requires write access to the vault as root or the service user, which the mediated sudoers policy does not grant. Tamper-evidence against a principal who can write the ledger cannot come from inside the ledger; a deployment that wants it must record the tip hash reported by `audit-verify` outside the vault and compare on each run.

Secret values are not zeroized in the client. `run` holds the exported environment as a byte buffer, then as parsed JSON, then copies each value into the child process — three plaintext copies in the client's heap, none scrubbed. This is accepted rather than fixed: Rust's `Vec`/`String` reallocation makes zeroization best-effort even with a dedicated crate, and the values are bound for the child's environment regardless, where they are readable for the whole life of that process. A deployment with a specific threat here — core dumps, or another process scraping memory — needs process hardening, not a scrubbing pass in this client.

## Verification

Use:

```bash
sudo-secretspec doctor
sudo-secretspec install --dry-run <install options>
```

The dry-run still uses `sudo` because protected metadata cannot be validated honestly from the operator UID. It never reads secret contents. Rollback snapshots cover installed artifacts only and deliberately preserve the runtime vault:

```bash
sudo-secretspec rollback /usr/local/libexec/sudo-secretspec-rollback-<timestamp>
```

Removing the boundary is `sudo-secretspec uninstall`, and it is an operator
action, not an automation one. Never run it to work around a failed check. It
too requires interactive authentication, and `--dry-run` prints the plan without
changing anything. It preserves the vault and the service identity unless
`--purge-vault` or `--remove-service-user` is passed; do not pass either.
