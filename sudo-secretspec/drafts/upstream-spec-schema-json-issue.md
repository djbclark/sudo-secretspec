# upstream issue for cachix/secretspec
# Status: POSTED 2026-08-16 as https://github.com/cachix/secretspec/issues/371
# Kept separate from the `Spec::to_toml()` ask (#370) per operator decision,
# so neither dilutes the other.
# Verified against upstream/main at dfa4b10.
# Retained as the record of what was sent; edit only to correct the record.

**Title:** No supported path from `Spec` to a JSON Schema — `secretspec schema` has no library equivalent

---

A second, much smaller follow-up to the `Spec` consolidation. Filing it apart
from #370 because it's a different shape of ask — that one wants new
capability, this one just wants an existing emitter to be reachable.

### ELI5

`secretspec schema` prints a JSON Schema of a manifest's typed shape. It's
exactly the right thing, it's value-free, and it already works.

A Rust program that has a `Spec` cannot do the same thing. There is no method
on `Spec`, and the emitter behind the CLI command isn't reachable from outside
the crate. So a library consumer has to shell out to the binary to get output
the library it already depends on could hand it directly.

### Technically

The machinery is all there, but each half is closed off in a different way:

- `codegen::build_ir(spec: &Spec) -> CodegenIr` (`codegen.rs:139`) takes a
  `Spec` and is public, but is reachable only through
  `#[doc(hidden)] pub mod __private` (`lib.rs:82`) — whose own doc comment
  says *"These document types are not part of the supported Rust SDK. Use
  `Spec` and its builder API instead."*
- `codegen::schema::emit(&CodegenIr, Option<&str>) -> Result<String, String>`
  (`codegen.rs:237`) is `pub(crate)` **and** `#[cfg(any(feature = "cli", test))]`,
  so it isn't re-exported anywhere and isn't even compiled unless `cli` is on.

The net effect is that the only supported way to obtain a schema is to run the
binary. `cli/mod.rs:1530` (`Commands::Schema`) does nothing a library caller
couldn't, given access — it calls `build_ir_from_manifest` and `schema::emit`
and prints the result.

Note the two halves fail differently, which is why enabling `cli` isn't a
workaround: `cli` makes `schema::emit` *exist*, but it stays `pub(crate)`, so
it's still unreachable. And taking `cli` is not free — it pulls `clap_complete`,
`clap_complete_nushell` and `is_executable` into a consumer that wants a string
of JSON.

### The ask

One method, mirroring the CLI's semantics exactly:

```rust
impl Spec {
    /// JSON Schema for this spec's typed shape. `None` emits the union
    /// `SecretSpec`; `Some(profile)` emits that profile's effective fields.
    pub fn schema_json(&self, profile: Option<&str>) -> Result<String>;
}
```

That fits the direction you stated on #357 — the public API being `Spec` —
rather than widening `__private`, and it lets `Commands::Schema` become a thin
caller of it. No new dependency: `serde_json` is already non-optional. The
`#[cfg(feature = "cli")]` gate on the `schema` module would need to go or be
widened, which is the only real decision here.

### Why I care

I maintain a downstream fork that runs `secretspec` as a privilege-separated
broker. One brokered operation emits a value-free JSON Schema of the runtime
manifest so a caller can learn the *shape* of the secrets it will be given —
which names exist and which are required — without being able to read a single
value. It reads no values by construction, because `build_ir` only ever sees
declarations.

That broker runs as root, so it deliberately cannot take the `cli` feature:
`clap` and `inquire` have no business inside a root-privileged process. Before
the `Spec` merge it reached `codegen` directly. After, it compiles only because
I added a local feature that widens the module's visibility — carrying a patch
against your internals, which is precisely what I'd rather not do, and which
will break silently the next time `__private` moves.

### What I'm offering

Happy to open the PR: add `Spec::schema_json`, reduce `Commands::Schema` to a
call of it, and adjust the feature gate. It's a small change and I don't think
it's controversial, but it's yours to shape — say the word if you'd rather it
were named differently, took `&str` with a separate union method, or returned
`serde_json::Value` instead of `String`.
