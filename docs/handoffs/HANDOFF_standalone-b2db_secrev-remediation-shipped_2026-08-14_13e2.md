---
schema_version: 1
handoff_id: 13e2
parent_handoff_ids: [f01d]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: c4a324839927770cc8af3d6914863e925441cd33
created_at: 2026-08-14T06:24:24-0400
writer: claude-code
---

# Handoff — security-review remediation implemented, released and verified live

## The Goal

Parent handoff `f01d` closed with a written but unexecuted remediation plan
(`PLAN-SECREV-FIXES.md`, 507 lines, 10 phases) covering 2 SECURITY and 11
ADVISORY findings from the privilege-boundary review posted at
<https://github.com/djbclark/sudo-secretspec/pull/1#issuecomment-5289608095>.
It carried a Phase 0 gate of three facts that needed operator privilege, and
five open decisions.

The operator supplied the Phase 0 outputs and said **"Do everything as you
suggest or recommend"** — i.e. execute all 10 phases, take the recommended
side of every open decision, ship as one release, and verify against the live
boundary. That is what this session did.

## Where We Are

**Complete. Nothing from this plan is outstanding.**

- All 10 phases implemented as 6 feature commits on `sudo-main`, each with its
  own user-facing `CHANGELOG.md` entry under `Unreleased`.
- `v0.19.1-djbclark.3` tagged, GitHub Release published (not draft) at
  `2026-08-14T09:54:16Z`, formula restamped, tap
  `djbclark/homebrew-sudo-secretspec` synced at `45a013d`, `brew reinstall` +
  `brew test` green, readback verified.
- **Installed on this host and verified live.** Both
  `/usr/local/{bin,libexec}/sudo-secretspec` report `0.19.1-djbclark.3`;
  policy and config rewritten `06:21`. `doctor` exits 0 with only the two
  known `LEGACY_VAULT_CLUTTER` advisories.
- `HEAD` = `c4a3248`, working tree **clean**, pushed (`local == origin/sudo-main`).
- Tests **102 → 124**. Clippy held at baseline **17** warnings, 0 errors.
  `cargo fmt --check` clean. `pytest tests/sudo_packaging` 19/19.

### Files changed (`13f7938..c4a3248`, 16 files, +1727 −121)

| File | What |
|---|---|
| `sudo-secretspec-cli/src/broker.rs` | +438/− : env purge, HOME pin, `classify_terminal`, `lchown` backups, 0600 check, `--reason-sha256` |
| `sudo-secretspec-cli/src/audit.rs` | +332/− : `VerifyMode`, `verify_read_only`, residual-risk + `canonical_event_json` doc fixes |
| `sudo-secretspec-cli/src/drift.rs` | +143/− : `uid_for_user`, read-only verify, `.rollback.` handling |
| `sudo-secretspec-cli/src/install.rs` | +118/− : sudoers flags, `profile` in config, `prepare_install_dir` |
| `sudo-secretspec-cli/src/main.rs` | +81/− : asymmetric broker identity, `--profile`, client-side reason hashing |
| `sudo-secretspec-cli/src/config.rs` | +37 : `profile` field + `valid_profile_name` |
| `sudo-secretspec-cli/tests/{config,install_rollback}.rs` | +57 : profile parsing, policy flags |
| `docs/design/secrev-remediation-plan.md` | +507 : the plan, moved out of the repo root |
| `CHANGELOG.md` | +91 : new `### Security` section plus `### Fixed` entries |
| `packaging/release.py` | +8 : companion test gate |
| `Cargo.toml` / `Cargo.lock` / formula | version stamp `.2 → .3` |
| `PROMPT-SECREV.md` | stale test count `58 → 102` |
| `sudo-secretspec/AI-GUIDANCE.md` | truncation claim corrected; zeroization accepted |

## What We Tried

Chronological. The failures here are the expensive part to rediscover.

### 1. An 8-agent design-validation workflow — total loss

Launched `secrev-design-validation` (7 per-phase validators + 1 cross-phase
critic) to pressure-test the plan's code sketches against the real code before
implementing. **All 8 agents died on "You've hit your session limit · resets
5:30am"**, returning `{"validators":[],"critic":null}` after burning ~590K
subagent tokens, 228 tool uses and 326s of wall clock for zero output.

Lesson: near a session-limit boundary, a fan-out is a coin flip that costs
real budget. Everything after this was verified by hand — which is what caught
three plan defects the validators were supposed to catch (see Key Decisions).
**Do not re-run that workflow**; the work it was meant to check is done.

