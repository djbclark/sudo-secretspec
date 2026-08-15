---
schema_version: 1
handoff_id: a9e5
parent_handoff_ids: [e7dd]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: e20059990781ba22b3c071f25b8191f68cd36d96
created_at: 2026-08-15T14:10:42-0400
writer: claude-code
---

# Handoff — Wrapper retired, fork moved to frdminc, four broken subcommands fixed

## The Goal

Resumed from `e7dd` via Tier 1. Its stated next action was "fix the stayturgid
wrapper, which is broken right now". Three further goals arrived from the
operator during the session, in order:

1. Fix the wrapper broken by the vault migration (approved: retire it).
2. Check whether the Shizuku fork actually uses the one credential file left
   alone in `e7dd`. Decide item 12's scope. → **Linux/BSD only, no Windows.**
3. Move the fork from `djbclark` to the new **`frdminc`** org (repos, packaging,
   local brew).
4. "Fix all the problems you found one by one." Then: close the exposed
   service, fix `add` ASAP, and audit whether anything *else* is broken.

## Where We Are

`sudo-secretspec` HEAD `e200599` on `sudo-main`, **tree clean, everything
pushed** to the new org. Five unreleased commits:

| SHA | What |
| --- | --- |
| `751ecd5` | Expose `template-check` as a public subcommand |
| `f263940` | Move the fork djbclark → frdminc |
| `ad126f8` | Repair the release workflow's tag leg and its formatting check |
| `b6af99b` | Make `add` declare; stop `check` prompting as root |
| `e200599` | Expose `audit-verify` as a public subcommand |

**Three PRs open on the coordinated ops suite**, none merged:

| PR | Branch head | Contents |
| --- | --- | --- |
| `djbclark/stayturgid#291` | `9150738` | Wrapper retirement, frdminc tap refs, FIRERPA MCP fail-closed |
| `djbclark/site-djbclark#153` | `f36925e` | OPS-RELEASES rewrite, frdminc tap, `just lint` fix |
| `djbclark/site-private#82` | `20ccae6` | Declare `FIRERPA_MCP_TOKEN` |

Worktrees: `~/src/ops-worktrees/secretspec-wrapper-retirement/{stayturgid,site-djbclark,site-private}`,
all on `feature/secretspec-wrapper-retirement`, all clean.

Files changed this session, by repo:

- **sudo-secretspec** (23, `2a8c576..e200599`): `.github/workflows/sudo-release.yml`,
  `CHANGELOG.md`, `CLAUDE.md`, `FORK-AI.md`, `PROMPT-REVIEW.md`,
  `PROMPT-SECREV.md`, `README.downstream.md`, `docs/design/{privilege-boundary-and-packaging,secrev-remediation-plan}.md`,
  `packaging/{README.md,homebrew/sudo-secretspec.rb,release.py}`,
  `secretspec/{Cargo.toml,src/cli/mod.rs,src/lib.rs,src/manifest_edit.rs}` (new),
  `sudo-secretspec-cli/{Cargo.toml,src/broker.rs,src/main.rs,tests/cli.rs}`,
  `sudo-secretspec/{AI-GUIDANCE.md,README.md}`, `tests/sudo_packaging/test_release.py`.
- **stayturgid** (15): deleted `control/bin/stayturgid-secretspec-wrapper.sh`,
  `control/config/sudoers.d/secretspec`, `control/lib/secretspec_env_exec.py`,
  `docs/operations/secretspec-wrapper-lifecycle.md`; added
  `docs/operations/secretspec-boundary-lifecycle.md`; modified
  `control/bin/{firerpa_mcp.py,publish_secrets.sh}`,
  `control/lib/secretspec_exec.py`,
  `control/tools/native-agent/reingest_soft_health.py`,
  `docs/operations/secretspec-secrets-management.md`, and five files under
  `tests/python/`.
- **site-djbclark** (2): `docs/OPS-RELEASES.md`, `justfile`.
- **site-private** (1): `secretspec.toml.example`.
- **Tap repo** `frdminc/homebrew-sudo-secretspec` (`6aa6e57`):
  `Formula/sudo-secretspec.rb`, `README.md`.

