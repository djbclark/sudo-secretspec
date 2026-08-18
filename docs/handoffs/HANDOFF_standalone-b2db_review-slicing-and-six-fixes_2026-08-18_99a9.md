---
schema_version: 1
handoff_id: 99a9
parent_handoff_ids: [efbb]
lineage: deterministic
chain: [standalone-b2db]
repo: sudo-secretspec
workspace: privilege-boundary
branch: sudo-main
head_sha: cf30054a2327a8e2d1436afe0913cb0c15a87d9e
created_at: 2026-08-18T12:41:00-04:00
writer: claude-code
---

# Handoff — the review finally ran, and six of its seven findings are fixed

## ⚠️ Read this before you touch anything

**The installed boundary is live shared infrastructure. Other AI agents use
it.** The operator's standing instruction, carried forward unchanged from
`216e` and `efbb`:

> *"remember that you can not make it stop working again without talking to me
> as other agents may be actively using it."*

`install`, `uninstall`, sudoers edits, vault migration and provider changes are
**announce-first**. This session wrote code that changes provider behaviour and
never installed any of it — the running boundary is still `0.19.1-sudo.20`,
byte-for-byte what `efbb` left. **The next session's first task changes the
sudoers policy**, which is squarely inside that constraint.

## The Goal

### The project goal (unchanged)

`sudo-secretspec` is a privilege-separated secrets broker: a root-owned vault at
`/var/db/sudo-secretspec/` reachable by unprivileged agents only through a
narrow sudoers-gated CLI. The feature under construction is **infinite secret
backup and restore**, redirected at `c124c0b` into a normal `secretspec`
provider plugin (`sqlite://`).

### This session's goal

Resume via `/baton` and run the code review `efbb` had queued as THE next
action. That turned out to be impossible as specified, so the real work became:
make the review runnable at all, then act on what it found.

## Where We Are

Branch `sudo-main` at `cf30054`, **tree clean, pushed**. Three commits, all
pushed:

| commit | what |
|---|---|
| `4f8eb2e` | removed committed scratch scripts `patch.py`, `patch_test.py` |
| `b3271cd` | release-path fixes: archive-fetch retry, failed-command diagnostics |
| `cf30054` | six defect fixes from the review |

Second worktree `/Users/djbclark/src/ss-370` at `b3637e2` (`spec-manifest-edit`),
untouched all session. Third worktree `/Users/djbclark/src/sudo-secretspec-review`
exists solely to host the review slices (see below).

**The boundary was verified UP at session start and never touched after:**
version `0.19.1-sudo.20`, `doctor: OK`, retrieval OK 40 chars (after `sudo -k`,
so the NOPASSWD path agents actually use was the one tested), `check` 47 found /
0 missing / 7 optional.

**Test baseline 218 → 223** in `sudo-secretspec-cli`, plus one new test in the
`sqlite` provider (16 in that module) and **21 → 26** in `tests/sudo_packaging`.
No new clippy warnings: 21 before, 21 after, compared by stashing.

**The ultra review ran and is spent: 1 of 3 free reviews used, 2 remain.**

## What We Tried

Chronological, including what failed — this is the expensive part to rediscover.

1. **Bare `/code-review ultra`, as `efbb` instructed. It failed.** "Diff is too
   large: 217 files, 47,739 lines changed (limits: 500 files, 12,000 lines)."
   `efbb`'s next-action was confidently wrong, and its confidence is why it was
   worth writing down that it failed.

2. **Diagnosed the size.** Two separate causes. The tool bases the diff on
   `origin/main` (`b51c378`), which is an *ancestor* of local `main`
   (`4b8ae3e`) — that alone inflates 167 files to 217. And ~12,400 of the
   remaining lines are pure prose (`docs/handoffs` 9,466, `docs/design` 1,961,
   `docs/src` 951) that would have burned review budget for nothing. Even the
   correct base is 39,888 lines, 3.3× the cap.

3. **Considered and rejected chronological slicing** (pass an ancestor commit on
   `sudo-main` as the base). The fork's history is linear and thematically
   mixed, so a commit-range slice cuts across every subsystem at once.

4. **Considered and rejected "base branch = sudo-main minus the slice".** If the
   base is a *child* of `sudo-main` the merge-base is `sudo-main` itself, so a
   three-dot diff is empty. The base must be a true **ancestor** of the branch
   under review. This constraint drove the entire design and is the single most
   important thing to preserve.

