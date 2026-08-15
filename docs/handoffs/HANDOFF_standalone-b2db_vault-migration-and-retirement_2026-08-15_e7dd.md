---
schema_version: 1
handoff_id: e7dd
parent_handoff_ids: [6c20]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 2fb72f54c7333ba1351320e466d6bd806a9e31f3
created_at: 2026-08-15T12:10:45-0400
writer: claude-code
---

# Handoff — Vault migrated to canonical, non-canonical stores retired

## The Goal

Continue from `6c20` (same session). Two goals arrived in sequence:

1. **Fix the boundary's data location.** The operator's read was "we need to
   change away from the legacy location of our secrets to the new canonical
   location." Investigation showed the *config* had already moved but the
   *data* had not — see Where We Are.
2. **Retire every non-canonical secrets store**, so anything still reading one
   fails loudly. Operator's words: *"I want errors if anything tried to access
   them."*

Both were executed by the operator at a real TTY via scripts written here;
`timestamp_timeout=0` makes interactive auth mandatory and blocks any
backgrounded/agent-driven run.

## Where We Are

Repo HEAD `2fb72f5` on `sudo-main`, tree clean. **No code changed in this repo
during this phase** — the work was host-state migration plus two operational
scripts that live outside the repo. The only repo commit is the parent handoff
`2fb72f5` itself.

Host state, all verified after the fact by independent commands, not just by
the scripts' own reports:

| | state |
| --- | --- |
| `/var/db/sudo-secretspec` (canonical) | live, 43 secrets, `doctor: OK` |
| `/var/db/stayturgid-secrets` (legacy) | `root:wheel 0000`, 10 paths, denied to non-root |
| 5 stale store paths | `root:wheel 0444` + `uchg` tombstones |
| installed client / engine | `0.19.1-sudo.5` |
| `doctor` | `{"ok": true, "findings": []}` — first fully-clean result in this chain |
| `check` | `Summary: 43 found, 0 missing, 7 optional` against `site-private` |

Scripts written (outside the repo, at `~/backups/sudo-secretspec/`):

- `migrate-to-canonical.sh` — 5-phase inspect/backup/migrate/install/verify
- `retire-noncanonical-secrets.sh` — tombstone + lock, no flags, tables at top

Backups: `/var/root/backups/sudo-secretspec-20260815-114407/` (both vaults,
sudoers, config, share), plus an unprivileged capture at
`~/backups/sudo-secretspec/2026-08-15-boundary-state/`. Manifests recording
every changed path's original owner/mode at
`~/backups/sudo-secretspec/retire-20260815-120900.manifest`.

**Newly live problem:** locking the legacy vault armed a tripwire.
`/usr/local/libexec/stayturgid-secretspec-wrapper.sh` hardcodes
`VAULT_DIR=/var/db/stayturgid-secrets` and runs as `_secretspec`. It will now
fail `EACCES`. Design items 9–10 moved from backlog to live work.

## What We Tried

Chronological. The failures are the expensive part.

1. **Read the situation as "migration pending" — WRONG, and it inverted the
   plan.** The config already pointed at `/var/db/sudo-secretspec`, so the
   migration looked done. A mediated `check` showed
   `Checking secrets in test-project ... 1 found, 2 missing`. The config had
   moved ahead of the data: an install on 2026-08-14 08:24 repointed vault
   *and* declarations at 3-secret test fixtures. Inspect later proved the
   canonical `.env` was **0 bytes** — it had never held a secret, and the
   `1 found` was `OPTIONAL_SECRET` resolving from its default.
2. **`sudo -n` to back up the vaults — REFUSED.** `sudo: a password is
   required`. The sudoers policy NOPASSWDs only the broker path
   (`__broker *`, `doctor`, `doctor *`). Nothing agent-side can read either
   vault. Every privileged step in this phase had to be operator-run.
