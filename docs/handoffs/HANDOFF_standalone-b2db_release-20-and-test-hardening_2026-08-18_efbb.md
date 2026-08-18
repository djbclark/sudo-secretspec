---
schema_version: 1
handoff_id: efbb
parent_handoff_ids: [216e]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: ed4f27eeda60e21a078b874e71a1db45c7231c69
created_at: 2026-08-18T11:21:51-04:00
writer: claude-code
---

# Handoff — 0.19.1-sudo.20 shipped, and the untested privileged paths got tests

## ⚠️ Read this before you touch anything

**The installed boundary is live shared infrastructure. Other AI agents use
it.** The operator's standing instruction, verbatim from handoff `216e`:

> *"remember that you can not make it stop working again without talking to me
> as other agents may be actively using it."*

`install`, `uninstall`, sudoers edits, vault migration and provider changes are
**announce-first**, even though this repo otherwise lets you commit and push to
`sudo-main` freely.

**A note on how that was interpreted this session, because it matters.** The
operator said *"Okay continue as you think best."* That was treated as
delegating **judgement**, NOT as the specific acknowledgement this constraint
asks for — so no boundary-touching action was taken on it. The release only
happened after the operator later said explicitly *"You can do both of those
things."* If you get a general go-ahead, it is not the ack. Ask.

## The Goal

### The project goal (unchanged)

`sudo-secretspec` is a privilege-separated secrets broker: a root-owned vault at
`/var/db/sudo-secretspec/` reachable by unprivileged agents only through a
narrow sudoers-gated CLI. The feature under construction is **infinite secret
backup and restore**, redirected at `c124c0b` into a normal `secretspec`
provider plugin (`sqlite://`) so versioning is a property of the storage
backend rather than the privileged CLI.

### This session's goal

Resume via `/baton` and work the follow-up list from handoff `216e`. It ran in
two phases: non-disruptive work first (tests, docs verification, CHANGELOG),
then — once explicitly approved — the held `0.19.1-sudo.20` release.

## Where We Are

Branch `sudo-main` at `ed4f27e`, **tree clean, pushed**. Second worktree
`/Users/djbclark/src/ss-370` at `b3637e2` (`spec-manifest-edit`), untouched all
session.

**`0.19.1-sudo.20` is RELEASED AND INSTALLED.** The version string no longer
lies — the installed binary had been reporting `.19` while carrying 11
unreleased commits. Verified live immediately after install:

```
version         sudo-secretspec 0.19.1-sudo.20
doctor          OK  (rc 0)
get             retrieval OK, 40 chars   (after `sudo -k`, so the NOPASSWD path
                                          agents actually use was the one tested)
check           47 found, 0 missing, 7 optional
secrets.db      39 rows
audit-verify    1588 events, chain intact
```

Install output: `installed sudo-secretspec 0.19.1-sudo.19 -> 0.19.1-sudo.20`,
adopted the vault its own config records, rollback snapshot
`/usr/local/libexec/sudo-secretspec-rollback-1787065653` (8 artifacts,
1 pruned).

**Test baseline moved 196 → 218.** No production behaviour changed by the test
work; the only non-test change was removing a dead parameter (below).

## What We Tried

Chronological, including what did not work — this is the expensive half.

### 1. Review scoping: got it wrong twice, operator caught both

**First attempt — limited the review to `sudo-secretspec-cli/src`.** The
operator asked *"Is there a good reason to limit the ultraview to /src?"*
There was not. That boundary was drawn around "the code I had just been working
in", not around what is at risk. It arbitrarily excluded:

- `sudo-secretspec-cli/tests/` (1,522 lines) — and this session's entire thesis
  is that a green 196-test suite passed straight through six defects. Reviewing
  the boundary but not its tests reviews half the safety argument. Worse, ~650
  lines of those tests were written *this session* and nobody has reviewed them.
- `packaging/` (724 lines: `release.py`, `stamp.py`, the Homebrew formula) —
  the **supply chain**, which had just been executed to publish `.20`. A defect
  there outranks any broker bug on blast radius, and it has failed before
  (`.15` died mid-publish at 100% disk, `.18` needed hand-finishing).
- `tests/sudo_postinstall/` (740 lines of Python) — the privileged postinstall
  suite.

**Second attempt — recommended a path target the command does not accept.**
Told the operator to type `/code-review ultra sudo-secretspec-cli`. It was
rejected:

```
"sudo-secretspec-cli" is not a branch in this repo. /code-review ultra takes a
PR number, a branch name, or no argument (reviews your current branch).
```

