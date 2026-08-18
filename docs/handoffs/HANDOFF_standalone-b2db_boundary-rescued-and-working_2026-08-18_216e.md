---
schema_version: 1
handoff_id: 216e
parent_handoff_ids: [74fa]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: 4628cb2e37595a0eb9eb1947c234e1bc4bee6928
created_at: 2026-08-18T10:07:09-04:00
writer: claude-code
---

# Handoff — the boundary was broken in six ways, is now working, and other agents are live on it

## ⚠️ Read this before you touch anything

**The installed boundary is live shared infrastructure. Other AI agents are
actively using it right now.** The operator confirmed at 2026-08-18 ~10:00 ET
that another agent loaded a 192-character token through it successfully.

His instruction, verbatim:

> *"remember that you can not make it stop working again without talking to me
> as other agents may be actively using it."*

Treat `install`, `uninstall`, sudoers edits, vault migration, and provider
changes as **announce-first**, even though this repo's convention otherwise lets
you commit and push to `sudo-main` freely. Code changes are cheap. Interrupting
the boundary is not yours to do unilaterally. I broke it once this session (see
"What We Tried" #3) and that instruction is the direct result.

## The Goal

### The project goal (unchanged)

`sudo-secretspec` is a privilege-separated secrets broker: a root-owned vault at
`/var/db/sudo-secretspec/` reachable by unprivileged agents only through a
narrow sudoers-gated CLI. The feature under construction is **infinite secret
backup and restore**, redirected at `c124c0b` into a normal `secretspec` provider
plugin (`sqlite://`) so versioning is a property of the storage backend rather
than the privileged CLI.

### This session's goal

Two distinct phases:

1. Resume the chain (`/baton`) and, at the operator's request, produce a
   **work-distribution document** assigning all remaining work across the
   available AI fleet with model and effort recommendations. Shipped as `74fa`
   / `730793a`.
2. The operator then handed over from **another AI that had run out of tokens**,
   with a broken boundary and a request to *"aquanit yourslf with it and review
   it for correctness"*, then *"Fix all of them, but get it to a basically
   working state asap"*.

Phase 2 is what this document is mostly about.

## Where We Are

Branch `sudo-main` at `4628cb2`, **tree clean, pushed**. Second worktree
`/Users/djbclark/src/ss-370` at `b3637e2` (`spec-manifest-edit`), untouched.

**The boundary is WORKING and was verified live at 2026-08-18 10:0x ET:**

```
doctor          OK  (rc 0)
get             works — GITHUB_TOKEN 40 chars, ANTHROPIC_API_KEY 108, OPENROUTER_API_KEY 73
run             works — secrets inject into the child environment
check           47 found, 0 missing, 7 optional
template-check  "no tracked declaration source configured"  (rc 0; was permanently failing)
audit-verify    1554 events, chain intact
```

Retrieval was re-verified **after `sudo -k`** cleared the cached sudo ticket, so
the NOPASSWD path agents actually use is the one that was tested, not a ticket
that happened to be warm.

**All 39 real secrets are intact**, byte-identical to the pre-migration `.env`
by SHA-256, checked at session start *and* again at session end after every
mutation.

### The one caveat

**The installed binary is a hand-staged dev build reporting `0.19.1-sudo.19`
while carrying six unreleased commits.** It behaves identically to what a
`0.19.1-sudo.20` release would install; the version string is simply lying. Only
`just release` normally populates `target/libexec`, so the install was done by
`cp target/debug/sudo-secretspec target/libexec/sudo-secretspec` first.

### Commits this session

| SHA | What |
|---|---|
| `730793a` | (phase 1) handoff `74fa`, the AI work-distribution document |
| `adc6950` | `style`: `cargo fmt` the CLI crate — the step 2–8 work never ran it |
| `33af6fb` | `fix`: honour an absent `declarations` source instead of erroring |
| `de47eff` | `fix`: make the restore verbs real, and guard the store they read |
| `562102c` | `fix`: restore could never find a value, and the broker lacked `sqlite` |
| `4628cb2` | `style`: clippy cleanups in the block the two fixes rewrote |

Inherited unpushed from the previous agent and pushed with these: `7b99824`
(`HOME=/var/empty`).

## What We Tried

### 1. Verified the migration before believing anything

The previous agent had migrated the live vault onto `sqlite://secrets.db` —
step 6, the item `74fa` had explicitly marked *"do not delegate, operator
present, rehearse with a rollback, never one-shot"*. It had been done anyway.

Rather than assume the worst or the best, I compared the stores directly: 39
names in `.env`, 39 rows in `secrets.db`, name sets **identical**, and all 39
value digests **identical**. The migration was sound. Re-checked at session end;
still 39/39.

This is the check to repeat first in any future session that suspects vault
trouble. It never prints a value:

```bash
sudo sqlite3 -noheader -separator '|' /var/db/sudo-secretspec/secrets.db \
  "select item, value from secrets order by item;"   # compare digests, not text
```

### 2. Six defects, found in a cascade

Each fix exposed the next. None were caught by the 196-test suite, which passed
green through all six.

**(a) `declarations: None` was never implemented** (`c207078`, step D3). The
commit changed the field to `Option<PathBuf>` and left *both* consumers doing
`eprintln!` + `return (2, ...)` on `None` — delivering **neither** behaviour the
type change existed to enable. `docs/design/template-check-resync.md`
§"Decision, 2026-08-17" specifies both explicitly, and the code did the opposite
of both:

| Case | Design says | Code did |
|---|---|---|
| `template-check`, no source | success, "no tracked declaration source configured" | rc 2 |
| `undeclare`, no source | proceeds | rc 2 |

This is why `doctor` had been red, and why `undeclare` refused — which is what
blocked removing the `TEST_AGENT_KEY` that broke `run`. Fixed in `33af6fb`, with
guard 1 extracted into `tracked_source_protects` so the distinction that makes
relaxing it sound is testable.

**(b) History was off in production.** `broker::execute` built
`sqlite://{db}` with **no `?history=true`**, so the live database had only a
`secrets` table — no `entries`, no `captured_values`, no `head`. Everything step
5 added was inert, and not harmlessly: `source-destroy` calls `secrets.delete()`
**first** and only then touches history, so on a vault of 39 real credentials it
removed the value and *then* failed on the missing tables.

**(c) `destroy` tombstoned nothing.** `captured_values.item` holds the
provider's full `{project}/{profile}/{key}` address; `destroy` matched the bare
secret name. Zero rows updated — it reported success while every captured copy
of the value stayed readable. That is the precise opposite of the verb's purpose.

**(d) `restore` could never find a value** — the same address mismatch in
reverse. `--name X` compared a bare name against a full address and matched
nothing; `--all` pushed full addresses back through `secrets.set` as if they
were declarations. Both branches now read the sequence once and translate in one
place.

**(e) `restore` could blank a live secret.**
`String::from_utf8(blob).unwrap_or_default()` turned an unreadable capture into
the empty string, so a restore would overwrite a live secret with `""` and
report success. Now refuses. Two adjacent query paths could `panic!` or silently
yield nothing; both report.

**(f) The value store was unguarded.** The per-invocation integrity gate
(`broker.rs` ~line 197) checked `secretspec.toml` and the **retired** `.env` for
symlinking, ownership and mode `0600`, but never `secrets.db` — where the values
now are. It also *required* `.env`, so deleting the file the boundary no longer
reads would have bricked every call. Both are now checked **if present**;
`secretspec.toml` stays required. `secrets.db` cannot be required: the provider
creates it lazily, so requiring it would refuse the very `set` that creates it.

### 3. FAILED — I broke the live boundary, mid-session

Installing my debug build took the boundary down for every agent on the machine.

`sudo-secretspec-cli/Cargo.toml` takes `secretspec` with
`default-features = false, features = ["manifest-edit", "codegen-schema"]` —
**`sqlite` was never enabled**. So the broker built a `sqlite://` URI for a
backend it had not linked, and every operation failed with `Provider backend
'sqlite' not found` against a vault already migrated onto it.

Two things that did *not* catch this, and are the lesson:

- `install --dry-run` reported a clean plan. It validates the *plan*, not
  whether the binary works.
- The 196-test suite passed. Nothing exercises the assembled binary's provider
  registry.

Recovery took ~2 minutes (add the feature, rebuild, reinstall) and vault data
was never at risk — only the binary. But during that window any agent asking for
a credential got nothing and had no way to know why. **This is what produced the
operating constraint at the top of this document.**

### 4. Proved the feature end to end by hand

Since the unit suite had passed through six real defects, I stopped trusting it
for this block and ran the verbs against the live vault on a throwaway secret
(`HANDOFF_SMOKE_TEST`, cleaned up afterwards):

- `add` → `set` → history `entries` shows `1|set`
- `delete` → `2|delete`, value gone from `secrets`
- `restore --to 1` → value returned **byte-exact by digest**
  (`ee5d88698dabfa90` both sides)
- `destroy` → both captured copies (sequences 1 and 3) went to `value_blob`
  NULL with `destroyed_by = 4`; before the fix this updated **zero** rows

### 5. Kept the formatting sweep out of the fixes

`cargo fmt -p sudo-secretspec-cli` touched 6 files and 221 lines — the step 2–8
work had never been formatted. Rather than bury the fixes in that churn I saved
my edits, reset, committed the formatting alone as `adc6950`, then reapplied.
Worth repeating if you inherit more of that work.

## Key Decisions

- **`?history=true` is not optional for the boundary**, whatever it is for an
  ordinary provider user. The restore verbs read `captured_values`, and
  `destroy` deletes before it touches history, so an inert chain is a silent
  path to loss rather than a missing feature.
- **`secrets.db` and `.env` are checked *if present*, `secretspec.toml` stays
  required.** Rejected making `secrets.db` required: the provider creates it
  lazily, so a fresh vault legitimately has none.
- **`.env` is kept, not deleted.** It is vestigial — but it is the only
  pre-migration reference copy of all 39 values. Keep it until `0.20` ships and
  is proven. The gate no longer requires it, so removing it later is safe.
- **The engine's JSONL audit sink was pointed at `<vault>/.state`** via an
  explicit `XDG_STATE_HOME`, with `.state` added to `drift.rs`'s allowlist.
  Rejected alternatives: a sibling directory (the service user cannot create
  under root-owned `/var/db`, so it would need installer support) and disabling
  the sink (there is no public API — only `pub(crate) set_audit_for_test` — and
  the broker deliberately wants the sink, since it records the same reason
  digest as the SQLite ledger so the two can be joined).
- **`sqlite` added to the CLI's feature list; the other defaults stay off.** A
  root-adjacent process has no business linking cloud SDKs, `clap`, or
  `inquire`.
- **Deferred, deliberately**: narrowing `capture_history`. See "Where We're
  Going" #3 — it is a real design deviation but changing it interacts with
  delete-ordering, and the boundary had to be working first.

## Evidence & Data

- **Vault integrity**: 39 names and 39 SHA-256 value digests identical between
  the pre-migration `.env` and live `secrets.db`, verified at session start and
  again at session end. Zero mismatches, zero extras.
- **Live verification** (post-fix, post-`sudo -k`): `doctor` rc 0; `get` rc 0 for
  three real secrets (40 / 108 / 73 chars, values never printed); `run` injected
  both tested secrets; `check` 47 found / 0 missing / 7 optional;
  `template-check` rc 0; `audit-verify` 1554 events, tip
  `a02c0ee9f96cecc9639fb89e7e3454bcce293dcd2bca5ca75e960c87e0b3d144`.
- **Tests**: `cargo test -p sudo-secretspec-cli` — **196 passed, 0 failed**
  (134 lib + 62 across 6 integration binaries). Three new tests pin the
  `declarations` decision: `no_tracked_source_protects_nothing_so_undeclare_proceeds`,
  `an_unparseable_tracked_source_still_fails_closed`,
  `a_configured_tracked_source_still_protects_the_names_it_declares`.
- **Clippy**: `sudo-secretspec-cli/src/broker.rs` carries **zero** warnings of
  its own. Remaining workspace warnings are pre-existing elsewhere
  (`sops/*.rs`, `json_field.rs`, `provider/path.rs`, and 8 collapsible-`if` in
  other CLI files).
- **End-to-end proof**: restore digest `ee5d88698dabfa90` matched the sentinel
  exactly; `destroy` set `value_blob` NULL and `destroyed_by = 4` on both
  captured copies.
- **Vault contents now**: `.env`, `.state`, `broker-audit.sqlite3`,
  `broker-history.sqlite3`, `secrets.db`, `secretspec.toml`.
- **Upstream, verified this session**: #374 / #373 / #362 are PRs, **#372 is an
  ISSUE, not a PR** (which is why `gh pr view 372` fails — the inherited
  Tier 1 note listed all four undifferentiated). All open, all zero maintainer
  engagement; #362's only non-author comment is a Cloudflare bot.
- **Disk**: 80Gi free; `target/` is 42G.

## Operator Feedback

- **"Pick up from another ai that has run out of tokens… review it for
  correctness"** — the review was the deliverable, not just the reported
  symptom. Reviewing found four defects nobody had reported, two of which could
  destroy credentials.
- **"Fix all of them, but get it to a basically working state asap"** — chose
  breadth *and* speed over sequencing. Ship the safety fixes together, install
  once, verify live.
- **"Is it working for basic doctor and retrieval at the moment? Other agents
  need it."** — the real question was operational availability, not code
  status. Answering it properly meant clearing the sudo ticket and testing the
  NOPASSWD path, not just re-running `doctor`.
- **"you can not make it stop working again without talking to me as other
  agents may be actively using it"** — the standing constraint. Saved to memory
  as `never-break-the-live-boundary-unannounced`.
- **Earlier, phase 1**: wanted live `aiuse --json` data joined to capability
  judgment with explicit thinking levels, and *"context and goals not only
  steps"*. Also corrected the deliverable format mid-turn: *"To be clear this
  should be a .md document, and you should give me its path."* Give the path.
- **Standing pattern**: this operator interrupts mid-build with scope-narrowing
  questions, wants real research before a recommendation, and delegates
  judgment calls — but not the decision to end a session.

## Where We're Going

1. **THE NEXT ACTION — cut and install `0.19.1-sudo.20`** so the installed
   boundary stops reporting a version it is not. **Announce before installing**
   per the constraint at the top. `just release 0.19.1-sudo.20`; check disk
   first (80Gi free, `target/` 42G). The helper **publishes the GitHub Release
   before the Homebrew step**, so a brew failure leaves a published release to
   finish by hand per the justfile's RESUME RULES — do not re-run it.
2. **Add tests for `source-restore` / `source-destroy`.** That block has **no
   coverage at all**; every one of its defects was found by hand against the
   live vault. This is the highest-value follow-up: the block handles
   destructive operations on real credentials and the existing suite demonstrably
   passes through catastrophic bugs in it.
3. **Decide `capture_history`'s scope.** It snapshots the **entire** secrets
   table on **every** `set`/`delete` — with 39 secrets that is 39 rows and 39
   full plaintext blobs per mutation, growing O(mutations × secrets). The design
   called for per-name rows plus a whole-file digest. Related: `delete` captures
   *after* the row is removed, so the deleted value is absent from that entry
   and recoverable only from the **prior** entry — which is fine only while every
   mutation snapshots everything. These two must be decided together.
4. **Verify step 3 (provider docs) was actually done.** Claimed at `6d63519`,
   not checked this session. Run
   `npm --prefix docs run check:provider-credentials` and confirm all seven
   `CLAUDE.md` locations carry `(0.20+)`.
5. **CHANGELOG debt**: still no entry for `5efb816`, `bd17934`, `dff3830`. Also
   the step 2 entry still claims the sqlite provider has *"no history retention
   yet"*, stale since `?history=true` landed.
6. **`.env` cleanup** — only after `0.20` ships and is proven. It is the last
   pre-migration reference copy.
7. **Do NOT delete** branch `explore/pr-334-rust-first-spec` — `bf0b25c` is
   linked by SHA from a public comment on upstream #357.
8. **Watch upstream** #374 / #373 / #362 (PRs) and #372 (issue); rebase
   `ss-370` (`dfa4b10` → `35791a2`) before any review round.
9. **DEFERRED per explicit operator instruction, do NOT resurface**:
   cross-platform sudo/Linux port
   (`docs/design/privilege-boundary-and-packaging.md:487`).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect 4628cb2 at tip, clean, pushed

# 1. IS IT UP? Do this first — other agents depend on it.
sudo-secretspec doctor
sudo -k && V=$(sudo-secretspec get GITHUB_TOKEN --reason "availability check") \
  && echo "retrieval OK, ${#V} chars"      # never echo $V itself
sudo-secretspec check --reason "availability check" | tail -1
#   expect: doctor: OK / retrieval OK, 40 chars / 47 found, 0 missing, 7 optional

# 2. Vault integrity, without printing any value:
sudo sqlite3 /var/db/sudo-secretspec/secrets.db "select count(*) from secrets;"   # expect 39
sudo sqlite3 /var/db/sudo-secretspec/secrets.db ".tables"   # expect captured_values entries head secrets
sudo-secretspec audit-verify

# 3. Build — system SQLite, and the feature that broke it once:
export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
export LIBRARY_PATH="/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH"
export CPATH="/opt/homebrew/opt/sqlite/include:$CPATH"
cargo test -p sudo-secretspec-cli        # expect 196 passed, 0 failed
cargo clippy -p sudo-secretspec-cli 2>&1 | grep 'broker.rs'   # expect nothing

# 4. Installing — ANNOUNCE FIRST. Only `just release` populates target/libexec;
#    a dev install needs it staged by hand, and --dry-run will NOT tell you
#    whether the binary actually works:
# cp target/debug/sudo-secretspec target/libexec/sudo-secretspec
# sudo ./target/libexec/sudo-secretspec install
#    …then immediately re-run step 1.  If it fails with
#    "Provider backend 'sqlite' not found", the build lost the `sqlite`
#    feature in sudo-secretspec-cli/Cargo.toml.
```