Host state changed this session:

- Homebrew fully migrated: tap `frdminc/sudo-secretspec`, keg provenance
  `frdminc/sudo-secretspec`, old tap untapped, `brew test` exit 0.
- `com.stayturgid.firerpa-mcp` **stopped and `launchctl disable`d** (persists
  across login).
- Boundary itself untouched and healthy throughout: `doctor: OK`, exit 0.

## What We Tried

Chronological. The failures are the expensive part.

1. **Assumed the wrapper fix was "repoint `VAULT_DIR`" — WRONG, and it would
   have been destructive.** `sync_source()` runs
   `chown _secretspec:staff "$VAULT_DIR"`. Retargeting at the canonical vault
   means the first `source-publish` **hijacks vault ownership away from
   `_sudo_secretspec`** — and `publish_secrets.sh:9` called `source-publish`
   as its very first action. It would also have failed regardless: the
   wrapper's consumer ops run as `_secretspec` (uid 503) and the canonical
   vault is `_sudo_secretspec:_sudo_secretspec` (uid 499) mode `0700`. Two
   service accounts was the real blocker, not the path.
2. **Wrote 43 plaintext secrets to `/tmp/exp.json`** while verifying that
   `export` returns the same shape as the old `automation-env`. Removed within
   the minute. Pipe into the parser instead; never land it.
3. **Two of my own new tests were too blunt** — they matched the retired
   wrapper's name in *explanatory prose*, not call paths. Fixed with a
   tokenize-based `_executable_source()` that strips comments and docstrings.
4. **`brew reinstall` does NOT change keg provenance.** Rebuilt for 11 minutes
   and `INSTALL_RECEIPT.json` still said `tap: djbclark/sudo-secretspec`, which
   is exactly what blocks `brew untap`. Needed `uninstall` + `install`.
5. **Misread a mid-build Homebrew state as a killed build.** Empty keg dir plus
   a missing `bin/secretspec` symlink is the *normal* intermediate state. My
   `pgrep -fl "brew|cargo"` missed it because Homebrew runs as `ruby`. Check
   the lock holder (`lsof` on `locks/<formula>.formula.lock`) before concluding
   anything died.
6. **Claimed `add` exits 0 on failure — WRONG, pipe artifact.** Read the exit
   code through `| head`, which reports `head`'s status. It exits 1 correctly.
   Same trap `6c20` recorded for `doctor`; it recurred.
7. **Declared `FIRERPA_MCP_TOKEN` as `required = true` first — reverted.** That
   makes `check` report it missing until a value exists, which fails
   `publish_secrets.sh` and the release gate on every host including ones that
   never run the bridge. Its two sibling declarations in the same block are optional for the same reason.
8. **Broke `just lint` twice while fixing it.** (a) `just` evaluates
   **backticks in recipe bodies as command substitution, even on comment
   lines** — my comment containing `` `just test` `` tried to execute it.
   (b) `site-djbclark`'s justfile sets `shell := ["bash","-uc"]`, under which a
   comment-only recipe line **runs no command and exits 1**, failing the whole
   recipe. Verified directly: `bash -uc '# x'; echo $?` → `1`. That is why no
   other recipe there has an indented comment.
9. **First attempt to expose the manifest editor used `secretspec::cli` —
   rejected.** The `cli` module is feature-gated and the companion uses
   `default-features = false` *deliberately*; enabling it would put `inquire`
   and `clap`-for-the-engine inside a root-privileged broker.
10. **A python edit script asserted mid-way and wrote nothing**, so a later
    script's dispatch line referenced an enum variant that did not exist. Write
    the file before the next assertion, or make each script idempotent.

## Key Decisions

**Retire the wrapper rather than repoint it.** Presented as a choice; operator
picked retirement. Routes stayturgid's eight consumers at the companion. The
companion's `run` is a strict superset of the deleted `secretspec_env_exec.py`
— same fetch-then-exec-as-caller, *plus* it purges every `SECRETSPEC_*` var and
records the target's basename in the audit ledger. Rejected: granting
`_secretspec` group access to the canonical vault (keeps two boundaries with
different semantics over one vault and weakens its 0700 isolation).