### 2. Taking the plan's `always_set_home` at face value

The plan said to add `always_set_home` to the libexec `Defaults!` line.
`man 5 sudoers` says it "has no effect unless the `env_reset` flag has been
disabled **or** `HOME` is present in the `env_keep` list" — and our policy
already sets `env_reset`, so as written it reads like a no-op.

Resolved by probe, not by reasoning (below). The answer changed the fix.

### 3. Running `install --adopt-existing` from a tool call

Hung with no output and had to be killed after 600s. Cause: `install`
elevates through `sudo`, and this project's own policy sets
`timestamp_timeout=0`, so it authenticates **every** time — a backgrounded /
non-TTY process has nowhere to prompt. Confirmed nothing was partially
applied (binaries, policy and config all still carried their pre-install
mtimes). The operator ran it in a real terminal and it succeeded first try.

### 4. `python3 -m pytest`

`No module named pytest` on this host. The `pytest` **binary** at
`~/.local/bin/pytest` works, and that is what `release.py` invokes
(`run(["pytest", ...])`), so the release gate was never affected.

### 5. Two small self-inflicted compile errors

`assert_eq!(&names, *want_names)` — `&Vec<String>` vs `Vec<String>`; dropped
the deref. And `open_connection` gained a third parameter, breaking its
`append_event` call site. Both caught by the first build after the edit.

## Key Decisions

### The five open decisions, all resolved as recommended

| # | Decision | Taken | Rejected |
|---|---|---|---|
| 2.3 | Profile pinning | **B** — `profile` key in the root-owned `0444` config, `serde(default)` so older configs still parse | A (hardcode `"default"` in `execute()`) — silently wrong if the vault ever uses another profile |
| 2.4 | Engine-side `set_ignore_global_config` | **Skipped** | Would be a permanent local delta in `secretspec/src/secrets.rs` to rebase on every upstream bump; the fork's shape is "companion crate beside an untouched engine" |
| 4.1 | `audit-verify` strictness | **Middle** — resolve `uid_for_user(service_user)`, pass `Some(uid)`, *without* `require_boundary` | Full boundary check — a drifted install could then no longer verify its own ledger, which is exactly when it matters |
| 8 | `--reason` handling | **A** — client sends `--reason-sha256` | B (delete the claim) — leaves argv exposure; C (pipe via stdin) — most work, least gain, and `set` already reads the value from stdin so the two would need framing |
| 10.1 | Zeroization | **Accept + document** in `AI-GUIDANCE.md` | Adding `zeroize` — best-effort at most given Rust reallocation, and the values are bound for the child's environment regardless |

### Six places this session deliberately departed from the plan

1. **Shipped `env_keep-="HOME"` *and* `always_set_home`**, not the plan's
   `always_set_home` alone. The probe showed each works independently; which
   one is load-bearing depends on a macOS default this project does not own.
2. **Purged the whole `XDG_` prefix**, not the plan's three named vars — the
   same predicate-not-a-list argument the plan itself makes for `SECRETSPEC_*`.
   Covers `XDG_CACHE_HOME` / `XDG_RUNTIME_DIR` and anything added later.
3. **`drift` still refuses a symlinked `.rollback.` entry.** The plan said to
   `continue` past `check_protected_file` for those entries, which would also
   have dropped `SYMLINK_FORBIDDEN` — and `Mutation::restore()` copies a backup
   back over the live manifest, so a symlink there is a redirection, not drift.
   Only the steady-state owner/group/mode rule is skipped.
4. **Phase 6 keeps the nested `share/sudo-secretspec`.** The plan's loop over
   `["bin","libexec","etc","share"]` would have created only `<prefix>/share`
   and dropped the nested dir that holds `MANIFEST.sha256`.
5. **Added `--profile` to the CLI** (not in the plan). Without it Option B had
   no supported way to set a non-default profile, since `install` rewrites that
   `0444` file on every run — the feature would have been inert.
6. **`audit::verify` kept its signature**; added `verify_read_only` beside it
   rather than threading a `VerifyMode` parameter through ~20 existing call
   sites. The mode is chosen by which function you call.

### Smaller implementation calls

- `lchown`, not `chown`, for rollback backups — a symlink appearing at a
  just-created path would otherwise redirect a root ownership change.
- `classify_terminal` and `prepare_install_dir` extracted as pure functions so
  the properties are testable: provoking the real failures needs root, a real
  vault and a colliding UUID, but the mappings are pure.
