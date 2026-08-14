---
schema_version: 1
handoff_id: 4e3a
parent_handoff_ids: [13e2]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: d51543c0cd73a5b6c7d1cee0e79d867b0043898f
created_at: 2026-08-14T07:06:57-0400
writer: claude-code
---

# Handoff — sudoers.d neighbour report (design-doc item 8), Phase 2 closed

## The Goal

Clear the four leftover items the previous handoff (`13e2`) listed as optional
follow-ups, which the operator asked for in one instruction: *"Fix all the
gotchas. For 1 fix it. For 2 delete it. For 3 might as well do that. Do 4."*

1. `/etc/sudoers.d/yabai` had a mode that made `visudo -c` fail on every install.
2. `~/.local/state/secretspec/audit.log` — 606 KB of superseded engine access log
   the pre-fix root broker had written into the caller's home.
3. The three items still open in `docs/design/privilege-boundary-and-packaging.md`:
   item 8 (`doctor` reports broken `sudoers.d` neighbours), items 9–10 (F5 legacy
   wrapper), item 12 (cross-platform `sudo`).
4. Run the live boundary smoke test.

## Where We Are

**HEAD `d51543c` on `sudo-main`, tree clean, pushed.** Tests 124 → **137**,
clippy held at the baseline **17**, `cargo fmt --check` clean.

| Item | State |
|---|---|
| 1 — yabai mode | **Done.** `sudo chmod 0440`; all four drop-ins now `parsed OK`. |
| 2 — stale audit log | **Done.** Deleted (1380 JSONL records). Directory kept. |
| 3 — item 8 | **Shipped**, two commits. **Phase 2 is now complete.** |
| 3 — items 9–10 | **Not started, blocked.** Cross-repo; needs an operator decision. |
| 3 — item 12 | **Not started.** Investigated only; findings below. |
| 4 — smoke test | **Done.** Exit 0, `43 found, 0 missing, 7 optional`, profile `default`. |

Two commits this session, both on `sudo-main`, both pushed:

- `dbdad69` — `feat(sudo-secretspec): report sudoers.d neighbours sudo silently ignores`
- `d51543c` — `fix(sudo-secretspec): sudo and visudo disagree; report them separately`

The second **corrects the first**. Read the next section before touching this
code; the correction is the single most valuable thing in this document.

Files changed: `sudo-secretspec-cli/src/drift.rs` (implementation + tests),
`CHANGELOG.md` (Unreleased), `docs/design/privilege-boundary-and-packaging.md`
(item 8 closed, plus a correction to the "Should we stop using sudoers?"
section). Nothing else. **Not released** — this is unreleased work sitting on
top of `v0.19.1-djbclark.3`, and the installed host binaries still predate it.

## What We Tried

Chronological, because the order is the lesson. The question was: *what exactly
makes sudo decline a `sudoers.d` drop-in?* Three probes, two of them wrong.

### Probe 1 — `visudo -c -f <file>` on an isolated tree. Measured nothing.

Built a throwaway sudoers tree under a root-owned temp dir, planted a subject
file at 0640, 0444, 0460, 0442, wrong owner, wrong group, and ran
`visudo -c -f`. **Every single one returned `parsed OK`.** Only genuinely
broken syntax failed.

Why: with `-f`, visudo checks syntax and applies **none** of the ownership or
mode rules. The probe answered a question nobody asked. Discarded.

### Probe 2 — unscoped `visudo -c` on the live tree. Measured the wrong authority.

This produced a clean, self-consistent, entirely plausible rule:

- mode must be **exactly 0440** — `0400` and `0444` both refused
- owner must be uid 0 **and** gid 0 — `root:staff` refused
- `#includedir` skips names containing `.` or ending `~`, before any stat
- symlinks are **followed** (a link to a root:wheel 0440 file loads fine)

I shipped `dbdad69` on this. The last two facts are correct and survived. The
first two are **visudo's** opinion, not sudo's, and the implementation asserted
them as "sudo will not apply this drop-in." That claim was false.

The tell was there and I read past it: the message is
`bad permissions, should be mode 0440`, which is visudo's `check_mode`
comparing for equality. Nothing about that output is evidence of what sudo's
own loader does at authentication time.

### Probe 3 — plant a live rule and observe. This is the one that works.

```bash
# in /etc/sudoers.d/<throwaway>
djbclark ALL=(root) NOPASSWD: /usr/bin/true
# then, as the target user:
sudo -u djbclark -H /usr/bin/sudo -k -n /usr/bin/true; echo $?
```