**Match on `djbclark/` with the slash, never the bare name.** `djbclark` means
three different things in this tree: the old **org** (rewrite), the old
**version serial** `-djbclark.N` in live tag names and regexes (**keep**), and
the operator's **macOS username** in sudoers rules, audit fixtures and paths
(**keep**). A blanket replace corrupts the last two.

**Verified the release tarball SHA-256 rather than assuming it.** Re-downloaded
`v0.19.1-sudo.5` from the new owner: byte-identical, because GitHub names the
archive's top-level dir after repo and tag, neither of which moved. No formula
rehash needed.

**Derived `TAP_NAME`/`FORMULA_NAME` from `FORK_REPO`.** All three carried the
org independently; this move had to edit three constants that can never
legitimately disagree.

**`add` writes the manifest IN PLACE, never temp-file-and-rename.** The vault
manifest is owned by the service user; a rename substitutes a root-created file
into a vault `drift` checks entry by entry, which fails `doctor`. Truncating in
place keeps the inode, and with it owner and mode — the same property
`Mutation::restore` already depends on. Rejected: the engine's own
`replace_manifest_atomically`, which is correct for the CLI and wrong here.

**New `manifest-edit` feature rather than enabling `cli`.** `add_secret_to_manifest`
and its name validation moved to `secretspec::manifest_edit`; `cli` now implies
it. The broker gains `toml_edit` and nothing else.

**`check` switched to `no_prompt: true`.** Rejected leaving it: an operation
named `check` could *write*, from a root process, reading whatever stdin the
caller passed.

**Declined to farm the `add` fix to `fable-deep`** despite the operator
offering. That agent is documented as not for security material, and this is
privilege-boundary code where a cold agent re-derives context already loaded.

**Stopped the FIRERPA service with `disable`, not just `bootout`.** `bootout`
alone lapses at next login while the plist remains installed — it would have
silently reopened.

## Evidence & Data

**The wrapper's blast radius was 8 live consumers**, not the 1–2 the prior log
implied: `deploy_fleet.py`, `ansible_exec.py`, `deploy_termux.py`,
`termux_pkg_nightly.py`, `verify_drift.py`, `serverapps.py:2051`,
`termux_ssh_bootstrap.py`, `firerpa_mcp.py` — all via `secretspec_run`.

**Replacement path proven equivalent before switching:**

```
sudo-secretspec export --reason ... -> exit 0, flat JSON, 43 string values
sudo-secretspec run --reason ... -- id -un -> djbclark   (caller, not root)
  with SECRETSPEC_PROVIDER=bogus set -> 0 SECRETSPEC_ vars survive, 97 total
```

**Constraint found empirically:** the client passes `target[0]` verbatim as
`--command-basename` and the broker refuses a path separator. `-- /usr/bin/id`
→ `audit denied: invalid command basename`; `-- id` → works.

**Four companion subcommands were broken or unreachable.** Found by diffing the
broker's operation list against the client's subcommand list:

| Command | Defect |
| --- | --- |
| `add` | Wired to the engine's `set`, which refuses an undeclared name — could never declare. Failed with `SecretNotFound` listing all 50 secrets. |
| `check` | `secrets.check(false)` — prompting **enabled inside the root broker**. A missing secret dropped root into interactive value entry, reading the caller's stdin and writing into the vault. Invisible + hanging with stdout redirected. |
| `template-check` | Implemented and allowlisted in the broker; no client subcommand. |
| `audit-verify` | Same. `AI-GUIDANCE.md` already told deployments to pin its tip hash externally. |

`add` verified reaching the editor: the new broker parses `--description` and
stops at `must run as root`. `audit-verify` verified end-to-end against the
**already-installed** `.5` broker: `30 events, tip <hash>`.

**FIRERPA MCP had two defects, both live since 2026-08-01:**

