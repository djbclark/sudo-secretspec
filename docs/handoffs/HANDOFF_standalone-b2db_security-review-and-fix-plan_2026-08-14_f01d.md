---
schema_version: 1
handoff_id: f01d
parent_handoff_ids: [3f21]
lineage: inferred
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: e640d0426e73df8d69218ee1682f72b3be4b6ef8
created_at: 2026-08-14T01:03:41-04:00
writer: claude-code
---

# Handoff — PROMPT-SECREV.md executed, findings posted, remediation plan written

> **Lineage note.** This session did *not* start from a resume prompt naming a
> parent; it started from the bare instruction `execute PROMPT-SECREV.md`.
> Parent `3f21` was recovered by chain match and is a clear continuation, hence
> `lineage: inferred`. Two Tier-2-unrecorded sessions sit between `3f21` and this
> one (commits `ecd03c2`, `df3be2f`, `0de3b49`, `e640d04` — F3 uninstall, the
> release-script parameterization, the `.2` release, and the `doctor --json`
> purity fix). Those *are* recorded in Tier 1's Recent History; read the
> canonical log at `~/.local/state/handoffs/chains/standalone-b2db/SESSION_LOG.md`
> for that gap.

## The Goal

Execute `PROMPT-SECREV.md` — the Rust privilege-boundary security review of the
`sudo-secretspec` companion — and post findings as PR comments on
<https://github.com/djbclark/sudo-secretspec/pull/1>.

Tier 1 had this listed as a **blocker** with the note *"PROMPT-SECREV.md still
never executed — operator is delegating it to a separate AI, by choice not
oversight."* The operator instead ran it here. That blocker is now cleared.

Two follow-on asks arrived after the review was posted:

1. Write a `.md` plan for fixing every finding, saying explicitly what is
   impossible, what needs an operator decision, and where a trade-off is
   involved.
2. Review what is currently on disk and report whether it differs significantly
   from what was reviewed.

A third arrived mid-turn: **order the plan by the sequence the fixes should be
done in**, not by severity.

## Where We Are

All three deliverables are done. **No production code was changed this session** —
it was review, analysis, and planning only.

- **Review posted:** <https://github.com/djbclark/sudo-secretspec/pull/1#issuecomment-5289608095>
  — 2 🔴 SECURITY, 10 🟡 ADVISORY, and a long 🟢 OK list covering every invariant
  in the checklist that holds.
- **Plan written:** `PLAN-SECREV-FIXES.md` at the repo root. **Currently
  untracked** (`git status` shows `?? PLAN-SECREV-FIXES.md`) — the next session
  must commit it or move it somewhere deliberate.
- **Disk vs. reviewed:** no security logic moved under me; details below.

Git state: branch `sudo-main`, HEAD `e640d0426e73df8d69218ee1682f72b3be4b6ef8`,
working tree clean **except** the untracked `PLAN-SECREV-FIXES.md`.

### The findings, in one table

