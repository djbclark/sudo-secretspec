# Design note: privilege boundary, packaging, and the authentication gate

**Status:** analysis complete; F2, F4, F6 implemented; F1, F3 outstanding
**Date:** 2026-08-13
**Scope:** fork-only (`djbclark/sudo-secretspec`). Not upstream material.
**Prompted by:** resolving the Homebrew link conflict after the
`v0.19.1-djbclark.1` release turned out to be far more hazardous than it
looked.

This note exists because the near-miss was instructive. The operator's
proposed fix — uninstall the brew package, `sudo rm` the other binary,
reinstall from scratch — was reasonable on its face and would have left a
half-torn-down privilege boundary. The reasoning below is what made that
visible, recorded so the next session does not have to re-derive it.

## Root cause

The privileged install is hardcoded to `/usr/local` — `PREFIX`
(`sudo-secretspec-cli/src/install.rs:16`) and `BROKER_PATH`
(`sudo-secretspec-cli/src/main.rs:17`) are compile-time constants — while
Homebrew delivers binaries to `/opt/homebrew`. These are two unrelated
half-installs and **neither knows the other exists.**

Brew has no knowledge of the boundary: its formula deliberately only ships
files, as its own caveats state. The boundary has no knowledge of brew:
`doctor` verifies installed paths and never asks whether those are the paths
that would actually run.

Note the asymmetry that makes this fixable: the drift side is *already*
prefix-agnostic. `Layout::from_config` (`drift.rs:45-58`) derives the prefix
from the configured engine path. Only the install side hardcodes it.

## Findings

### F1 — The Touch ID gate is not per-operation

The design's central claim is stated at `install.rs:475-478`: *"Boundary
lifecycle must stay behind Touch ID via the public client."*

The generated policy (`sudoers_text()`, `install.rs:479-488`) sets
`Defaults!` only on the **libexec broker** path, which is `NOPASSWD` anyway.
There is no `Defaults!` on the client path and no `timestamp_timeout`
anywhere. Touch ID itself is enabled globally via `pam_tid.so` in
`/etc/pam.d/sudo` and `/etc/pam.d/sudo_local` — configuration this project
does not own.

Consequence: the gate is enforced by sudo's shared timestamp, default 5
minutes, per-tty. `sudo ls` followed 30 seconds later by
`sudo sudo-secretspec install` prompts for **nothing**. The privilege
boundary holds — an unauthorised user still cannot elevate — but the
interactive-authentication guarantee the design claims is borrowed from
global state any other command can satisfy on our behalf.

This is the same phenomenon an earlier session diagnosed as "a cached sudo
timestamp, not unauthenticated privilege." That diagnosis was mechanically
correct; the conclusion "the privilege boundary is intact" was true but
under-stated the problem.

**Fix:** `Defaults!/usr/local/bin/sudo-secretspec timestamp_timeout=0`.
Cheap, and worth doing whether or not the sudo dependency is ever removed.

### F2 — `doctor` verifies the installation, not the invocation

`INSTALLED_HASH_MISMATCH` (`drift.rs:311`) checks
`/usr/local/bin/sudo-secretspec` against `MANIFEST.sha256`. Nothing checks
whether a *different* `sudo-secretspec` sits earlier in `PATH`.

Observed on this host: `/opt/homebrew/bin` is `PATH` position 11,
`/usr/local/bin` is position 20, and the two binaries have different hashes
(`f37c41…` vs `80ca48…`) despite reporting the same version. Linking the keg
would silently shadow the installed client and `doctor` would still report
OK.

**Fixed.** Implemented as `check_client_shadowing` in `drift.rs`, with two
codes: `CLIENT_SHADOWED` (fails the check) and `CLIENT_DUPLICATE` (advisory).

Three things changed from the original sketch, each for a reason worth keeping:

- **Severity is not decided by version.** Asking a binary its version means
  *executing* a binary we just concluded might not be ours. The classifier
  compares content hashes and never runs the candidate. It also refuses to
  hash anything that is not a regular file, because `fs::read` on a fifo
  planted in a search directory would block this process as root.
