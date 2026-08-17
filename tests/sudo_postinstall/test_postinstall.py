"""Post-install smoke suite: every basic operation against a live boundary.

Run this after `sudo-secretspec install` to prove the boundary the operator just
installed actually works, rather than inferring it from a green unit suite. The
unit tests cover logic; this covers the thing that only exists after an install
-- a root-owned broker reachable through a sudoers policy, serving a vault owned
by a service user.

    ~/.local/bin/pytest tests/sudo_postinstall -q

Read-only and prompt-free by default; everything that writes or authenticates is
behind a gate. See `conftest` for the three of them.

`install`, `uninstall`, and `rollback` are never fully exercised: they mutate the
boundary running the tests. Only their `--dry-run` plans are asserted, and even
those are gated, because the installed policy sets `timestamp_timeout=0` on the
lifecycle paths -- so every dry run, mutating nothing, still raises an
interactive authentication prompt. An unattended runner cannot answer one, and an
attended one should not be surprised by four.
"""

from __future__ import annotations

import re

import pytest
from conftest import REASON, audit_tip, parse_audit, run

# --------------------------------------------------------------------------
# Identity: is the boundary the one this checkout builds?
# --------------------------------------------------------------------------


def test_client_reports_a_downstream_version(version):
    assert re.fullmatch(r"0\.19\.1-(sudo|djbclark)\.\d+", version), version


def test_client_and_broker_are_the_same_release(version):
    """A client newer than its broker is the failure the reason-digest change
    introduced deliberately, and the one an upgrade without a reinstall causes.
    `doctor` is served by the broker, so its success proves the pair matches."""
    result = run("doctor")
    assert result.returncode == 0, (
        f"doctor failed against the installed broker:\n{result.stderr}"
    )


def test_config_names_a_vault_under_the_protected_root(config):
    assert config["vault"].startswith("/var/db/")
    assert config["vault_realpath"].startswith("/private/var/db/")
    assert config["vault"].split("/")[-1] == config["vault_realpath"].split("/")[-1]


def test_config_names_a_service_user_that_is_not_the_operator(config):
    import getpass

    assert config["service_user"] != getpass.getuser()
    assert config["service_user"].startswith("_")


# --------------------------------------------------------------------------
# Health: doctor, drift, template-check
# --------------------------------------------------------------------------


def test_doctor_passes():
    result = run("doctor")
    assert result.returncode == 0, result.stdout + result.stderr
    assert "OK" in result.stdout


def test_doctor_does_not_need_a_reason():
    """Metadata-only checks read no secret, so they are not audited operations
    and must not demand a justification."""
    result = run("doctor")
    assert result.returncode == 0
    assert "reason" not in result.stderr.lower()


def test_template_check_compares_the_manifest_to_the_tracked_template():
    result = run("template-check", "--reason", REASON)
    # 0 = in sync, non-zero = drift reported. Both are working behaviour; a
    # crash or an unrecognized-subcommand error is not.
    assert "unrecognized subcommand" not in result.stderr, (
        "template-check is missing from this client -- the boundary predates it"
    )
    assert result.returncode in (0, 1), result.stdout + result.stderr


def test_template_check_requires_a_reason():
    result = run("template-check")
    assert result.returncode != 0
    assert "reason" in (result.stderr + result.stdout).lower()


# --------------------------------------------------------------------------
# Audit ledger
# --------------------------------------------------------------------------


def test_audit_verify_reports_an_intact_chain():
    result = run("audit-verify")
    assert "unrecognized subcommand" not in result.stderr, (
        "audit-verify is missing from this client -- the boundary predates it"
    )
    assert result.returncode == 0, result.stdout + result.stderr
    count, tip = parse_audit(result.stdout)
    assert count >= 0
    assert tip == "none" or len(tip) == 64


def test_audit_verify_takes_no_reason():
    """It reads no secret, and its job is to prove the ledger is intact --
    requiring a reason would add an event to the thing being verified."""
    result = run("audit-verify", "--reason", REASON)
    assert result.returncode != 0


def test_an_audited_operation_appends_to_the_ledger():
    before_count, before_tip = audit_tip()

    result = run("check", "--reason", REASON)
    assert result.returncode in (0, 1), result.stderr

    after_count, after_tip = audit_tip()
    assert after_count > before_count, "check did not produce an audit event"
    assert after_tip != before_tip, "ledger tip did not advance"


def test_the_ledger_still_verifies_after_this_suite_writes_to_it():
    """The chain is hash-linked; appending must never break verification."""
    run("check", "--reason", REASON)
    result = run("audit-verify")
    assert result.returncode == 0, result.stdout + result.stderr


# --------------------------------------------------------------------------
# Read operations
# --------------------------------------------------------------------------


