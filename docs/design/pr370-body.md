<!-- Posted as https://github.com/cachix/secretspec/pull/374 on 2026-08-17.
     Branch `spec-manifest-edit` at b3637e2, built on a worktree off
     upstream/main (dfa4b10), NOT cherry-picked from sudo-main. The fork-local
     equivalent is 0483d3a on sudo-main. Kept as a tracked file rather than in
     scratch so the exact submitted wording survives the session. -->

Closes #370.

Opening this against the shape you described on #357 — everything lands on `Spec`, `Config` stays internal, and no `toml_edit` or `Config` appears in any public signature.

### What it does

`Spec` can read a `secretspec.toml` and can build a new one, but cannot write an existing one back out. So editing one declaration in a real file means either regenerating the whole document from the parsed model — losing every comment and reordering every profile, since `Config.profiles` is a `HashMap` — or hand-rolling `toml_edit` outside the library.

Five methods, the surface proposed in the issue:

```rust
impl Spec {
    pub fn add_secret_to_text(&self, profile: &str, name: &str, secret: Secret) -> Result<Spec>;
    pub fn remove_secret_from_text(&self, profile: &str, name: &str) -> Result<Spec>;
    pub fn declares_secret_in_text(&self, profile: &str, name: &str) -> bool;
    pub fn preserved_text(&self) -> Option<&str>;
    pub fn to_toml(&self) -> Result<String>;
}
```

`Spec::from_toml(s)?.preserved_text() == Some(s)`, and add-then-remove of the same key returns the original bytes.

### How it stays safe

The edits are `toml_edit` surgery on the retained text; `config`/`compiled` are then re-derived by **reparsing the result through the same validated path every other `Spec` already goes through**. The semantic view is always derived from the text, never hand-mutated alongside it — one synchronization point instead of one per method, so the two cannot disagree, and an edit that would not validate fails at the edit rather than at some later load.

Per the issue, `SpecBuilder`'s general edit surface is **not** made format-preserving. `into_builder()`/`to_builder()` stays a hard boundary: only `Spec` carries source text.

### The two wrinkles I flagged, and how they're handled

**1. `extends`.** `Spec::from_toml` rejects a non-empty `project.extends`, having nowhere to resolve paths from, so reparsing edited text through it would fail on every inheriting project. `Config::from_text_in` is the `extends`-aware variant, seeded with the `base_dir` `Spec` already records. `ConfigGraphLoader` gained an in-memory entry point, and its `extends` walk is now shared with the path-based one rather than duplicated.

**2. Root file only.** `Config::try_from` folds parents into the child, so retaining the *merged* text and writing it back would silently inline every inherited declaration into a file that had merely referenced them. A test asserts the parent's declaration does not appear in the child's text while `secrets()` still sees it.

### Module and feature

The `toml_edit` surgery moves out of `cli` into a `manifest_edit` module behind a new `manifest-edit` feature, which `cli` enables — nothing changes for existing users, but an embedder can take the editing surface without `clap` and `inquire`.

`toml_edit` gains its `serde` feature so a whole `Secret` renders through the same representation the parser reads, keeping written and accepted keys from drifting as the schema grows. The value serializer refuses nested tables — which `ref`, `refs`, `extract`, `generate` and a presence group's `required` all produce — so declarations serialize via `to_document` and are flattened to an inline table. `Cargo.lock` gains only `serde_core` and `serde_spanned`, both already in the tree via `toml`; no new crates.

**Requiredness is untouched.** As I said on the issue, #334 settled it — `add_secret_to_manifest` keeps its existing description-only signature, and anything richer goes through the `Secret` that `add_secret_to_text` already takes.

### Two deviations from the issue's sketch, both deliberate

- `to_toml` returns `Result<String>`, not `String`. TOML serialization is fallible and I would rather surface that than unwrap inside the library.
- The three editing methods are `#[cfg(feature = "manifest-edit")]`, since they need `toml_edit`. `preserved_text` and `to_toml` are unconditional.

Happy to change either, or the naming, if you'd prefer something else.

### Tests

New tests cover the byte-exact round-trip; that comments, declaration order, and a full `[profiles.x.NAME]` table all survive an edit; that the returned `Spec` is revalidated rather than merely rewritten, and that the spec it derived from is untouched; that an invalid edit fails at the edit; that a declaration carrying `providers`, `as_path` and a nested `ref` round-trips; duplicate-add and absent-remove errors; `declares_secret_in_text` parsed rather than substring-matched, so a name inside a comment or another secret's description does not fool it; and a builder-built spec reporting no text rather than inventing one.

Four more cover inheritance against real files on disk: root-only retention, editing a spec that `extends` another, an inherited declaration not being editable in the child, and the round-trip holding through inheritance.

Full suite green on this branch. The 21 `provider::sops::*` failures in my environment are all `The 'sops' CLI is not installed` and occur identically on an unmodified `dfa4b10`.