| Tag | ID | One line | Exploitable today? |
|---|---|---|---|
| 🔴 | SEC-1 | Root broker loads the caller's `~/.config/secretspec/config.toml`; its `[audit]` table aims a root writer at any absolute path (`mkdir -p 0700`, open `0600`, append **plaintext reason**, and `set_len(0)` when `max_size_bytes` is crossed — set it to `1` for truncate-on-every-write). `[defaults] profile` also picks the manifest profile. | **UNRESOLVED** — depends on whether `HOME` survives `sudo`. See blockers. |
| 🔴 | SEC-2 | `broker.rs::execute` purges 4 of ~20 `SECRETSPEC_*` vars; the 4 `*_CLI_PATH` ones name an executable the engine spawns as root. | No — `set_provider` collapses every route to the pinned dotenv store. Latent. |
| 🟡 | ADV-1 | `fs::copy` doesn't chown, so root-created `.rollback.<uuid>` backups are root-owned → `drift` turns the deliberately-advisory `PENDING_ROLLBACK` into a hard `METADATA_MISMATCH` → `doctor` exits 1 → every agent wedges after a crashed mutation. | Yes (availability, not privilege) |
| 🟡 | ADV-2 | `Mutation::begin` failure is the one path that appends an attempt with no terminal event. | Yes (ledger integrity) |
| 🟡 | ADV-3 | Both verify paths pass `expected_uid = None`, skipping the ownership check on the command whose job is proving the ledger intact. | Yes |
| 🟡 | ADV-4 | `require_boundary` accepts `mode & 0o077 == 0` (so `0700`/`0400` pass) where `drift` demands exactly `0600` — the enforcing side is the looser one. | Yes |
| 🟡 | ADV-5 | `audit.rs` residual-risk note claims tail truncation is detected; it isn't — `head` lives in the same DB, so delete-tail + rewrite-head re-verifies cleanly. | Doc overstatement |
| 🟡 | ADV-6 | `--reason` crosses in plaintext argv; the checklist asserts it's hashed first. | Same-uid readable |
| 🟡 | ADV-7 | `validate_protected_ancestors()` omits `/usr/local/{bin,libexec,etc,share}` — the dirs the installer actually writes the NOPASSWD binary into — despite `is_protected_dir`'s doc claiming otherwise. | Latent on this host |
| 🟡 | ADV-8 | `release.py::run_tests` never runs `cargo test -p sudo-secretspec-cli`. | Process gap |
| 🟡 | ADV-9 | `run_target` keeps 3 unzeroized plaintext copies of every secret. | Bounded |
| 🟡 | ADV-10 | `invoked_as_privileged_broker()` fails **open** when `current_exe()` fails. | Defence-in-depth only |

### ⚠️ ADV-11 exists only in the plan file, not in the PR comment

Found *while writing the plan*, after the review was already posted:
`drift::inspect`'s module doc says "Never mutates state", but it calls
`audit::verify` → `open_connection`, which as root does
`set_permissions(0600)`, `libc::chown(…)`, and `ensure_schema`
(`CREATE TABLE IF NOT EXISTS`). A non-repairing checker that silently repairs.
Nothing dangerous happens today. **If you re-read the PR comment as the
authoritative finding list, you will miss this one** — `PLAN-SECREV-FIXES.md`
Phase 4.2 has it.

## What We Tried

Chronological, including the dead ends — these are the expensive ones to
rediscover.

1. **`gh pr view 1` resolved to the wrong repo.** With both `origin`
   (djbclark/sudo-secretspec) and `upstream` (cachix/secretspec) configured, bare
   `gh` picks **upstream**, and `gh repo view --json nameWithOwner` returned
   `cachix/secretspec`. `gh pr view 1` failed with *"Could not resolve to a
   PullRequest"*. **Always pass `-R djbclark/sudo-secretspec`** to every `gh`
   call in this repo.
