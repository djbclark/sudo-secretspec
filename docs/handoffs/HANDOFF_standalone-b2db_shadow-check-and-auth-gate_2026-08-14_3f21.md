---
schema_version: 1
handoff_id: 3f21
parent_handoff_ids: [7168]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: fc92c7e46f46d7cb85044b87a31dffa7be7f1b57
created_at: 2026-08-14T00:09:52-0400
writer: claude-code
---

# Handoff — F2 client-shadow check and F1 per-operation auth gate

## The Goal

Finish Phase 2 of `docs/design/privilege-boundary-and-packaging.md` so the
companion release can be cut. Phase 2 has four items; two were already done
when this session started (F6 snapshot GC in `0f5dbfa`, plus all of Phase 1
packaging). This session was asked to start on **F2**, then continued to **F1**.

**F3 (uninstall) is the only Phase 2 item left**, and it is the largest.

## Where We Are

Branch `sudo-main`, HEAD `fc92c7e`, **working tree clean**, everything pushed
to `origin/sudo-main`.

Three commits this session:

| SHA | What |
| --- | --- |
| `0ec6d40` | F2 — `doctor` checks which client would actually run |
| `25fe151` | docs — F1 demonstrated rather than inferred; F2 root-verification recorded |
| `fc92c7e` | F1 — `timestamp_timeout=0` makes the auth gate per-operation |

Files changed across the session (`git diff --stat b1c4925..HEAD`):

```
 CHANGELOG.md                                    |  20 ++
 FORK-AI.md                                      |   6 +
 docs/design/privilege-boundary-and-packaging.md | 172 +++++++++--
 sudo-secretspec-cli/src/drift.rs                | 392 +++++++++++++++++++++++-
 sudo-secretspec-cli/src/install.rs              |  47 ++-
 sudo-secretspec-cli/src/lib.rs                  |   2 +-
 sudo-secretspec-cli/src/main.rs                 |  98 +++++-
 sudo-secretspec-cli/tests/drift.rs              |  55 ++++
 sudo-secretspec-cli/tests/install_rollback.rs   |  70 ++++-
 sudo-secretspec/AI-GUIDANCE.md                  |   3 +-
 10 files changed, 808 insertions(+), 57 deletions(-)
```

Phase 2 status: **3 of 4 done.** F6 `0f5dbfa`, F2 `0ec6d40`, F1 `fc92c7e`.
F3 remains. Design-doc item 11 is **closed** (see Key Decisions) and a new
item 12 was added for cross-platform `sudo` support.

### What F2 shipped

`check_client_shadowing` in `sudo-secretspec-cli/src/drift.rs`, with two codes:

- **`CLIENT_SHADOWED`** — non-advisory, fails `doctor`. Emitted when another
  `sudo-secretspec` wins the caller's `PATH`, or its bytes differ, or it is
  reachable through a directory that is not root-owned or is group/world
  -writable.
- **`CLIENT_DUPLICATE`** — advisory. A byte-identical copy in a root-owned
  search directory. Added to `ADVISORY_CODES`.

Supporting pieces:

- `InspectOptions { caller_path: Option<OsString> }`; `inspect()` signature
  changed from `inspect(&Layout)` to `inspect(&Layout, &InspectOptions)`.
  Exported from `lib.rs`. Only one non-test caller existed (`main.rs`).
- `SEARCH_DIRS` — fixed list of ten conventional macOS search directories,
  always scanned.
- `search_dirs()`, `path_winner()`, `unsafe_prefix()`, `classify_candidate()`
  — `classify_candidate` is pure (takes `wins: bool` and
  `unsafe_dir: Option<&Path>` as parameters) precisely so every severity branch
  is testable without root and without depending on what is installed on the
  host.
- `install.rs`: `validate_protected_ancestors()` refactored into `is_real_dir`
  + `is_root_only_dir` + `pub(crate) is_protected_dir`, so the installer and
  the drift check share one predicate. Existing error strings preserved
  (`unsafe protected directory {dir}` vs `... metadata {dir}`).
- `main.rs`: hidden `--caller-path` flag on `Doctor`; the unprivileged client
  fills it from its own `PATH` before elevating.