- **`PATH` is resolved on the caller's side, not the broker's.** `doctor`
  re-execs itself through `sudo`, and our own policy sets
  `secure_path=/usr/bin:/bin:/usr/sbin:/sbin` — so the privileged process
  cannot see the `PATH` whose shadowing is the entire question. The client
  passes its search path as `--caller-path` before elevating. Because a fixed
  list of standard directories is scanned unconditionally, that caller-supplied
  value can only *add* findings, never suppress one.
- **The unsafe-prefix predicate is applied to the shadow, not re-applied to the
  layout.** `check_ancestor_chain` (`drift.rs`) already walks each installed
  path's ancestors and emits `REMOVABLE_ANCESTOR` for exactly the
  not-root-owned-or-writable condition, so a second layout-side check would only
  duplicate findings. What was genuinely missing is the same predicate applied
  to wherever a *second* client is reachable from. `validate_protected_ancestors`
  was refactored so both callers share one `is_protected_dir` (`install.rs`).

A copy under a writable prefix fails even when it loses `PATH` order today:
losing is an accident of ordering, while write access is a standing ability to
swap the bytes.

Version skew fell out of this: a client built from a newer tree can meet an
older installed broker, since brew hands over a new bootstrap binary before
`install` replaces the pair. The older broker rejects `--caller-path` with
clap's usage exit, which would have turned a health check into an
argument-parsing error. `doctor` now retries without the flag on exit 2.

### F3 — There is no uninstall

`grep -ri uninstall` across the companion, docs, and packaging returns zero
hits. `rollback` is not an uninstall: it restores prior artifacts from a
snapshot (`rollback.rs:37-47`), explicitly preserves the vault, and can only
restore paths captured from a *previous* install. A first install captures
nothing (`install.rs:108-109`), which is why three of the six snapshot
directories on this host are empty.

Removing the boundary today requires manual root surgery across the sudoers
policy, two binaries, `share/`, config, the `_secretspec` user, and the
vault. That is the whole reason the packaging question was dangerous.

**Fix:** the hard part already exists. `installed_artifacts()`
(`install.rs:72-85`) is the complete ownership manifest with modes, and is
already the trust root `rollback` uses to refuse foreign destinations.
Uninstall is that list plus three policy decisions:

- **Sudoers first, then binaries.** Close the grant before removing the path
  it names. Harmless on this host because `/usr/local/libexec` is root-owned,
  but correct in general and load-bearing for any prefix that is not.

  To be explicit about scope, because "remove the sudoers policy" invites the
  wrong reading: our policy is not lines appended to a shared file. `install`
  writes a **dedicated drop-in**, `/private/etc/sudoers.d/sudo-secretspec`
  (`SUDOERS_PATH`, `install.rs:18`), listed in `installed_artifacts()` at mode
  `0440`. Uninstall removes that one file and never touches `/etc/sudoers`, and
  never touches another vendor's drop-in — `/etc/sudoers.d/yabai` is a live
  neighbour on this host. Uninstall must also verify the file is ours (hash it
  against the install manifest) before unlinking, and leave it in place with a
  warning if it is not: the path is predictable, so a file sitting there is not
  proof we wrote it.
- **Never auto-delete the vault.** Separate `--purge-vault` flag with its own
  confirmation; default is leave-it.
- **Service user removal opt-in** (`--remove-service-user`). `_secretspec`
  has other dependents — the stayturgid wrapper is one.

Gate it like `install`: add `Uninstall` to the non-broker match at
`main.rs:126` so it is refused through the `NOPASSWD` path.

### F4 — Homebrew ships a binary that should never be linked

The keg's `bin/sudo-secretspec` is only ever a *bootstrap* — the thing run
once to perform `install`, after which the real client lives at
`/usr/local/bin`. Linking it creates the F2 shadow permanently.

**Fix:** `keg_only`, or install to the keg's `libexec` and link only
`secretspec`. This makes the brew layer purely "deliver files," which is what
its caveats already claim.

**Rejected:** making `PREFIX` configurable so brew could own the privileged
install. `BROKER_PATH` is compared against `current_exe()` to decide
privilege (`main.rs:110-118`); a configurable broker path is far more
delicate to get right. `/usr/local` is root-owned, `/opt/homebrew` is
`admin`-writable. The hardcoding is a security property. Fix the packaging,
not the constant.