3. **First script run HUNG with no output** at `--- current check ---`.
   Diagnosed by reading the detail file: `check` is **not** a read-only
   report. When required secrets are missing it falls into the ENGINE's
   interactive value-entry flow (`? [1/2] Enter value for API_KEY:`), and
   because the command's stdout was redirected to a file the prompt was
   invisible while it blocked on the TTY. Fix: `< /dev/null` on both `check`
   calls, and `run()` now redirects stdin from `/dev/null` generally.
   **This was latent precisely because it was tested first** — agent-side
   stdin is not a TTY, so the engine got EOF and printed the summary. The bug
   only exists in the environment the script is for.
4. **`run()` helper returned the wrapped command's exit code — would have
   aborted the whole script.** Under `set -e`, a bare `run sudo diff ...`
   returning 1 (the legitimate "manifests differ" case) kills the script and
   fires the ERR trap — the opposite of what a diagnostic helper is for.
   Found by chasing a *cosmetic* shellcheck SC2034 about an unused variable.
   `run()` now returns 0 unconditionally and publishes `RUN_RC`.
5. **`sudo /usr/bin/wc -l < "$FILE"` — would have failed permission-denied.**
   Caught by shellcheck SC2024: the redirect opens as the unprivileged user,
   who cannot read a root-only vault. Fixed to `wc -l "$FILE"` (root opens it
   itself, which also keeps secret bytes out of a user-space pipe).
6. **`git worktree add` to materialize a tag — leaked `tagtest_wt` into a
   commit** via `git add -A` (earlier in session, see `6c20`). Use
   `git archive <tag> | tar -x` instead; nothing to leak.
7. **Assumed `root:wheel 0444` would prevent recreation — WRONG, tested.**
   Unlink permission comes from the DIRECTORY's write bit, not the file's
   mode. Empirical test: `chmod 0444` then `rm` → **deleted**. With
   `chflags uchg` → **blocked**. Since root owns the tombstone, the operator's
   account cannot clear the flag either. This is what makes the retirement
   actually hold.
8. **Assumed three separate `.env` stores needed merging — they are one
   dangling symlink target.** `~/.config/.env`, `~/ops/stayturgid/.env` and
   `~/ops/site-djbclark/.env` all point at `~/ops/site-private/.env`, which
   **does not exist**. Nothing to merge.
9. **Misreported a file mode as 0600 when it was 0644.** `stat` showed
   `op-service-account-shizuku-keystore.env` world-readable while holding a
   1Password service-account token. Corrected and tightened to 0600.
10. **`grep -P '\xEF\xBF\xBD'` (earlier in session, from `6c20`)** — grep here
    is ugrep 7.5.0 and silently ignores hex byte escapes under `-P`. Use
    `grep -c $'\xef\xbf\xbd'`.

## Key Decisions

**Migrate data to canonical rather than repoint config back to legacy.** The
operator chose route B explicitly. Route A (point the boundary back at
`/var/db/stayturgid-secrets`) was offered as the faster restore-of-function and
rejected in favour of the correct end state.

