---
schema_version: 1
handoff_id: 7168
parent_handoff_ids: [5218]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 0f5dbfa4241501d07f2db792906c4385c8f4b458
created_at: 2026-08-13T23:36:29-0400
writer: claude-code
---

# Handoff — Homebrew packaging closed out, Phase 2 started, host cache corruption found and fixed

## The Goal

Resume from parent handoff 5218, which left one blocking question: which
`secretspec` owns `/opt/homebrew/bin/secretspec`. That was expected to be a
ten-minute unblock.

It expanded three times, each time because the operator pushed on something:

1. "It's too easy to mess up" → a full design analysis of the privilege
   boundary and its packaging, written to the repo.
2. "Do we have to use the same sudoers as everything else?" → an options
   study of macOS privilege mechanisms.
3. A `cargo test` failure mid-work → discovery that a host maintenance job
   had been silently destroying Rust caches all evening.

## Where We Are

**Phase 1 (Homebrew packaging): DONE and verified.** The release blocker from
5218 is fully resolved.

**Phase 2 (companion release): 1 of 4 items done** (F6). F2, F1, F3 remain.

**Host cache corruption: diagnosed, fixed, and repaired.** This was not a
sudo-secretspec bug — it was `~/.config/system-maintainer/system_maintainer.py`.

Git state: branch `sudo-main`, HEAD `0f5dbfa`, **working tree clean**, pushed.

Commits this session:
- `aa284d8` packaging: stop linking the companion, declare the formula version
- `0f5dbfa` feat(sudo-secretspec): prune rollback snapshots on install

Tap `djbclark/homebrew-sudo-secretspec` at `70ed5c1`, pushed.

Files changed in this repo:
- `docs/design/privilege-boundary-and-packaging.md` (new — the important one)
- `packaging/homebrew/sudo-secretspec.rb`
- `sudo-secretspec-cli/src/install.rs`
- `sudo-secretspec-cli/tests/install_rollback.rs`
- `CHANGELOG.md`

Files changed **outside** this repo (host state, no version control):
- `~/.config/system-maintainer/config.toml`
- `~/.config/system-maintainer/system_maintainer.py`

## What We Tried

Chronological, including the wrong turns — these are the expensive ones.

**`keg_only` for the Homebrew formula — WRONG, caught before shipping.**
Proposed it twice in conversation before realising `keg_only` applies to the
*whole formula*, so it would also unlink `secretspec`. The stayturgid wrapper
hardcodes `SECRETSPEC_BIN=/opt/homebrew/bin/secretspec`, so that would have
broken it. Correct approach: install the companion to the keg's `libexec`,
which Homebrew does not link, and let `bin/secretspec` link normally.

**Claimed `uninstall` could not be safely tested on this host — WRONG.**
The claim rested on "the stayturgid wrapper and `_secretspec` automation depend
on the boundary." They do not. The wrapper has its own sudoers file
(`/etc/sudoers.d/secretspec`, separate from `/etc/sudoers.d/sudo-secretspec`)
and calls the engine directly; it never goes through the broker. The operator
pushed back, correctly. The uninstall/reinstall round trip **is** safe to test —
see Where We're Going.

**Ran `brew style djbclark/sudo-secretspec` and read a false pass.** That
inspects the *tapped clone* at `/opt/homebrew/Library/Taps/...`, not the edited
file in `~/src/homebrew-sudo-secretspec`. Style the file path directly. The real
run then found a genuine offence (`refute_predicate` → `refute_path_exists`).

**Assumed disk space caused the cargo corruption — ruled out.** 84GB free.

**Assumed `brew cleanup` caused the Homebrew cache corruption — wrong,
plausible, and a useful near-miss.** `brew cleanup` does prune that cache by
age and does run after every install, so it fit. The actual culprit was
`system_maintainer.py`. The log timestamps settled it.

**Deleted `~/.cargo/registry/src` before preserving forensic evidence.** Applied
the documented remedy immediately and destroyed the evidence. Recovered because
the *Homebrew* cache was independently corrupted and still intact as a specimen.
Next time: copy one damaged directory aside before repairing.

## Key Decisions

**Chosen — install the companion to `libexec`, not `bin`.** The keg copy is only
a bootstrap for the first `install`. A linked copy shadowed
`/usr/local/bin/sudo-secretspec` (PATH 11 vs 20) with a same-version,
different-hash binary (`f37c41…` vs `80ca48…`) that `doctor` cannot detect.
Added `refute_path_exists bin/"sudo-secretspec"` so a future edit cannot
silently re-link it.

