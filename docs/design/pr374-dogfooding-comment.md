# Comment posted on cachix/secretspec#374

Submitted 2026-08-17. Kept as a tracked file for the same reason as
`pr370-body.md` and `pr362-comment.md`: a review round has to be read against
the exact submitted wording, and scratch does not survive a session.

---

I've put this API through a real consumer before review rather than after, so
here's a dogfooding report from a downstream fork that maintains a
root-privileged manifest editor.

**Context:** the consumer is a privilege-separated broker that owns a
root-owned `secretspec.toml` in a protected vault. Its `add`/`undeclare` verbs
were calling the old `manifest_edit` helpers directly. I ported them to the
`Spec` surface this PR proposes.

**What worked without adjustment**

- `Secret::new` / `Secret::required` / `Secret::optional` turn out to express
  exactly the tri-state I had been passing around as `Option<bool>` — leave
  requiredness to the profile default, or pin it either way. I had assumed
  adopting the `Secret`-shaped API would cost me that distinction, and it
  doesn't. Worth noting for anyone else weighing the same migration.
- The byte-exactness holds under the test I care about most: add a declaration
  and remove it again, and the document is restored byte for byte. My undo path
  compares manifests as bytes, so a round trip that merely preserved *meaning*
  would be useless to me. This is now asserted downstream.
- Revalidating the edited document as a whole is a real improvement for a
  privileged writer: a declaration that would not load is refused before
  anything reaches the vault, instead of surfacing at the next `check`.

**`Spec::from_toml` is load-bearing for a privileged editor, somewhat by accident**

It's the constructor I want in a root process specifically because it never
touches the filesystem and refuses `project.extends` — so an edit cannot be
induced into reading whatever a parent path points at. Right now that property
falls out of the extends refusal rather than being stated as a guarantee. It
might be worth saying so in the doc comment: "does not read the filesystem" is
the reason a caller in a privileged context picks this over
`TryFrom<&Path>`, and it's currently something you have to derive by reading
`reparse`.

**One call site I deliberately did not port**

A guard that asks whether a *different* document — a tracked template the
editor doesn't hold as a `Spec` — declares a given name, and which must fail
closed: an unparseable template is not evidence that a name is absent from it,
and the guard exists to protect exactly the names it might have failed to read.

`Spec::declares_secret_in_text` answers `bool`, so an unrepresentable document
reads as "not declared". In practice that branch is near-unreachable, since
constructing the `Spec` already parsed the source — this is not a bug report.
But the predicate is about the spec's *own* text, and my question is about
someone else's; routing it through a `Spec` would also mean full validation and
an `extends` refusal on a document that is allowed to inherit.

So that one stays on `manifest_edit::declares_secret`, which takes text and
returns `Result`. This works only because the PR promotes `manifest_edit` to a
public module behind its own feature. Flagging it in case that surface is ever
considered for narrowing: the `Spec` methods and the text functions serve
genuinely different callers, and at least one real consumer needs both.

Happy to share the downstream diff if it's useful.
