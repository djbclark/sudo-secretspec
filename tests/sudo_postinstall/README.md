# Post-install smoke suite

Proves that a boundary you just installed actually works. The unit suites cover
logic against fixtures; this covers the thing that only exists after
`sudo-secretspec install` — a root-owned broker reachable through a sudoers
policy, serving a vault owned by a service user. None of that can be faked, so
none of it is covered anywhere else.

```bash
~/.local/bin/pytest tests/sudo_postinstall -q
```

With no boundary installed, every test skips. That is the intended behaviour in
CI: the suite is inert until pointed at a real one.

## What it checks

Read-only, no prompts, safe to run any time:

- **Identity** — the client reports a downstream version, the broker answers it
  (a client newer than its broker fails here, which is what an upgrade without a
  reinstall produces), the configured vault is under the protected root, and the
  service user is not the operator.
- **Health** — `doctor`, `template-check`.
- **Audit** — `audit-verify` reports an intact chain, an audited operation
  advances the tip, and the chain still verifies afterwards.
- **Reads** — `check` (summary on stderr, stdout clean, never prompts), `get`,
  `export` (keys only — never values), `run` (environment reaches the child,
  `--` passes through, child exit code propagates).
- **Refusals** — every credential operation requires `--reason`; `get` refuses an
  undeclared name; `audit-verify` refuses one, since requiring a reason would add
  an event to the thing being verified.

## Gates

Three opt-ins, each guarding a different cost.

| Variable | Unlocks | Cost |
| --- | --- | --- |
| `SUDO_SECRETSPEC_POSTINSTALL_WRITES=1` | `set` / `get` / `delete` round-trip | Writes a scratch value to the live vault, then deletes it. |
| `SUDO_SECRETSPEC_POSTINSTALL_DECLARE=1` | `add` | **No inverse.** Nothing removes a declaration. |
| `SUDO_SECRETSPEC_POSTINSTALL_LIFECYCLE=1` | `install` / `uninstall` dry-run plans | One interactive auth prompt *per test*. |

`DECLARE` is separate from `WRITES` because declaring is the one operation that
cannot be undone: there is no subcommand that removes a name from the runtime
manifest, so the declaration stays until it is mirrored into the tracked
declarations file or the manifest is restored by hand. Until then
`template-check` reports the drift — correctly.

`LIFECYCLE` is separate because the installed policy sets `timestamp_timeout=0`
on `install`, `uninstall`, and `rollback`, so sudo demands fresh interactive
authentication for each — including for `--dry-run`, which mutates nothing. Left
ungated, a routine test run fires a burst of Touch ID prompts at whoever happens
to be at the keyboard. Only set this when you are there and expecting it.

The full run, at a keyboard, prepared to authenticate:

```bash
SUDO_SECRETSPEC_POSTINSTALL_WRITES=1 \
SUDO_SECRETSPEC_POSTINSTALL_DECLARE=1 \
SUDO_SECRETSPEC_POSTINSTALL_LIFECYCLE=1 \
  ~/.local/bin/pytest tests/sudo_postinstall -q
```

## Testing a build before installing it

`SUDO_SECRETSPEC_CLIENT` points the suite at a candidate binary instead of the
installed one:

```bash
cargo build -p sudo-secretspec-cli
SUDO_SECRETSPEC_CLIENT=$PWD/target/debug/sudo-secretspec \
  ~/.local/bin/pytest tests/sudo_postinstall -q
```

This is how you tell a client-side fix from one needing a boundary reinstall. If
the candidate passes against the installed broker, shipping the client is
enough. If it fails on the broker protocol, the boundary has to be reinstalled
too. The `run` basename fix was diagnosed exactly this way.

## Secret hygiene

`export` emits every secret this host holds as a JSON object on stdout. Tests
assert on **keys only**, and nothing derived from a value may reach an assertion
message — pytest prints the whole expression on failure, and a captured-output
dump of `export` would put the entire vault in the log. Value comparisons are
allowed only for the scratch secret the suite wrote itself.
