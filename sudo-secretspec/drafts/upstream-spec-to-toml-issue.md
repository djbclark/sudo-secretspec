# DRAFT — upstream issue for cachix/secretspec
# Status: NOT POSTED. Awaiting operator approval.
# Synthesis of two independent agent designs + verification against dfa4b10.

**Title:** Format-preserving single-declaration edits on `Spec` — following up on your note in #356/#357

---

Opening this at your invitation on #356 and #357, and deliberately shaped around
your reasoning on #357 rather than re-proposing the same thing a third time:

> I mean that the public api will be `Spec` and `Spec::from(path)` instead of
> the internal Config.

That's the right call and I'm not asking you to reverse any of it. `Config`
should stay internal. What follows lands entirely on `Spec`.

Two things from #356 I'm explicitly **not** re-raising, because #334 settled
them: tri-state requiredness (`Secret::optional`/`required`, and
`required_setting() -> Option<bool>`) is exactly the semantics I was reaching
for, and it landed better in #334 than in my PR.

### ELI5

`Spec` can **read** a `secretspec.toml`, and can **build** a new one from
scratch. It cannot **write an existing one back out**.

So if you already have a real file on disk — with a team's comments, their key
order, their quoting — and you want code to add or remove one declaration in it,
there's no supported way to do that. You either regenerate the whole file from
the parsed model (losing every comment and reordering every profile) or you
hand-roll `toml_edit` yourself outside the library.

**The ask:** two narrow methods on `Spec` that perform the one-line edit and
leave every other byte of the file exactly as it was.

### Technically

`Spec { config, compiled, base_dir }` has no `to_toml`, no `Display`, and no
`toml_edit` anywhere in `spec.rs`. `SpecBuilder::build()` ends at
`Spec::from_config_document(self.config)` — an in-memory `Config` that is never
written anywhere. `Config.profiles` is a `HashMap`, so key order isn't preserved
even in principle. There is currently no serialization path from `Spec` back to
a file at all, exact or otherwise.

Proposed surface — no new public type, no `Config` and no `toml_edit` in any
signature:

```rust
impl Spec {
    /// Add one declaration, returning a new fully-revalidated `Spec` whose
    /// preserved text differs from the original only by that declaration.
    pub fn add_secret_to_text(&self, profile: &str, name: &str, secret: Secret) -> Result<Spec>;

    /// The inverse, same guarantee.
    pub fn remove_secret_from_text(&self, profile: &str, name: &str) -> Result<Spec>;

    /// Whether `profile` declares `name` in this spec's own source text
    /// (not only via inheritance).
    pub fn declares_secret_in_text(&self, profile: &str, name: &str) -> bool;

    /// Render as TOML, freshly formatted. Always available.
    pub fn to_toml(&self) -> String;

    /// This spec's exact backing text, when the byte-exactness guarantee still
    /// holds. `None` for a `Spec::builder()`-constructed spec.
    pub fn preserved_text(&self) -> Option<&str>;
}
```

Guarantee: `Spec::from_toml(s)?.preserved_text() == Some(s)`, and add-then-remove
of the same key returns the original bytes.

**Implementation shape that makes this safe.** These methods edit the retained
source text with `toml_edit`, then re-derive `config`/`compiled` by reparsing the
result through the same validated path every other `Spec` already goes through.
The semantic view is always *derived* from the text, never hand-mutated in
parallel with it — one synchronization point, not one per method, and no
possibility of the two disagreeing.