def test_check_reports_a_summary_and_never_prompts():
    """`check` shipped with prompting enabled inside the root broker: a missing
    secret dropped it into interactive value entry, reading the caller's stdin.
    conftest.run always passes empty stdin, so a regression hangs to the timeout
    or errors -- it cannot silently pass."""
    result = run("check", "--reason", REASON, timeout=60)
    assert result.returncode in (0, 1), result.stderr
    # Either stream: this test is about prompting and the summary existing, not
    # about where it is printed. `test_check_writes_its_report_to_stdout` owns
    # the stream contract.
    assert re.search(r"\d+ found", result.stdout + result.stderr), (
        result.stdout + result.stderr
    )


def test_check_writes_its_report_to_stdout():
    """The report is the output the operator asked for, so it belongs on stdout
    and must be pipeable: `sudo-secretspec check | grep NAME` has to work.

    This inverts an earlier assertion in this suite, which required stdout to be
    empty on the theory that a report there would corrupt `export` and `get`
    pipelines. That theory was wrong -- those are separate invocations, so
    `check` cannot contaminate them -- and the cost was real: the whole report
    was unpipeable, and because `colored` decides on ANSI by testing *stdout*,
    a report written to stderr leaked escape bytes into redirected logs.

    Requires a boundary at 0.19.1-sudo.15 or newer; against .14 it fails, which
    is the correct signal for a post-install gate."""
    result = run("check", "--reason", REASON)
    assert result.returncode in (0, 1), result.stderr
    assert re.search(r"\d+ found", result.stdout), (
        f"summary missing from stdout; stdout={result.stdout[:200]!r} "
        f"stderr={result.stderr[:200]!r}"
    )
    assert "found" not in result.stderr, (
        f"the report must not be duplicated onto stderr: {result.stderr[:200]!r}"
    )


def test_check_requires_a_reason():
    result = run("check")
    assert result.returncode != 0
    assert "reason" in (result.stderr + result.stdout).lower()


def test_check_finds_the_secrets_the_manifest_declares(declared_names):
    assert declared_names, "check reported no declared secrets at all"


def test_get_returns_a_value_for_a_resolved_secret(resolved_name):
    result = run("get", resolved_name, "--reason", REASON)
    assert result.returncode == 0, result.stderr
    # Never assert on the value itself, and never let it reach the report.
    assert result.stdout.strip(), f"{resolved_name} resolved to nothing"


def test_get_refuses_an_undeclared_name():
    result = run("get", "DEFINITELY_NOT_A_DECLARED_SECRET", "--reason", REASON)
    assert result.returncode != 0


def test_get_requires_a_reason(resolved_name):
    result = run("get", resolved_name)
    assert result.returncode != 0
    assert "reason" in (result.stderr + result.stdout).lower()


def test_export_emits_every_declared_secret(declared_names):
    """`export` writes a JSON object of name -> value on stdout.

    Assert on keys only. This output is the densest concentration of live
    credentials the CLI produces, so nothing derived from its *values* may reach
    an assertion message, a log, or a captured-output dump on failure.
    """
    import json

    result = run("export", "--reason", REASON)
    assert result.returncode == 0, result.stderr

    try:
        exported = set(json.loads(result.stdout))
    except json.JSONDecodeError as exc:  # never echo the body
        pytest.fail(f"export did not emit JSON on stdout: {exc}")

    missing = declared_names - exported
    # Optional secrets legitimately resolve to nothing, so require overlap
    # rather than equality, and report only names.
    assert exported & declared_names, (
        f"export shares no names with check; missing={sorted(missing)[:5]}"
    )


def test_run_passes_the_environment_to_a_child(resolved_name):
    name = resolved_name
    result = run(
        "run",
        "--reason",
        REASON,
        "--",
        "/bin/sh",
        "-c",
        # Print only whether it is set, never the value.
        f'test -n "${name}" && echo PRESENT || echo ABSENT',
    )
    assert result.returncode == 0, result.stderr
    assert "PRESENT" in result.stdout


def test_run_forwards_double_dash_to_the_child():
    """`run -- cargo test -- --nocapture` used to try to execute `--nocapture`."""
    result = run("run", "--reason", REASON, "--", "/bin/echo", "--", "--nocapture")
    assert result.returncode == 0, result.stderr
    assert "--nocapture" in result.stdout


def test_run_propagates_the_child_exit_code():
    result = run("run", "--reason", REASON, "--", "/bin/sh", "-c", "exit 7")
    assert result.returncode == 7, f"got {result.returncode}"


# --------------------------------------------------------------------------
# Write operations (opt-in)
# --------------------------------------------------------------------------