The failure was one of confidence calibration, not information: the caveat
*"if ultra rejects the path form, fall back to the bare form"* was actually
stated, but the uncertain form was still led with. **`/code-review ultra` takes
a PR number, a branch name, or NO ARGUMENT. Not a path.**

**The bare form was the right answer from the start, and is also the broadest.**
Current branch is `sudo-main` and `main` is only the upstream mirror, so the
branch diff is the entire fork delta — `166 files changed, 38539 insertions(+),
947 deletions(-)`. That sweeps in everything the hand-picked path plan was
reaching for and more. The whole three-target plan was solving a problem that
did not exist.

### 2. `/ultrareview` never actually started — no review has been run

The operator ran `/ultrareview` bare and got no confirmation. Diagnosis from
message shape: the first invocation arrived with `<command-name>`,
`<command-args>` and `<local-command-stdout>`; the second arrived as **plain
text with none of those**, consistent with it never dispatching.
`ListAgents` showed no reachable agents, which is weak evidence (it would not
necessarily show an operator-launched cloud review).

The operator confirmed: **it did not start.**

**Therefore no free review has been consumed — 3 remain, not 2.** An earlier
version of the Tier 1 log said "1 is now spent"; that was wrong and is
corrected here.

### 3. Two theories rejected on inspection, not assumed

- **`rollback.rs` is NOT uncovered.** It reads as 0 in-file tests, which looks
  alarming for a privileged path. It is well covered by
  `sudo-secretspec-cli/tests/install_rollback.rs` — `plan_restore` is
  deliberately split out from `run` precisely so its trust decisions test
  without root. The design note about "rollback never covering vault values" is
  a *scope* statement (line 4: "Never touches vault secret values"), not a gap.
  Same for `config.rs` via `tests/config.rs`. **Do not redo this survey.**
- **`capture_history` is NOT deviating from the design.** See Key Decisions.

## Key Decisions

### Chosen: test the restore verbs against a REAL vault, not a seeded table

`source-restore` / `source-restore-force` / `source-destroy` had **zero** tests.
All four defects previously found in them were found by hand against the live
vault while the suite passed green — the gap was the reason they shipped broken.

The tests drive the real `execute` against a real `sqlite://…?history=true`
vault rather than hand-seeding `captured_values`. **Rejected: hand-seeding.**
Three of those four defects were mismatches between the bare name a caller
passes and the full `{project}/{profile}/{key}` address the provider stores — a
hand-seeded table would have reproduced that wrongly and so still missed them.

### Chosen: verify every test by mutation before trusting it

Each test was checked by reverting the fix it pins and re-running. 4 of 4
caught. **This is what found a fifth gap**: dropping `?history=true` from the
URI `execute` builds left all 35 other tests green, because a fixture seeded
through a history-enabled handle supplies enough history for the restore verbs
to pass even when `execute` itself retains nothing. That combination is exactly
what makes `delete` an irrecoverable loss. Retention is now asserted on the
entry `execute` writes itself.

The same method on `history.rs` confirmed 4 of 4 weakened chain checks caught.

### Chosen: `capture_history`'s full snapshot is CORRECT — the premise was wrong

Handoff `216e` carried this as an open question, framed as *"the design called
for per-name rows plus a whole-file digest"*, implying the implementation
deviated. **It does not.**
`docs/design/infinite-secret-backup-restore.md:327` literally specifies *"Each
captured transaction stores one row per name, plus the digest of the whole
original file"*, and its `values_` table has `PRIMARY KEY (sequence, name)` —
a full snapshot. "Per-name rows" meant decomposing the `.env` so
`destroy`-by-name can NULL one row without breaking the hash chain; it never
meant capturing only the mutated name.

The design's `manifest_blob` / `env_file_sha` columns are correctly *absent*:
after the `c124c0b` redirect, the provider sees only the `secrets` table and has
no notion of a manifest file or a `.env`.

**The real issue is plaintext duplication**, and it is measured, not theorised.
See Evidence.

**Rejected: changed-items-only capture.** It breaks the "entry_hash covers the
whole state" property and adds exactly the reconstruct-walk logic that produced
four defects the first time. It would also require moving capture to *before*
the mutation first — `delete` calls `capture_history` AFTER removing the row
(`sqlite.rs:274-285`), so the deleted value lives only in the PRIOR entry. That
is safe **only** while every mutation snapshots everything. Any future change
here must handle that coupling or the deleted value is lost outright.

### Chosen: remove `history::list`'s `name` parameter rather than test around it