2. **PR #1 turned out to be MERGED**, with HEAD five commits beyond it. Inline
   review comments on a merged diff would have anchored to stale lines and
   missed `uninstall.rs` entirely (it didn't exist in that PR). Posted one
   structured conversation comment instead, stating the review commit up front.
3. **Tried to verify whether `sudo` preserves `HOME` — could not.** This is the
   single unresolved fact in the whole review.
   - `grep env_keep /etc/sudoers` → permission denied (mode 0440, and this
     session was unprivileged by design).
   - `sudo -n /usr/bin/printenv HOME` → `sudo: a password is required`. No
     NOPASSWD rule covers `printenv`, and `-n` is what makes it safe to attempt.
   - Did **not** run interactive `sudo` — it would have prompted the operator
     mid-review for a fact-finding call.
   - Consequence: SEC-1's severity is stated conditionally in both the PR comment
     and the plan, with the exact command the operator must run.
4. **Empirically settled ADV-1 rather than reasoning about it.** Rust's
   `fs::copy` on macOS goes through `fcopyfile`, and whether `COPYFILE_STAT`
   chowns when running as root is not something to guess at. Compiled a
   one-line program in the scratchpad:

   ```bash
   printf 'x' > src.txt && chgrp everyone src.txt && chmod 600 src.txt
   cat > cp.rs <<'EOF'
   fn main(){ std::fs::copy("src.txt","dst.txt").unwrap(); }
   EOF
   rustc -O -o cp cp.rs && ./cp && ls -l src.txt dst.txt
   ```

   Result: `src.txt` `-rw------- djbclark everyone` → `dst.txt`
   `-rw------- djbclark wheel`. **Mode preserved, ownership not.** That single
   fact is what promotes ADV-1 from speculation to a confirmed defect, and it is
   also what proves `Mutation::restore()` is *safe* (the destination already
   exists, so the copy truncates in place and the inode keeps its owner).
5. **Chased the "is the dotenv pin absolute?" question into the engine** before
   deciding how to rate SEC-2. `plan.rs:64-65` and `plan.rs:225-226`: an explicit
   `--provider`/`SECRETSPEC_PROVIDER`/builder override *collapses* any per-secret
   `providers` chain to that single store with no fallback. So a manifest-declared
   chain cannot escape the pinned `dotenv:///var/db/…/.env`, which is exactly why
   SEC-2 is latent rather than live. Without this check the finding would have
   been overstated.
6. **Traced the user-global config path** rather than assuming `~/.config`:
   `secrets.rs:833` → `GlobalConfig::load()` → `config.rs:2609-2614`
   `choose_app_strategy(app_strategy_args())` with `app_name: "secretspec"` and
   empty domain/author → XDG strategy on macOS → `$XDG_CONFIG_HOME` else
   `$HOME/.config/secretspec/config.toml`. And `config.rs:1317-1320`:
   `etcetera::home_dir()` first, `$HOME` as fallback — so `HOME` governs. This is
   why the plan pins `XDG_CONFIG_HOME`/`XDG_STATE_HOME`/`XDG_DATA_HOME` and not
   just `HOME`.
7. **Enumerated the env surface instead of eyeballing it:**
   `grep -rhoE '"SECRETSPEC_[A-Z_]+"' secretspec/src --include=*.rs | sort -u`
   → ~20 names, four of which (`SECRETSPEC_OPCLI_PATH`, `_BWS_CLI_PATH`,
   `_PASSBOLT_CLI_PATH`, `_PROTONPASS_CLI_PATH`) name an executable.
8. **Ran the suite** — `cargo test -p sudo-secretspec-cli` → **101 passed, 0
   failed** (lib 50, audit 19, cli 3, config 3, drift 10, install_rollback 16).
   `PROMPT-SECREV.md` claims 58; that number is stale by 43.
9. **Did not use subagents or workflows.** The system prompt forbids both unless
   the operator asks, and the operator did not. The whole review is ~6,100 lines
   of Rust plus two packaging files — comfortably an inline read.

## Key Decisions

**Chosen:**

- **One conversation comment on the merged PR**, not inline review comments.
  Rejected inline because the diff is stale relative to HEAD; rejected opening a
  new issue because `PROMPT-SECREV.md` names PR #1 explicitly.
- **SEC-2 tagged SECURITY, not ADVISORY**, on the grounds that the threat-model
  table *claims* `execute()` purges ambient `SECRETSPEC_*` — so the stated
  invariant is violated even though nothing is exploitable today. The comment
  says so plainly rather than implying a live hole.
- **Env-pinning recommended over an engine-side `set_ignore_global_config`** for
  SEC-1. The engine fix is structurally better (it kills the class rather than
  the paths I can enumerate, and it would mirror the existing
  `set_ignore_ambient_scope` API), but it means a permanent delta in
  `secretspec/src/secrets.rs` to rebase on every upstream bump, against a fork
  whose whole shape is "companion crate beside an untouched engine". Recorded as
  *revisit if you'd upstream it* — upstreaming drops the maintenance cost to zero.
- **Plan ordered as a work sequence**, per the mid-turn instruction: zero-decision
  items first (they cost nothing and the test-gate protects everything after),
  then SEC-1, then broker correctness, with the decisions deferred to their own
  phases so implementation isn't blocked waiting on answers.
- **Everything batched into one release.** The sudoers change, the config-schema
  change, and every broker change each require a re-`install`, and each changes
  `MANIFEST.sha256` (so `doctor` reports `INSTALLED_HASH_MISMATCH` until the
  operator reinstalls). One reinstall, not six.

**Rejected:**

- Running interactive `sudo` to settle the `HOME` question mid-review — it would
  have prompted the operator for Touch ID during a read-only analysis pass.
- Adding the `zeroize` crate for ADV-9. Rust's `Vec`/`String` reallocation makes
  it best-effort anyway, and the values are headed into the child's environment
  regardless. Recommended documenting it as accepted residual risk instead.
- Implementing `audit-verify --expect-tip` as part of the ADV-5 fix. It's the
  only thing that makes the residual-risk advice actionable, but it's a feature,
  and it's only worth building if a watcher will actually run it.

## Evidence & Data

- **Review comment:** <https://github.com/djbclark/sudo-secretspec/pull/1#issuecomment-5289608095>
- **Plan:** `PLAN-SECREV-FIXES.md` (repo root, untracked) — 10 phases, a Phase 0
  verification gate, and a summary table with effort estimates and which items
  need a decision.
- **Tests:** `cargo test -p sudo-secretspec-cli` → 101 passed / 0 failed.
- **Host facts:** `uname -m` → `arm64`, so the Homebrew prefix is `/opt/homebrew`
  and `/usr/local` is uncontested. `ls -ld` confirms `/usr/local`,
  `/usr/local/bin`, `/usr/local/etc`, `/usr/local/libexec` are all
  `root wheel drwxr-xr-x` — which is why ADV-7 is latent here rather than live.
- **`fs::copy` ownership experiment:** command and result in *What We Tried* §4.
- **Key file:line anchors used in the review** (verified against `e640d04`):
  - `broker.rs:299-333` attempt-before-operation; `:336-343` `Mutation::begin`
    (the ADV-2 gap); `:349-356` restore/commit; `:422-434` the 4-name purge;
    `:441` `Secrets::load_from` (the SEC-1 entry point).
  - `audit.rs:26-30` the overstated residual-risk note; `:349-418`
    `open_connection` (the ADV-11 mutations at steps 4-5); `:455` the
    `canonical_event_json` comment that's wrong in the safe direction.
  - `drift.rs:588-622` the vault scan that double-judges `.rollback.` entries
    (ADV-1); `:625` `audit::verify(…, None)` (ADV-3).
  - `install.rs:78-91` `installed_artifacts`; `:558-581`
    `validate_protected_ancestors` (ADV-7); `:624-634` `sudoers_text`.
  - `main.rs:144-152` `invoked_as_privileged_broker` (ADV-10); `:570` plaintext
    `--reason` (ADV-6); `:640-644` the correct full-purge loop SEC-2 should copy.
  - Engine: `secrets.rs:825-868` `load_from`, `:913` `set_provider`, `:937`
    `set_profile`, `:977` `set_ignore_ambient_scope`, `:1855-1870`
    `resolve_profile_name`; `config.rs:1218-1242` `AuditConfig`;
    `secretspec/src/audit.rs:217-266` `JsonlSink::new`.

### Disk vs. what was reviewed (the operator's second question)

Reviewed at `ecd03c2`; HEAD is now `e640d04`, three commits later. Two landed
*during* the review — the `Cargo.toml` and formula version bumps arrived as
mid-session file-change notices.

| Commit | Change | Effect on the review |
|---|---|---|
| `df3be2f` | `release.py` parameterized (`--version`), workspace stamped `0.19.1-djbclark.2` | I read the **new** `release.py`; ADV-8 applies to it verbatim |
| `0de3b49` | Formula restamped to `.2` | None — formula findings stay 🟢 |
| `e640d04` | `visudo -c` switched `.status()` → `.output()` so its "parsed OK" line stops corrupting `doctor --json` | Unrelated to every finding; conflicts with nothing. Shifts `drift.rs` line numbers after ~526 by +5 |

**`broker.rs`, `audit.rs`, `install.rs`, `rollback.rs`, `uninstall.rs`,
`config.rs`, `main.rs` are byte-identical to what the review was written
against.** No security logic moved.

## Operator Feedback

- *"Make a plan .md file about how exactly to fix all of these problems. Tell me
  if it is impossible or I need to decide something or decide if a tradeoff is
  worthwhile."* — hence the explicit **[bounded]** markers for the three things
  that are only mitigable, and a decision table per phase rather than prose.
- *"Also review what is currently on disk … let me know if that differs
  significantly."* — answered as Phase 0 of the plan and repeated above.
- *"Also the plan should be in the order you think they should be fixed."* —
  arrived mid-turn; the plan is a work sequence, and the summary table preserves
  the finding IDs so it can still be read severity-first.
- Standing, from this repo's `CLAUDE.md`: downstream work goes on `sudo-main`,
  committed and pushed directly — no feature branches, no PRs.

## Where We're Going

1. **NEXT ACTION — run the three Phase 0 verification commands** (needs Touch ID;
   they are in `PLAN-SECREV-FIXES.md` Phase 0). The first one decides whether
   SEC-1 is live or latent and therefore how urgent the whole plan is:

   ```bash
   sudo /usr/bin/printenv HOME XDG_CONFIG_HOME XDG_STATE_HOME
   ```

   Prints your home → SEC-1 is **exploitable now**. Prints `/var/root` → still
   fix it, but it rides the release as defence-in-depth.
2. **Answer the five open decisions** — all documented with a recommendation in
   `PLAN-SECREV-FIXES.md`: profile pinning (2.3 A vs B), the engine-side option
   (2.4), how strict `audit-verify` should be (4.1), `--reason` hashing (8 A/B/C),
   and whether to accept ADV-9 (10.1).
3. **Implement Phase 1** — zero decisions, ~35 minutes total: add
   `cargo test -p sudo-secretspec-cli --locked` to `release.py::run_tests`, and
   replace the 4-name purge in `broker.rs::execute` with the full
   `SECRETSPEC_*` loop that `main.rs:640-644` already uses.
4. **Phases 2–9 in order**, one `CHANGELOG.md` Unreleased entry per change per
   this repo's `CLAUDE.md`.
5. **Ship as `0.19.1-djbclark.3`** (Phase 10.2): `python3 packaging/release.py
   --version 0.19.1-djbclark.3 --dry-run`, then live, then
   `sudo-secretspec install --adopt-existing`, then `doctor --json`. Note there
   is already one unreleased fix on `sudo-main` awaiting `.3` (`e640d04`).
6. **Decide where `PLAN-SECREV-FIXES.md` lives.** It is untracked at the repo
   root. `docs/design/` sits beside `privilege-boundary-and-packaging.md` and is
   probably the right home.
7. **Still open from the prior chain, untouched here:** Phase 2 item 8 — `doctor`
   does not report broken neighbours in `sudoers.d` (the `yabai` drop-in's mode
   is the live example `visudo -c` surfaces on every install and `doctor` stays
   silent about). Also the separate F5 wrapper-hardening workstream (design doc
   items 9-10) and item 12 cross-platform `sudo`.

**Blockers / open questions:**

- **Phase 0 (a) is unresolved and gates SEC-1's severity.** Nothing else in the
  plan is blocked by it.
- **Phase 7 (ADV-4 mode tightening) must not ship without Phase 0 (b).** If the
  adopted vault's files are `0400` or `0700` today, tightening to exactly `0600`
  turns every operation into an immediate fail-closed refusal.
- The five decisions above.
- Pre-existing, unrelated, carried forward from Tier 1: `cargo test --all` can't
  build `ext-php-rs` (no `php-config` on this host); `/etc/sudoers.d/yabai` has
  mode != 0440 so `sudo` ignores it and `visudo -c` reports it on every install.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect e640d04 at HEAD
git status -s                 # expect only ?? PLAN-SECREV-FIXES.md

# The plan, and the review it came from
$EDITOR PLAN-SECREV-FIXES.md
gh pr view 1 -R djbclark/sudo-secretspec --comments   # NOTE: -R is required here

# THE next action (Touch ID)
sudo /usr/bin/printenv HOME XDG_CONFIG_HOME XDG_STATE_HOME
sudo stat -f '%Sp %Su %Sg %N' \
  /var/db/stayturgid-secrets/secretspec.toml /var/db/stayturgid-secrets/.env
ls -lde /usr/local /usr/local/bin /usr/local/libexec /usr/local/etc /usr/local/share

# The real gates (cargo test --all is broken on this host for unrelated reasons)
cargo test -p sudo-secretspec-cli      # expect 101 passed
pytest tests/sudo_packaging -q

# Reproduce the fact ADVISORY-1 rests on, if you doubt it
cd "$(mktemp -d)" && printf 'x' > src.txt && chgrp everyone src.txt && chmod 600 src.txt
printf 'fn main(){ std::fs::copy("src.txt","dst.txt").unwrap(); }\n' > cp.rs
rustc -O -o cp cp.rs && ./cp && ls -l src.txt dst.txt   # mode kept, group NOT
```