### F5 — The legacy wrapper is a second, weaker privileged path

`/usr/local/libexec/stayturgid-secretspec-wrapper.sh` is the predecessor of
the Rust broker. Both are live against the same vault
(`/var/db/stayturgid-secrets`, where `doctor`'s two `LEGACY_VAULT_CLUTTER`
advisories live). The bash path has no audit database and no mediation.

Two specific problems:

- `SECRETSPEC_BIN=/opt/homebrew/bin/secretspec` (line 8) is executed as root
  from an `admin`-writable prefix. Admin-user write becomes root exec. The
  Rust side already learned this lesson — `const SUDO: &str = "/usr/bin/sudo"`
  with *"Never resolve this through PATH"* (`main.rs:8-12`). **This coupling
  is what made the brew-uninstall plan risky**: a privileged script pinned to
  a package-manager-managed path.
- Root is mostly unnecessary. Three operations (`verify-sync`,
  `automation-env`, `firerpa-mcp-token`) already run as `_secretspec`. The
  `source-*` operations require root for exactly one reason: `sync_source()`'s
  `chown _secretspec:staff` plus `chmod`. If the vault files were created by
  `_secretspec` under umask 0077 in a directory it already owns, that chown is
  a no-op and root is not needed — one-time root setup, then
  `sudo -u _secretspec` for steady-state work. `source-template-check` merely
  `cmp`s two files and needs no privilege at all.

The strongest fix is not hardening the wrapper but retiring it onto the Rust
broker, which already has `add/set/delete/get/check/export/run` with mandatory
`--reason` and an audit trail. Blocked on `verify-sync` and
`source-template-check` having no Rust equivalent.

### F6 — Rollback snapshots are never collected

One directory per `install` run under `/usr/local/libexec/`, forever; six on
this host, three of them empty. `PENDING_ROLLBACK` only scans the *vault* for
`.rollback.` names (`drift.rs:388`), so `doctor` never sees them.

## Should we stop using sudoers?

The shared-directory coupling is real and has been paid for twice: the scoped
`visudo -c` handling in `install.rs:270-292` and `rollback.rs:104-131` exists
solely because another vendor's broken file can fail the combined parse. This
host has a live instance (`/etc/sudoers.d/yabai` has a mode that makes sudo
ignore it). But F1 — the shared *timestamp* — is the bigger problem than the
shared *directory*.

Options considered:

| Approach | Verdict |
| --- | --- |
| Own setuid-root binary | **Rejected** |
| launchd root daemon + socket/XPC | **Rejected** — see below |
| Authorization Services / `SMAppService` | **Rejected** with launchd |
| Harden within sudoers | **Adopted**, and extended to other platforms |

**Setuid — rejected.** Inherits sudo's entire hostile-environment problem:
`argv[0]`, inherited file descriptors, rlimits, signal dispositions,
controlling tty, environment scrubbing. Thirty years of CVEs teaching lessons
we would re-learn. macOS strips `DYLD_*` for setuid binaries, which handles
one vector out of many. Touch ID is unavailable from a setuid CLI —
LocalAuthentication expects a signed binary in a GUI session context. And
Homebrew cannot set the bit (it installs unprivileged), so our root installer
would have to, meaning the packaging problem does not improve either.

**launchd daemon — rejected (operator decision, 2026-08-13).** Two reasons,
neither of them about the technical merits below:

- **It is macOS-only, and this project is not going to be.** The direction is
  the opposite one — keep `sudo` as the mechanism and extend it to other
  operating systems and distributions, because `sudo` is the portable common
  denominator. A launchd path would be a second, platform-specific privilege
  mechanism to maintain alongside the one every other platform still needs.
- **Someone else already built it.** Tools following the daemon pattern exist
  and are documented in an issue. If that pattern turns out to be the right
  one, the move is to adopt those tools, not to reimplement them here.

Item 11 is therefore closed, not deferred. The sudoers path is the design, and
investing in it further — F1's `timestamp_timeout`, F2's shadow check, F3's
uninstall — is no longer contingent on this decision.