### What F1 shipped

One line added to `sudoers_text()` in `install.rs`:

```
Defaults!/usr/local/bin/sudo-secretspec timestamp_timeout=0
```

`sudoers_text` was made `pub` so a test can run the real generated policy
through the real `visudo`.

## What We Tried

Chronological, including the things that did not work.

1. **Resolving `PATH` inside the privileged `doctor` — rejected before
   writing it.** `doctor` re-execs itself as root via
   `sudo -n <broker> doctor`, and our own policy sets
   `secure_path=/usr/bin:/bin:/usr/sbin:/sbin`. The privileged process
   therefore never sees the `PATH` whose shadowing is the entire question, and
   a naive `which`-style resolution would have silently reported "not found"
   forever. This is the single most important trap in F2 and it is not obvious
   from reading the finding description. Fixed by passing `--caller-path`.

2. **Deciding severity by comparing `--version` — rejected.** This is what the
   original F2 sketch in the design doc called for. It requires *executing* a
   binary we just concluded might not be ours. Replaced with content-hash
   comparison. Related hazard found while writing it: `fs::read` on a fifo left
   in a caller-supplied directory would block the root process, so the
   classifier only hashes regular files (`fs::metadata(...).is_file()`, which
   stats rather than opens) and treats anything else as differing.

3. **A separate layout-side `UNSAFE_PREFIX` finding — rejected as duplicate.**
   The design doc asked for an unsafe-prefix check on privileged components.
   Reading `check_ancestor_chain` showed it already walks each installed path's
   ancestors and emits `REMOVABLE_ANCESTOR` for exactly the
   not-root-owned-or-group/world-writable condition. Verified by tracing
   `check_ancestor_chain(&layout.engine)` → `/usr/local/libexec`, `/usr/local`,
   `/usr`. Adding a second check would only have produced duplicate findings.
   The predicate was applied to the *shadow's* directory instead, which is what
   was genuinely missing.

4. **Version skew — a real runtime failure, found by smoke-testing.** Running
   the freshly built client against the installed (pre-F2) broker produced:

   ```
   error: unexpected argument '--caller-path' found
   Usage: sudo-secretspec doctor --json
   ```

   A newer client meets an older installed broker whenever brew hands over a
   new bootstrap binary before `install` replaces the pair — so an
   argument-parsing error would have stood in for a health check. Fixed in
   `main.rs`: when `caller_path` is set, the first elevated attempt is run with
   `.output()` (captured, so the clap error is not printed), and on exit code 2
   it retries without the flag. `doctor` itself exits 0 or 1, so a 2 from that
   path means the broker did not understand the request. Verified live: new
   client + old broker → `doctor: OK` with the two known advisories.

5. **A wrong claim I made and corrected.** After running several `sudo`
   commands across separate Bash tool calls, I implied they were riding one
   shared cached timestamp — evidence for F1. That was wrong: `sudo -n true`
   afterwards reported "a password is required". Timestamps here are **per-tty**,
   and each tool call gets its own tty, so each authenticated independently.
   F1 can only be observed **within a single shell invocation**. Recorded in
   the Tier 1 log so it is not rediscovered.

6. **A commit message that overclaimed.** `25fe151`'s first draft said it
   recorded the F2 root verification, which I had not actually written into the
   design doc. Caught before pushing; added the section and amended.

7. **`clippy` regressions, both fixed.** Baseline for this crate is **17**
   warnings. A nested `if let` inside `check_client_shadowing` and another in
   `main.rs`'s command builder each added one; collapsed to `matches!(...)` and
   `if let (true, Some(path)) = (..)` respectively. Final count is back to 17.

8. **`cargo fmt` had to catch up.** `tests/install_rollback.rs` was committed
   unformatted by the F6 commit `0f5dbfa`; `cargo fmt` reformatted it as a side
   effect. Kept, since `pre-commit run -a` would flag it anyway. It accounts for
   the otherwise-surprising ~35 lines of diff in a file this session did not
   otherwise touch.

## Key Decisions