- The **digest** is passed to the engine's `with_reason`, so its JSONL audit
  now records the same value as the SQLite ledger and the two logs join on it.
  Cost: the engine's log no longer carries readable prose.
- **No plaintext fallback on version skew.** A newer client meeting an older
  broker fails with a "re-run install" hint. Retrying with `--reason` would
  hand anyone who can downgrade the broker a way to get the prose back.
- `lifecycle()` still uses `.status()` not `.output()` — capturing would put a
  copy of the secret `get` returns into the client's memory (ADVISORY-9's
  exact concern).
- A test asserts a **limitation on purpose**
  (`truncating_the_tail_and_rewriting_head_is_not_detectable`). If the chain
  is ever strengthened enough to break it, the doc note is what should change.

## Evidence & Data

### Phase 0 (operator-run, opened the gate)

- **(a)** `sudo /usr/bin/printenv HOME XDG_CONFIG_HOME XDG_STATE_HOME` →
  `/Users/djbclark`, both XDG vars unset. **SECURITY-1 live, not latent.**
- **(b)** Both `/var/db/stayturgid-secrets/{secretspec.toml,.env}` →
  `-rw------- _secretspec staff`. Phase 7's 0600 tightening safe to ship.
- **(c)** `/usr/local{,/bin,/etc,/libexec,/share}` all `root wheel drwxr-xr-x`,
  no `+` ACL marker. ADVISORY-7 latent, Phase 6 routine.

### The sudo/HOME probe (sudo 1.9.17p2) — do not re-run

Throwaway drop-in gating `/usr/bin/printenv` only, visudo-checked before going
live, removed by an `EXIT` trap. `/etc/sudoers.d` verified intact afterwards.

| Command-scoped flags | resulting `HOME` |
|---|---|
| `env_reset` only — **what shipped before** | `/Users/djbclark` ❌ |
| `env_reset,always_set_home` | `/var/root` ✅ |
| `env_reset,env_keep-="HOME"` | `/var/root` ✅ |
| `env_reset,env_keep-="HOME",always_set_home` | `/var/root` ✅ |

macOS's global `env_keep += "HOME"` is simultaneously *why* `always_set_home`
works and *why* `env_reset` alone was never sufficient.

### Mechanism confirmed by reading the dependency

`etcetera-0.11.0/src/app_strategy.rs` uses `create_strategies!(Apple, Xdg)` on
macOS — first arg is `choose_native_strategy`, second is `choose_app_strategy`.
SecretSpec calls `choose_app_strategy`, so on macOS it resolves through the
**Xdg** strategy: `$XDG_CONFIG_HOME` else `$HOME/.config`. Confirms
`~/.config/secretspec/config.toml`, not `~/Library/…`.

`secrets.rs:1855-1871` `resolve_profile_name` fallback chain: explicit arg →
`set_profile` → `SECRETSPEC_PROFILE` → **user-global config** → `"default"`.
Rung 3 closed by the purge, rung 4 by the HOME pin and by 2.3B.

### The live config that made it real

`~/.config/secretspec/config.toml` is `-rw-------` djbclark-owned and contains
`[defaults] provider = "dotenv"`, `profile = "default"`. The root broker was
reading it. No `[audit]` table present, so the dangerous half was **reachable
but not exercised**. Because it already said `default`, pinning the profile
changed no behaviour.

### Test progression

Start 102 (lib 50, audit 19, install_rollback 16, drift 11, cli 3, config 3).
Phase 1 → 104 · Phase 2 → 108 · Phase 3 → 111 · Phase 4 → 115 ·
Phases 5-7 → 118 · Phase 8 → 122 · Phase 9 → **124**.
Clippy 17 warnings / 0 errors at every checkpoint. `cargo test --locked` 124.

### Post-install verification (the proof the fix is live)

```
doctor: OK   (exit 0, only the 2 known LEGACY_VAULT_CLUTTER advisories)
```

Broker protocol, all three directions against the installed `.3` broker:

| Input | Result |
|---|---|
| `--reason-sha256 <valid digest>` | exit 0 — 43 found, 0 missing, 7 optional, `profile: default` |
| plaintext `--reason` | exit 2 — *"unexpected argument '--reason' … a similar argument exists: '--reason-sha256'"* |
| `--reason-sha256 e3b0c442…` (SHA-256 of `""`) | exit 2 — *"must be the hex SHA-256 of a non-empty reason"* |

