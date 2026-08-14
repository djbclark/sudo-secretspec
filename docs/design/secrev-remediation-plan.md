# PLAN-SECREV-FIXES.md — remediation plan for the privilege-boundary review

Companion to `PROMPT-SECREV.md` and the review posted at
<https://github.com/djbclark/sudo-secretspec/pull/1#issuecomment-5289608095>.

Ordered in the sequence I would actually do the work. Each item states the fix,
the test that proves it, and — where there is one — the decision or trade-off
that is yours rather than mine.

Nothing here is impossible. Three things are *inherently* only mitigable, not
solvable, and they are marked **[bounded]** where they appear.

---

## 0. State of the tree vs. what was reviewed

Reviewed at `ecd03c2`. Current `HEAD` is `0de3b49`, two commits ahead, plus
uncommitted work:

| Change | Effect on the review |
|---|---|
| `df3be2f` — `release.py` parameterized, workspace stamped `0.19.1-djbclark.2` | I read the **new** `release.py`; ADVISORY-8 still applies verbatim to it |
| `0de3b49` — formula restamped to `.2` | No effect; formula findings unchanged (still 🟢) |
| Uncommitted: `src/drift.rs` + `tests/drift.rs` — `visudo -c` switched from `.status()` to `.output()` so its "parsed OK" line stops corrupting `doctor --json` | Unrelated to any finding, does not conflict with anything below. Line numbers in `drift.rs` after ~526 shift by +5 |

**No reviewed security logic changed under me.** `broker.rs`, `audit.rs`,
`install.rs`, `rollback.rs`, `uninstall.rs`, `config.rs`, `main.rs` are
byte-identical to what the review was written against. The one substantive
delta is that `release.py` is now argument-driven, which makes ADVISORY-8 a
smaller edit than it would have been.

Everything below assumes the usual fork rules: work directly on `sudo-main`, no
feature branches, and every Rust change gets one user-facing `CHANGELOG.md`
entry under `Unreleased`.

---

## Phase 0 — three facts I could not establish, needed before Phase 2 and 8

These need your Touch ID; I could not run them.

```bash
# (a) Decides whether SECURITY-1 is live today or only latent.
sudo /usr/bin/printenv HOME XDG_CONFIG_HOME XDG_STATE_HOME

# (b) Decides whether ADVISORY-4's tightening is safe on the deployed vault.
sudo stat -f '%Sp %Su %Sg %N' \
  /var/db/stayturgid-secrets/secretspec.toml \
  /var/db/stayturgid-secrets/.env

# (c) Confirms ADVISORY-7 is currently latent on this host (I saw root:wheel 755
#     on bin/etc/libexec unprivileged; this also covers share/ and any ACLs).
ls -lde /usr/local /usr/local/bin /usr/local/libexec /usr/local/etc /usr/local/share
```

Interpretation:

- **(a)** prints your home → SECURITY-1 is **exploitable now**, treat Phase 2 as
  urgent. Prints `/var/root` → still fix it, but it is defence-in-depth and can
  ride the same release.
- **(b)** anything other than `-rw------- _secretspec` on both files means the
  tightening in Phase 8 would break the live install; adjust that step instead
  of shipping it blind.
- **(c)** anything not `root wheel drwxr-xr-x` with no `+` ACL marker means
  Phase 7 is urgent rather than routine.

---

## Phase 1 — the two zero-decision changes (do first, they cost nothing)

### 1.1 ADVISORY-8 — make the release gate run the companion's tests

`packaging/release.py::run_tests` builds the engine and runs the packaging
pytest; it never runs the 101 tests that cover the boundary. Do this *first* so
every later phase is actually gated.

```python
def run_tests(*, dry_run: bool) -> None:
    run(["pytest", "tests/sudo_packaging", "-q"], dry_run=dry_run, capture=False)
    # The companion is what carries privilege; its suite gates the release.
    run(["cargo", "test", "-p", "sudo-secretspec-cli", "--locked"],
        dry_run=dry_run, capture=False)
    run(["cargo", "build", "-p", "secretspec", "--locked"],
        dry_run=dry_run, capture=False)
```