The analysis below is retained because it is what closed the question, and
because the `timestamp_timeout=0` fix (F1) is what recovers the per-operation
guarantee that Authorization Services would otherwise have been needed for.

The mechanics, for the record. A root LaunchDaemon owns the vault; the client
connects over a socket in a root-owned directory.

- Payoffs: no sudoers entry, so the shared-parser coupling vanishes; no grant
  pinned to a binary path, which also dissolves F2 and F4; authorization
  becomes ours rather than borrowed.
- Costs: a long-running root daemon is more attack surface than a short-lived
  exec'd broker. **Peer identification is the hard part** — uid via
  `LOCAL_PEERCRED` is fine, but pid-based checks are racy, and the audit-token
  routes that fix that depend on XPC plus private API or Developer-ID-signed
  bundles. `/Library/LaunchDaemons` is still shared global state, though
  meaningfully better than sudoers: a broken plist from another vendor does
  not affect ours, whereas one bad sudoers file can affect the combined parse.

**Authorization Services — authenticates, does not escalate.** A custom right
(`class: user, group: admin, authenticate-user: true, shared: false,
timeout: 0`) with `AuthorizationCreate` produces the system prompt, Touch ID
capable, **per-operation and not shared with sudo's timestamp** — which is the
strongest single argument for leaving sudo. But something must still be root,
so this pairs with the daemon rather than replacing it. Full
`SMAppService`/`SMJobBless` additionally needs a Developer ID and
code-signing-based client authentication, which a Homebrew source build cannot
provide: Apple Silicon brew builds carry ad-hoc signatures with no stable
identity.

## Plan

### Phase 1 — packaging (formula only, no Rust change)

1. Add `version "0.19.1-djbclark.1"` to the formula. Homebrew currently parses
   the version as `1`, breaking upgrade detection.
2. `keg_only` the companion so only `secretspec` links (F4).
3. Rebuild: `brew uninstall secretspec && brew uninstall sudo-secretspec &&
   brew install djbclark/sudo-secretspec/sudo-secretspec && brew test …`.
   Nothing depends on the upstream formula (`brew uses --installed secretspec`
   is empty).
4. Verify `secretspec --version`, `doctor`, and that
   `/opt/homebrew/bin/secretspec` still resolves — the stayturgid wrapper
   hardcodes it, so leaving that path empty breaks it.

### Phase 2 — one companion release

These belong together: the policy change only takes effect when `install` is
re-run, and re-running `install` churns snapshot directories, so the GC fix
must ship in the same version.

5. `timestamp_timeout=0` on the client path (F1).
6. `sudo-secretspec uninstall` (F3).
7. ~~`doctor`: `CLIENT_SHADOWED` and unsafe-prefix checks (F2).~~ **Done.**
8. `doctor`: report broken neighbours in `sudoers.d`; GC rollback snapshots
   (F6). Snapshot GC is done; the `sudoers.d` neighbour report is not.

Then re-run `install` to apply the new policy and confirm `doctor` is clean.

### Separate workstream

9. Wrapper hardening — root-owned engine path, drop `source-*` to
   `_secretspec` (F5).
10. Retire the wrapper onto the Rust broker (F5).

### Decided

11. ~~launchd + Authorization Services.~~ **Rejected** (see above): macOS-only,
    and existing tools already implement that pattern. `sudo` stays the
    mechanism.

### Next

12. Extend the `sudo` path beyond macOS. Everything platform-specific is
    currently compile-time constants and `Command` calls to macOS binaries:
    `PREFIX`/`BROKER_PATH`, `dscl` for the service identity
    (`ensure_service_group`/`ensure_service_user`), `/usr/sbin/visudo`,
    `/usr/sbin/chown`, the `/private/etc` and `/private/var` spellings, and
    `check_ancestor_chain`'s `/var`, `/etc`, `/tmp` alias-root exemptions.
    Note the constraint established in F4: `PREFIX` being hardcoded is a
    *security property*, not an oversight — a per-platform constant is fine, a
    runtime-configurable one is not.
