---
schema_version: 1
handoff_id: 2312
parent_handoff_ids: [a9e5]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 223a307dfdcace3198783f026676657ab2882264
created_at: 2026-08-15T18:05:00-0400
writer: claude-code
---

# Handoff — Post-install suite, five releases, and `undeclare`

## The Goal

Resume from `a9e5` and execute its plan: cut `v0.19.1-sudo.6`, reinstall the
boundary, verify, merge the three wrapper-retirement PRs, do a coordinated ops
release, re-enable the FIRERPA MCP bridge, remove the task workspace.

The operator added four things mid-session, in order:

1. Build a test suite covering all basic `secretspec` functions **after an
   install**.
2. Hand all FIRERPA work to another agent (Herdr pane `w1K:p1`) and point it at
   usage docs.
3. Cut a `.9` "where delete actually works".
4. Fix outdated/wrong documentation; freeze or remove dead directories, provided
   nothing is lost.

A fifth arrived from a peer session near the end: `add` had no inverse. That
became the last release.

## Where We Are

`HEAD 223a307` on `sudo-main`, tree clean, nothing unpushed.

Boundary installed and verified at **`0.19.1-sudo.10`** (client and broker):

| check | result |
|---|---|
| `doctor` | OK |
| `template-check` | rc=0 — runtime manifest matches template |
| `check` | 43 found, **0 missing**, 7 optional |
| `audit-verify` | 284 events, chain intact (tip recorded in Tier 1, not here) |
| `pytest tests/sudo_postinstall -q` | 23 passed, 8 skipped |

Five releases published today: `.6` (superseded), `.7`, `.8`, `.9`, `.10`.

Done and closed:

- All three wrapper-retirement PRs merged in order — site-private#82 →
  stayturgid#291 → site-djbclark#153, 18:47:52–18:47:59Z.
- Ops task workspace `~/src/ops-worktrees/secretspec-wrapper-retirement` removed.
- `~/src/sudo-secretspec-worktrees` (14M) deleted after verification.
- FIRERPA handed to session `djbclark-6e` / pane `w1K:p1`.
- Operator confirmed the leaked API key is rotated ("consider closed").

**No blockers.** Everything remaining is an operator decision or another
session's work.

## What We Tried

Chronological. The failures are the expensive part.

**1. Released `.6` with a latent installer bug; had to supersede it.**
The plan's first step was `install --adopt-existing`. I dry-ran it rather than
running it, and it planned to adopt `/var/db/stayturgid-secrets` with the retired
`_secretspec:staff` identity — on a host whose config names
`/var/db/sudo-secretspec` and `_sudo_secretspec`. `detect_existing_vault()`
(`main.rs:417`) scanned a hardcoded candidate list with the retired vault
**first** and never read the installed config. Migration leaves that directory on
disk deliberately (second copy of the secrets, plus the hash-chained
pre-migration ledger that could not be spliced), so a scan will always find it.
`.6` was already published, so `.7` superseded it — matching the `.4`→`.5`
precedent rather than deleting a tag.

**2. My own post-install suite fired a burst of Touch ID prompts at the
operator.** They asked twice what the prompts were for; I first told them nothing
of mine was waiting on them, which was wrong. `install`/`uninstall` re-exec
through plain `sudo` (`main.rs:490`, not `sudo -n`) and the installed policy sets
`timestamp_timeout=0`, so **every** invocation authenticates — including
`--dry-run`, which mutates nothing. Two suite tests called those. Gated behind
`SUDO_SECRETSPEC_POSTINSTALL_LIFECYCLE=1`.

**3. Leaked ~40 characters of a real API key into the session transcript.** I
printed the first 40 bytes of `export`'s stdout to learn its output *shape*,
expecting a variable name; `export` emits JSON **with values**. The suite now
parses that output and asserts on keys only, and both `tests/sudo_postinstall/README.md`
and the module docstring carry the rule. Operator has rotated the key.