**Do NOT migrate `broker-audit.sqlite3`.** Both vaults hold one (legacy 28,672 B
to Aug 14 06:33; canonical 20,480 B and live). It is hash-chained and
tamper-evident: overwriting the live ledger with the older one would splice two
independent chains, break verification, and discard every entry since the
canonical vault went active. Accepted cost, confirmed at the prompt by the
operator: **audit history is SPLIT at this boundary**, both halves independently
verifiable. Originally this was accidental (the script simply didn't copy it);
it was made an explicit, logged, confirmed decision.

**`--adopt-existing` is asserted, not assumed.** `install.rs` runs
`fs::File::create(<vault>/.env)` in the non-adopt branch, which TRUNCATES.
Against a vault holding 43 real secrets that is total loss. The flag is built
into an array and pattern-checked before the call; absence exits 5.

**Tombstones over `rm`.** Operator's reasoning: deleted paths can simply be
recreated. Chose root-owned immutable explainer files. Rejected: plain `rm`
(recreatable), and `schg` (does stop root, but needs single-user mode to clear
— disproportionate).

**`0000` for the vault, `0444` for tombstones.** The vault still holds real
secrets and nothing should read it; tombstones hold instructions and everything
should. Stated explicitly: **neither stops root**, and the broker runs as root.

**Scope held narrow on the other non-canonical files.** `~/.config/.env` and
friends were retired; the unrelated `.env` files for hermes, aiuse, superbrain,
collie, yt-playlist-organizer were left alone — locking them would break tools
unrelated to this work. `op-service-account-shizuku-keystore.env` was left in
place by operator decision late in the session ("ignore ... for now").

**Removed the deletion block rather than emptying its array.** `/bin/bash` here
is **3.2.57**, where `"${ARR[@]}"` on an empty array under `set -u` is a fatal
unbound-variable error. An empty `DELETIONS=()` would have killed the script on
its first phase.

**`must` tees, `run` stays quiet.** `must` wraps consequential steps the
operator should watch live on a TTY; `run` wraps diagnostics whose place is the
log.

## Evidence & Data

**Pre-migration state (inspect phase).**

```
legacy   /var/db/stayturgid-secrets  drwx------ _secretspec:staff
  .env                  2452 B, 35 lines, Aug 11 14:11
  secretspec.toml       8655 B  -- diff vs tracked declarations: IDENTICAL
  broker-audit.sqlite3 28672 B, Aug 14 06:33
  .ansible/ .local/     <- the two LEGACY_VAULT_CLUTTER advisories
canonical /var/db/sudo-secretspec  drwx------ _sudo_secretspec:_sudo_secretspec
  .env                     0 B   <- never held a secret
  secretspec.toml        335 B   <- test-project
  broker-audit.sqlite3 20480 B   <- live
```

**35 lines vs 43 found reconciles as** ~36 `.env` entries (35 newlines + a final
line with no trailing newline) plus 7 optional-from-default = 43. Inferred, then
confirmed by the post-migration check returning the same `7 optional` as the
pre-regression Aug 14 smoke test — the secret set is intact, not merely
non-empty.

**Migration result.**

```
$ sudo cmp /var/db/stayturgid-secrets/.env /var/db/sudo-secretspec/.env  -> exit 0
doctor --json -> {"ok": true, "findings": []}
doctor        -> OK
--version     -> sudo-secretspec 0.19.1-sudo.5   (was .4)
check         -> Checking secrets in site-private (profile: default)...
                 Summary: 43 found, 0 missing, 7 optional   [exit 0]
```

All four sudoers drop-ins parsed OK after the policy rewrite (`sudoers`,
`secretspec`, `sudo-secretspec`, `yabai`) — the neighbour-report work from `4e3a`
confirming the rewrite didn't disturb foreign files. Rollback snapshot:
`/usr/local/libexec/sudo-secretspec-rollback-1786808661`, 8 artifacts.

**`doctor` is fully clean for the first time in this chain.** Prior handoffs all
recorded 2 permanent `LEGACY_VAULT_CLUTTER` advisories for
`/var/db/stayturgid-secrets/.local` and `.ansible`. Those live only in the
legacy vault and were not copied, so they are gone along with the
`INSTALLED_HASH_MISMATCH`.

**Retirement result.** 5 paths tombstoned, 10 vault paths locked:

```
root:wheel 444 uchg  ~/.config/.env                  (was dangling symlink)
root:wheel 444 uchg  ~/ops/stayturgid/.env           (was dangling symlink)
root:wheel 444 uchg  ~/ops/site-djbclark/.env        (was dangling symlink)
root:wheel 444 uchg  ~/ops/site-private/.env         (was the missing target)
root:wheel 444 uchg  ~/.config/secretspec.toml       (was stale declarations)
   each: readable yes, delete-proof yes  <- a real rm attempt, failing
OK: /var/db/stayturgid-secrets denied to djbclark
doctor: OK ; Summary: 43 found, 0 missing, 7 optional
git status in site-private / stayturgid / site-djbclark: 0 changed
```

`~/ops/site-private/.env` is gitignored (`.gitignore:19`) so no deploy checkout
was dirtied. Side effect to know: `git clean -xdf` in that repo will now FAIL on
the immutable file rather than silently removing it.

**`~/.config/secretspec.toml` contributed nothing** — `comm -23` against the
tracked declarations returned empty; it was a stale subset, short by two
keys that the tracked file has. Declarations are schema, not values, so no
secret was in it. (Names withheld: this repo is public and this chain keeps
secret names out of handoffs.)

**`OP_SERVICE_ACCOUNT_TOKEN` is a provider credential, not a managed secret** —
`docs/src/data/provider-credentials.json:110` lists it under
`environmentFallbacks` for the onepassword provider. It is how secretspec
authenticates *to* 1Password, read from the environment before any provider can
resolve, so folding it into the vault is a bootstrap-dependency problem, not a
simple import.

**Code facts established by reading, worth not re-deriving:**

- `broker.rs:607` — the authoritative runtime manifest is `<vault>/secretspec.toml`.
  `cfg.declarations` is only compared against it (`:685-696`) to warn on drift.
  Both must be set to the same file.
- `main.rs:471-497` — `install` re-execs itself under `sudo` with
  `--non-interactive`, resolving prompts in the unprivileged pass and passing
  them as explicit flags. Supply every flag and no prompt can fire.
- `install.rs:676-680` — `timestamp_timeout=0` is scoped to
  `{prefix}/bin/sudo-secretspec` **only**. Generic `sudo` uses the normal ~5-min
  timestamp, which is why one `sudo -v` covers a whole script.
- `main.rs:330-344` — declaration auto-detect order: `$SUDO_SECRETSPEC_DECLARATIONS`,
  `$HOME/ops/site-private/secretspec.toml.example`,
  `/usr/local/share/sudo-secretspec/secretspec.toml`, then cwd-relative.
- `main.rs:388-398` — `detect_existing_vault()` prefers `/var/db/stayturgid-secrets`
  over `/var/db/sudo-secretspec`. Relevant if anyone runs a bare `install`.
- `install.rs:894-908` — the non-adopt branch truncates `<vault>/.env`.

## Operator Feedback

- **"It may either be already done or incorrect, please do not assume it is
  true."** The standing mode for this chain. It paid off repeatedly: the
  supplied grep could not detect what it was checking for, the supplied
  `sqlite => :build` was wrong, and the operator's own read of the migration
  direction was inverted.
- **"I want errors if anything tried to access them."** The point of the
  retirement is discovery of stale consumers, so breaking the stayturgid
  wrapper is a feature, not collateral damage — flagged and accepted.
- **"I don't want rm as then they could just be re-created."** Drove the
  tombstone design, and by extension the `uchg` finding.
- **"Maybe we should merge them ... using the proper tools, not at a file
  level."** Correct instinct; the merge turned out to be already done.
- **"The script doesn't need flags, we can just change what it does between
  runs."** Tables at the top of the script, edit and re-run.
- **"Make sure it makes it easy for you to debug."** Hence timestamped logs
  with per-command exit codes, a shareable main log, and secret NAMES split
  into a separate 0600 `.check-detail` that is explicitly not for sharing.
- Repo policy (`CLAUDE.md`): commit and push directly to `sudo-main`; never
  touch `main`. `~/CLAUDE.md`: `~/ops` is deploy-only.

## Where We're Going

1. **THE NEXT ACTION — fix the stayturgid wrapper, which is now broken.**
   `/usr/local/libexec/stayturgid-secretspec-wrapper.sh` hardcodes
   `VAULT_DIR=/var/db/stayturgid-secrets` and runs as `_secretspec`; that path
   is now `root:wheel 0000`, so it fails `EACCES` on next invocation. Source of
   truth is `~/ops/stayturgid/control/bin/stayturgid-secretspec-wrapper.sh`
   (byte-identical to the deployed copy). This is design items 9–10 and it
   CROSSES A REPO BOUNDARY: per `~/CLAUDE.md`, `~/ops` is deploy-only, so it
   needs a task workspace under `~/src/ops-worktrees/`, a PR, and a coordinated
   `ops-vX.Y.Z` release. Also check `~/ops/stayturgid/control/bin/publish_secrets.sh`,
   which invokes the wrapper. Needs an operator go-ahead to start.
2. **Decide item 12's scope** — Linux/BSD only, or including Windows? Windows
   sudo is a UAC shim with no sudoers file, no NOPASSWD, no timestamp control,
   so the mediated-broker model has no Windows expression. `4e3a` recommends
   Linux/BSD only, reopening Windows against design item 11. Operator-only;
   blocks all item 12 code.
3. **Decide what happens to `OP_SERVICE_ACCOUNT_TOKEN`.** Left in place at
   `~/.config/secretspec/op-service-account-shizuku-keystore.env`, mode
   tightened 0644 → 0600 this session. Options: leave it (simplest, keeps the
   bootstrap dependency honest); declare it and inject via `sudo-secretspec run`;
   or move it to the login keychain. Note deleting the file would NOT revoke the
   credential — that must happen in 1Password. Nothing on this host appears to
   source it automatically.
4. **Retire the legacy vault properly, but not yet.** `/var/db/stayturgid-secrets`
   still holds the only second copy of the secrets and the pre-migration audit
   ledger. Leave it locked for a few days of normal operation first. Do not
   delete it while item 1 is open — the wrapper fix may want to read it.
5. **Consider a guard in `install.rs`**: refuse when `<vault>/.env` is non-empty
   and `--adopt-existing` was not passed. The truncation path is a real sharp
   edge that only a shell-level assertion currently protects against.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3        # expect 2fb72f5 at HEAD, sudo-main, clean

# Read this chain's last two handoffs first
$EDITOR docs/handoffs/HANDOFF_standalone-b2db_vault-migration-and-retirement_2026-08-15_e7dd.md
$EDITOR docs/handoffs/HANDOFF_standalone-b2db_release-repair-sudo-5_2026-08-15_6c20.md

# Repo baselines on this HEAD
cargo test -p sudo-secretspec-cli --locked        # 137 passed
~/.local/bin/pytest tests/sudo_packaging -q       # 21 passed
cargo clippy -p sudo-secretspec-cli --all-targets # 17 warnings (baseline)
cargo fmt --all --check                           # clean
# cargo test --all cannot build ext-php-rs here; `python3 -m pytest` does NOT work.

# Host state
sudo-secretspec doctor                             # OK, exit 0
sudo-secretspec check --reason "orientation" < /dev/null   # 43 found, 0 missing, 7 optional
#   `< /dev/null` MATTERS: with missing secrets, check drops into the engine's
#   interactive "Enter value for X" prompt and blocks. On a TTY with stdout
#   redirected, that prompt is invisible and it just hangs.

# Operational scripts (outside the repo; run from a REAL Terminal, as yourself)
~/backups/sudo-secretspec/migrate-to-canonical.sh --inspect
~/backups/sudo-secretspec/retire-noncanonical-secrets.sh     # no flags; edit tables at top

# Undo a tombstone
sudo chflags nouchg <path> && sudo rm <path>
# Undo the vault lock (original modes recorded in the manifest)
cat ~/backups/sudo-secretspec/retire-20260815-120900.manifest

# Backups
sudo ls -la /var/root/backups/sudo-secretspec-20260815-114407/
ls -la ~/backups/sudo-secretspec/2026-08-15-boundary-state/

# HOST GOTCHAS
#  * grep is ugrep 7.5.0: `grep -P '\xEF\xBF\xBD'` silently finds NOTHING on
#    files that contain U+FFFD. Use: grep -c $'\xef\xbf\xbd' <files>
#  * /bin/bash is 3.2.57: "${EMPTY[@]}" under `set -u` is a fatal error.
#  * sudo -n works ONLY for the broker path; everything else needs a TTY.
#  * `git clean -xdf` in ~/ops/site-private now FAILS on the immutable .env
#    tombstone. That is expected.
```
