"""Fixtures for the post-install suite.

Everything here is gated so the suite is inert unless it is pointed at a real,
installed boundary. That is the whole safety story: these tests drive the
operator's live vault, not a fixture, because a boundary is exactly the thing
that cannot be faked -- it is defined by root-owned files, a sudoers policy, and
a service identity that only a real install creates.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest

#: The client under test. Defaults to the installed one, which is what "after an
#: install" means. Override to vet a candidate build against the live boundary
#: *before* installing it -- client-side fixes need no reinstall, so this is how
#: you tell which ones do.
CLIENT = Path(
    os.environ.get("SUDO_SECRETSPEC_CLIENT", "/usr/local/bin/sudo-secretspec")
)
CONFIG = Path("/usr/local/etc/sudo-secretspec.toml")

#: Override for the package manager's copy of the broker; see `_package_broker`.
INSTALLER_ENV = "SUDO_SECRETSPEC_INSTALLER"

#: Opt-in for tests that write a *value* to the live vault. They use a scratch
#: secret and delete it again.
WRITE_GATE = "SUDO_SECRETSPEC_POSTINSTALL_WRITES"

#: Separate, louder opt-in for `add`. An `add` here moves the runtime manifest
#: away from the tracked template, which `template-check` then reports as drift
#: until the declaration is mirrored into the tracked file and released.
#:
#: `undeclare` is the inverse and reaches a scratch name -- it refuses only
#: names present in the tracked template, and names still holding a value. The
#: gate stays because clearing the drift is still a deliberate second step, not
#: because the declaration is unremovable.
DECLARE_GATE = "SUDO_SECRETSPEC_POSTINSTALL_DECLARE"

#: Opt-in for the lifecycle dry-run plans. `install`, `uninstall`, and
#: `rollback` are covered by `timestamp_timeout=0` in the installed policy, so
#: sudo demands fresh interactive authentication for each one -- a Touch ID or
#: password prompt per test, even for `--dry-run`, which mutates nothing. An
#: unattended runner cannot answer those, and a runner that is merely *nearby*
#: an operator should not spray prompts at them either.
LIFECYCLE_GATE = "SUDO_SECRETSPEC_POSTINSTALL_LIFECYCLE"

#: Reason recorded against every audited operation this suite performs, so the
#: ledger says plainly why a burst of smoke-test events appeared at once.
REASON = "post-install smoke test"


def run(*argv: str, stdin: str | None = None, timeout: int = 60):
    """Invoke the installed client.

    `stdin` is always supplied -- never inherited. A brokered operation that
    unexpectedly prompts must fail on EOF rather than silently consume the
    test runner's stdin and hang, which is the failure mode `check` actually
    shipped with.
    """
    return subprocess.run(
        [str(CLIENT), *argv],
        input=stdin if stdin is not None else "",
        capture_output=True,
        text=True,
        timeout=timeout,
        # Non-zero is data here, not an error: most of this suite asserts on
        # refusals. Raising would turn every expected denial into a test error.
        check=False,
    )


def _package_broker() -> Path | None:
    """The package manager's copy of the broker, or None if it is not present.

    `install` is the one verb `CLIENT` cannot drive. `resolve_media` refuses to
    source an install from the installed boundary itself -- that would copy
    every artifact onto itself and report success while upgrading nothing -- so
    an install plan asked of `/usr/local/bin/sudo-secretspec` comes back as a
    denial, not a plan. The real installer is the libexec copy Homebrew ships,
    kept off PATH so it cannot shadow the installed client.
    """
    override = os.environ.get(INSTALLER_ENV)
    if override:
        path = Path(override)
        return path if path.is_file() else None

    brew = shutil.which("brew")
    if brew is None:
        return None
    result = subprocess.run([brew, "--prefix"], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        return None
    path = Path(result.stdout.strip()) / "opt/sudo-secretspec/libexec/sudo-secretspec"
    return path if path.is_file() else None


def run_installer(*argv: str, timeout: int = 60):
    """Invoke the package manager's broker copy -- the only one `install` runs from.

    Same stdin discipline as `run`: never inherited, so an unexpected prompt
    fails on EOF instead of consuming the runner's stdin.
    """
    broker = _package_broker()
    assert broker is not None, "no package broker; the installer fixture should have skipped"
    return subprocess.run(
        [str(broker), *argv],
        input="",
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )


def _boundary_reason() -> str | None:
    """Why this host cannot run the suite, or None if it can."""
    if not CLIENT.is_file():
        return f"no installed client at {CLIENT}"
    if not CONFIG.is_file():
        return f"no installed boundary config at {CONFIG}"
    if shutil.which("sudo") is None:
        return "sudo not available"
    return None


def pytest_collection_modifyitems(config, items):
    reason = _boundary_reason()
    if reason is None:
        return
    skip = pytest.mark.skip(
        reason=f"post-install suite needs a live boundary: {reason}"
    )
    for item in items:
        item.add_marker(skip)


@pytest.fixture(scope="session")
def config() -> dict:
    """The installed protected config, as the boundary itself reads it."""
    import tomllib

    return tomllib.loads(CONFIG.read_text())


@pytest.fixture(scope="session")
def version() -> str:
    result = run("--version")
    assert result.returncode == 0, result.stderr
    return result.stdout.strip().split()[-1]


@pytest.fixture
def writes_allowed():
    if os.environ.get(WRITE_GATE) != "1":
        pytest.skip(f"set {WRITE_GATE}=1 to write a scratch secret to the live vault")


@pytest.fixture
def declares_allowed():
    if os.environ.get(DECLARE_GATE) != "1":
        pytest.skip(
            f"set {DECLARE_GATE}=1 to declare into the live runtime manifest. "
            "`undeclare` reverses it, but until then `template-check` reports "
            "the drift, and clearing it is a deliberate second step."
        )


@pytest.fixture
def lifecycle_allowed():
    if os.environ.get(LIFECYCLE_GATE) != "1":
        pytest.skip(
            f"set {LIFECYCLE_GATE}=1 to exercise the lifecycle dry-run plans. "
            "Each one triggers an interactive sudo authentication prompt "
            "(Touch ID or password), because the installed policy sets "
            "timestamp_timeout=0 on install/uninstall/rollback. Only run this "
            "when you are at the keyboard and expecting to authenticate."
        )


@pytest.fixture
def installer() -> Path:
    """The broker copy an install must be sourced from.

    Skips rather than fails when it is absent: a host can carry a perfectly
    good boundary installed from media that is no longer on disk, and that is
    not a defect in the boundary this suite is vetting.
    """
    broker = _package_broker()
    if broker is None:
        pytest.skip(
            "no package broker copy found; set "
            f"{INSTALLER_ENV} to the libexec broker an install would be sourced from"
        )
    return broker


@pytest.fixture
def scratch_name() -> str:
    """A declared-but-unused name reserved for this suite.

    Deliberately not random: a name that changed every run would accumulate
    undeletable declarations. One stable name can be declared once and reused.
    """
    return "POSTINSTALL_SMOKE_SCRATCH"


#: `check` marks each secret: resolved, optional-and-unset, or required-and-missing.
_RESOLVED, _OPTIONAL, _MISSING = "✓", "○", "✗"


def _check_report() -> dict[str, set[str]]:
    """Parse `check` into {marker: names}.

    Reads **both** streams on purpose. 0.19.1-sudo.15 moved the report from
    stderr to stdout, and this helper only wants the names -- pinning it to one
    stream would make it silently return nothing against a boundary from the
    other side of that change, which is worse than useless here: an empty
    result feeds `resolved_name`, which skips rather than fails, quietly
    disabling every test that reads a value. `test_check_writes_its_report_to_stdout`
    is where the stream itself is asserted.
    """
    result = run("check", "--reason", REASON)
    assert result.returncode in (0, 1), result.stderr

    report: dict[str, set[str]] = {_RESOLVED: set(), _OPTIONAL: set(), _MISSING: set()}
    for line in (result.stdout + "\n" + result.stderr).splitlines():
        parts = line.strip().split(None, 2)
        if (
            len(parts) >= 2
            and parts[0] in report
            and parts[1].replace("_", "").isalnum()
        ):
            report[parts[0]].add(parts[1])
    return report


@pytest.fixture(scope="session")
def check_report() -> dict[str, set[str]]:
    return _check_report()


@pytest.fixture
def declared_names(check_report) -> set[str]:
    """Every name the runtime manifest declares, resolved or not."""
    return set().union(*check_report.values())


@pytest.fixture
def resolved_name(check_report) -> str:
    """One name that actually has a value.

    Tests that read a value must use this, not merely a declared name: an
    optional secret is legitimately declared and legitimately empty, so picking
    any declared name at random tests the vault against its own optionality.
    """
    resolved = check_report[_RESOLVED]
    if not resolved:
        # A vault where nothing resolves is a legitimate state, but so is a
        # parsing bug that found nothing at all. Distinguish them: if the
        # report named no secrets whatsoever, the parse failed and skipping
        # would hide it.
        assert set().union(*check_report.values()), (
            "check reported no secrets at all -- _check_report failed to parse "
            "the report, so skipping here would silently disable the read tests"
        )
        pytest.skip("no secret in this vault resolves to a value")
    return min(resolved)


def audit_tip() -> tuple[int, str]:
    """Event count and tip hash the ledger currently reports."""
    result = run("audit-verify")
    assert result.returncode == 0, f"audit-verify failed: {result.stderr}"
    return parse_audit(result.stdout)


def parse_audit(text: str) -> tuple[int, str]:
    import re

    match = re.search(
        r"(\d+)\s+events?.*?([0-9a-f]{64}|\bnone\b)", text, re.DOTALL | re.IGNORECASE
    )
    assert match, f"unparseable audit-verify output: {text!r}"
    return int(match.group(1)), match.group(2)


__all__ = [
    "CLIENT",
    "CONFIG",
    "REASON",
    "audit_tip",
    "parse_audit",
    "run",
]