5. **Built the slice generator that works** (see Key Decisions). First cut was
   two slices, A (8,103 lines) and B (7,317). When the operator capped the
   budget at one review, A+B could not merge: 15,420 > 12,000. Rebuilt as a
   single `core` slice at 11,326 with a 4,094-line `rest` slice held in reserve.

6. **`git checkout packaging/release.py` to undo a mutation test — and it
   reverted my own uncommitted fixes along with it.** Had to reapply everything.
   Use a file copy (`cp file /tmp/bak`) as the mutation-test backup, never
   `git checkout`, when the working tree holds unstaged work.

7. **My first partial-download test was worthless and the mutation run caught
   it.** `HalfThenFail.read` raised immediately without ever yielding bytes, so
   hoisting the hasher out of the retry loop changed nothing and the test still
   passed. Rewrote it to serve one chunk and *then* fail. This is the second
   session running where mutation testing caught a bad test rather than bad
   code — it is earning its keep.

## Key Decisions

**Review slicing: base = fork delta MINUS slice, tip = base + slice.** Each
slice is two commits on a branch rooted at `main`: a scaffold commit holding the
entire fork delta *except* the slice paths, then a commit adding them back. The
diff is therefore exactly the slice, while the checkout is the complete fork so
reviewers can read every surrounding file. Every tip is verified byte-identical
to `sudo-main` with `git diff --quiet sudo-main HEAD`. Rejected the obvious
alternative — checking out only the slice's files — because it would have left
reviewers unable to read the code around what they were judging.

**One review, not three.** The operator asked to save the free reviews for other
projects. Cut `drift.rs`, the release tooling and the postinstall tests from the
slice and reviewed those **locally, for free**, rather than spending a second
billed review. That local pass found a real defect the cloud review never saw
(finding 7 below) plus two in `release.py`, so the free path was not a
consolation prize.

**Finding 5 refuses rather than fabricating attribution.** `source-destroy` on a
name with no live value has no history entry to point `destroyed_by` at.
Considered: synthesising an entry from the broker (rejected — would duplicate
the provider's hash-chain logic in a second place, and getting a chain wrong is
worse than refusing), and relaxing the `CHECK` constraint to allow a null
`destroyed_by` (rejected — schema change on the live vault, which belongs with
the `.21` migration, not smuggled in beside it). Chose the refusal, and wrote
the limitation into the error message. **This leaves a real functional gap**:
captured copies of an already-source-deleted name currently cannot be destroyed
at all. Closing it properly is a candidate for the `.21` migration work.

**Extended finding 6 past what the review said.** The review flagged the missing
`PRAGMA foreign_keys` in the provider. The broker opens its *own* `rusqlite`
connection for the tombstone `UPDATE` — the exact write the `REFERENCES` clause
exists to constrain — and the pragma is per-connection, so fixing only the
provider would have left the important write unguarded. Both now set it.

**Finding 4's fix removes a file.** When `restore()` finds no backup because the
source never existed, reporting a clean rollback while leaving the file the
failed mutation created would be the same misreport inverted. Verified from the
call site (`broker.rs:687`) that `restore()` runs *only* on the failed-operation
path, so the only thing that can have created that file is the mutation being
undone.

**No upstream tickets filed, deliberately.** The operator asked to search
upstream and comment or open an issue if any finding was upstream's. Checked:
none are. `install.rs`, `broker.rs` and `drift.rs` live in `sudo-secretspec-cli`,
a crate that does not exist upstream; `sqlite.rs` is fork-added (`239d1ab`),
absent from `origin/main`, and in neither open upstream PR; and upstream
`secretspec/src` contains no `rusqlite`, `REFERENCES` or `foreign_keys` usage at
all, so finding 6's pattern cannot exist there. Filing any of these would have
been reporting our own bug against someone else's repo.

**Did not install anything.** Every fix in `cf30054` is code-only. Taking the
`.21` release and install as a separate, announced step keeps the running
boundary at a version that is proven.

## Evidence & Data

**The six cloud findings, all verified live on `review/slice-core`, all now
fixed in `cf30054`:**

