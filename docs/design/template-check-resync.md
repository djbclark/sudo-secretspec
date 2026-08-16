# Design note: `template-check` byte-exactness and a lighter resync

**Status:** analysed, deliberately **not implemented**. No known users.
**Date:** 2026-08-15
**Scope:** fork-only (`frdminc/sudo-secretspec`). Not upstream material.
**Prompted by:** an operator report, after they hit the resync cost and
resolved it by abandoning the tracked-declarations workflow entirely.

This records the reasoning so it survives the decision not to build it. The
useful content here is the *rejection* of the obvious fix, which is a trap.

## The friction

`template-check` compares the runtime manifest (`/var/db/sudo-secretspec/
secretspec.toml`, root-owned) against a git-tracked declaration template. It
compares **raw bytes**, not parsed structure.

`add` edits the runtime manifest immediately, over the cheap NOPASSWD broker
path. So one `add` puts the two files out of sync and `template-check` goes
red — correctly; that is the drift it exists to report. The problem was
getting back to green. The only route was `install --declarations <file>`, a
full boundary **lifecycle** command behind interactive `sudo`/Touch ID.

That left exactly two granularities available: "declare one secret" and
"reinstall the entire boundary." Nothing in between, for what is conceptually
a one-line change to a text file.

## The obvious fix is wrong

The tempting fix is to make the comparison **semantic** — parse both files,
compare the declarations, ignore formatting. Do not do this.

Byte-exactness is load-bearing, not incidental strictness:

- It is what makes `template-check` green mean *the running file is the exact
  bytes a human reviewed*. A semantic comparison downgrades that to "the
  running file means something equivalent to what a human reviewed," which is
  a materially weaker claim about a file that decides which secrets exist.
- It is what makes `add` → `undeclare` a **provable** undo. `toml_edit`
  preserves untouched formatting, so removing a declaration `add` inserted
  restores the file byte for byte. Two tests assert exactly this
  (`removing_an_added_declaration_restores_the_original_bytes`, and
  `..._explicit_requiredness_...` for the `--optional`/`--required` path). A
  semantic comparison would let a reserialising undo pass while silently
  leaving the file permanently different from the template.

Weakening the check to buy ergonomics trades a security property for
convenience. The friction is real, but it is not the comparison's fault.

## The fix worth building, when someone needs it

Add a value-free read — call it `export-declarations` — that streams the
runtime manifest's bytes to stdout over the same NOPASSWD broker path
`schema` and `export` already use. Resync then costs no privilege escalation
at all:

```bash
sudo-secretspec export-declarations --reason "resync after add" \
  > secretspec.toml.example
git commit -am "declare FOO"
```

Why this is small and safe:

1. **The privileged step is only the read.** The tracked template is an
   ordinary git-owned file in the operator's repo, so the *write* needs no
   root. Only reading the root-owned manifest crosses the boundary.
2. **No new privilege class.** Manifest reads are already exposed: `schema`
   does precisely this and is audited like everything else. The manifest holds
   declarations, never values — values live in the `.env` beside it.
3. **It preserves byte-exactness instead of weakening it.** Copying the exact
   bytes is what makes the next `template-check` green. The guarantee is kept;
   only the cost of restoring it changes.

It would need the same treatment as every other mediated operation: a
`--reason`, an audit event, and refusal on the privileged-broker path.

## Why it was not built

The operator's own resolution was to delete the tracked-declarations file and
its repo symlinks outright, treating the runtime vault as the single source of
truth. `add`/`set`/`check`/`schema` are now the whole interface, so the
tracked-file workflow has **zero users**.

Shipping an unused code path into a root-privileged broker is a liability, not
an asset: it is surface that must be maintained and audited while nothing
exercises it. This is roughly an hour of work whenever a real user appears —
cheaper to build then than to carry untested now.

If the tracked-file pattern is ever revived, build `export-declarations`; do
not touch the comparison.
