# Handoff — upstream realignment after the #334 consolidation

- **Chain:** standalone-b2db
- **Parent:** `HANDOFF_standalone-b2db_boundary-at-14-verified_2026-08-16_aa4e.md`
- **Written:** 2026-08-16
- **Repo/branch:** frdminc/sudo-secretspec, `sudo-main` @ `a457eba`
- **Upstream at time of writing:** `cachix/secretspec` main = `dfa4b10`

## Where we are

The parent handoff closed with "watch #334 and #356 for replies." Both replied,
and the answer reshaped the fork's upstream position entirely. This session was
research and decision-making only — **no merge, no code fix, no release yet.**
Everything decided is on disk and pushed; the execution is fully specified below.

### What happened upstream (verified, not inferred)

- **#334 MERGED** (`cdda3e7`). Our reported compile break was accepted — the
  maintainer replied "Rebased and fixed," and the merged tree carries our
  one-line fix (`build_ir_from_config` at `codegen.rs:573`).
- **#356 CLOSED**, **#357 CLOSED** — sixty seconds apart, identical sentence:
  "closing in favor of #334, please open an issue with your use case if that
  doesn't fit." On #357 the maintainer stated the reasoning outright:
  > "I mean that the public api will be `Spec` and `Spec::from(path)` instead
  > of the internal Config."
- We now have **zero open PRs**. Our only open thread is **issue #64**
  (out-of-tree providers via gRPC), which we didn't know we had — we closed our
  own #345 as a duplicate of it, so #64 now carries the fork's entire `exec://`
  provider-plugin interest.

This is a deliberate consolidation, not a rejection. **Operator directive: do
not push back on upstream's direction — reshape our code to match it.**

### Decisions made

1. **The `Spec::to_toml()` ask goes on `Spec`, not as a free-function module.**
   Two independent agents were run on this. They agreed on "land it on `Spec`"
   and split on how. Adjudicated in favor of the reparse design:
   `toml_edit` surgery on retained text, then **re-derive** `config`/`compiled`
   by reparsing — one synchronization point, not one per builder method.
   Rejected the alternative (fuse a `DocumentMut` into `SpecBuilder` and mirror
   every edit) for three reasons, one decisive: a single polymorphic `to_toml()`
   silently degrades from byte-exact to regenerated based on hidden state, and
   **our broker writes the manifest as root** — it would write a reformatted
   file believing it was exact. `preserved_text() -> Option<&str>` forces the
   question at the type level.
   Also: the `cargo add` prior art actually cuts *against* fusion — cargo-edit
   keeps `toml_edit` surgery separate from the serde `Manifest`.
2. **`source-schema` will go through `__private` as a deliberate stopgap**
   (operator-approved), because there is no supported alternative — see below.
   Recorded as "shape debt" in `sudo-secretspec/UPSTREAM-CONTACT.md` with a
   revisit trigger, to be migrated *even if nothing user-observable changes*.
3. **The `check` stdout/stderr bug is upstream's**, so it gets the full
   treatment: 2 reviews, then an upstream PR + bug report, as well as the
   downstream fix.

### Verified facts that constrain the work

- **`Spec` cannot introspect declarations.** Its only accessors are `project()`,
  `profiles()`, `secrets()` — and `secrets()` yields `Item = &str`, names only.
  No `Spec::secret(profile, name) -> Option<&Secret>` exists. Schema emission
  needs description/required/type, so `emit_schema` genuinely cannot migrate to
  the supported API. This is why `__private` is the only door today.
- **`build_ir` is only reachable via `#[doc(hidden)] pub mod __private`**, whose
  own doc comment says "not part of the supported Rust SDK."
- **`codegen::schema` is `pub(crate)` AND `#[cfg(any(feature = "cli", test))]`** —
  unreachable for a library consumer.
- Consequence: **the upstream merge WILL break `sudo-secretspec-cli/src/broker.rs`
  `emit_schema` (~line 891)**, which currently calls
  `secretspec::codegen::build_ir(config)` + `secretspec::codegen::schema::emit`.
  Expect this; it is not a new bug.
