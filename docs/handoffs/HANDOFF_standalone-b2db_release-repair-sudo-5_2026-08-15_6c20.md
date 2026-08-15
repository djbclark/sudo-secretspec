---
schema_version: 1
handoff_id: 6c20
parent_handoff_ids: [4e3a]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: bcb111af34cff7c43113463b2e56842dc300cd83
created_at: 2026-08-15T11:08:11-0400
writer: claude-code
---

# Handoff — Release repair: v0.19.1-sudo.4 was uninstallable, .5 supersedes it

## The Goal

Resume the `standalone-b2db` chain from Tier 1, fix whatever gaps the resume
plan surfaced, then apply two operator-supplied fixes (mojibake in the PROMPT
files; a missing SQLite build dependency in the Homebrew formula). The operator
explicitly framed the two fixes as unverified — *"it may either be already done
or incorrect, please do not assume it is true"* — so both had to be checked
against the actual files before being applied.

Mid-session the operator extended the goal: cut the release, publish it, install
it locally via brew, and confirm it works.

## Where We Are

`v0.19.1-sudo.5` is published, tap-synced, installed, and verified working. HEAD
is `bcb111a` on `sudo-main`, tree clean, everything pushed.

The session began by discovering the Tier 1 log was stale (`head_sha 39bfbb7`
vs. actual `53dd2be`) and that the four intervening commits had left the
previously-cut `v0.19.1-sudo.4` **published but uninstallable**.

Commits added this session, all on `sudo-main`:

| SHA | What |
| --- | --- |
| `5d30740` | Repair the `.4` bump: regenerate `Cargo.lock`, fix the failing fork-identity test, format `install.rs`, correct `CLAUDE.md`'s version scheme |
| `6a5f9bf` | The two operator-requested fixes: PROMPT mojibake, formula `sqlite`/`pkg-config`/`:macos` |
| `4831d28` | Bump to `0.19.1-sudo.5`; teach `release.py` the `-sudo.N` serial; add the lockfile preflight guard |
| `ed9bc7a` | Formula restamp for `v0.19.1-sudo.5` (written by `release.py`, not by hand) |
| `bcb111a` | Demote `sqlite` from `:build` to a full runtime dependency |

Files changed across those commits: `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`,
`CLAUDE.md`, `PROMPT-REVIEW.md`, `PROMPT-SECREV.md`,
`packaging/homebrew/sudo-secretspec.rb`, `packaging/release.py`,
`sudo-secretspec-cli/src/install.rs`, `sudo-secretspec-cli/tests/cli.rs`,
`tests/sudo_packaging/test_release.py`.

Tap `djbclark/homebrew-sudo-secretspec` is at `b445a48`.

**Not fixed, and it needs a human at a terminal:** `doctor` exits 1 on this host
with `INSTALLED_HASH_MISMATCH`. See Evidence for why this is pre-existing host
drift rather than a release defect.

## What We Tried

Chronological, including the approaches that failed — these are the expensive
ones to rediscover.

1. **`grep -nP '\xEF\xBF\xBD'` to detect the mojibake — FAILED SILENTLY, and I
   reported the wrong conclusion from it.** The operator supplied this command
   as the verification. It exited 1, so I initially told the operator both
   PROMPT files were already clean. That was wrong. `grep` on this host is
   **ugrep 7.5.0**, which does not honor `\xNN` byte escapes under `-P`; it
   reports no matches on files that demonstrably contain the bytes. The
   corruption was real and exactly where the operator said it was.
2. **`LC_ALL=C grep -nP '\xEF\xBF\xBD'` — FAILED too.** The locale is not the
   problem; the matcher is. Do not reach for this as the fix.
3. **`perl -ne 'print if /\x{FFFD}/'` — FAILED.** Perl does not decode the file
   as UTF-8 without `-CSD`, so the pattern never matches. A red herring that
   looked like confirmation the file was clean.
4. **What actually works: `grep -c $'\xef\xbf\xbd' <files>`** — shell-expanded
   literal bytes, no `-P`. Returned 1 line in `PROMPT-REVIEW.md` and 3 in
   `PROMPT-SECREV.md`, matching the operator's stated line numbers (126, and
   167–169). `hexdump -C` was what settled it first.
5. **Planning to "fix the release gap" by publishing a GitHub Release for
   `v0.19.1-sudo.4` — ABANDONED once the tag was tested.** The tag itself is
   broken, so a Release would have published a pointer to something nobody can
   install. Superseded forward-only by `.5`.