Cost: ~30 s of release time. No trade-off. Also update the "58 tests" line in
`PROMPT-SECREV.md` — the suite is at 101.

### 1.2 SECURITY-2 — purge every `SECRETSPEC_*`, not four of them

`broker.rs::execute` names four variables. The library reads ~20, four of which
(`SECRETSPEC_OPCLI_PATH`, `SECRETSPEC_BWS_CLI_PATH`,
`SECRETSPEC_PASSBOLT_CLI_PATH`, `SECRETSPEC_PROTONPASS_CLI_PATH`) name an
executable it will spawn — as root. The client already does this correctly at
`main.rs:640`; mirror it, and hoist it to the top of `run()` so nothing between
dispatch and execution can consult ambient state.

```rust
/// True for any variable that could steer the engine's control plane.
/// A predicate rather than a list: a new `SECRETSPEC_*` knob upstream is
/// covered the day it lands, which a fixed list is not.
pub(crate) fn is_ambient_control_var(key: &str) -> bool {
    key.starts_with("SECRETSPEC_")
}

fn purge_ambient_env() {
    for (key, _) in std::env::vars_os() {
        if is_ambient_control_var(&key.to_string_lossy()) {
            // SAFETY: single-threaded broker process; no concurrent env readers.
            unsafe { std::env::remove_var(&key) }
        }
    }
}
```

**Test it as a predicate, not by mutating the environment.** `std::env::set_var`
is process-global and racy under the test harness's threads; assert
`is_ambient_control_var` covers each of the ~20 known names plus the four
`*_CLI_PATH` ones, and leave the loop untested.

Not exploitable today — `set_provider` collapses every per-secret route to the
pinned `dotenv://` store — but it is one routing change away from root code
execution, and the threat model already claims this purge is complete.

---

## Phase 2 — SECURITY-1: stop the root broker reading the caller's config

The broker calls `Secrets::load_from`, which calls `GlobalConfig::load()`, which
resolves through `etcetera`'s XDG strategy: `$XDG_CONFIG_HOME` else
`$HOME/.config`, at `secretspec/config.toml`. Inside a root process that file
controls (i) `[audit] path` — an arbitrary absolute path root will `mkdir -p
0700`, open `0600`, append the **plaintext reason** to, and `set_len(0)` when
`max_size_bytes` is exceeded (set it to `1` and every write truncates first);
and (ii) `[defaults] profile`, which picks which profile of the protected
manifest resolves.

Three layers. Do 2.1 and 2.2 unconditionally; 2.3 is a decision.

### 2.1 Pin the environment the engine resolves from

In the same `purge_ambient_env()` from 1.2:

```rust
// The engine resolves its user-global config through XDG, i.e. from HOME.
// Root's own home is the only one inside the boundary.
unsafe {
    std::env::remove_var("XDG_CONFIG_HOME");
    std::env::remove_var("XDG_STATE_HOME");
    std::env::remove_var("XDG_DATA_HOME");
    std::env::set_var("HOME", "/var/root");
}
```

`set` rather than `remove` for `HOME` deliberately: with `HOME` unset,
`etcetera` falls back to `getpwuid(0)`, which is `/var/root` on macOS anyway —
but if that lookup ever fails, `GlobalConfig::load()` errors and the broker
fails closed on a *healthy* system. Setting it explicitly makes the resolved
path a constant instead of a directory-service round-trip.

### 2.2 Pin `HOME` in the policy too, so it holds before the process starts

`install::sudoers_text`, on the libexec line that already sets
`env_reset,secure_path=…,umask=0077`:

```
Defaults!{prefix}/libexec/sudo-secretspec env_reset,secure_path=…,umask=0077,always_set_home
```

`always_set_home` makes sudo set `HOME` to the target user's home regardless of
what the stock macOS `env_keep` does. Belt and braces with 2.1, and unlike 2.1
it is visible to anyone auditing the policy. The existing
`the_shipped_policy_parses_and_gates_boundary_lifecycle` test should grow an
assertion for the flag.

> Requires a re-`install` to take effect, and changes the policy bytes → the
> installed `MANIFEST.sha256` changes → `doctor` reports `INSTALLED_HASH_MISMATCH`
> until the operator reinstalls. See Phase 10 on sequencing.