| # | site | defect |
|---|---|---|
| 1 | `install.rs:840` | **NOT FIXED — next session.** sudoers grants `source-restore --*`, which also matches `--all` |
| 2 | `broker.rs:1085` | `Connection::open` defaults to `SQLITE_OPEN_CREATE`, creating an empty schema-less `secrets.db` |
| 3 | `broker.rs:301-329` | `Mutation::begin` leaves an orphan rollback copy on partial failure |
| 4 | `broker.rs:346-361` | `Mutation::restore` misclassifies a first mutation as `Unknown` (exit 125) |
| 5 | `broker.rs:1104` | `source-destroy` discards `secrets.delete`'s bool → tombstone stamps an unrelated entry |
| 6 | `sqlite.rs:164-172` | no `PRAGMA foreign_keys=ON`, so `REFERENCES` clauses are documentation |

**Finding 7, from the local review of the excluded material** —
`drift.rs:415`: `path_winner` used `symlink_metadata().is_ok()`, i.e. treated
"exists" as "the shell would run it". A non-executable file *or a directory*
named `sudo-secretspec` on the caller's `PATH` raised `CLIENT_SHADOWED`, which
is not in `ADVISORY_CODES` and so hard-fails `doctor` — which AI-GUIDANCE tells
every agent to treat as a stop. Fixed; the old test was literally named
`path_winner_picks_the_first_existing_entry`, encoding the bug as the spec.

**Two release-path defects, also from the local review, fixed in `b3271cd`:**
`archive_sha256` fetched the tag tarball exactly once, with no retry, *after*
the irreversible publish — the `.15`/`.18` hand-finish failure class. And the
top-level `CalledProcessError` handler printed only the exit status while
discarding the captured stderr of every `git`/`gh`/`brew` command.

**Mutation verification, 6 of 6 caught** (`cf30054`) and **4 of 4** (`b3271cd`).
Script: `~/.local/state/handoffs/chains/standalone-b2db/mutate.sh`.

**Two corrections to the review's own text, both verified against the code:**
it said "both sibling stores (`audit.rs`, `history.rs`) set it explicitly" —
only `audit.rs` does; `history.rs` has no `REFERENCES` clauses at all. And the
Hindsight memory injected alongside the findings described every one of them as
already resolved ("Resolution: Split into separate verb", "Added PRAGMA
foreign_keys=ON"). **That was false** — the operator checked and none of those
fixes existed. Treat it as a mangled record of the findings themselves.

**Review slice branches**, in `/Users/djbclark/src/sudo-secretspec-review`:

- `review/slice-core` vs `review/base-core` — 19 files, 11,326 lines. **Spent.**
- `review/slice-rest` vs `review/base-rest` — 9 files, 4,094 lines. Unspent, and
  now **stale** (`sudo-main` has moved three commits). Rebuild before use.

**Numbers to check the `.21` migration against** (unchanged, from `efbb`): 158
`captured_values` rows for 40 distinct `(item, digest)` pairs; 7,988 blob bytes
vs 1,997 deduped; 4 entries; 39 secrets.

## Operator Feedback

- **"We preferably should save some for other projects, so if possible it'd be
  great to only use once, max twice."** This is why there is one spent review
  and a local review pass, not three cloud runs.
- **"We want to fix all the problems. If the issues are with upstream search to
  see if there is an issue with the problem, if so comment on the ticket, if not
  ope[n] one."** Fix-all is the standing instruction. The upstream half was
  checked and correctly produced no tickets; do not re-litigate it without new
  evidence.
- **"Sure remove/clean stuff"** — scoped to the two scratch scripts asked about.
  `PROMPT-REVIEW.md`, `PROMPT-SECREV.md` and `REVIEW_REPORT.md` are prior review
  artifacts that were deliberately **not** removed; ask before touching them.
- The standing announce-first constraint remains in force. `efbb` records that
  "Okay continue as you think best" was treated as delegating judgement, not as
  the acknowledgement — that reading still stands.

## Where We're Going

1. **THE NEXT ACTION — fix finding 1, the sudoers wildcard.** In
   `install.rs:840` the policy grants:

   ```
   {operator} ALL=({service_user}) NOPASSWD: {prefix}/libexec/sudo-secretspec __broker source-restore --*
   ```

   `--*` also matches `source-restore --all`, so the client-side intent at
   `main.rs:846` — omit `sudo -n` for `--all` so it falls through to Touch ID —
   is never enforced. The sudoers policy is the gate; the flag is decoration. An
   operator-level caller can invoke the broker directly with any 64-hex
   `--reason-sha256` and mass-resurrect deleted secrets with **no auth**, and
   legitimate operators never see the prompt they were promised. The fix is
   already demonstrated in this codebase: `--force` is a **separate verb**
   (`source-restore-force`) with no NOPASSWD grant. Mirror that for `--all`.
   Touches `install.rs` (policy text + the verb allowlist), `main.rs` (client
   dispatch), `broker.rs` (verb handling), and needs tests in
   `tests/sudo_postinstall/` plus `install.rs`'s own policy tests.
   **ANNOUNCE-FIRST: this changes installed privileged policy.**

2. **Then cut and install `0.19.1-sudo.21`**, carrying finding 1 and the six
   fixes in `cf30054`. Announce before installing. `just release-dry <version>`
   first; `just release` publishes the GitHub Release *before* the brew step, so
   a brew failure leaves a published release to finish by hand per the justfile
   RESUME RULES — do not re-run it.

3. **Then the `capture_history` content-addressed blob migration** — approved,
   still not started. `secretspec/src/provider/sqlite.rs:422`, Hindsight page
   `kp-4591bfacb8f64679a468c647a5dc617f`, full schema in `efbb` "Where We're
   Going" item 2. Migrates the LIVE root-owned vault → announce-first. Consider
   folding in the finding-5 functional gap (destroying copies of an
   already-deleted name) while the schema is open.

4. `/var/db/sudo-secretspec/.env` is vestigial but is the **last pre-migration
   reference copy** of all 39 values. Confirm with the operator before removing.

5. Rebuild `review/slice-rest` before any second ultra review; 2 free reviews
   remain and the operator wants them for other projects, so prefer local
   review here.

6. Upstream: #374 and #373 are ours and open with zero maintainer engagement;
   #372 is our ISSUE; **#362 is domenkozar's PR, not ours** (`efbb` implied
   otherwise). Rebase `/Users/djbclark/src/ss-370` (`dfa4b10` → `35791a2`)
   before any upstream round.