**Deliberately not proposed:** making `SpecBuilder`'s general edit surface
format-preserving. `replace_secret`, `profile`, `provider`, `scope` and the
`Secret`/`Profile` setters have no single canonical textual form (does replacing
a value reorder an inline table's keys, or append?), and pretending otherwise
means maintaining two hand-mutated representations in lockstep forever, for
every future schema field. `into_builder()`/`to_builder()` stays a hard
boundary: only `Spec` carries source text.

For the same reason `to_toml()` and `preserved_text()` are two methods rather
than one. A single polymorphic `to_toml()` whose exactness depends on hidden
state is a footgun for exactly the callers who need the guarantee — they'd get a
silently regenerated document. Making the exact path return `Option` forces the
question at the type level.

**Precedent.** This is the problem `cargo add` has, and I'd suggest the same
resolution. Cargo has a serde model of `Cargo.toml` *and* needs
formatting-preserving edits; the team explicitly didn't want two TOML parsers,
so `cargo-edit` performs `toml_edit` surgery kept separate from the serde
`Manifest` rather than fused into it. They still invest in it
(rust-lang/cargo#12838, "fix(add): Preserve more comments"), and epage maintains
both `toml_edit` and `cargo-add`.

**Most of this already exists here, privately.** `cli/mod.rs:618` is
`add_secret_to_manifest(source, profile, name, description)` — `toml_edit`-backed,
whose own doc comment says it "retains the user's comments, whitespace,
ordering, and any syntax that is not represented by `Config`."
`cli/mod.rs:479` is `generate_toml_with_comments(&Config)`, a complete
deterministic writer used by `init`. And `toml_edit = "0.23"` is already a
workspace dependency, just gated `cli = ["dep:toml_edit", ...]`. So this is
mostly promotion and a feature-gate change (`manifest-edit = ["dep:toml_edit"]`,
`cli = ["manifest-edit", ...]`), not new machinery. Removal is the one genuinely
new capability — and it's what makes the round-trip *provable*, since you can't
assert byte-exact undo with `add` alone.

**One wrinkle I'd rather flag than have surface at review.** `Spec::from_toml`
rejects a document with a non-empty `project.extends`, having no path to resolve
against; `Spec::try_from(path)` handles it and already stores `base_dir`. So the
internal reparse needs an `extends`-aware variant seeded with `self.base_dir`.
Solvable with an internal helper shared with `try_from` — no new public API —
but it's real work, not a one-liner. Relatedly, the retained text must be the
**root file only**: `Config::try_from(path)` merges parents, so writing a merged
config back into the root document would silently inline inherited declarations
into the child file.

### Use cases

**1. Config-automation bots opening PRs.** Any tool that proposes a
`secretspec.toml` change as a pull request — service scaffolding, a CI job
declaring a newly-required secret when a workflow changes, a Renovate-style bot
— needs that diff to be a clean one-line addition. A `Config` round-trip can't
produce that diff by construction: it reorders every profile, strips every
comment, and collides with every other open PR touching the file. Whether the
diff is one line or a full-file rewrite decides whether the change is reviewable
at all.

**2. Editor and pre-commit tooling.** A "declare this env var" quick-fix, or a
pre-commit hook adding a missing declaration, shouldn't reformat a file a human
maintains by hand in order to add one line to it.

**3. Provable undo (my case, briefly).** I maintain a downstream fork running
`secretspec` as a privilege-separated broker, where `add` has a runtime inverse
and the guarantee is that add → undeclare restores the manifest byte for byte,
not merely to something semantically equivalent. That's testable precisely
because `toml_edit` doesn't touch what it wasn't asked to touch.

### What I'm offering

The `toml_edit` surgery exists and is tested in my fork as an extension of your
`cli/mod.rs:618`, plus the removal counterpart. The tests assert the round-trip
directly, including against a document mixing the shapes most likely to trigger
reserialization — a `required` table for a presence group, a full
`[profiles.x.NAME]` table, a quoted dotted key, and inline tables in one file —
across all three requiredness states. Plus negative cases: the "is it declared"
predicate parsed rather than substring-matched (so it isn't fooled by a comment
or by the name appearing inside another secret's description), an unparseable
manifest reported as an error rather than `false`, and removal of an undeclared
name an error rather than a silent no-op.

Porting that onto the `Spec` shape above is more work than exposing my module
directly, but it's the right amount of work given where you've said the public
surface is going. Happy to open the PR against this shape, or a different one
you'd prefer — I mainly wanted the use case on record first, as you asked.

---

## Notes to self (NOT part of the issue)

- Do NOT claim #334 "fully covers" #357. Verified false twice over: `build_ir`
  is reachable only via `#[doc(hidden)] pub mod __private`, which upstream's own
  doc comment disclaims as "not part of the supported Rust SDK"; and `Spec` has
  no accessor returning declaration metadata at all — `secrets()` yields
  `Item = &str` names only. The honest concession is the architectural point
  (Config stays internal), which is what the draft makes.
- Do NOT lead with template-check: docs/design/template-check-resync.md records
  that workflow as having zero users. The live property is add -> undeclare.
- Do NOT use the dependency-weight argument: clap/inquire/miette/tempfile are
  all non-optional upstream. Visibility + the toml_edit feature gate are the
  real, airtight blockers.
- Separate, second ask once this lands: `Spec::schema_json(profile)`, to get our
  broker off `__private::codegen::build_ir`. Tracked in
  sudo-secretspec/UPSTREAM-CONTACT.md under "shape debt".