**Item 11 — launchd + Authorization Services: REJECTED, closed not deferred.**
Operator decision, verbatim reasoning: launchd is macOS-specific, and someone
else has already written tools that follow that pattern, documented in an
issue — so if that pattern turns out to be right, the move is to adopt those
tools rather than reimplement. The direction is the opposite one: **keep `sudo`
and extend it to other operating systems and distributions**, because `sudo` is
the portable common denominator. The prior analysis is retained in the design
doc because it is what closed the question, and because F1's fix is what
recovers the per-operation guarantee that Authorization Services would
otherwise have been needed for.

New **item 12** records what is macOS-specific today and would need porting:
`PREFIX`/`BROKER_PATH` constants, `dscl` in `ensure_service_group` /
`ensure_service_user`, `/usr/sbin/visudo`, `/usr/sbin/chown`, the
`/private/etc` and `/private/var` spellings, and `check_ancestor_chain`'s
`/var`, `/etc`, `/tmp` alias-root exemptions. Constraint carried over from F4:
**`PREFIX` being hardcoded is a security property, not an oversight** — a
per-platform constant is fine, a runtime-configurable one is not, because
`BROKER_PATH` is compared against `current_exe()` to decide privilege.

**F2 severity by content hash, not by version.** See What We Tried #2.

**Caller-supplied `PATH` can only widen the scan, never narrow it.** The fixed
`SEARCH_DIRS` list is always scanned, so a doctored `--caller-path` can add
findings but cannot hide one. Relative `PATH` entries are dropped, because
resolving one inside a root process would depend on a working directory the
caller also controls. This is why it was acceptable to let caller-supplied data
influence `Report.ok` at all.

**A copy under a writable prefix fails even when it loses `PATH` order.**
Losing is an accident of ordering; write access is a standing ability to swap
the bytes.

**F1 verified before shipping, not after.** `visudo -c` proves syntax only.
Whether sudo honors a **command-scoped** `timestamp_timeout` at authentication
time is the load-bearing question and sudoers(5) does not state it either way.
Shipping on inference would have repeated the exact mistake F1 documents. See
Evidence.

**F3 sudoers scope — answered for the operator, recorded in the design doc.**
The operator asked whether "sudoers removed first" meant only the lines we
added, out of concern for a pre-existing or shared file. Answer: narrower than
that. Our policy is **not** lines appended to a shared file — `install` writes a
dedicated drop-in `/private/etc/sudoers.d/sudo-secretspec` (`SUDOERS_PATH`,
`install.rs:18`), listed in `installed_artifacts()` at mode `0440`. Uninstall
removes that one file, never `/etc/sudoers`, never another vendor's drop-in.
Additional requirement added: uninstall must hash the file against the install
manifest before unlinking and leave it in place with a warning if it is not
ours, because the path is predictable and a file sitting there is not proof we
wrote it.

## Evidence & Data

**Tests.** 72 at session start → **86 at HEAD**, all passing. F2 added 13
(11 unit in `drift.rs`, 2 integration in `tests/drift.rs`); F1 added 1
(`the_shipped_policy_parses_and_gates_boundary_lifecycle`). Breakdown at HEAD:
37 lib + 19 + 3 + 3 + 10 drift + 14 install_rollback + 0 doctests.

**Lint/format.** `cargo clippy -p sudo-secretspec-cli --all-targets` = 17
warnings, equal to the pre-session baseline (verified by `git stash`).
`cargo fmt --all -- --check` clean.

**F2 verified as root against the live install, 2026-08-14.**

- Clean run: `sudo ./target/debug/sudo-secretspec doctor --json --caller-path "$PATH"`
  → `ok: true`, **zero `CLIENT_*` findings**, only the two known
  `LEGACY_VAULT_CLUTTER` advisories for
  `/var/db/stayturgid-secrets/.local` and `.ansible`.
- Positive run with a decoy of differing bytes prepended to `--caller-path`:

  ```
  CLIENT_SHADOWED | advisory: False
  detail: this resolves ahead of the installed client
          /usr/local/bin/sudo-secretspec in the caller's PATH and its bytes differ
  ```

  `ok: false`, non-JSON run exits **1**. Decoy removed afterwards.