It was documented as *"preserved for API compatibility"* and then silently
discarded — a caller could filter by name and receive everything. That is the
same shape as defect 1 from `216e` (the `Option<PathBuf>` both consumers
ignored), which is this codebase's recurring failure mode. This store archives
*declarations, not values*, so it has no name to filter on. Removed, so the
absence is a compile error rather than a wrong answer. **Rejected: writing a
test asserting "name is currently ignored"** — that documents a bug as intended
behaviour.

No CLI verb exposes `list`, so nothing user-facing changed and no CHANGELOG
entry was owed.

### Chosen: release `.20` BEFORE the migration

`.20` ships the 11 commits already proven running, so the version string stops
lying against a known-good tree, and the unproven migration lands as `.21` on a
clean released base. **Rejected: bundling the migration into `.20`.**

### Chosen: two separate CHANGELOG entries for history, not one

There are **two** history stores with distinct roles, which is easy to miss:
`broker-history.sqlite3` (`sudo-secretspec-cli/src/history.rs`) archives the
**manifest**; `secrets.db` `captured_values`
(`secretspec/src/provider/sqlite.rs`) archives **values**.

## Evidence & Data

### Commits this session

| SHA | What |
|---|---|
| `8b2d2cf` | 10 tests for the restore verbs (+412 lines). No production code changed. |
| `d6137cf` | CHANGELOG: 3 missing entries + 1 stale correction |
| `890f3da` | 12 history tamper-evidence tests (+244) and `list`'s dead `name` param removed |
| `721de23` | `chore: stamp workspace 0.19.1-sudo.20` |
| `ed4f27e` | `Update Homebrew formula for v0.19.1-sudo.20` |

Diffstat `0a49e8e..HEAD`: 8 files, 692 insertions(+), 25 deletions(-).

### Test results

- `cargo test -p sudo-secretspec-cli`: **218 passing, 0 failed** (was 196).
- `cargo fmt -p sudo-secretspec-cli -- --check`: clean.
- `cargo clippy -p sudo-secretspec-cli --all-targets`: no warnings on the new
  code in `broker.rs` or `history.rs`.
- `npm --prefix docs run check:provider-credentials`: passes, **15 providers,
  34 credentials**.

Note: `tests/install_rollback.rs` prints a `syntax error` line from
`an_invalid_policy_never_reaches_the_live_path`, which deliberately stages an
invalid sudoers policy. **Expected output, not a failure.**

### The capture_history measurement (live vault, BEFORE any migration)

```sql
captured rows            158
distinct (item,digest)    40
blob bytes total        7988
blob bytes deduped      1997
entries                    4
secrets                   39
```

~75% redundancy after **four** mutations. Rotating any one secret writes a
fresh plaintext copy of the other 38; the copy count of an untouched secret
grows linearly with total mutations across the whole vault. Size is not the
crisis (~40MB at 10k mutations) — the confidentiality footprint is.

**Keep these numbers.** They are the baseline the migration must be verified
against.

### Provider-docs verification (step 3 from `216e`, previously unverified)

All seven CLAUDE.md locations carry `(0.20+)`, **both sub-locations each**:
provider page + version notice; sidebar **and** the `starlightLlmsTxt`
providers sentence; concepts table row; reference section **and** Security
Considerations row; `providerMetadata` **and** the hero mini-terminal;
quick-start `config init` output; README bullet **and** its `config init`
output. `6d63519` was a real claim. **Nothing further owed here.**

### Review scope sizing