**4. Wrote a test that would have called real `delete` against live provider
backends.** `every_provider_implementing_delete_declares_that_it_does` iterated
registered providers and invoked `delete` to infer capability. Deleted before it
ever ran.

**5. Tried adding `toml_edit` to the broker crate.** Build failed —
`sudo-secretspec-cli` has `toml`, not `toml_edit`. The failure was useful: adding
a TOML editor to a root-privileged binary was the wrong design anyway. Parsing
moved to `secretspec::manifest_edit`, which already owns `toml_edit`.

**6. Ran a mutating command while calling it a probe.** To verify `undeclare`'s
second guard I ran it against `CLAUDE_VERIFY_10D7D6` expecting a refusal — but
the peer had already deleted that value, so the precondition was false and the
operation simply *succeeded*. It performed the peer's cleanup, which I had told
them was theirs to run. Outcome was the intended one; the judgment was not. Check
a guard's precondition before "testing" it with a live call.

**7. Suite assumed the wrong streams.** First run: 15 failures. `check` writes
its report to **stderr** and keeps stdout clean; `export` writes **JSON** to
stdout. Both assumptions were wrong and both are now asserted explicitly.

**8. Picked the alphabetically-first declared name for value tests.** It was an
*optional, unset* secret, so `get` and `run` legitimately returned nothing. Split
the fixture into `declared_names` and `resolved_name`.

## Key Decisions

**Supersede `.6`, don't retract it.** Same as `.4`→`.5`. Tags stay; the changelog
names `.6` as superseded and says why. Rejected deleting the release — it was
already public, and a vanishing tag is worse than a documented dead one.

**Cut `.8` immediately for the `run` fix** rather than leaving it pending, so the
operator's one reinstall picked up everything. Rejected batching, which would
have meant reinstalling twice.

**`uchg`, not `schg`, for freezing the legacy vault.** `uchg` is clearable by
root, so a later decision to discard that history stays a normal operation.
`schg` would make it a single-user-mode chore.

**`undeclare` is narrower than `add`, by two guards.** A name in the tracked
template is refused (removal stays review-and-release, preserving what
AI-GUIDANCE always said); a name still holding a value is refused (otherwise the
value is stranded in the store with nothing declaring it). Consequence worth
keeping: `undeclare` can only move the runtime manifest **toward** the template.
Rejected an unguarded inverse — that would have made the NOPASSWD path able to
edit policy.

**`declares_secret` returns `Result<bool>`, not `bool`.** A malformed template is
`Err`, never `false`, so the fail-closed decision is visible at the broker's call
site rather than smuggled into a library default. "I could not parse it" is not
"the name is absent from it."

**Parse the template; never substring-match it.** A name inside a comment or
another secret's description would trip a naive check, and the inverse mistake is
silent.

**Left `docs/handoffs/` and `docs/design/` untouched** during the documentation
fix. They record what was true when written; editing them to match today destroys
the only thing they are for. Also left `CLAUDE.md`'s `-djbclark.N` reference — it
is correctly explaining the superseded scheme.

**Did not delete `/var/db/stayturgid-secrets`.** It holds the second copy of the
secrets *and* the only pre-migration audit ledger. That is a retention decision,
not cleanup. Wrote a freeze script instead (below).

**Deleted `~/src/sudo-secretspec-worktrees` only after proving nothing was
unique**: branch 55 behind / 0 ahead; all three uncommitted files already
superseded on `sudo-main`; no stashes; no local-only tags; nothing untracked but
`.ruff_cache/`; and **all 115 clone-only branches confirmed present on a remote**.

## Evidence & Data

**Releases** (all published, tap synced, `brew test` green):
`.6` 18:22:48Z · `.7` 18:37:56Z · `.8` 19:02:30Z · `.9` · `.10`.

