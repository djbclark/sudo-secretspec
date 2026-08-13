#!/usr/bin/env python3
"""Release sudo-secretspec downstream v0.19.1-djbclark.1 and its tap.

The release keeps upstream crate versions at 0.19.1. Use ``--dry-run`` to print
all mutating commands. The live path validates fork lineage, creates an
annotated tag and GitHub Release, rewrites the formula checksum, synchronizes
the tap, and performs a live Homebrew readback test.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import signal
import subprocess
import tempfile
import urllib.request
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.19.1-djbclark.1"
TAG = f"v{VERSION}"
UPSTREAM_TAG = "v0.19.1"
RELEASE_TITLE = "SecretSpec 0.19.1 — sudo-secretspec downstream 1"
FORK_REPO = "djbclark/sudo-secretspec"
UPSTREAM_REPO = "cachix/secretspec"
ORIGIN_URL = f"https://github.com/{FORK_REPO}.git"
UPSTREAM_URL = f"https://github.com/{UPSTREAM_REPO}.git"
FORMULA_REL = Path("packaging/homebrew/sudo-secretspec.rb")
FORMULA = ROOT / FORMULA_REL
DEFAULT_TAP = Path.home() / "src" / "homebrew-sudo-secretspec"
FORMULA_NAME = "djbclark/sudo-secretspec/sudo-secretspec"
ARCHIVE_URL = f"https://github.com/{FORK_REPO}/archive/refs/tags/{TAG}.tar.gz"
VERSION_RE = re.compile(r"^0\.19\.1-djbclark\.1$")


class ReleaseError(RuntimeError):
    """Release invariant failed."""


def log(message: str) -> None:
    print(message, flush=True)


def run(
    argv: list[str],
    *,
    cwd: Path = ROOT,
    capture: bool = True,
    check: bool = True,
    dry_run: bool = False,
) -> subprocess.CompletedProcess[str]:
    if dry_run:
        log("[dry-run] " + " ".join(str(value) for value in argv))
        return subprocess.CompletedProcess(argv, 0, stdout="", stderr="")
    log("+ " + " ".join(str(value) for value in argv))
    return subprocess.run(argv, cwd=cwd, text=True, capture_output=capture, check=check)


def output(argv: list[str], **kwargs: object) -> str:
    return (run(argv, **kwargs).stdout or "").strip()


def validate_version(version: str) -> None:
    if not VERSION_RE.fullmatch(version):
        raise ReleaseError(
            f"this downstream release is pinned to {VERSION}, got {version!r}"
        )


def validate_release_url(url: str) -> None:
    parsed = urlsplit(url)
    if parsed.scheme != "https" or parsed.hostname != "github.com":
        raise ReleaseError(f"refuse non-GitHub HTTPS release URL: {url!r}")


def _require(value: bool, message: str) -> None:
    if not value:
        raise ReleaseError(message)


def preflight(*, allow_dirty: bool) -> None:
    status = output(["git", "status", "--porcelain"])
    _require(
        allow_dirty or not status,
        "working tree is dirty; commit/stash first or pass --allow-dirty",
    )
    branch = output(["git", "branch", "--show-current"])
    _require(branch == "sudo-main", "downstream release must run from sudo-main")
    _require(
        output(["git", "remote", "get-url", "origin"]) == ORIGIN_URL,
        f"origin must be {ORIGIN_URL}",
    )
    _require(
        output(["git", "remote", "get-url", "upstream"]) == UPSTREAM_URL,
        f"upstream must be {UPSTREAM_URL}",
    )
    output(["git", "rev-parse", f"{UPSTREAM_TAG}^{{commit}}"])
    run(["git", "merge-base", "--is-ancestor", UPSTREAM_TAG, "HEAD"])
    manifest = output(["git", "show", f"{UPSTREAM_TAG}:Cargo.toml"])
    _require(
        'version = "0.19.1"' in manifest,
        f"{UPSTREAM_TAG} is not upstream workspace version 0.19.1",
    )
    _require(
        f'version = "{VERSION}"' in (ROOT / "Cargo.toml").read_text(encoding="utf-8"),
        "workspace downstream version mismatch",
    )
    repo = json.loads(
        output(
            [
                "gh",
                "repo",
                "view",
                FORK_REPO,
                "--json",
                "nameWithOwner,parent,defaultBranchRef",
            ]
        )
    )
    _require(repo.get("nameWithOwner") == FORK_REPO, f"gh target is not {FORK_REPO}")
    _require(
        (repo.get("defaultBranchRef") or {}).get("name") == branch,
        "current branch is not the fork default branch",
    )
    parent = (repo.get("parent") or {}).get("nameWithOwner")
    _require(
        parent == UPSTREAM_REPO, f"fork parent must be {UPSTREAM_REPO}, got {parent!r}"
    )


def run_tests(*, dry_run: bool) -> None:
    run(["pytest", "tests/sudo_packaging", "-q"], dry_run=dry_run, capture=False)
    # Core is unchanged; compile the pinned upstream engine without invoking
    # provider integration tests that depend on optional host CLIs (for example sops).
    run(
        ["cargo", "build", "-p", "secretspec", "--locked"],
        dry_run=dry_run,
        capture=False,
    )


def rewrite_formula(path: Path, version: str, sha256: str) -> None:
    validate_version(version)
    text = path.read_text(encoding="utf-8")
    text, urls = re.subn(
        r'url "https://github\.com/djbclark/sudo-secretspec/archive/refs/tags/v[^\"]+\.tar\.gz"',
        f'url "https://github.com/{FORK_REPO}/archive/refs/tags/v{version}.tar.gz"',
        text,
        count=1,
    )
    text, hashes = re.subn(
        r'sha256 "[0-9a-f]{64}"', f'sha256 "{sha256}"', text, count=1
    )
    if urls != 1 or hashes != 1:
        raise ReleaseError(f"failed to rewrite exactly one URL and checksum in {path}")
    path.write_text(text, encoding="utf-8")


def archive_sha256(*, dry_run: bool) -> str:
    validate_release_url(ARCHIVE_URL)
    if dry_run:
        log(f"[dry-run] hash {ARCHIVE_URL}")
        return "0" * 64
    digest = hashlib.sha256()
    request = urllib.request.Request(
        ARCHIVE_URL, headers={"User-Agent": "sudo-secretspec-release"}
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        while chunk := response.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def create_tag_and_release(notes: str, state: CleanupState, *, dry_run: bool) -> None:
    if output(["git", "tag", "--list", TAG]) and not dry_run:
        raise ReleaseError(f"local tag already exists: {TAG}")
    run(["git", "tag", "-a", TAG, "-m", RELEASE_TITLE], dry_run=dry_run)
    state.tag_created = not dry_run
    run(["git", "push", "origin", TAG], dry_run=dry_run)
    state.tag_pushed = not dry_run
    if dry_run:
        run(
            [
                "gh",
                "release",
                "create",
                TAG,
                "--repo",
                FORK_REPO,
                "--title",
                RELEASE_TITLE,
                "--notes-file",
                "<temporary-notes>",
            ],
            dry_run=True,
        )
        return
    with tempfile.NamedTemporaryFile("w", suffix=".md", encoding="utf-8") as notes_file:
        notes_file.write(notes)
        notes_file.flush()
        run(
            [
                "gh",
                "release",
                "create",
                TAG,
                "--repo",
                FORK_REPO,
                "--title",
                RELEASE_TITLE,
                "--notes-file",
                notes_file.name,
            ]
        )


def update_formula_commit(*, dry_run: bool, state: CleanupState) -> None:
    sha = archive_sha256(dry_run=dry_run)
    if dry_run:
        log(f"[dry-run] rewrite {FORMULA_REL} for {TAG} sha256={sha}")
        run(["git", "add", str(FORMULA_REL)], dry_run=True)
        run(["git", "commit", "-m", f"Update Homebrew formula for {TAG}"], dry_run=True)
        run(["git", "push", "origin", "HEAD"], dry_run=True)
        return
    rewrite_formula(FORMULA, VERSION, sha)
    state.formula_changed = True
    run(["git", "add", str(FORMULA_REL)])
    run(["git", "commit", "-m", f"Update Homebrew formula for {TAG}"])
    state.formula_changed = False
    run(["git", "push", "origin", "HEAD"])


def sync_tap(tap_path: Path, *, dry_run: bool) -> None:
    destination = tap_path / "Formula" / "sudo-secretspec.rb"
    if dry_run:
        log(f"[dry-run] copy {FORMULA_REL} -> {destination}")
        run(["git", "add", "Formula/sudo-secretspec.rb"], cwd=tap_path, dry_run=True)
        run(
            ["git", "commit", "-m", f"sudo-secretspec {VERSION}"],
            cwd=tap_path,
            dry_run=True,
        )
        run(["git", "push", "origin", "HEAD"], cwd=tap_path, dry_run=True)
        return
    _require((tap_path / ".git").exists(), f"tap clone missing or not Git: {tap_path}")
    _require(
        not output(["git", "status", "--porcelain"], cwd=tap_path),
        f"tap working tree is dirty: {tap_path}",
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(FORMULA, destination)
    run(["git", "add", "Formula/sudo-secretspec.rb"], cwd=tap_path)
    if output(["git", "diff", "--cached", "--name-only"], cwd=tap_path):
        run(["git", "commit", "-m", f"sudo-secretspec {VERSION}"], cwd=tap_path)
        run(["git", "push", "origin", "HEAD"], cwd=tap_path)


def brew_refresh_and_test(*, dry_run: bool) -> str:
    run(["brew", "update", "--force"], dry_run=dry_run, capture=False)
    tap_repo = output(
        ["brew", "--repository", "djbclark/sudo-secretspec"], dry_run=dry_run
    )
    if not dry_run:
        tap = Path(tap_repo)
        _require(tap.is_dir(), "Homebrew did not return the tap checkout")
        run(["git", "fetch", "origin", "main"], cwd=tap, capture=False)
        run(["git", "merge", "--ff-only", "origin/main"], cwd=tap, capture=False)
        _require(
            TAG in (tap / "Formula/sudo-secretspec.rb").read_text(encoding="utf-8"),
            "live tap formula is stale",
        )
    run(["brew", "reinstall", FORMULA_NAME], dry_run=dry_run, capture=False)
    prefix = (
        output(["brew", "--prefix", FORMULA_NAME], dry_run=dry_run)
        or "/opt/homebrew/opt/sudo-secretspec"
    )
    run(["brew", "test", FORMULA_NAME], dry_run=dry_run, capture=False)
    return prefix


def verify_readback(prefix: str) -> None:
    repo = json.loads(
        output(["gh", "repo", "view", FORK_REPO, "--json", "nameWithOwner,parent"])
    )
    _require(repo.get("nameWithOwner") == FORK_REPO, "repository readback mismatch")
    _require(
        (repo.get("parent") or {}).get("nameWithOwner") == UPSTREAM_REPO,
        "fork lineage readback mismatch",
    )
    ref = json.loads(output(["gh", "api", f"repos/{FORK_REPO}/git/ref/tags/{TAG}"]))
    _require(ref.get("ref") == f"refs/tags/{TAG}", "tag readback mismatch")
    release = json.loads(
        output(
            [
                "gh",
                "release",
                "view",
                TAG,
                "--repo",
                FORK_REPO,
                "--json",
                "tagName,name,isDraft",
            ]
        )
    )
    _require(
        release == {"tagName": TAG, "name": RELEASE_TITLE, "isDraft": False},
        "release readback mismatch",
    )
    version = output([str(Path(prefix) / "bin" / "secretspec"), "--version"])
    _require(VERSION in version, f"installed engine version mismatch: {version!r}")


@dataclass
class CleanupState:
    tag_created: bool = False
    tag_pushed: bool = False
    formula_changed: bool = False


def cleanup_interrupted(state: CleanupState) -> None:
    if state.tag_created and not state.tag_pushed:
        run(["git", "tag", "-d", TAG])
    if state.formula_changed:
        run(["git", "restore", "--staged", "--worktree", "--", str(FORMULA_REL)])


@contextmanager
def interrupt_cleanup(state: CleanupState) -> Iterator[None]:
    previous = {
        number: signal.getsignal(number) for number in (signal.SIGINT, signal.SIGTERM)
    }

    def handler(number: int, _frame: object) -> None:
        cleanup_interrupted(state)
        raise ReleaseError(f"interrupted by {signal.Signals(number).name}")

    for number in previous:
        signal.signal(number, handler)
    try:
        yield
    finally:
        for number, old_handler in previous.items():
            signal.signal(number, old_handler)


def default_notes() -> str:
    return (
        "Downstream packaging release based on upstream SecretSpec 0.19.1.\n\n"
        "Adds the opt-in `sudo-secretspec` privilege-boundary companion. Homebrew installs files only; "
        "run `sudo-secretspec install` explicitly to configure privileged state.\n"
    )


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--allow-dirty", action="store_true")
    parser.add_argument("--skip-tests", action="store_true")
    parser.add_argument("--skip-homebrew", action="store_true")
    parser.add_argument("--notes-file", type=Path)
    parser.add_argument("--tap-path", type=Path, default=DEFAULT_TAP)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    validate_version(VERSION)
    if not args.dry_run:
        preflight(allow_dirty=args.allow_dirty)
    if not args.skip_tests:
        run_tests(dry_run=args.dry_run)
    notes = (
        args.notes_file.read_text(encoding="utf-8")
        if args.notes_file
        else default_notes()
    )
    state = CleanupState()
    with interrupt_cleanup(state):
        create_tag_and_release(notes, state, dry_run=args.dry_run)
        if not args.skip_homebrew:
            update_formula_commit(dry_run=args.dry_run, state=state)
            sync_tap(args.tap_path, dry_run=args.dry_run)
            prefix = brew_refresh_and_test(dry_run=args.dry_run)
            if not args.dry_run:
                verify_readback(prefix)
    log(f"done: {RELEASE_TITLE} ({TAG})")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ReleaseError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from error