| Target | Lines |
|---|---|
| repo root (`.`) | 93,650 (≈89% vendored upstream) |
| `sudo-secretspec-cli` (src + tests) | 12,117 |
| `packaging/` | 724 |
| `tests/sudo_postinstall/` | 740 |
| `secretspec/src/provider/sqlite.rs` | 805 |
| **`sudo-main` vs `main` (the bare review's actual scope)** | **166 files, 38,539 insertions** |

## Operator Feedback

- **The standing constraint** (verbatim, from `216e`, still in force): *"you can
  not make it stop working again without talking to me as other agents may be
  actively using it."*
- *"continue as you think best"* was issued while two boundary-touching items
  were held. Treated as delegating judgement, not lifting the constraint. The
  operator did not object, and later gave the explicit approval separately —
  which suggests that reading was right.
- **The operator caught both review-scoping errors.** *"Is there a good reason
  to limit the ultraview to /src?"* — there was not.
- The operator wanted the review to cover **as much as possible, not just one
  diff**. The bare no-argument form delivers that; a path target does not work
  at all.
- Deferred by explicit instruction, **do not resurface**: cross-platform
  sudo/Linux port (`docs/design/privilege-boundary-and-packaging.md:487`).

## Where We're Going

1. **THE NEXT ACTION — the operator runs the code review.** It cannot be
   launched by an agent; it is user-triggered and billed. Exactly this, no
   argument:
   ```
   /code-review ultra
   ```
   3 free reviews remain (none spent). If it visibly skips whole areas — watch
   for `packaging/` and `tests/` being absent from the findings — suspect
   truncation of a 38.5k-insertion bundle rather than a clean bill of health,
   and follow up with a review scoped **by branch, not path**.
2. **Then implement the `capture_history` migration** (approved). Tracked as
   Hindsight initiative page `kp-4591bfacb8f64679a468c647a5dc617f`. Fold in
   anything the review says about `sqlite.rs` first. Recommended schema:
   ```sql
   CREATE TABLE value_blobs (
     item TEXT NOT NULL,
     value_sha256 TEXT NOT NULL,
     value_blob BLOB,                 -- NULL once destroyed
     destroyed_by INTEGER REFERENCES entries(sequence),
     PRIMARY KEY (item, value_sha256)
   );
   CREATE TABLE captured_values (
     sequence INTEGER NOT NULL REFERENCES entries(sequence),
     item TEXT NOT NULL,
     value_sha256 TEXT NOT NULL,
     PRIMARY KEY (sequence, item),
     FOREIGN KEY (item, value_sha256) REFERENCES value_blobs(item, value_sha256)
   );
   ```
   Restore semantics unchanged ("state at sequence N" is still the rows at N);
   `entry_hash` unchanged (it already binds digests, not bytes); plaintext drops
   from O(mutations × secrets) to O(distinct values); `destroy` collapses from a
   cross-entry scan — where defect 3 lived — to a single-row update. Keying on
   `(item, value_sha256)` rather than digest alone stops destroying one secret
   collaterally destroying another that happens to hold the same value.
   **Requires a migration for the live root-owned vault → announce-first.**
   Ship as `0.19.1-sudo.21` and spend a review on it.
3. **Verify the migration against the recorded baseline** in Evidence above
   (158 rows / 40 distinct pairs / 7988 → 1997 bytes / 39 secrets).
4. `/var/db/sudo-secretspec/.env` is vestigial but is the **last pre-migration
   reference copy of all 39 values**. `.20` has now shipped and is proven, so it
   is removable — **confirm with the operator first.**
5. Do **NOT** delete branch `explore/pr-334-rust-first-spec` — `bf0b25c` is
   linked by SHA from a public comment on upstream #357.
6. Upstream: #374 / #373 / #362 are PRs, **#372 is an ISSUE not a PR**
   (`gh pr view 372` fails). All open, zero maintainer engagement. Rebase
   `/Users/djbclark/src/ss-370` (`dfa4b10` → `35791a2`) before any review round.

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect ed4f27e at tip, clean, pushed

# 1. IS IT UP? Do this first — other agents depend on it.
sudo-secretspec --version                       # expect 0.19.1-sudo.20
sudo-secretspec doctor
sudo -k && V=$(sudo-secretspec get GITHUB_TOKEN --reason "availability check") \
  && echo "retrieval OK, ${#V} chars"           # never echo $V itself
sudo-secretspec check --reason "availability check" | tail -1
#   expect: doctor: OK / retrieval OK, 40 chars / 47 found, 0 missing, 7 optional

# 2. Vault integrity, without printing any value:
sudo sqlite3 /var/db/sudo-secretspec/secrets.db "select count(*) from secrets;"   # 39
sudo-secretspec audit-verify                    # expect chain intact

# 3. The capture_history baseline the migration must be checked against:
sudo sqlite3 /var/db/sudo-secretspec/secrets.db "
  select 'captured rows', count(*) from captured_values
  union all select 'distinct (item,digest)', count(*)
    from (select distinct item, value_sha256 from captured_values);"
#   expect 158 / 40 as of this handoff

# 4. Build — system SQLite. Required for EVERY cargo command here:
export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
export LIBRARY_PATH="/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH"
export CPATH="/opt/homebrew/opt/sqlite/include:$CPATH"
cargo test -p sudo-secretspec-cli        # expect 218 passed, 0 failed
cargo clippy -p sudo-secretspec-cli --all-targets 2>&1 | grep 'broker.rs\|history.rs'

# 5. Releasing — ANNOUNCE FIRST. `just release-dry <version>` publishes nothing
#    and is worth running first; it caught nothing this time but costs a minute.
#    `just release` publishes the GitHub Release BEFORE the brew step, so a brew
#    failure leaves a published release to finish BY HAND per the justfile
#    RESUME RULES — do not re-run it. Installing is separate and explicit:
# sudo /opt/homebrew/opt/sudo-secretspec/libexec/sudo-secretspec install
#    …then immediately re-run step 1.
```