`-k` invalidates the timestamp, `-n` refuses to prompt, so **exit 0 can only
mean a NOPASSWD rule matched — i.e. sudo actually read the file.** A control
run with no drop-in present returns 1, which is what makes it conclusive rather
than an artifact.

Result on sudo 1.9.17p2:

| State | sudo reads it | `visudo -c` accepts it |
|---|---|---|
| `root:wheel 0440` | yes | yes |
| `root:wheel 0640` | **yes** | no |
| `root:wheel 0444` / `0400` | **yes** | no |
| `root:wheel 0460` (group-writable, gid 0) | **yes** | no |
| `root:staff 0440` | **yes** | no |
| `root:staff 0460` / `0420` | no | no |
| `root:wheel 0442` (world-writable) | no | no |
| `djbclark:wheel 0440` | no | no |
| name contains `.` or ends `~` | no | *not mentioned* |
| dangling symlink | no | *not mentioned* |

So sudo's loader (`sudo_secure_file`) asks only that the file be root-owned and
unwritable by anyone who is not root: not world-writable, and not
group-writable unless the group is gid 0. `visudo -c` is strictly tighter.

**Consequence that reframes the whole item: `/etc/sudoers.d/yabai` was never
being ignored.** Its rules were in force at mode 0640 the entire time. What was
actually broken is that `visudo -c` fails over it — and `visudo -c` validates
the whole directory at once, which is exactly why `install.rs:270-292` and
`rollback.rs:104-131` scope their own check to a single file, and exactly what
the "problem elsewhere in the sudoers configuration" warning during install was
reporting. The design doc asserted the wrong thing in two separate places; both
are corrected in `d51543c`, with the probe table recorded inline.

### Smaller things that cost time

- **The symlink instinct was wrong.** `drift.rs` uses `symlink_metadata`
  almost everywhere, deliberately — a fifo or link at an owned path must never
  be followed. Applying that idiom here would have reported every symlinked
  drop-in as broken, because sudo opens the entry and stats the descriptor.
  Caught by probe before shipping, and the departure is documented at the
  function.
- Test helper named `classify` collided with the existing `classify` wrapper for
  `classify_candidate`; renamed to `neighbour`.
- The `sed`/regex used for that rename only matched literal-digit call sites and
  missed the one inside a `for mode in [...]` loop — one compile error.

## Key Decisions

**Three codes, not one boolean.** Sudo and visudo disagreeing means two failures
with different consequences and different fixes, plus a third that is neither:

- `SUDOERS_NEIGHBOUR_IGNORED` — sudo will not read it; the rules are dead.
- `SUDOERS_NEIGHBOUR_SKIPPED` — `#includedir` skips the name; rules are dead and
  the fix is a **rename**, not a `chmod`. Split out precisely so the operator is
  not sent to change permissions on a file whose permissions are irrelevant.
- `SUDOERS_NEIGHBOUR_VISUDO_REJECTED` — sudo applies it, but `visudo -c` fails
  over it, breaking the syntax check for every tool on the host.

**All three advisory**, for two independent reasons — the general one (agents are
told to treat a `doctor` failure as a hard stop, so a permanent non-advisory
finding wedges automation forever) and the specific one (these files belong to
other vendors; `uninstall` already refuses to touch a neighbour's drop-in, so
this project cannot clear what it reports). A test pins all three as advisory so
a future edit cannot quietly promote one.

**`fs::metadata`, not `symlink_metadata`** — a deliberate, documented departure
from the module's idiom. The job here is to *model what sudo does*, not to decide
whether a path is trustworthy.

**Rejected:**

- *Repairing a broken neighbour.* Report only. Not our file.
- *Judging a neighbour's contents.* What another vendor grants is their business;
  whether sudo is reading it at all is diagnosable fact. Scope held to the latter.
- *Reporting our own drop-in here.* Already judged by `check_protected_file` at
  the same 0440 root:wheel expectation; reporting it twice would double every
  real finding.
- *Reporting dotfiles.* `.DS_Store` is on every macOS host and is not an
  attempted rule.

**Stable sort** on the directory listing — `read_dir` order is unspecified, and
an unstable report is noise for anything diffing `doctor --json` between runs.