```
$ grep -c "without token authentication" ~/.config/stayturgid/logs/firerpa-mcp.log
10
$ lsof … pid 1003 -> TCP <tailnet-ip>:8000 (LISTEN)     # tailnet, not localhost
$ curl … /mcp                      -> 421
$ curl -H "Authorization: Bearer definitely-wrong" … -> 421   # identical: no auth layer
error.log: "Invalid Host header: <tailnet-ip>:8000"
```

Root cause of the 421: `TransportSecuritySettings` defaults
`enable_dns_rebinding_protection=True` with `allowed_hosts=[]`, rejecting every
Host. **Zero successful MCP sessions in the service's entire lifetime** (0 hits
for `POST /mcp` or `200 OK` across both logs).

**The release workflow's tag leg had fired exactly once, ever.** `gh run list`
shows one `push` run on `v0.19.1-djbclark.1` — the single literal it was pinned
to. `.2`, `.3`, `sudo.4`, `sudo.5` all published with it skipped.

**`ruff format --check` had never passed.** Not a line-length mismatch: tested
88/100/104/110/120, all fail. Formatted with ruff defaults (88 = least churn,
28 lines, and no config to keep in sync with CI's bare invocation).

**Test results.**

| Suite | Before | After |
| --- | --- | --- |
| `cargo test -p sudo-secretspec-cli --locked` | 137 | **140** |
| `pytest tests/sudo_packaging -q` | 21 | 21 |
| `clippy -p sudo-secretspec-cli --all-targets` | 17 | 17 (baseline held) |
| stayturgid `pytest tests/python` | 734 + 1 skip | 734 + 1 skip |
| stayturgid `just check` | PASS 18/18 | PASS 18/18 |
| site-djbclark `just lint` | **exit 1** | **exit 0**, Ran 105 OK |

`cargo test -p secretspec --lib` fails **21 sops tests — PRE-EXISTING**,
identical on a clean tree (1195 passed / 21 failed both ways). `sops` is not
installed.

**Shizuku question, answered NO.** Nothing sources
`~/.config/secretspec/op-service-account-shizuku-keystore.env`: every hit across
`~/src`, `~/ops`, `~/.config` and the shell rc files is prose. The fork mentions
it only in `OPTIONS.md:61`. Its CI signs from GitHub Actions secrets
(`.github/workflows/app.yml:28,58-62`). `site-private` memory
(`project_secretspec_onepassword_integration.md:44-57`) records the service
account as serving one already-completed keystore backup. Safe to revoke in
1Password; no consumer to break.

## Operator Feedback

- **"Go ahead with the proposed plan."** Standing authorization for the resume
  plan, which is how the retirement got built.
- **Item 12 scope: "Linux/BSD only. Do not include Windows."** Settled.
- **"We def want to fix add asap."** Drove this session's last phase.
- **"You can farm out the add fix to a higher spec ai model or effort if you
  think that would make sense."** Explicitly left to judgement; declined, with
  reasons recorded above.
- **"We want our version of secretspec to support all the options."** Drove the
  full subcommand audit — which is what found `check` and `audit-verify`.
- Repo policy (`CLAUDE.md`): commit and push directly to `sudo-main`; never
  touch `main`. `~/CLAUDE.md`: `~/ops` is deploy-only.

## Where We're Going

1. **THE NEXT ACTION — cut `v0.19.1-sudo.6`** (needs operator authorization;
   `.5`'s did not carry forward). **The `add` fix requires a boundary
   reinstall, not just a client upgrade** — the installed `.5` broker rejects
   `--description`. `template-check` and `audit-verify` do not.
2. Merge `site-private#82` → `stayturgid#291` → `site-djbclark#153`, then a
   coordinated `ops-vX.Y.Z`.
3. Set `FIRERPA_MCP_TOKEN` on a TTY, **before** #291 reaches the control node —
   the service now fails closed and its launchd job is `KeepAlive` with
   `ThrottleInterval 10`, so a missing value is a restart loop. Currently moot:
   the agent is disabled.
4. Re-enable and verify the bridge. A request with **no** `Authorization` must
   return **401** — not 421 (the old DNS-rebinding rejection), not 200.
5. **Open design call:** the companion does not expose the engine's `init`,
   `schema`, `config`, `import`, `cache`, `audit`. `config` and `import` must
   stay excluded — they would let a caller repoint provider/profile or
   exfiltrate to another provider, defeating the boundary. `schema` is
   read-only and safe. **None of these exclusions is documented anywhere; that
   is itself the gap.**
6. Untested subcommands, no agent TTY: `install`, `uninstall`, `rollback`.
7. Item 12 unblocked. Re-probe `drift.rs::classify_neighbour` on Linux first
   (it encodes macOS sudo 1.9.17p2 behaviour; gid-0 wheel-vs-root and visudo's
   `SUDOERS_MODE` are the likely divergences). Also needs a delivery decision:
   no Linux packaging exists, and the companion is excluded from both non-macOS
   CI legs (`test.yml:75`, `:154`).
8. Do **not** delete `/var/db/stayturgid-secrets` yet — only second copy of the
   secrets plus the pre-migration audit ledger.
9. Then remove the task workspace (command in Quick Start).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -5        # expect e200599 at HEAD, sudo-main, clean

# Read the parent first
$EDITOR docs/handoffs/HANDOFF_standalone-b2db_vault-migration-and-retirement_2026-08-15_e7dd.md

# Baselines on this HEAD
cargo test -p sudo-secretspec-cli --locked        # 140
~/.local/bin/pytest tests/sudo_packaging -q       # 21
cargo clippy -p sudo-secretspec-cli --all-targets # 17 warnings
cargo fmt --all --check                           # clean
uvx ruff format --check packaging tests/sudo_packaging   # clean
# `cargo test -p secretspec --lib` fails 21 sops tests: PRE-EXISTING, sops absent.

# Release (item 1) — needs operator authorization
python3 packaging/release.py --version 0.19.1-sudo.6 --dry-run --skip-tests
python3 packaging/release.py --version 0.19.1-sudo.6
brew upgrade frdminc/sudo-secretspec/sudo-secretspec
sudo-secretspec install --adopt-existing     # REAL TTY; required for the add fix

# Verify the four repaired subcommands
sudo-secretspec audit-verify                             # "N events, tip <hash>"
sudo-secretspec template-check --reason 'post-release'
sudo-secretspec add SOME_NAME --description 'x' --reason 'y'   # declares now
sudo-secretspec check --reason 'post-release' </dev/null       # read-only now

# FIRERPA bridge — currently STOPPED and DISABLED
launchctl enable    gui/$(id -u)/com.stayturgid.firerpa-mcp
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.stayturgid.firerpa-mcp.plist
# (only after the token is set; no-Authorization must give 401)

# Remove the task workspace when the PRs land
for r in stayturgid site-djbclark site-private; do
  git -C ~/src/ops-worktrees/.store/$r.git worktree remove \
    ~/src/ops-worktrees/secretspec-wrapper-retirement/$r
done && rmdir ~/src/ops-worktrees/secretspec-wrapper-retirement

# HOST GOTCHAS
#  * `djbclark` = old org (rewrite) | old version serial -djbclark.N (KEEP)
#    | operator's macOS username (KEEP). Match on `djbclark/` with the slash.
#  * GitHub redirects the old org URLs, so a missed reference works SILENTLY.
#  * `run` audits by BASENAME; the broker refuses a path separator.
#    Pass `python3 script.py`, never `./script.py`.
#  * site-djbclark justfile: shell is `bash -uc`; an indented `#` recipe line
#    runs no command and EXITS 1. `just` also evaluates backticks in recipe
#    bodies. Put comments ABOVE the recipe.
#  * Exit codes read through a pipe are the LAST command's. Redirect, check $?.
#  * A bare PR-create via the GitHub CLI is blocked by an upstream-review hook
#    (cachix/secretspec remote). Pass -R djbclark/<repo> --head --base, and
#    push the branch first or it fails "No commits between".
#  * `brew reinstall` preserves the recorded source tap; only uninstall+install
#    moves provenance.
```