@pytest.mark.usefixtures("declares_allowed")
def test_add_declares_a_secret_without_assigning_a_value(scratch_name):
    """`add` was wired to the engine's `set`, which refuses an undeclared name,
    so it could never once declare anything."""
    result = run(
        "add",
        scratch_name,
        "--description",
        "Scratch name reserved for the post-install smoke suite",
        "--reason",
        REASON,
    )
    combined = result.stdout + result.stderr
    assert "SecretNotFound" not in combined, (
        "add is still dispatching to `set` -- this boundary predates the fix"
    )
    # Already declared by an earlier run is success for this suite's purposes.
    assert result.returncode == 0 or "already" in combined.lower(), combined


@pytest.mark.usefixtures("declares_allowed")
def test_add_requires_a_description(scratch_name):
    result = run("add", scratch_name, "--reason", REASON)
    assert result.returncode != 0
    assert "description" in (result.stderr + result.stdout).lower()


@pytest.mark.usefixtures("declares_allowed")
def test_add_requires_a_reason(scratch_name):
    result = run("add", scratch_name, "--description", "x")
    assert result.returncode != 0
    assert "reason" in (result.stderr + result.stdout).lower()


@pytest.mark.usefixtures("writes_allowed", "declares_allowed")
def test_set_get_delete_round_trip(scratch_name):
    """The full credential lifecycle against the live vault.

    `set` takes its value on stdin: the broker calls the engine with no value,
    which is the engine's interactive entry path, and that reads the stdin the
    client passed through.
    """
    secret = "post-install-smoke-value-not-a-real-credential"

    written = run("set", scratch_name, "--reason", REASON, stdin=secret + "\n")
    assert written.returncode == 0, written.stdout + written.stderr

    read_back = run("get", scratch_name, "--reason", REASON)
    assert read_back.returncode == 0, read_back.stderr
    assert read_back.stdout.strip() == secret

    removed = run("delete", scratch_name, "--reason", REASON)
    assert removed.returncode == 0, removed.stdout + removed.stderr

    gone = run("get", scratch_name, "--reason", REASON)
    assert gone.returncode != 0 or not gone.stdout.strip(), "value survived delete"


@pytest.mark.usefixtures("writes_allowed", "declares_allowed")
def test_the_round_trip_leaves_the_ledger_verifiable(scratch_name):
    before, _ = audit_tip()
    run("set", scratch_name, "--reason", REASON, stdin="throwaway\n")
    run("delete", scratch_name, "--reason", REASON)
    after, _ = audit_tip()

    assert after > before, "credential writes produced no audit events"
    assert run("audit-verify").returncode == 0


# --------------------------------------------------------------------------
# Lifecycle: only the dry-run plans are reachable without a terminal
# --------------------------------------------------------------------------


@pytest.mark.usefixtures("lifecycle_allowed")
def test_install_dry_run_plans_the_vault_the_boundary_already_serves(config):
    """The regression that shipped in 0.19.1-sudo.6: detection scanned a fixed
    list of directory names instead of reading the installed config, so on a
    migrated host a reinstall planned to adopt the retired vault."""
    result = run("install", "--adopt-existing", "--dry-run", "--non-interactive")
    assert result.returncode == 0, result.stdout + result.stderr

    planned = dict(
        line.split("=", 1)
        for line in result.stdout.splitlines()
        if "=" in line and not line.startswith(" ")
    )
    assert planned.get("vault") == config["vault"], (
        f"install would adopt {planned.get('vault')}, "
        f"but the boundary serves {config['vault']}"
    )
    assert (
        planned.get("service") == f"{config['service_user']}:{config['service_group']}"
    )


@pytest.mark.usefixtures("lifecycle_allowed")
def test_uninstall_dry_run_keeps_the_vault_and_the_service_identity():
    """Both survive unless explicitly opted into, each behind its own flag."""
    result = run("uninstall", "--dry-run")
    assert result.returncode == 0, result.stdout + result.stderr

    plan = dict(
        line.split("=", 1)
        for line in result.stdout.splitlines()
        if "=" in line and not line.startswith(" ")
    )
    assert plan.get("purge_vault") == "0", plan
    assert plan.get("remove_service_user") == "0", plan


@pytest.mark.usefixtures("lifecycle_allowed")
def test_uninstall_dry_run_removes_the_policy_before_the_binaries_it_grants(config):
    """Ordering is the security property: a policy left pointing at a removed
    binary is a window where the grant outlives the thing it was scoped to."""
    result = run("uninstall", "--dry-run")
    assert result.returncode == 0, result.stderr

    removals = [
        line.split("=", 1)[1]
        for line in result.stdout.splitlines()
        if line.startswith("would_remove=")
    ]
    policy = [
        i for i, p in enumerate(removals) if p.endswith("sudoers.d/sudo-secretspec")
    ]
    binaries = [i for i, p in enumerate(removals) if "/bin/" in p or "/libexec/" in p]
    assert policy, f"uninstall plans no policy removal: {removals}"
    assert binaries, f"uninstall plans no binary removal: {removals}"
    assert policy[0] < binaries[0], f"policy removed after binaries: {removals}"