6. **`git worktree add` into the scratchpad to test the tag — created
   `tagtest_wt` in the repo root even though the command appeared to fail.**
   That directory then got swept into `4831d28` by a `git add -A`, producing an
   "adding embedded git repository" warning. Fixed with
   `git rm --cached tagtest_wt`, `git worktree remove --force`, `git worktree
   prune`, and `git commit --amend`. `git archive | tar -x` is the right way to
   materialize a tag for testing here — no worktree, nothing to leak.
7. **Reading `doctor`'s exit code through a pipe — misread as 0.** `doctor ...
   | tail -25; echo $?` reports `tail`'s status, not doctor's. Redirect to a
   file and check `$?` directly; doctor actually exits 1.
8. **Testing the `.4` tag: `cargo metadata --locked` in an extracted tarball**
   — this one worked and is the reusable technique. Exit 101 on `.4`, exit 0 on
   `.5`.

## Key Decisions

**Cut a new tag `v0.19.1-sudo.5` rather than repairing `.4` in place.** Rejected:
deleting/retagging `v0.19.1-sudo.4` (it is published; retagging rewrites history
others may have fetched) and publishing a Release for `.4` (the tree is broken
regardless of whether a Release exists). `.4` is left in place, marked in the
CHANGELOG as superseded and not to be installed. Forward-only, consistent with
the ops release policy in `~/CLAUDE.md`.

**Declared `sqlite` as a runtime dependency, contradicting the operator's
instruction to use `=> :build`.** Evidence: `otool -L` on the installed
companion shows it linked against `/opt/homebrew/opt/sqlite/lib/libsqlite3.dylib`
— Homebrew's copy, not the one in `/usr/lib`. pkg-config resolves rusqlite's
system-SQLite build to the Homebrew keg, so the library is needed to *run*, not
only to compile. Declared `:build`, Homebrew would be free to remove sqlite and
leave an installed binary that cannot start. `pkg-config` correctly stays
`:build`. Did **not** revert to rusqlite bundled, per the operator's explicit
instruction and `FORK-AI.md` (bundled hangs in libsqlite3-sys on macOS).

**Put the lockfile guard in `preflight`, not in a post-tag tarball check.** I had
originally proposed to the operator that the release script verify the published
tarball builds locked. Moved it earlier: `cargo metadata --locked` now runs in
preflight, so the failure lands *before* a tag exists rather than after one is
public and needs superseding. Cheaper and it fails in the recoverable direction.

**`ANY_VERSION_RE` matches both `-sudo.N` and `-djbclark.N`, while
`parse_release` accepts only `-sudo.N`.** Deliberate asymmetry: the script must
*recognize* the pre-rename spelling to restamp a formula carried across the
rename, but must never *cut* one. A test asserts each half.

**Split the work into two commits** rather than the single commit the operator
asked for. The requested pair (`6a5f9bf`) is self-contained; the release-repair
work (`5d30740`) is a different concern and belongs separately in the history.

**CHANGELOG entries stay under `## [Unreleased]`.** Per `CLAUDE.md`: don't create
new release subsections. This matches existing practice in the file — `.1`/`.2`/
`.3` all shipped without promoting the section.

**Did not run `sudo-secretspec install` to clear the doctor failure.**
`timestamp_timeout=0` forces interactive authentication every time, and it cannot
prompt from a backgrounded process. This is documented in parent handoff `4e3a`
and was hit again here. Operator must run it in a real TTY.

## Evidence & Data

**The `.4` breakage, proven not argued.** Extracted the tag with `git archive
v0.19.1-sudo.4 | tar -x` and ran `cargo metadata --locked --format-version 1`:

```
exit=101
error: the lock file ... needs to be updated but --locked was passed to prevent this
```

The same test against `v0.19.1-sudo.5` exits 0. Root cause: `f2dfc72` bumped
`Cargo.toml` to `0.19.1-sudo.4` but never regenerated `Cargo.lock`, which still
named `0.19.1-djbclark.3` for every workspace member. The formula builds with
`cargo install --locked`, so the tag aborted before compiling anything — and the
tap was already synced to it.

**Why `.4` was hand-cut.** `packaging/release.py` accepted only `-djbclark.N`
(`VERSION_RE`), so the rename to `-sudo.N` made the script unusable and the
release was done by hand. The script's `run_tests` already runs `cargo test -p
sudo-secretspec-cli --locked`, which would have caught the stale lockfile. The
whole failure is downstream of the helper not knowing the new serial.

**Mojibake, byte-level.** `hexdump -C` of `PROMPT-REVIEW.md:126`:

```
20 60 ef bf bd ef bf bd  ef bf bd ef bf bd 20 53   | `............ S|
```