**No shadow exists on this host.** Scanning all ten `SEARCH_DIRS` plus every
entry of the real `PATH` found exactly one copy:
`/usr/local/bin/sudo-secretspec`, sha256 prefix `80ca48a34d9f97fa`. This also
confirms Phase 1 removed the `/opt/homebrew/bin` copy (`f37c41…`) that
originally motivated F2.

**F1 demonstrated (the problem), 2026-08-14.** One tty, no prior timestamp:

```
$ sudo true                                                  # unrelated; authenticates
$ sudo -n /usr/local/bin/sudo-secretspec install --dry-run --non-interactive
would install sudo-secretspec ...                            # exit 0
```

`-n` makes sudo refuse rather than prompt. It did not refuse — an unrelated
command paid for boundary lifecycle access. Nothing was mutated (`--dry-run`
returns before any write) and the timestamp was cleared with `sudo -k` after.

**F1 mechanism probed (the fix), 2026-08-14.** Operator explicitly approved
writing a throwaway drop-in. `/etc/sudoers.d/zz-sudo-secretspec-probe`
containing only `Defaults!/usr/bin/true timestamp_timeout=0`, `visudo -c`
checked before installing, installed `0440 root:wheel`, removed immediately
after via a bash `trap ... EXIT`:

```
TEST:    sudo -n /usr/bin/true  -> "sudo: a password is required"   (gated)
CONTROL: sudo -n /bin/ls        -> exit 0                           (same timestamp, still valid)
```

The **control is what makes it conclusive** — it proves the timestamp was still
valid, so the refusal was command-scoped rather than expiry. sudo **1.9.17p2**.
Cleanup verified: probe absent, `/etc/sudoers.d` back to its original three
files, `sudo -k` run.

**`/etc/sudoers.d` contents on this host** (relevant to F3):

```
-r--r-----  root wheel  302  Aug 11 10:08  secretspec        <- legacy bash wrapper, NOT ours
-r--r-----  root wheel  329  Aug 13 21:19  sudo-secretspec   <- ours
-rw-r-----  root wheel  136  Jul  1 07:57  yabai             <- third party, wrong mode
```

`visudo -c` reports `/private/etc/sudoers.d/yabai: bad permissions, should be
mode 0440` on every run. Pre-existing, unrelated, untouched.

**Nothing in the tree invokes `sudo <client>` non-interactively**, so F1
introduces no regression for agents — verified by grepping `*.md *.sh *.rs *.py
*.rb` for `sudo /usr/local/bin/sudo-secretspec`, `sudo sudo-secretspec`, and
`sudo -n /usr/local/bin`. The only hits are a `broker.rs` usage string and the
design doc itself.

## Operator Feedback

- **On F3 scoping, unprompted mid-turn:** "When you say 'sudoers removed first'
  I assume you mean only the lines we added, correct? We don't want to remove
  the whole file if it was there before us or still being used by others."
  Answered and recorded in the design doc (see Key Decisions). The concern was
  well-founded even though the mechanism differs — there really is a foreign
  `secretspec` drop-in and a foreign `yabai` drop-in in that directory.
- **On item 11:** "we are never going to use launchd, but we do want to add
  support via sudo for other operating systems/distros. Reasons: launchd is
  mac-specific, and also someone else has already written tools that follow
  that pattern, they are doc'ed in an issue. So we will just use other tools if
  we decide that is the pattern to go for."
- **On live testing:** "I am here, you can do a test that requires my
  fingerprint." Then explicitly approved the throwaway sudoers probe over the
  alternatives of shipping on documentation or running the real install early.
- Standing, from `CLAUDE.md`: all downstream work commits directly to
  `sudo-main`, no branches, no PRs; keep `CHANGELOG.md` Unreleased entries
  user-facing; privileged install stays explicit.

## Where We're Going