**Commits this session** (oldest first):
`8e4e3c5` bump .6 · `b90e7d0` formula .6 · `2168d7e` **install vault-detection
fix** · `d3a9840` bump .7 · `fe959a3` formula .7 · `34909a3` **post-install suite
+ `run` basename fix** · `5636b96` bump .8 · `32c9f2f` formula .8 · `787b1cc`
**deletion-preflight fix** · `93217c5` bump .9 · `05f064a` formula .9 ·
`3e788ac` **mediated-surface docs** · `6b71922` **doc corrections** · `6adcb6d`
**`undeclare` + bump .10** · `223a307` formula .10.

**Test baselines, current:**

| suite | count |
|---|---|
| `cargo test -p sudo-secretspec-cli --locked` | **154** (was 140) |
| `cargo test -p secretspec --lib --locked` | **1206 passed / 21 failed** |
| `pytest tests/sudo_packaging -q` | 21 |
| `pytest tests/sudo_postinstall -q` | 23 passed, 8 skipped |
| `cargo fmt --all --check` / `clippy` | clean / exit 0 |
| `ruff format --check`, `ruff check` | clean |

The 21 engine failures are **all sops**, pre-existing, caused by no `sops` CLI on
this host. Filtering by `dotenv` surfaces 3 of them because sops test names
contain the word. Separately,
`provider::vault_common::tests::approle_get_many_refreshes_a_token_between_slow_waves`
is a genuine **flake under parallel load** — passed 3/3 in isolation. Do not
chase either.

**Bugs found and fixed, with how each was found:**

| bug | found by |
|---|---|
| `install --adopt-existing` adopts the retired vault | dry-running the reinstall instead of running it |
| `run` denied for any absolute-path command | the new post-install suite, first run |
| `check_deletable` approves deletions the provider can't do | investigating an operator-relayed claim |
| `add` had no inverse | peer session's end-to-end verification |
| `packaging/README.md` release examples all fail | grepping docs for stale version strings |
| two crate manifests stamped pre-rename | same sweep |

**`run` bug detail:** the client passed `target[0]` verbatim as the audit
ledger's `--command-basename`, which the broker validates as a *basename* (no
separators). So `run -- /bin/echo hi` was denied `audit denied: invalid command
basename`; only bare `run -- sh -c ...` ever worked. Fixed in
`audit::command_basename`, next to the validator it must satisfy.

**`check_deletable` detail:** `Provider::delete` defaults to unsupported, but
`check_deletable` defaulted to merely resolving coordinates. 19 of 28 provider
modules override neither, so preflight said yes and the deletion phase then
failed — after the copy phase had already written the destination. No secret was
ever at risk: copies precede deletions, so the first refusal aborts. Fixed with a
`supports_delete` capability defaulting to `false` in lockstep with `delete`.
**This does not give the 19 providers the ability to delete** — see open
questions.

**Providers implementing `delete`:** dotenv, file, gopass, keeper, keyring,
openbao, pass, vault (+ `vault_common` shared core). Of those, only dotenv,
keeper, openbao, vault, vault_common also override `check_deletable`.

## Operator Feedback

- **"do the cut and the whole plan"** — authorization for the full `a9e5` plan
  including releases and PR merges.
- **"After everything else, create a test suite that tests all of the basic
  functions of secretspec after an install."**
- **"I have given my fingerprint several times so I am not sure it is that"** and
  **"Just did my fingerpriny about 4 times. What for?"** — the Touch ID burst. My
  first answer was wrong; the second, grounded in `main.rs:490`, was right.
- **"But I will do it seperately that's fine"** — on running `install` themselves.
- **"Consider ANTHROPIC_API_KEY item closed."**
- **"Pass off everything having to do with firerpa to w1K:p1 - and also point it
  to doc on how to use the new sudo-secretspec."**
- **"We def want to do a .9 release where delete actually works."** Phrase has two
  readings; only the narrow one shipped. Flagged explicitly.
- **"I love getting rid of no longer needed directories as long as it is certain
  we will not lose info by removing them."** — the standard applied before the
  `sudo-secretspec-worktrees` deletion.