Four consecutive U+FFFD per marker, not one. Both files are valid UTF-8
(`iconv -f UTF-8 -t UTF-8` succeeds), which is why nothing else flagged them.
Post-fix: `grep -c $'\xef\xbf\xbd'` returns 0 for both; a repo-wide sweep over
`*.md`/`*.rs`/`*.rb`/`*.toml` finds no others.

**Test results.**

- `cargo test -p sudo-secretspec-cli --locked`: **137 passed** (matches the
  baseline in parent `4e3a`). Before the fix, `version_identifies_downstream_distribution`
  FAILED — it asserted `reported.contains("-djbclark.")` against an actual
  `sudo-secretspec 0.19.1-sudo.4`.
- `~/.local/bin/pytest tests/sudo_packaging -q`: **21 passed** (was 19; +2 new —
  the lockfile-guard test and the both-spellings regex test).
- `cargo clippy -p sudo-secretspec-cli --all-targets`: **17** warnings, the
  established baseline, unchanged.
- `cargo fmt --all --check`: clean. It was NOT clean on arrival — `7e8250f` left
  `install.rs` unformatted (an `install_file(...)` call that fits on one line).
- `ruby -c packaging/homebrew/sudo-secretspec.rb`: Syntax OK.
- `brew test djbclark/sudo-secretspec/sudo-secretspec`: exit 0.

**`brew style` reports 2 offenses that are NOT defects.** Both are
`Lint/DuplicateMethods` on `install` and `caveats`, because rubocop also reads
the installed tap copy at
`/opt/homebrew/Library/Taps/djbclark/homebrew-sudo-secretspec/Formula/sudo-secretspec.rb`,
which defines the same `SudoSecretspec` class. Verified by `git stash` that the
count is identical before and after the edits. Do not try to "fix" these in the
repo file.

**Post-release verification.**

```
brew list --versions sudo-secretspec  -> sudo-secretspec 0.19.1-sudo.5
<keg>/bin/secretspec --version        -> secretspec 0.19.1-sudo.5
<keg>/libexec/sudo-secretspec --version -> sudo-secretspec 0.19.1-sudo.5
brew info --json=v2 ... dependencies  -> runtime: ['sqlite'], build: ['pkg-config','rust']
otool -L <keg>/libexec/sudo-secretspec -> /opt/homebrew/opt/sqlite/lib/libsqlite3.dylib
```

Built from source in 9 minutes, which is what exercises the new dependencies.

**The `doctor` failure is pre-existing host drift, not a release defect.**
`doctor` exits 1 with `INSTALLED_HASH_MISMATCH (/usr/local/etc/sudo-secretspec.toml)`.
Walking `/usr/local/share/sudo-secretspec/MANIFEST.sha256` by hand:

```
OK     /usr/local/bin/sudo-secretspec
OK     /usr/local/libexec/sudo-secretspec
OK     /usr/local/share/sudo-secretspec/secretspec.toml
OK     /usr/local/share/sudo-secretspec/sudo-secretspec-retired.toml
OK     /usr/local/share/sudo-secretspec/AI-GUIDANCE.md
DIFFER /usr/local/etc/sudo-secretspec.toml
DIFFER /private/etc/sudoers.d/sudo-secretspec
```

Exactly the two artifacts the installer *generates* mismatch; all five it
*copies* verify. The decisive datum is mtime ordering:

```
08:24:45  MANIFEST.sha256
08:24:45  /private/etc/sudoers.d/sudo-secretspec
08:24:58  /usr/local/etc/sudo-secretspec.toml   <- 13s AFTER its own manifest
```

`install.rs` writes the config *before* computing the manifest (config at
~line 861, manifest loop at ~line 876), so within one run they cannot disagree.
The config was therefore edited after that install finished, on 2026-08-14. The
live file carries `adopted_vault = true`, the field added by `966397e`. The
installed client at `/usr/local/bin` is still `0.19.1-sudo.4` — the brew keg only
places the libexec bootstrap, it never touches `/usr/local`.

## Operator Feedback

- **"Note it may either be already done or incorrect, please do not assume it is
  true."** Applied to both supplied fixes. Fix 1's premise held but its
  verification command did not; Fix 2's premise held but its prescription
  (`=> :build`) was wrong. Both were caught only by checking rather than
  applying. This is the operating mode for this chain.
