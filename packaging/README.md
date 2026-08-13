# sudo-secretspec downstream packaging

This downstream release preserves the upstream SecretSpec crate/workspace version
`0.19.1` and uses a packaging-only downstream tag:

- Tag: `v0.19.1-djbclark.1`
- Release title: `SecretSpec 0.19.1 — sudo-secretspec downstream 1`
- Fork: `djbclark/sudo-secretspec` (parent `cachix/secretspec`)

## Release

The maintainer entrypoint is `packaging/release.py`. It validates a clean
`main`/`master` tree, HTTPS remotes, the upstream `v0.19.1` ancestry/version,
and GitHub fork parent before any write.

```bash
# Safe preview; performs no writes.
python3 packaging/release.py --dry-run

# Live release after packaging and companion changes are committed on main.
python3 packaging/release.py

# Alternate local tap clone.
python3 packaging/release.py --tap-path ~/src/homebrew-sudo-secretspec
```

The live flow runs focused Python and Rust tests, creates and pushes an
annotated tag, creates the titled GitHub Release with `gh`, downloads the tag
tarball over HTTPS, rewrites the formula SHA-256, commits/pushes that formula,
syncs the configured tap clone, force-refreshes Homebrew and the tap checkout,
reinstalls and tests the formula, then reads back repository parent, tag,
release title/state, and installed engine version. Interrupt cleanup removes
only a locally-created unpushed tag and an uncommitted formula rewrite; it
never deletes a pushed tag.

## Homebrew safety boundary

The formula builds the upstream `secretspec` engine and installs companion
files from `sudo-secretspec/bin`, `sudo-secretspec/libexec`, and
`sudo-secretspec/share` when those paths exist in the tagged source. Missing
companion paths are tolerated so upstream-only snapshots remain buildable.

Installation has no privileged side effects: it never invokes `sudo`, edits
sudoers, creates users, or mutates `/var/db`. The caveat tells the operator to
explicitly run `sudo-secretspec install` after reviewing the installed files.

Canonical formula: `packaging/homebrew/sudo-secretspec.rb`.
Tap clone default: `~/src/homebrew-sudo-secretspec`.