- **"Fix the outdated/wrong documentation, you have permission to ignore the
  forbids language for the update."**
- **"/var/db/stayturgid-secrets should have had the same owned by root / 0000 /
  ch?? as those other files."** — it already was root:wheel 0000; only the
  `chflags` was missing.

## Where We're Going

**1. THE NEXT ACTION — run the legacy-vault freeze script.** It is dry-run by
default. The full script is inlined in Quick Start below because the copy written
this session lives in a session-scratchpad that will not survive.

2. **Operator decision: implement `delete` for the 19 providers that lack it?**
   `.9` makes the preflight *refuse* honestly; it does not add the capability.
   That is a feature project (1Password, Bitwarden, AWS, Azure, GCP, KeePass, …),
   not a patch. **This deployment is unaffected** — the broker pins the dotenv
   provider (`broker.rs:619`), which does implement `delete`.

3. **Operator decision: expose `schema` / `init` / `cache` / `audit` on the
   companion?** `config` and `import` are now documented as permanently excluded,
   with reasoning, in AI-GUIDANCE's new `## Mediated surface` section. `schema` is
   read-only and safe; the other three are judgement calls.

4. **Operator decision: retention of `/var/db/stayturgid-secrets`.** Deleting it
   permanently discards pre-2026-08-15 audit history. Freeze first (step 1), decide
   later.

5. **Inherited, still open: design-doc item 12** (Linux/BSD `classify_neighbour`).
   Re-probe `drift.rs::classify_neighbour` on Linux first; also needs a delivery
   decision — no Linux packaging exists and it is excluded from both non-macOS CI
   legs (`test.yml:75`, `test.yml:154`).

6. **FIRERPA is not this workspace's work.** Handed to `djbclark-6e` / pane
   `w1K:p1`. `template-check` returning rc=0 was the last thing blocking their
   coordinated ops release.

7. **Optional: close out `undeclare` guard 2 against a live boundary.** Guard 1
   (template-tracked names refused) verified live. Guard 2 (value-holding names
   refused) is covered only by unit tests — `add` a throwaway, `set` it, then
   `undeclare` before `delete`; it should refuse.

## Quick Start

```bash
cd ~/src/sudo-secretspec
git log --oneline -3          # expect 223a307 at tip, clean, nothing unpushed

# Health of the installed boundary
sudo-secretspec doctor                                   # OK
sudo-secretspec audit-verify                             # chain intact
sudo-secretspec template-check --reason 'session start'  # rc=0
~/.local/bin/pytest tests/sudo_postinstall -q            # 23 passed, 8 skipped

# Full test sweep
cargo test -p sudo-secretspec-cli --locked   # 154
cargo test -p secretspec --lib --locked      # 1206 pass / 21 sops fail (expected)
~/.local/bin/pytest tests/sudo_packaging -q  # 21
```

**Post-install suite gates** (all off by default; see
`tests/sudo_postinstall/README.md`):
`SUDO_SECRETSPEC_POSTINSTALL_WRITES=1` (scratch value round-trip),
`SUDO_SECRETSPEC_POSTINSTALL_DECLARE=1` (`add`; **no inverse before `.10`**),
`SUDO_SECRETSPEC_POSTINSTALL_LIFECYCLE=1` (**one Touch ID prompt per test**).
`SUDO_SECRETSPEC_CLIENT=<path>` points the suite at a candidate build — that is
how a client-side fix is told apart from one needing a reinstall.

**Freeze script for the retired vault** (step 1 above). Already root:wheel 0000;
only the immutable flag is missing. Order is load-bearing — `uchg` blocks `chown`
and `chmod`, so flags come off first and go back on last:

```bash
cat > /tmp/freeze-legacy-vault.sh <<'EOF'
#!/bin/bash
set -euo pipefail
TARGET=/var/db/stayturgid-secrets
CANONICAL=/var/db/sudo-secretspec
APPLY=0; [[ "${1:-}" == "--apply" ]] && APPLY=1
die() { echo "refusing: $*" >&2; exit 1; }

[[ -e "$TARGET" ]] || die "$TARGET does not exist"
[[ -d "$TARGET" ]] || die "$TARGET is not a directory"
[[ -L "$TARGET" ]] && die "$TARGET is a symlink"
[[ "$TARGET" == "$CANONICAL" ]] && die "target is the CANONICAL vault"
if [[ -e "$CANONICAL" ]] && [[ "$(stat -f %d,%i "$TARGET")" == "$(stat -f %d,%i "$CANONICAL")" ]]; then
  die "$TARGET and $CANONICAL are the same inode"
fi
if [[ -r /usr/local/etc/sudo-secretspec.toml ]] \
  && grep -q "\"$TARGET\"" /usr/local/etc/sudo-secretspec.toml 2>/dev/null; then
  die "the installed boundary config still names $TARGET as its vault"
fi
(( APPLY )) && [[ "$(id -u)" -ne 0 ]] && die "--apply needs root"

echo "before:"; ls -ldO "$TARGET"
if (( ! APPLY )); then
  echo "DRY RUN. With --apply: chflags -R nouchg; chown -R root:wheel; chmod 000; chflags -R uchg"
  exit 0
fi
chflags -R nouchg "$TARGET"
chown -R root:wheel "$TARGET"
find "$TARGET" -type d -exec chmod 000 {} +
find "$TARGET" -type f -exec chmod 000 {} +
chmod 000 "$TARGET"
chflags -R uchg "$TARGET"
echo "after:"; ls -ldO "$TARGET"
echo "undo (required before deleting): sudo chflags -R nouchg $TARGET"
EOF
chmod +x /tmp/freeze-legacy-vault.sh
/tmp/freeze-legacy-vault.sh              # dry run
sudo /tmp/freeze-legacy-vault.sh --apply
```

### Gotchas that cost real time

- **Every lifecycle subcommand authenticates, even `--dry-run`.**
  `install`/`uninstall`/`rollback` re-exec through plain `sudo` (`main.rs:490`)
  under `timestamp_timeout=0`. Never put one in an unattended loop.
- **`check` reports to STDERR; `export` writes name→value JSON to STDOUT.** Never
  print `export` output — parse it and assert on keys.
- **`template-check` is a RAW BYTE comparison**, not semantic. Any undo must
  restore bytes exactly. `manifest_edit` uses `toml_edit`, which preserves
  untouched formatting; a test asserts `add`→`undeclare` round-trips identically.
- **`install --declarations` does NOT prune** runtime-only declarations.
  `install.rs` copies declarations into the runtime manifest only under
  `if !req.adopt_existing`; the adopt path merely asserts the runtime files
  exist. It is not a cleanup path.
- **`install --adopt-existing` used to scan a hardcoded vault list** and ignore
  the installed config. Fixed in `2168d7e`, but the retired vault directory still
  exists on purpose, so any future scan will keep finding it.
- **Reading an exit code through a pipe returns the LAST command's status.**
  `cargo check … | tail` reported `EXIT=0` while cargo had actually failed on
  `secretspec-php` (no `php` on PATH — pre-existing, unrelated).
- **`djbclark` still means three things**: the old org (rewrite to `frdminc`), the
  old version serial in tags (keep), and the operator's macOS username (keep).
  Match on `djbclark/` with the slash.

### Cross-session

`djbclark-6e` (Herdr pane `w1K:p1`, cwd `~`) owns FIRERPA. Messages sent this
session: `99211c49` (handoff), `520bb66a` (`undeclare` shipped), `4cf35eab`
(boundary at `.10`; I ran their cleanup by mistake). Addressing requires the full
`name [ref]` form on first send — a bare name is rejected with a confirmation
prompt. This session is pane `w1J:p1`.