**SECURITY-1 fixed, observed not inferred.** The post-install run announced
`recording secret access to /var/root/.local/state/secretspec/audit.log`.
That path resolves from `HOME`. The pre-install file at
`~/.local/state/secretspec/audit.log` — **djbclark-owned, 606303 bytes, mtime
Aug 14 05:38** — is the same engine log a *root* process had been writing into
the caller's home. Before and after in one artifact.

### Release facts

Tag `v0.19.1-djbclark.3`, published `2026-08-14T09:54:16Z`, not draft.
Tap `45a013d`. Formula `version "0.19.1-djbclark.3"`. Keg built in 10 min,
44.5MB, 12 files. `brew test` green.

## Operator Feedback

- **"Do everything as you suggest or recommend."** Blanket authorization to
  take the recommended side of all five open decisions and to ship. Treated as
  covering the release (tag + public GitHub Release), since the plan's Phase
  10.2 spelled those commands out.
- Supplied the Phase 0(a)/(b) outputs unprompted, in one message, as raw
  terminal transcript.
- Sent a bare **"continue"** mid-turn — no course correction implied.
- Asked **"Status?"** once; wanted live state re-checked, not a recap.
- Ran the blocked `install --adopt-existing` in a real TTY when told why the
  tool call could not.
- Standing repo policy (`CLAUDE.md:26`): all downstream work commits **and
  pushes** directly to `sudo-main` — no branches, no PRs.

## Where We're Going

1. **Nothing is required. This plan is finished end to end.** Do not re-open
   it, and do not re-run the design-validation workflow.
2. *Optional, cosmetic:* `sudo chmod 0440 /etc/sudoers.d/yabai`. Its mode is
   wrong, so `sudo` ignores that file and `visudo -c` reports it on every
   install — which is the "problem elsewhere in the sudoers configuration"
   warning the install printed. Our own drop-in parsed OK and installed.
3. *Optional, leftover data:* `~/.local/state/secretspec/audit.log` (606KB) is
   now superseded by the `/var/root` copy. It holds secret **names and
   reasons, never values**. Delete if the history is not wanted.
4. Still open from the **prior** chain, untouched by this session and not part
   of this plan: `doctor` does not report broken neighbours in `sudoers.d`
   (the yabai mode problem is the live example — design doc Phase 2 item 8);
   F5 wrapper hardening (design doc items 9-10); item 12 cross-platform sudo.
5. If a future session revisits the boundary, the ADVISORY-11 fix means
   `doctor`/`drift` now go through `audit::verify_read_only`. A hot journal
   cannot be replayed read-only, so an interrupted write surfaces as
   `AUDIT_VERIFY_FAILED` rather than being silently recovered. That is
   intended: the checker reports, the broker repairs.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -8          # expect c4a3248 at HEAD, tree clean
```

Confirm the boundary is still the one this handoff describes:

```bash
/usr/local/bin/sudo-secretspec --version      # 0.19.1-djbclark.3
/usr/local/libexec/sudo-secretspec --version  # 0.19.1-djbclark.3
/usr/local/bin/sudo-secretspec doctor         # OK + 2 LEGACY_VAULT_CLUTTER
```

Exercise the mediated path (note: `--reason` is refused by design now):

```bash
sudo -n /usr/local/libexec/sudo-secretspec __broker source-check \
  --client unknown \
  --reason-sha256 "$(printf 'smoke test' | shasum -a 256 | cut -d' ' -f1)"
```

The real gates — `cargo test --all` cannot build `ext-php-rs` on this host
(no `php-config`), and `python3 -m pytest` is not installed:

```bash
cargo test -p sudo-secretspec-cli     # 124 passing
cargo clippy -p sudo-secretspec-cli --all-targets 2>&1 | grep -cE '^warning|^error'   # 17
cargo fmt --check
pytest tests/sudo_packaging -q        # 19 passed  (binary, not `python3 -m`)
```

Every `gh` call needs the fork explicitly — this repo's default remote
resolves to upstream `cachix/secretspec`:

```bash
gh release view v0.19.1-djbclark.3 -R djbclark/sudo-secretspec
```

Reference material: the executed plan is `docs/design/secrev-remediation-plan.md`
(moved from the repo root this session); the review it implements is at
<https://github.com/djbclark/sudo-secretspec/pull/1#issuecomment-5289608095>;
parent handoff is `docs/handoffs/HANDOFF_standalone-b2db_security-review-and-fix-plan_2026-08-14_f01d.md`.