- **"Fix the gaps. Run the resume plan."** Standing authorization for the
  resume-plan items, which is how the `.4` repair got done.
- **"Cut it and release it and install it locally via brew and make sure it
  works."** Explicit authorization for the outward-facing publish actions —
  tag, GitHub Release, tap push. Note this authorization was specific to `.5`
  and does not carry forward.
- Repo policy, from `CLAUDE.md`: commit and push directly to `sudo-main`, no
  feature branches, no PRs. Do not touch `main` (upstream mirror).

## Where We're Going

1. **THE NEXT ACTION — clear the `doctor` failure and move the installed client
   to `.5`, in a real TTY.** Both are the same operation:
   `/opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec install --declarations <path>`.
   `install` rewrites the config and the manifest together, so it resolves the
   `INSTALLED_HASH_MISMATCH` as a side effect. Must NOT be backgrounded —
   `timestamp_timeout=0` means it authenticates every time and cannot prompt
   from a non-TTY. Confirm afterward with `sudo-secretspec doctor` (expect exit
   0 with only the 2 known `LEGACY_VAULT_CLUTTER` advisories).
2. **Decide item 12's scope** — Linux/BSD only, or including Windows? Windows
   sudo (Microsoft's and gsudo) is a UAC shim with no sudoers file, no policy
   language, no NOPASSWD and no timestamp control, so the mediated-broker model
   has no Windows expression. Parent handoff `4e3a` recommends scoping item 12
   to Linux/BSD and reopening Windows against design item 11. Operator-only
   call; blocks any item 12 code.
3. **Decide items 9–10** — the wrapper source is
   `~/ops/stayturgid/control/bin/stayturgid-secretspec-wrapper.sh`, not this
   repo. Per `~/CLAUDE.md`, `~/ops` is deploy-only: needs a task workspace under
   `~/src/ops-worktrees/`, a PR, and a coordinated `ops-vX.Y.Z` release.
   Operator-only call.
4. **If item 12 proceeds:** `drift.rs::classify_neighbour` encodes sudo's
   `sudoers.d` rules as probed on macOS sudo 1.9.17p2. Re-probe on Linux before
   trusting it — the gid-0 assumption (wheel vs. root as group 0) and visudo's
   `SUDOERS_MODE` are the likely divergences. Full probe technique is in parent
   handoff `4e3a`. Item 12 also needs a *delivery* decision: there is no Linux
   packaging for the companion anywhere in the tree, and it is excluded from
   both non-macOS CI legs (`test.yml:75` and `:154`).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -5          # expect bcb111a at HEAD, sudo-main, clean

# Read the parent first — probe table, the visudo/sudo correction, item 12 recon
$EDITOR docs/handoffs/HANDOFF_standalone-b2db_sudoers-neighbour-report_2026-08-14_4e3a.md

# Verification baselines on this HEAD
cargo test -p sudo-secretspec-cli --locked        # 137 passed
~/.local/bin/pytest tests/sudo_packaging -q       # 21 passed
cargo clippy -p sudo-secretspec-cli --all-targets # 17 warnings (baseline)
cargo fmt --all --check                           # clean
# NOTE: `cargo test --all` cannot build ext-php-rs here.
# NOTE: `python3 -m pytest` does NOT work; use ~/.local/bin/pytest.

# Host state
/usr/local/bin/sudo-secretspec --version   # 0.19.1-sudo.4 until item 1 is done
sudo-secretspec doctor > /tmp/d.txt 2>&1; echo $?   # 1 — do NOT read $? through a pipe
brew list --versions sudo-secretspec       # 0.19.1-sudo.5

# Cutting the next release (works end to end now)
python3 packaging/release.py --version 0.19.1-sudo.6 --dry-run --skip-tests
python3 packaging/release.py --version 0.19.1-sudo.6
# Preflight now runs `cargo metadata --locked` and refuses a lockfile that
# disagrees with Cargo.toml. `pytest` must be on PATH; the script calls it bare.

# Verify any tag is installable before syncing the tap
git archive <tag> | (mkdir -p /tmp/t && tar -x -C /tmp/t) && \
  (cd /tmp/t && cargo metadata --locked --format-version 1 >/dev/null; echo "exit=$?")

# HOST GOTCHA: grep is ugrep 7.5.0. `grep -P '\xEF\xBF\xBD'` silently finds
# nothing on files that DO contain U+FFFD. Use literal bytes instead:
grep -c $'\xef\xbf\xbd' PROMPT-REVIEW.md PROMPT-SECREV.md   # expect 0 0
```