### 2.3 Pin the profile — **decision needed**

`resolve_profile_name` falls through to the user-global default. Pick one:

| Option | Cost | Recommendation |
|---|---|---|
| **A.** Hardcode `s.set_profile("default")` in `execute()` | Zero, but silently wrong if the deployed manifest ever uses a non-default profile | Only if you are certain the vault manifest is single-profile |
| **B.** Add `profile` to `/usr/local/etc/sudo-secretspec.toml` (root-owned, `0444`, already boundary-validated), `#[serde(default = "default_profile")]` so existing configs keep parsing, written by `install`, passed via `set_profile` | ~20 lines across `config.rs` + `install.rs` + `broker.rs` | **Recommended.** The profile becomes part of the protected control plane instead of an ambient fallback |

`deny_unknown_fields` means a *new* field is fine but a *missing* one is not —
hence `#[serde(default)]`. Note the inverse: a config written by the new
installer will be rejected by an older broker binary. They are installed as a
pair, so this only matters if someone rolls back the binary without the config;
`rollback.rs` restores both together, so that is covered.

### 2.4 The one alternative I am not recommending

The robust fix is an engine-side `Secrets::set_ignore_global_config(true)`,
mirroring the existing `set_ignore_ambient_scope`. It removes the whole class
rather than the paths I can enumerate. I am not putting it first because it
means a permanent local delta in `secretspec/src/secrets.rs` that has to be
rebased onto every upstream bump, and the fork's whole shape is "companion crate
beside an untouched engine". **Worth revisiting if you ever intend to upstream
it** — it is a defensible upstream feature ("a privileged embedder must be able
to refuse user-global config"), and upstreaming converts the maintenance cost to
zero. Your call; the env pin in 2.1 is sufficient in the meantime.

**[bounded]** Even with all of the above, "the engine never reads anything
outside the boundary" is enforced by enumeration, not by construction. Only 2.4
makes it structural.

---

## Phase 3 — broker mutation and audit correctness (one refactor, two findings)

Both live in `broker.rs::run`; do them in a single pass.

### 3.1 ADVISORY-1 — `chown` the rollback backups

Verified empirically: `std::fs::copy` on macOS preserves mode but **not**
ownership (source `0600 user:everyone` → destination `0600 user:wheel`). So
root-created `.rollback.<uuid>` backups land root-owned in a service-user vault,
and `drift`'s per-entry `check_protected_file` turns the deliberately-advisory
`PENDING_ROLLBACK` into a non-advisory `METADATA_MISMATCH` → `doctor` exits 1 →
every agent told to treat drift failure as a hard stop is wedged after any
crashed mutation.

```rust
fn begin(cfg: &Config, transaction: uuid::Uuid, uid: u32, gid: u32) -> Result<Self, i32> {
    …
    std::fs::copy(src, &dst)?;
    // `fs::copy` carries the mode across but not the owner, and these sit in a
    // vault `drift` checks entry by entry. Root-owned backups would turn the
    // advisory `PENDING_ROLLBACK` into a hard doctor failure.
    let c = std::ffi::CString::new(dst.as_os_str().as_bytes()).map_err(|_| 2)?;
    if unsafe { libc::chown(c.as_ptr(), uid, gid) } != 0 { … return Err(2) }
}
```

`uid` is what `require_boundary` already returns; take `gid` from
`fs::metadata(&cfg.vault)?.gid()`, exactly as `audit::open_connection` step 5
does for the ledger.

**Also loosen `drift` for these entries** — a backup left by an *older* broker is
already root-owned, so the chown alone does not un-wedge an existing host. In
the vault scan, `continue` after pushing `PENDING_ROLLBACK` instead of falling
through to `check_protected_file`. Small design call: I think transient
mutation artifacts should be reported by their own code and not double-judged
by the steady-state metadata rule. If you disagree and want them strictly
checked, then the chown must be paired with a release note telling operators to
delete pre-existing `.rollback.*` files by hand.

Test: create a `0600` root-owned `.rollback.` entry in a temp vault and assert
`report.ok` is true with exactly one `PENDING_ROLLBACK` finding.

### 3.2 ADVISORY-2 — make the terminal event structural

`Mutation::begin` is the only early return that fires *after* the attempt event,
leaving an attempt with no terminal. Rather than adding one more `append_event`
call at that site, funnel it:

```rust
let outcome = (|| -> Result<(u8, Vec<String>), i32> {
    let mutation = …;            // begin() failure now returns through here
    let (rc, names) = execute(broker, &cfg);
    …
    Ok((rc, names))
})();
// Exactly one terminal event, on every path that appended an attempt.
let (terminal_rc, names, phase) = match outcome { … };
audit::append_event(…)?;
```

The property worth testing is the funnel, not the I/O failure: the rollback-path
collision is essentially unreachable in a test (the suffix is a fresh UUID, and
root ignores mode bits so you cannot make the copy fail by permissions). Test
that every `Err` arm of the inner closure maps to a terminal phase; leave the
collision itself untested and say so in the comment.

---

## Phase 4 — `audit.rs`: assert ownership, and stop mutating on the read path

### 4.1 ADVISORY-3 — pass the expected uid on both verify paths

`run_audit_verify` calls `audit::verify(&cfg.vault, None)`; `drift::inspect`
does the same. With `None`, both `check_protected_dir` and
`check_ledger_metadata` skip their owner comparison — on the one command whose
job is to prove the ledger is intact.

**Decision — how strict should `audit-verify` be?** Running the full
`require_boundary()` first would be consistent with the other operations, but it
means a drifted install (wrong binary mode, missing manifest) can no longer
verify its own ledger, which is precisely when you want to. I recommend the
middle: resolve `uid_for_user(&cfg.service_user)` and pass `Some(uid)` **without**
the full boundary check, so ledger ownership is asserted while forensics stay
available. `drift` passes the uid it already has in `layout.service_user`.

### 4.2 ADVISORY-11 — `drift` says "never mutates state" and then mutates

*Not in the posted review — noticed while planning this.* `drift::inspect`'s
module doc is "Never reads secret values. Never mutates state." It then calls
`audit::verify` → `open_connection`, which as root does
`set_permissions(0600)`, `libc::chown(…)`, and `ensure_schema` (`CREATE TABLE
IF NOT EXISTS`). So `doctor` run as root can rewrite the ledger's metadata and
create its schema. Nothing dangerous happens today — the chown targets the vault
owner, which is where the ledger should be anyway — but a non-repairing checker
that silently repairs is exactly the kind of surprise this codebase otherwise
refuses.

```rust
pub enum VerifyMode { ReadWrite, ReadOnly }
// ReadOnly: Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY),
// skip ensure_schema and the chmod/chown block, and use a plain `BEGIN`
// (deferred) — `BEGIN IMMEDIATE` needs a write lock.
```

`drift` uses `ReadOnly`; the broker keeps `ReadWrite`. Watch for
`SQLITE_READONLY_RECOVERY`: a hot journal cannot be replayed read-only, so that
error must surface as an `AUDIT_VERIFY_FAILED` finding rather than a panic.
`PRAGMA integrity_check` is fine read-only.

---

## Phase 5 — ADVISORY-10: fail closed on the in-binary lifecycle allowlist

`invoked_as_privileged_broker()` returns `false` whenever either
`current_exe()` or `canonicalize(BROKER_PATH)` fails, which lets
`install`/`rollback`/`uninstall` past the allowlist that `main.rs:157-163`
presents as the layer catching a sudoers mistake.

The two failures are not the same and must not be collapsed:

- `canonicalize(BROKER_PATH)` fails → the broker is **not installed** → this
  process cannot be it → `false` is correct, and is what makes the Homebrew
  libexec bootstrap able to run `install` at all.
- `current_exe()` fails → we cannot tell → **`true`** (refuse lifecycle).

```rust
match (std::env::current_exe(), std::fs::canonicalize(BROKER_PATH)) {
    (Ok(me), Ok(broker)) => std::fs::canonicalize(me).map(|me| me == broker).unwrap_or(true),
    (Err(_), _) => true,   // cannot identify ourselves: assume the privileged path
    (_, Err(_)) => false,  // no installed broker: this cannot be it
}
```

Ten-line change, no trade-off, but note the asymmetry in the comment or someone
will "simplify" it back.

---

## Phase 6 — ADVISORY-7: validate the directories the installer writes into

`is_protected_dir`'s doc claims "The installer requires it of every directory it
writes into". `validate_protected_ancestors()` actually lists only
`/usr`, `/usr/local`, `/private`, `/private/var`, `/private/var/db`,
`/private/etc`, `/private/etc/sudoers.d` — not `/usr/local/{bin,libexec,etc,share}`,
which is where the NOPASSWD broker binary itself lands.

Ordering matters: they may not exist on a first install. So create, then
validate, and set the mode explicitly rather than inheriting a umask:

```rust
for dir in ["bin", "libexec", "etc", "share"] {
    let path = PathBuf::from(PREFIX).join(dir);
    fs::create_dir_all(&path)?;
    // `create_dir_all` takes the ambient umask, and the policy sets umask=0077
    // for the libexec path — a 0700 /usr/local/bin would be unreadable to the
    // operator it exists to serve.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    if !is_protected_dir(&path) {
        return Err(InstallError::Denied(format!("unsafe install directory {}", path.display())));
    }
}
```

`drift` already covers these via `check_ancestor_chain`, so this is install-time
only, and on this host they are already `root:wheel 0755` (confirm with Phase 0
(c)). The scenario it closes is a machine carrying a legacy Intel-Homebrew
`chown -R` of the prefix.

---

## Phase 7 — ADVISORY-4: tighten the broker's file-mode check (**gated on Phase 0 (b)**)

`require_boundary` accepts any mode with `mode & 0o077 == 0`, so `0700` and
`0400` pass; `drift` requires exactly `0600`. The enforcing side should not be
the permissive one:

```rust
if meta.permissions().mode() & 0o777 != 0o600 { … }
```

**Do not ship this without (b).** If the adopted vault's files are `0400` or
`0700` today, this turns every operation into an immediate fail-closed refusal.
If (b) shows anything other than `0600`, the right move is either to normalize
the live files first (`sudo chmod 600 …`, a one-time operator action, *not*
something the broker should do — it must not repair) or to keep the loose check
and instead tighten `drift` to match it. I would normalize and tighten.

---

## Phase 8 — ADVISORY-6: `--reason` in argv (**decision**)

The checklist asserts the reason is SHA-256 hashed before crossing the boundary.
It is not: `main.rs:570` passes plaintext to `sudo -n … --reason <text>`, and the
broker hashes it. On macOS `KERN_PROCARGS2` is same-uid-or-root, so this is not
cross-user exposure — but every other agent running as you can read it, and it
lands in shell history.

| Option | Consequence |
|---|---|
| **A.** Client sends `--reason-sha256 <hex>`; broker validates 64-hex, rejects the digest of the empty string, and passes the digest to `with_reason` | Matches the stated invariant. The engine's own JSONL audit then records a digest instead of prose — arguably correct for a value-free design, but you lose human-readable reasons in that log |
| **B.** Keep plaintext, delete the claim from `PROMPT-SECREV.md` | Zero code. Honest. Reasons stay readable |
| **C.** Keep plaintext but move it off argv (pipe it to the broker's stdin) | Removes argv exposure *and* keeps prose. Costs a protocol change and complicates `set`, which already reads the secret value from stdin — the two would have to be framed |

I lean **A** for a tool whose entire thesis is "the ledger stores only
`SHA-256(reason)`" — but it is your log to read, so if you actually grep those
reasons, **B** is the honest choice and costs nothing. **C** is the most work
for the least gain and I would not do it.

Whichever you pick, keep the client-side non-empty check: it must not be
possible to send `SHA-256("")` and satisfy the gate.

---

## Phase 9 — the two documentation-level corrections

### 9.1 ADVISORY-5 — the ledger's residual-risk note overstates truncation detection

`audit.rs:26-30` says the chain detects tail truncation "because every event
commits with the singleton `head` row", and names only whole-ledger deletion as
undetectable. But `head` lives in the same database: delete the last N events,
rewrite the singleton head to the new tip, and `verify_rows` re-verifies cleanly.
Truncation and deletion have the *same* mitigation — an externally pinned tip.

Correct the paragraph. **[bounded]** — tamper-evidence against a principal who
can write the ledger cannot be achieved inside the ledger; that is arithmetic,
not a code defect. The optional feature that makes the existing advice usable:

```
sudo-secretspec audit-verify --expect-tip <sha256>   # non-zero exit on mismatch
```

so a watcher (launchd job, ops cron) can pin the tip outside the vault. Worth
doing only if you actually intend to run such a watcher; otherwise the doc fix
alone is the right scope.

### 9.2 `canonical_event_json`'s comment is wrong in the safe direction

It says "without sequence, timestamp_ns, event_hash"; the code includes
`sequence` and `timestamp_ns` in the hashed payload. The code is *stronger* than
the comment. Fix the comment, not the code — changing the code would break every
existing chain.

---

## Phase 10 — ADVISORY-9 and shipping

### 10.1 ADVISORY-9 — unzeroized secrets in the client (**decision, recommend accept**)

`run_target` holds the export as a `Vec<u8>`, then a `serde_json::Value`, then
copies each value into the child's environment: three plaintext heap copies, none
zeroized. **[bounded]** — Rust's `Vec`/`String` reallocation means zeroization is
best-effort even with the `zeroize` crate, and the values are headed into the
child's environment regardless, which is readable for the process's whole
lifetime. My recommendation is to **document it as accepted residual risk** in
`AI-GUIDANCE.md` rather than add a dependency that buys a fraction of a
mitigation. Take the code path only if you have a specific threat (core dumps,
memory-scraping) in mind.

### 10.2 Ship it as one release

Phases 2.2 (sudoers `always_set_home`), 2.3B (new config field), and every
change to the broker binary all require the operator to re-run
`sudo-secretspec install`, and each changes `MANIFEST.sha256` so `doctor` reports
`INSTALLED_HASH_MISMATCH` until they do. **Batch all of it into a single
`0.19.1-djbclark.3`** so that is one reinstall, not six. Sequence:

1. Phases 1–9 committed to `sudo-main`, each with its `CHANGELOG.md` entry.
2. `cargo test -p sudo-secretspec-cli` green (now also enforced by 1.1).
3. `packaging/release.py --version 0.19.1-djbclark.3 --dry-run`, then live.
4. `sudo-secretspec install --adopt-existing` on the host, then
   `sudo-secretspec doctor --json` to confirm a clean report — that is also the
   end-to-end proof that 2.2, 2.3 and 6 did not break the live boundary.
5. Re-run Phase 0 (a): with `always_set_home` in the policy, `sudo
   /usr/bin/printenv HOME` through the libexec path must now report `/var/root`.

---

## Summary table

| # | Finding | Effort | Needs a decision |
|---|---|---|---|
| 1.1 | ADV-8 release gate | 5 min | — |
| 1.2 | SEC-2 full env purge | 30 min | — |
| 2 | SEC-1 HOME/XDG pin, policy, profile | half a day | **Yes** — 2.3 A/B, and 2.4 if you want it structural |
| 3.1 | ADV-1 chown backups | 1 h | Minor — whether `drift` skips `.rollback.` entries |
| 3.2 | ADV-2 terminal always emitted | 1–2 h | — |
| 4.1 | ADV-3 expected uid on verify | 30 min | **Yes** — how strict `audit-verify` should be |
| 4.2 | ADV-11 read-only verify for drift | 2 h | — |
| 5 | ADV-10 fail closed | 15 min | — |
| 6 | ADV-7 installer dir validation | 1 h | — |
| 7 | ADV-4 mode tightening | 15 min | **Gated** on Phase 0 (b) |
| 8 | ADV-6 reason hashing | 1 h (A) / 0 (B) | **Yes** — A, B or C |
| 9 | ADV-5 + comment fixes | 30 min | Optional `--expect-tip` feature |
| 10.1 | ADV-9 zeroization | — | **Yes** — recommend accept + document |