**Rejected — a bespoke setuid-root helper.** Inherits sudo's whole
hostile-environment problem (argv[0], inherited fds, rlimits, signal
dispositions, controlling tty, env scrubbing). Touch ID is unavailable from a
setuid CLI. Homebrew cannot set the bit anyway. Trading a heavily audited
privileged binary for a bespoke one is strictly worse.

**Rejected — making `PREFIX` configurable so brew could own the privileged
install.** `BROKER_PATH` is compared against `current_exe()` to decide privilege
(`main.rs:110-118`); a configurable broker path is far more delicate.
`/usr/local` is root-owned, `/opt/homebrew` is admin-writable. The hardcoding is
a security property — fix packaging, not the constant.

**Deferred, operator decision — launchd daemon + Authorization Services.** Would
subsume the packaging fixes and give a per-operation auth gate we own. Costs: a
long-running root daemon, hard peer identification (pid checks are racy;
audit-token routes need XPC plus private API or Developer-ID signing, which a
Homebrew source build cannot supply — Apple Silicon brew builds are ad-hoc
signed), and losing pam_tid Touch ID unless Auth Services is adopted
deliberately. Item 11 in the design note.

**Chosen — "delete whole cache entries, never files inside them"** as the
general fix for the corruption class, implemented as a structural guard rather
than per-cache config edits.

## Evidence & Data

**Homebrew swap, verified:** keg version `0.19.1-djbclark.1` (was `1`);
`/opt/homebrew/bin/secretspec` → fork engine; `/opt/homebrew/bin/sudo-secretspec`
does not exist; `sudo-secretspec` on PATH resolves to `/usr/local/bin`;
`brew test` exit 0; `doctor` OK with the two known `LEGACY_VAULT_CLUTTER`
advisories. `brew uses --installed secretspec` was empty before uninstalling.

**Tests:** 68 passing at session start (baseline confirmed), **72 after F6**
(`install_rollback.rs` 9 → 13). `cargo check` clean.

**Cache corruption, from `~/.cache/system-maintainer/cleaner.log`:**
```
22:01:30  Cleaned ~/.cargo: 23845 items, ~270.4 MiB freed
23:00:40  Cleaned ~/Library/Caches: 23375 items, ~2474.3 MiB freed
22:02:58  Cleaned ~/src: 9 items, ~4589.9 MiB freed      <- live build state
23:01:47  Cleaned ~/src: 6 items, ~79.3 MiB freed
```
Mechanism: cargo preserves `.crate` tarball mtimes (`Dec 31 1969`,
`Jul 23 2006`), so every extracted file is expired from birth against any
cutoff. `.cargo-ok` is written fresh at extraction and always survives. Cargo
treats that marker as proof of a complete extraction, so it never re-extracts
and every build dies on `failed to read .../Cargo.toml`. Measured: 479/479 dirs
hollow in `~/.cargo`, 474/474 in the Homebrew cache, the only surviving files
being the 474 `.cargo-ok` markers stamped `Aug 13 22:03`/`22:09`.

**Time bomb defused:** the `~/.rustup` rule (`toolchains/**/*`, 48h, only
`toolchains/*/bin/*` excluded) would have deleted `lib/` and `lib/rustlib/`
around **11:00 on Aug 15**, leaving a `rustc` that cannot compile. Toolchain
files are stamped `Aug 13 10:55`.

**Guard proven end-to-end:** replaying the *original* buggy entry (`~/.cargo`,
`patterns = ["registry/**/*"]`, 48h) against the live cache now reports
`Protected 9575 files inside indivisible cache entries` and deletes 0. Full
dry-run: 0 toolchain targets, 0 intra-crate deletes, 0 hits on this repo's
`target/`.

**Damage repaired:** removed the hollow Homebrew src tree; 474 `.crate` tarballs
retained so cargo re-extracts offline. `~/.cargo` re-extracted clean (0 broken).

## Operator Feedback

- Pushed back on "we can't test uninstall": *"It's fine if it doesn't work for a
  few minutes."* They were right; the objection was based on a wrong dependency
  claim. **Phase 2 should include the real uninstall/reinstall round trip.**
- Wanted the pre-`brew install` autoremove of rust/llvm/z3 prevented. Fixed via
  `installed_on_request` on `rust` alone (transitively protects llvm/z3/libgit2).