**Items 9–10 deliberately not started.** The wrapper source is
`~/ops/stayturgid/control/bin/stayturgid-secretspec-wrapper.sh` (verified
byte-identical to the live `/usr/local/libexec` copy) — it belongs to
**stayturgid, not this repo**. Per `~/CLAUDE.md`, `~/ops` is deploy-only, so
that work needs a task workspace under `~/src/ops-worktrees/`, a PR, and a
coordinated `ops-vX.Y.Z` release across all three repos. Starting it from here
would have violated the deploy-checkout policy.

## Evidence & Data

**Tests.** 124 → 135 (`dbdad69`) → **137** (`d51543c`, after the wrong-rule tests
were replaced). `cargo clippy -p sudo-secretspec-cli --all-targets` = 17
warnings, unchanged baseline. `cargo fmt --check` clean.

**Live verification, as root against the real `/etc/sudoers.d`**, five planted
subjects each granting nothing, each removed by an `EXIT` trap. All five
classified correctly and `report.ok` stayed `true`:

```
SUDOERS_NEIGHBOUR_IGNORED          .../zzv2-dangle    :: cannot be resolved (dangling symlink or unreadable)
SUDOERS_NEIGHBOUR_IGNORED          .../zzv2-dead      :: world-writable (mode 0442)
SUDOERS_NEIGHBOUR_VISUDO_REJECTED  .../zzv2-live0640  :: mode is 0640 rather than exactly 0440
SUDOERS_NEIGHBOUR_IGNORED          .../zzv2-notroot   :: owned by djbclark rather than root
SUDOERS_NEIGHBOUR_SKIPPED          .../zzv2-skip.conf :: name contains `.` or ends in `~`
```

Baseline before and after planting: no neighbour findings, `report.ok = true`.
Post-cleanup `visudo -c`: all four real drop-ins `parsed OK`.

**Item 1.** `/etc/sudoers.d/yabai` was `-rw-r-----` (0640), now `-r--r-----`
(0440). `sudo /usr/sbin/visudo -c` exits 0 with all four files `parsed OK`.

**Item 2.** `~/.local/state/secretspec/audit.log` — 606303 bytes, 1380 JSONL
records, fields `[action, actor, error_kind, id, key, outcome, profile, project,
reason, seq, session_id, ts, v, version]`. Confirmed no value field before
deleting. Directory retained (mode 0700, now empty).

**Item 4.** `sudo -n /usr/local/libexec/sudo-secretspec __broker source-check
--client unknown --reason-sha256 $(printf 'smoke test' | shasum -a 256 | cut -d' ' -f1)`
→ exit 0, `Summary: 43 found, 0 missing, 7 optional`, echoing `profile: default`.

**Item 12 reconnaissance** (operator asked directly; nothing was changed):

- **No Linux packaging exists anywhere in the tree for the companion** — no
  `.deb`, `.rpm`, `PKGBUILD`, `APKBUILD`, nixpkgs derivation, Flatpak or Snap.
  The only packaging is `packaging/homebrew/sudo-secretspec.rb`.
- The companion is excluded from both non-macOS CI legs:
  `.github/workflows/test.yml:75` (Linux) and `:154` (Windows) both pass
  `--exclude sudo-secretspec-cli`; `sudo-release.yml` is `runs-on: macos-15` and
  is the only place its suite runs.
- The **upstream engine** does ship Linux binaries — `dist-workspace.toml`
  targets `aarch64-unknown-linux-gnu` and `x86_64-unknown-linux-gnu` — but that
  is cargo-dist tarballs plus a shell installer, not distro packaging, and does
  not cover the companion. `devenv.nix` is a dev shell, not a package.
- macOS coupling in `sudo-secretspec-cli/src/`, by occurrence count:
  `/usr/local` ×28, `/private/{etc,var}` ×17, `dscl` ×8, `/usr/sbin/visudo` ×6,
  `/usr/sbin/chown` ×6.
- The Homebrew formula has **no `depends_on :macos` guard** — brew-on-Linux
  would attempt an install whose `install.rs` targets `/private/etc`. Worth
  adding regardless of whether item 12 proceeds.
