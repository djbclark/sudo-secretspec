# Upstream contact ledger — `cachix/secretspec`

Every thread this fork has opened or spoken in upstream, and what state it was
in when last checked. **Re-check this whole file at the start of every session**
(`/baton` / `/resume`); upstream moves fast and closes stale work.

Last verified: **2026-08-16** (upstream `main` at `dfa4b10`).

## How to re-check

```bash
git -C ~/src/sudo-secretspec fetch upstream main
git -C ~/src/sudo-secretspec log --oneline HEAD..upstream/main | wc -l   # commits we're behind

# Union of everything we authored or commented on:
gh search issues --repo cachix/secretspec --author    djbclark --include-prs --limit 40 \
  --json number,title,state,isPullRequest \
  --jq '.[] | "\(.number) [\(if .isPullRequest then "PR" else "issue" end)/\(.state)] \(.title)"'
gh search issues --repo cachix/secretspec --commenter djbclark --include-prs --limit 40 \
  --json number,title,state,isPullRequest \
  --jq '.[] | "\(.number) [\(if .isPullRequest then "PR" else "issue" end)/\(.state)] \(.title)"'
```

`--commenter` catches threads we don't own (e.g. #334, #64); `--author` catches
ours. Neither is a superset of the other — run both.

Then diff the result against the table below and update this file. A thread that
changed state since "last verified" is the session's first order of business.

## Open — needs attention

| # | Kind | Title | Why it's live |
|---|------|-------|----------------|
| [64](https://github.com/cachix/secretspec/issues/64) | issue | Support out-of-tree providers via gRPC interface | **Our only open upstream thread.** We closed our own #345 as a duplicate of this one, so it now carries the fork's entire `exec://` / provider-plugin interest. Not ours; we're a commenter. Watch for a maintainer decision on plugin architecture — it determines whether the broker can ever ship a provider without patching upstream. |

## Closed / merged — history

| # | Kind | State | Title | Outcome |
|---|------|-------|-------|---------|
| [357](https://github.com/cachix/secretspec/pull/357) | PR | closed | feat(sdk): expose `Secrets::config()` publicly | Closed 2026-08-16T19:35:50Z "in favor of #334". Maintainer: *"the public api will be `Spec` and `Spec::from(path)` instead of the internal Config."* **Do not delete branch `explore/pr-334-rust-first-spec`** — commit `bf0b25c` is linked by SHA from our public comment here. |
| [356](https://github.com/cachix/secretspec/pull/356) | PR | closed | feat(cli): declare secret requiredness at add time, extracting manifest_edit | Closed 2026-08-16T19:36:30Z, 60s after #357, same sentence: "in favor of #334, please open an issue with your use case if that doesn't fit". The tri-state requiredness half **did** land in #334 (`Secret::optional`/`required`, `required_setting() -> Option<bool>`). The manifest-editing half did not. Invitation to open an issue is **outstanding and unactioned**. |
| [355](https://github.com/cachix/secretspec/pull/355) | PR | merged | feat(codegen): emit description in JSON Schema properties | Merged 2026-08-16. Its merge semantically broke #334 (textually clean, compile failure); we reported it and supplied the one-line fix. |
| [354](https://github.com/cachix/secretspec/pull/354) | PR | merged | fix(provider): add `Provider::supports_delete` capability | Merged 2026-08-16. |
| [345](https://github.com/cachix/secretspec/issues/345) | issue | closed | Proposal: Generic `exec://` Provider Plugin Protocol (and an AI privilege boundary showcase) | Closed **by us** 2026-08-14 as a duplicate of #64. Maintainer: *"something we still need to explore as there are many ways."* |
| [334](https://github.com/cachix/secretspec/pull/334) | PR | merged | Add Rust-first Spec API | **Not ours** — we commented. Merged `cdda3e7` 2026-08-16T22:06Z. We reported that #355 broke its compile and supplied the fix (`build_ir` → `build_ir_from_config` at `codegen.rs:573`); maintainer replied "Rebased and fixed" and the merged tree carries our fix. This PR is now the centre of gravity for the public API. |

## Owed / planned

Not yet filed. Track here so they don't get lost.

- **Issue: `Spec` has no path back to TOML.** Explicitly invited by the
  maintainer on both #356 and #357. Must land the capability **on `Spec`**
  (`to_toml()` + builder carrying the source document), not as a parallel
  free-function API — he declined that shape twice in one minute. Draft and
  design rationale: see session notes / `docs/design/`.
- **Bug + PR: `check` writes its entire report to stderr.** Reproduces
  identically on upstream `main` (`secretspec/src/secrets.rs`, `check()` and
  both `display_validation_*` helpers, all `eprintln!`). `secrets.rs` is
  upstream-owned — the fork has touched it once since the merge base, upstream
  7 times — so a fork-local-only fix is a permanent conflict site.

## Deliberate stopgaps to revisit — not functionality debt, *shape* debt

These work. They are recorded because they are uglier than they should be, and
the right shape depends on an upstream answer we have not received yet. Revisit
each one when the corresponding thread moves, **even if there is no functional
gain** — the point is to stop depending on surfaces upstream has disclaimed.

### `source-schema` goes through `__private` (added 0.19.1-sudo.15)

`sudo-secretspec-cli/src/broker.rs` `emit_schema` reaches codegen through
`secretspec::__private::codegen::build_ir`. Upstream marks that module
`#[doc(hidden)]` and says in its own doc comment:

> These document types are not part of the supported Rust SDK. Use `Spec` and
> its builder API instead.

So we are knowingly building on a surface upstream disclaims, and it can change
without notice in any release.

- **Why we did it:** the upstream merge moved `build_ir` to take `&Spec` and
  left `codegen::schema` as `pub(crate)` + `#[cfg(feature = "cli")]`, so there
  is no supported path to JSON Schema emission for a library consumer. The
  alternative was blocking a release that also carries the `check` stdout fix.
- **The clean shape:** a `Spec`-shaped method upstream, e.g.
  `Spec::schema_json(profile) -> Result<String>`. That fits the maintainer's
  stated consolidation ("the public api will be `Spec`") rather than fighting
  it, and it is the second ask to raise alongside the `to_toml()` issue.
- **Revisit when:** the `Spec::to_toml()` issue gets a maintainer response, or
  any upstream release changes `__private`. Check whether `schema::emit` became
  reachable; if it did, migrate off `__private` **even though nothing the user
  can observe changes**.
- **Canary:** if a future upstream merge breaks `emit_schema` compilation, that
  is this debt coming due, not a new bug. Fix it by asking for the `Spec` method,
  not by reaching deeper into internals.

## Standing rules

- Upstream `main` moved twice during one session (`cdda3e7` → `dfa4b10`).
  Never trust a previous session's upstream analysis without re-fetching.
- Where upstream has stated a direction, **follow it rather than arguing** —
  reshape our code to match and offer it as a reference implementation.
- GitHub redirects `djbclark/*` → `frdminc/*`, so stale references keep working
  silently. Prefer `frdminc/...` in anything we post upstream.