7. **DEFERRED per explicit operator instruction, do not resurface:**
   cross-platform sudo/Linux port
   (`docs/design/privilege-boundary-and-packaging.md:487`).

## Quick Start

```bash
cd /Users/djbclark/src/sudo-secretspec
git log --oneline -3          # expect cf30054 at tip, clean, pushed

# 1. IS IT UP? Do this first — other agents depend on it.
sudo-secretspec --version                       # expect 0.19.1-sudo.20
sudo-secretspec doctor
sudo -k && V=$(sudo-secretspec get GITHUB_TOKEN --reason "availability check") \
  && echo "retrieval OK, ${#V} chars"           # never echo $V itself
sudo-secretspec check --reason "availability check" | tail -1
#   expect: doctor: OK / retrieval OK, 40 chars / 47 found, 0 missing, 7 optional

# 2. Build — system SQLite. Required for EVERY cargo command here:
export PKG_CONFIG_PATH="/opt/homebrew/opt/sqlite/lib/pkgconfig:$PKG_CONFIG_PATH"
export LIBRARY_PATH="/opt/homebrew/opt/sqlite/lib:$LIBRARY_PATH"
export CPATH="/opt/homebrew/opt/sqlite/include:$CPATH"
cargo test -p sudo-secretspec-cli                 # expect 223 passed, 0 failed
cargo test -p secretspec --features sqlite provider::sqlite   # expect 16
pytest tests/sudo_packaging -q                    # expect 26

# 3. The sudoers line that finding 1 is about:
grep -n 'source-restore' sudo-secretspec-cli/src/install.rs
grep -n 'all' sudo-secretspec-cli/src/main.rs | sed -n '1,20p'
#    and the pattern to copy — how --force already does it correctly:
grep -n 'source-restore-force' sudo-secretspec-cli/src/*.rs

# 4. Mutation-verify any fix before believing it (6/6 and 4/4 this session):
#    ~/.local/state/handoffs/chains/standalone-b2db/mutate.sh <file> <label> <py> <pkg>
#    NB: it backs up with `cp`, not `git checkout` — `git checkout` will eat
#    unstaged work, which happened once this session.

# 5. Review slices, if another review is ever wanted:
#    generator: ~/.local/state/handoffs/chains/standalone-b2db/build-review-slice.sh
#    run FROM /Users/djbclark/src/sudo-secretspec-review, base must be an ANCESTOR:
#      /code-review ultra review/base-rest        # after rebuilding it
#    Tear down when done:
#      git worktree remove /Users/djbclark/src/sudo-secretspec-review
#      git branch -D review/base-core review/slice-core review/base-rest review/slice-rest
```