- **Upstream already has our function.** `cli/mod.rs:618` is
  `add_secret_to_manifest(source, profile, name, description)`, `toml_edit`-backed
  and private; `cli/mod.rs:479` is `generate_toml_with_comments(&Config)`.
  `toml_edit = "0.23"` is already a workspace dep, gated `cli = [...]`. The
  upstream ask is **promotion + a feature-gate change**, not new machinery.
- **The `check` bug's real severity:** `colored 3.1.1` gates colorization on
  `io::stdout().is_terminal()` (`control.rs:108`) while the report goes to
  stderr. So `check 2>log` with a TTY stdout writes raw ANSI into the log, and
  `check >file` with a TTY stderr strips colour from what the user is reading.
  Moving the report to stdout makes the existing TTY detection correct.

### Traps — do not repeat these

- **`cargo test --all` cannot run here.** `ext-php-rs` needs a PHP toolchain and
  `--exclude secretspec-php` does *not* prevent it being built. Use:
  `cargo test -p secretspec -p secretspec-derive -p sudo-secretspec-cli`.
- **Never pipe cargo through `tail`** — you get `tail`'s exit code, not cargo's.
  This produced a false "baseline passed, exit 0" earlier in this session when
  the build had actually failed.
- **Baseline to compare against: 1213 passed / 22 failed**, failures confined to
  `provider::sops::*` and `provider::vault_common::*` (sops CLI not installed).
- **Do not claim #334 "fully covers" #357** in anything posted upstream. It's
  false twice over (see verified facts). The honest concession is the
  architectural point only.
- **Do not lead the upstream issue with `template-check`** —
  `docs/design/template-check-resync.md` records that workflow as having **zero
  users**. The live property is `add` → `undeclare`.
- **Do not use the dependency-weight argument** — `clap`, `inquire`, `miette`,
  `tempfile` are all non-optional upstream. Our own `manifest_edit.rs:4-6`
  comment overstates this. Visibility + the `toml_edit` feature gate are the
  real, airtight blockers.
- **Do not delete branch `explore/pr-334-rust-first-spec`** — `bf0b25c` is linked
  by SHA from a public comment on #357.

## What's left, in order

1. **Merge `upstream/main` (`dfa4b10`) into `sudo-main`.** 87 commits ahead,
   129 files, +14,550/−2,981. Carries #334 (`spec.rs`) and #313 (Azure App
   Configuration provider). Expect `emit_schema` to break; fix via `__private`
   per decision 2. Re-fetch first — upstream moved twice during one session.
2. **Fix the `check` stdout/stderr bug** in `secretspec/src/secrets.rs`:
   `check()` header, `display_validation_success`, `display_validation_errors`
   — `eprintln!` → `println!`. Keep the constraint-violation lines with the
   report on stdout (splitting one report across two streams is the same bug in
   miniature); flag the choice in the PR. Preserve exit codes; don't touch
   `ensure_secrets`. CHANGELOG: one user-facing entry under Unreleased.
3. **Two independent reviews of that fix**, then file the upstream bug report +
   PR. Draft is at `sudo-secretspec/drafts/upstream-check-stdout-issue.md` —
   re-verify its line numbers against the merged tree and replace the
   illustrative console block with a real captured transcript before posting.
4. **Reimplement the manifest-edit behavior on the `Spec` shape** so the fork is
   a working reference implementation for the issue.
5. **Post the `Spec::to_toml()` issue.** Draft at
   `sudo-secretspec/drafts/upstream-spec-to-toml-issue.md`, operator has seen the
   analysis but **not yet approved posting**.
6. **Cut `0.19.1-sudo.15`.** Update the skill and `AI-GUIDANCE.md` *before*
   tagging — they ship inside the release, so a post-tag doc fix never reaches
   the installed copy (the trap the .12 release hit).

## Open questions for the operator

- Approve posting the `Spec::to_toml()` issue as drafted?
- Should the `Spec::schema_json(profile)` ask be folded into that issue or filed
  separately? Current draft keeps it separate to avoid diluting a focused ask.