1. **NEXT ACTION — F3: `sudo-secretspec uninstall`.** The last Phase 2 item.
   Split out a testable `plan_uninstall()` for root-free testing, the way
   `rollback::plan_restore` and `install::plan_prune` already are. Add
   `--dry-run`. The ownership manifest already exists: `installed_artifacts()`
   at `install.rs:78`. Policy decisions, all already settled in the design doc's
   F3 section:
   - Sudoers **first**, then binaries — close the grant before removing the
     path it names.
   - Only our own dedicated drop-in `/private/etc/sudoers.d/sudo-secretspec`.
     Never `/etc/sudoers`; never the foreign `secretspec` or `yabai` drop-ins.
     Hash it against the install manifest first; leave it with a warning if it
     is not ours.
   - Never auto-delete the vault — `--purge-vault` opt-in with its own
     confirmation.
   - `--remove-service-user` opt-in; `_secretspec` has other dependents,
     including the stayturgid wrapper.
   - Add `Uninstall` to the non-broker guard at `main.rs:126` so it is refused
     through the NOPASSWD path.
2. Update `CHANGELOG.md` Unreleased with a user-facing entry for uninstall.
3. Ship the release, then re-run
   `sudo /usr/local/bin/sudo-secretspec install --adopt-existing` to apply the
   new policy (needs Touch ID), then `doctor`.
4. Confirm F1 live after that install — see Quick Start. Must now be **refused**
   where it previously succeeded.
5. Confirm F2 live after that install — see Quick Start. Expect zero `CLIENT_*`
   findings.
6. **Operator explicitly wants the live uninstall round trip tested.** Restore
   runs from `/opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec`,
   because uninstall removes `/usr/local/bin/sudo-secretspec`. Commands are in
   parent handoff `7168`'s Quick Start.
7. Remaining Phase 2 leftover from item 8: `doctor` does not yet report broken
   neighbours in `sudoers.d`. The `yabai` mode problem is a live example that
   `visudo -c` already surfaces but `doctor` does not attribute.

Out of scope but open: F5 wrapper hardening / retirement (separate
workstream, items 9–10); item 12 cross-platform `sudo` support.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -4            # expect fc92c7e at HEAD, clean tree, branch sudo-main

# Read in this order:
#   docs/design/privilege-boundary-and-packaging.md   <- F3 section has the full spec
#   docs/handoffs/HANDOFF_standalone-b2db_packaging-cache-corruption_2026-08-13_7168.md

# Build and test
cargo test -p sudo-secretspec-cli          # expect 86 passing
cargo clippy -p sudo-secretspec-cli --all-targets 2>&1 | grep -c '^warning:'   # expect 17
cargo fmt --all -- --check

# Prove no client shadow exists, no root needed
for d in /usr/local/bin /usr/local/sbin /opt/homebrew/bin /opt/homebrew/sbin \
         /opt/local/bin /opt/local/sbin /usr/bin /usr/sbin /bin /sbin \
         $(echo "$PATH" | tr ':' ' '); do
  [ -e "$d/sudo-secretspec" ] && shasum -a 256 "$d/sudo-secretspec"
done | sort -u
# expect exactly one: /usr/local/bin/sudo-secretspec  80ca48a3...

# --- after the release + `install --adopt-existing` only ---

# F1 live check. MUST be refused now; it succeeded before fc92c7e.
# Single shell invocation only: sudo timestamps are per-tty here.
sudo -k
sudo /bin/ls / >/dev/null                                    # authenticate, unrelated command
sudo -n /usr/local/bin/sudo-secretspec install --dry-run --non-interactive   # expect REFUSED
sudo -n /bin/ls / >/dev/null && echo "control OK: timestamp still valid"     # else inconclusive
sudo -k

# F2 live check.
sudo /usr/local/bin/sudo-secretspec doctor --json --caller-path "$PATH"      # zero CLIENT_* findings
# then, positive control:
D=$(mktemp -d); printf 'decoy' > "$D/sudo-secretspec"; chmod 755 "$D/sudo-secretspec"
sudo /usr/local/bin/sudo-secretspec doctor --caller-path "$D:$PATH"          # expect CLIENT_SHADOWED, exit 1
rm -rf "$D"
```

**Gotcha carried forward from `7168`:** if cargo builds fail with
`failed to read .../Cargo.toml`, the host maintainer script is corrupting
caches again. A crate directory containing only `.cargo-ok` is hollow. Repair
with `rm -rf` on the `registry/src/index.crates.io-*` tree; the tarballs in
`registry/cache/` re-extract offline. Then find out why the structural guard
did not hold.