- Chose to do Phase 2 in the proposed order rather than handing off first.
- Directed that Hermes be consulted about the host automation, then that all
  findings be fixed and reported back.
- Standing repo policy (CLAUDE.md): all work commits directly to `sudo-main`,
  no branches, no PRs.

## Where We're Going

1. **THE NEXT ACTION — F2: add `CLIENT_SHADOWED` and unsafe-prefix findings to
   `doctor`** in `sudo-secretspec-cli/src/drift.rs`. Resolve `sudo-secretspec`
   through `PATH`; if it is not `layout.client`, report it (advisory when
   versions match, failure when they do not). Reuse
   `validate_protected_ancestors()` (`install.rs:425`) for the prefix predicate.
2. **F1**: add `Defaults!/usr/local/bin/sudo-secretspec timestamp_timeout=0` to
   `sudoers_text()` (`install.rs`, now ~line 570 after the F6 insert). The Touch
   ID gate is currently not per-operation — it rides sudo's shared 5-minute tty
   timestamp.
3. **F3**: `sudo-secretspec uninstall`. Ownership manifest already exists
   (`installed_artifacts()`, `install.rs:72`). Sudoers removed first,
   `--purge-vault` opt-in, `--remove-service-user` opt-in, add `Uninstall` to the
   non-broker guard at `main.rs:126`, split a testable `plan_uninstall()` the way
   `rollback::plan_restore` is split, and ship `--dry-run`.
4. **Ship the release**, then re-run `install --adopt-existing` to apply the new
   sudoers policy (needs Touch ID), then `doctor`.
5. **Live uninstall round trip** (operator explicitly wants this tested; inputs
   verified recoverable — the installed declarations are byte-identical to
   `~/ops/site-private/secretspec.toml.example`, `ed6a30b6…`).
6. Open decision: item 11 (launchd + Authorization Services) in the design note.
7. Not started, separate workstream: F5 wrapper hardening.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3            # expect 0f5dbfa at HEAD, clean tree
cargo test -p sudo-secretspec-cli   # expect 72 passing

# Read this first — every finding has file:line evidence:
#   docs/design/privilege-boundary-and-packaging.md
# F6 is done; F2, F1, F3 remain (Phase 2 section of that note).
```

Live uninstall round trip, when you get to step 5:

```bash
sudo /usr/local/bin/sudo-secretspec uninstall --dry-run   # inspect first
sudo /usr/local/bin/sudo-secretspec uninstall
sudo /opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec install \
  --adopt-existing \
  --declarations ~/ops/site-private/secretspec.toml.example \
  --vault /var/db/stayturgid-secrets \
  --service-user _secretspec --service-group staff
/usr/local/bin/sudo-secretspec doctor
```

The restore must run from the brew `libexec` bootstrap, because uninstall
removes `/usr/local/bin/sudo-secretspec`. That also exercises the new packaging
path end to end.

## Gotchas for the next session

- **`brew style <tap-name>` inspects the tapped clone, not your edit.** Pass the
  file path.
- **The tapped clone is a separate checkout.** After pushing the tap, run
  `git -C "$(brew --repo djbclark/sudo-secretspec)" pull --ff-only`.
- **`brew uninstall` cascades an autoremove** of build-only deps. `rust` is now
  pinned via `installed_on_request`; `brew autoremove --dry-run` is empty.
- **If cargo builds fail with `failed to read .../Cargo.toml`:** the maintainer
  is corrupting caches again. Check
  `ls <crate-dir>` — only `.cargo-ok` present means hollow. Repair by removing
  the `registry/src/index.crates.io-*` tree; tarballs in `registry/cache/`
  re-extract offline. Then find out why the guard did not hold.
- **Hermes edits `~/.config/system-maintainer/` concurrently.** It widened the
  guard's marker set on its own initiative during this session (adding `.lock`,
  `CACHEDIR.TAG`, and `npm`/`cargo` as atomic dir names). That fails safe but now
  blocks 1492 file-level deletions per run, so reclaim may effectively stop.
  Worth measuring. Coordinate before editing that file.
- **`~/.config/system-maintainer/` is still not in git** and there is no backup
  of the pre-fix config.
- **Unrelated, pre-existing:** `/etc/sudoers.d/yabai` has a mode that makes sudo
  ignore it. Fix: `sudo chmod 0440 /etc/sudoers.d/yabai`.
- **`PROMPT-SECREV.md` remains unexecuted** — delegated to a separate AI by
  operator choice, not oversight.