- **Sudo for Windows exists** (Microsoft, Windows 11 24H2, open source, Rust)
  and so does `gsudo`, but both are UAC elevation shims: no sudoers file, no
  policy language, no per-command allowlist, no `NOPASSWD`, no configurable
  timestamp, no `-u` (Microsoft's). Every mechanism this boundary rests on is
  absent. The fork's core proposition — agents run mediated credential ops with
  no prompt via the NOPASSWD broker while lifecycle ops require interactive auth
  — has no Windows expression.

## Operator Feedback

- Wanted all four leftovers handled in one go, with explicit dispositions:
  fix 1, delete 2, do 3, run 4. No hedging requested on any of them.
- Mid-session: *"stop at next convenient stopping point"* — honoured by
  finishing the correction commit, pushing, and writing Tier 1 rather than
  starting item 12.
- Asked specifically which Linux distros the project packages for, and whether
  sudo exists on Windows. Both answered from the repo and recorded above.
- Standing policy from this repo's `CLAUDE.md`, exercised throughout: all
  downstream work goes on `sudo-main`, committed and pushed directly, no feature
  branches and no PRs.

## Where We're Going

1. **Decide the scope of item 12 before writing any code: Linux/BSD only, or
   does it include Windows?** This is a design call only the operator can make,
   and it is now blocking. Item 11 in the design doc rejected the launchd daemon
   partly on the grounds that "`sudo` is the portable common denominator" — that
   reasoning holds for Linux and the BSDs, and is **false at Windows**, where
   reaching parity needs a privileged service plus a named pipe with a peer-token
   check, or COM elevation. That is architecturally the same shape as the daemon
   design item 11 declined, except on Windows it would be the only option. My
   recommendation: scope item 12 to Linux/BSD explicitly and reopen Windows as a
   separate question against item 11.
2. If item 12 proceeds, note it now has a **dependency created this session**:
   `drift.rs::classify_neighbour` encodes sudo's `sudoers.d` rules exactly as
   probed on macOS sudo 1.9.17p2. Re-probe on Linux with the Probe 3 technique
   before trusting it there — the gid-0 assumption is the likely divergence
   (`wheel` vs `root` as group 0), along with visudo's `SUDOERS_MODE`.
3. Item 12 also needs a **delivery decision, not just code**: there is currently
   no Linux packaging at all. Constants alone do not make it installable.
4. Add `depends_on :macos` to the Homebrew formula (small, independent of the
   above).
5. Items 9–10 need an operator decision before any work starts (cross-repo, see
   Key Decisions). If taken up: F5's real defect is
   `SECRETSPEC_BIN=/opt/homebrew/bin/secretspec` at wrapper line 8, executed as
   root from an admin-writable prefix. Item 10 (retire onto the Rust broker)
   stays blocked on `verify-sync` and `source-template-check` having no Rust
   equivalent — and note `source-template-check` hardcodes
   `/Users/djbclark/ops/site-private/secretspec.toml.example`, so porting it into
   the general broker would bake another repo's path into this tool.
6. This work is **unreleased**. The installed host binaries predate it, so
   `doctor` on this host will not emit the new codes until a release is cut and
   installed. Cutting one is `packaging/release.py --version <serial>`.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect d51543c, dbdad69, 308b86e
git status -s                 # expect clean

# Gates (cargo test --all cannot build ext-php-rs here: no php-config)
cargo test -p sudo-secretspec-cli                      # expect 137 passed
cargo clippy -p sudo-secretspec-cli --all-targets 2>&1 | grep -c '^warning'   # expect 17
cargo fmt --check -p sudo-secretspec-cli
~/.local/bin/pytest tests/sudo_packaging -q            # python3 -m pytest does NOT work here

# Host state
sudo /usr/sbin/visudo -c                               # expect all 4 drop-ins parsed OK
sudo -n /usr/local/libexec/sudo-secretspec __broker source-check \
  --client unknown \
  --reason-sha256 $(printf 'smoke test' | shasum -a 256 | cut -d' ' -f1)
# expect exit 0; a bare --reason is REFUSED by design

# The decisive sudo probe, if item 12 needs it re-run on another platform.
# Plant in /etc/sudoers.d (throwaway, grants nothing beyond an existing admin):
#   djbclark ALL=(root) NOPASSWD: /usr/bin/true
# Then:
#   sudo -u djbclark -H /usr/bin/sudo -k -n /usr/bin/true; echo $?
# exit 0 proves sudo READ the file. Always run the no-drop-in control first
# (expect 1), always visudo -c -f the content BEFORE placing it, and always
# remove it from an EXIT trap.
```

Read `docs/design/privilege-boundary-and-packaging.md` item 8 before changing
the neighbour check — it carries the full probe table and the reasoning behind
the three codes.

`gh` in this repo resolves to the **upstream** remote (`cachix/secretspec`) by
default; every `gh` call needs `-R djbclark/sudo-secretspec`.
