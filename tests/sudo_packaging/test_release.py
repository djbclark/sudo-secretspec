"""Focused tests for the downstream release helper; never touch the network."""

from __future__ import annotations

import importlib.util
import json
import re
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
RELEASE_PY = ROOT / "packaging" / "release.py"


def load_release():
    spec = importlib.util.spec_from_file_location("sudo_secretspec_release", RELEASE_PY)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


@pytest.fixture
def release():
    return load_release()


@pytest.fixture
def cut(release):
    """The release this checkout is stamped at, so a version bump is inert here."""
    return workspace_release(release)


def completed(argv, stdout="", returncode=0):
    return subprocess.CompletedProcess(argv, returncode, stdout=stdout, stderr="")


def workspace_release(release):
    """The release the checked-in workspace version corresponds to.

    `preflight` reads the real `Cargo.toml`, so the tests have to agree with
    whatever it is stamped at. Deriving it keeps a version bump from breaking
    tests that are not about the version.
    """
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'version = "(\d+\.\d+\.\d+-djbclark\.\d+)"', text)
    assert match, "workspace Cargo.toml is not stamped with a downstream version"
    return release.parse_release(match.group(1))


# The git half of a passing preflight; each test supplies its own `gh` response.
PREFLIGHT_GIT_OUTPUTS = {
    ("git", "status", "--porcelain"): "",
    ("git", "branch", "--show-current"): "sudo-main\n",
    (
        "git",
        "remote",
        "get-url",
        "origin",
    ): "https://github.com/djbclark/sudo-secretspec.git\n",
    (
        "git",
        "remote",
        "get-url",
        "upstream",
    ): "https://github.com/cachix/secretspec.git\n",
    ("git", "rev-parse", "v0.19.1^{commit}"): "abc\n",
    ("git", "merge-base", "--is-ancestor", "v0.19.1", "HEAD"): "",
    ("git", "show", "v0.19.1:Cargo.toml"): '[workspace.package]\nversion = "0.19.1"\n',
}


def test_release_identity_is_derived_from_the_serial(release, cut):
    cut = release.parse_release("0.19.1-djbclark.2")
    assert cut.serial == 2
    assert cut.version == "0.19.1-djbclark.2"
    assert cut.tag == "v0.19.1-djbclark.2"
    assert cut.title == "SecretSpec 0.19.1 — sudo-secretspec downstream 2"
    assert cut.archive_url == (
        "https://github.com/djbclark/sudo-secretspec/archive/refs/tags/"
        "v0.19.1-djbclark.2.tar.gz"
    )


def test_only_downstream_versions_on_the_pinned_upstream_base_are_accepted(release):
    # The upstream base stays a constant: rebasing onto a new upstream tag is a
    # separate decision, not something a --version argument may do implicitly.
    for bad in (
        "0.20.0-djbclark.1",
        "0.19.1",
        "v0.19.1-djbclark.1",
        "0.19.1-djbclark.0",
        "0.19.1-djbclark.01",
        "0.19.1-other.1",
        "",
    ):
        with pytest.raises(release.ReleaseError):
            release.parse_release(bad)


def test_https_release_url_guard(release):
    release.validate_release_url(
        "https://github.com/djbclark/sudo-secretspec/archive/refs/tags/v0.19.1-djbclark.1.tar.gz"
    )
    for url in ("http://github.com/x", "file:///tmp/x", "https://example.com/x"):
        with pytest.raises(release.ReleaseError):
            release.validate_release_url(url)


def test_formula_rewrite_restamps_every_version_site(tmp_path: Path, release, cut):
    # Rewriting only the `url` shipped a formula that fetched the new tarball
    # while declaring and asserting the previous version, so `brew test` failed
    # after the tag and the GitHub Release were already published.
    formula = tmp_path / "sudo-secretspec.rb"
    formula.write_text(
        '  url "https://github.com/djbclark/sudo-secretspec/archive/refs/tags/'
        'v0.19.1-djbclark.1.tar.gz"\n'
        '  version "0.19.1-djbclark.1"\n'
        f'  sha256 "{"0" * 64}"\n'
        "  test do\n"
        '    assert_match "0.19.1-djbclark.1", shell_output("secretspec --version")\n'
        '    assert_match "sudo-secretspec 0.19.1-djbclark.1", shell_output("x")\n'
        "  end\n",
        encoding="utf-8",
    )
    cut = release.parse_release("0.19.1-djbclark.2")

    release.rewrite_formula(formula, cut, "a" * 64)

    text = formula.read_text(encoding="utf-8")
    assert "0.19.1-djbclark.1" not in text, text
    assert text.count("0.19.1-djbclark.2") == release.FORMULA_VERSION_SITES
    assert f'url "{cut.archive_url}"' in text
    assert f'sha256 "{"a" * 64}"' in text


def test_formula_rewrite_refuses_an_unexpected_number_of_version_sites(
    tmp_path: Path, release
):
    formula = tmp_path / "sudo-secretspec.rb"
    formula.write_text(
        '  url "https://github.com/djbclark/sudo-secretspec/archive/refs/tags/'
        'v0.19.1-djbclark.1.tar.gz"\n'
        f'  sha256 "{"0" * 64}"\n',
        encoding="utf-8",
    )
    with pytest.raises(release.ReleaseError, match="version references"):
        release.rewrite_formula(
            formula, release.parse_release("0.19.1-djbclark.2"), "a" * 64
        )


def test_the_real_formula_has_exactly_the_expected_version_sites(release):
    """The count `rewrite_formula` enforces must match the shipped formula.

    This is the regression test for the bug above: the explicit `version`
    stanza and the two `brew test` assertions were added to the formula long
    after the rewriter was written, and nothing tied the two together.
    """
    text = release.FORMULA.read_text(encoding="utf-8")
    assert (
        len(release.ANY_VERSION_RE.findall(text)) == release.FORMULA_VERSION_SITES
    ), text


def test_preflight_validates_branch_remotes_lineage_and_fork(monkeypatch, release):
    calls = []

    def fake_run(argv, **kwargs):
        calls.append(argv)
        key = tuple(argv)
        outputs = {
            ("git", "status", "--porcelain"): "",
            ("git", "branch", "--show-current"): "sudo-main\n",
            (
                "git",
                "remote",
                "get-url",
                "origin",
            ): "https://github.com/djbclark/sudo-secretspec.git\n",
            (
                "git",
                "remote",
                "get-url",
                "upstream",
            ): "https://github.com/cachix/secretspec.git\n",
            ("git", "rev-parse", "v0.19.1^{commit}"): "abc\n",
            ("git", "merge-base", "--is-ancestor", "v0.19.1", "HEAD"): "",
            (
                "git",
                "show",
                "v0.19.1:Cargo.toml",
            ): '[workspace.package]\nversion = "0.19.1"\n',
            (
                "gh",
                "repo",
                "view",
                "djbclark/sudo-secretspec",
                "--json",
                "nameWithOwner,parent,defaultBranchRef",
            ): json.dumps(
                {
                    "nameWithOwner": "djbclark/sudo-secretspec",
                    "parent": {"nameWithOwner": "cachix/secretspec"},
                    "defaultBranchRef": {"name": "sudo-main"},
                }
            ),
        }
        return completed(argv, outputs.get(key, ""))

    monkeypatch.setattr(release, "run", fake_run)
    release.preflight(workspace_release(release), allow_dirty=False)
    assert ["git", "merge-base", "--is-ancestor", "v0.19.1", "HEAD"] in calls


def test_preflight_refuses_a_serial_that_is_already_published(monkeypatch, release):
    """Catch a re-used serial before the tests run and a local tag exists.

    The remote is the authority, not the local tag list: a serial can have been
    published from another checkout entirely.
    """

    def fake_run(argv, **kwargs):
        if argv[:2] == ["git", "ls-remote"]:
            return completed(argv, "9f4c…\trefs/tags/v0.19.1-djbclark.9\n")
        if argv[:4] == ["gh", "repo", "view", "djbclark/sudo-secretspec"]:
            return completed(
                argv,
                json.dumps(
                    {
                        "nameWithOwner": "djbclark/sudo-secretspec",
                        "parent": {"nameWithOwner": "cachix/secretspec"},
                        "defaultBranchRef": {"name": "sudo-main"},
                    }
                ),
            )
        return completed(argv, PREFLIGHT_GIT_OUTPUTS.get(tuple(argv), ""))

    monkeypatch.setattr(release, "run", fake_run)
    with pytest.raises(release.ReleaseError, match="already published"):
        release.preflight(workspace_release(release), allow_dirty=False)


def test_preflight_refuses_a_workspace_stamped_at_another_version(monkeypatch, release):
    def fake_run(argv, **kwargs):
        if argv[:4] == ["gh", "repo", "view", "djbclark/sudo-secretspec"]:
            return completed(
                argv,
                json.dumps(
                    {
                        "nameWithOwner": "djbclark/sudo-secretspec",
                        "parent": {"nameWithOwner": "cachix/secretspec"},
                        "defaultBranchRef": {"name": "sudo-main"},
                    }
                ),
            )
        return completed(argv, PREFLIGHT_GIT_OUTPUTS.get(tuple(argv), ""))

    monkeypatch.setattr(release, "run", fake_run)
    # A serial nobody will ever cut, so it cannot match the real Cargo.toml.
    with pytest.raises(release.ReleaseError, match="bump Cargo.toml"):
        release.preflight(release.parse_release("0.19.1-djbclark.999"), allow_dirty=False)


def test_parent_slug_accepts_every_gh_parent_shape(release):
    # gh 2.97 omits nameWithOwner inside parent and returns its parts instead.
    assert (
        release.parent_slug(
            {
                "parent": {
                    "id": "R_x",
                    "name": "secretspec",
                    "owner": {"login": "cachix"},
                }
            }
        )
        == "cachix/secretspec"
    )
    # Older/other versions supply the composed slug directly.
    assert (
        release.parent_slug({"parent": {"nameWithOwner": "cachix/secretspec"}})
        == "cachix/secretspec"
    )
    for repo in ({"parent": None}, {}, {"parent": {}}, {"parent": {"name": "x"}}):
        assert release.parent_slug(repo) is None


def test_preflight_accepts_gh_parent_without_name_with_owner(monkeypatch, release):
    def fake_run(argv, **kwargs):
        if argv[:4] == ["gh", "repo", "view", "djbclark/sudo-secretspec"]:
            return completed(
                argv,
                json.dumps(
                    {
                        "nameWithOwner": "djbclark/sudo-secretspec",
                        "parent": {
                            "id": "R_kgDOPHAtAA",
                            "name": "secretspec",
                            "owner": {"login": "cachix"},
                        },
                        "defaultBranchRef": {"name": "sudo-main"},
                    }
                ),
            )
        return completed(argv, PREFLIGHT_GIT_OUTPUTS.get(tuple(argv), ""))

    monkeypatch.setattr(release, "run", fake_run)
    release.preflight(workspace_release(release), allow_dirty=False)


def test_preflight_rejects_wrong_fork_parent(monkeypatch, release):
    def fake_run(argv, **kwargs):
        if argv[:4] == ["gh", "repo", "view", "djbclark/sudo-secretspec"]:
            return completed(
                argv,
                json.dumps(
                    {
                        "nameWithOwner": "djbclark/sudo-secretspec",
                        "parent": None,
                        "defaultBranchRef": {"name": "sudo-main"},
                    }
                ),
            )
        outputs = {
            ("git", "status", "--porcelain"): "",
            ("git", "branch", "--show-current"): "sudo-main\n",
            (
                "git",
                "remote",
                "get-url",
                "origin",
            ): "https://github.com/djbclark/sudo-secretspec.git\n",
            (
                "git",
                "remote",
                "get-url",
                "upstream",
            ): "https://github.com/cachix/secretspec.git\n",
            ("git", "rev-parse", "v0.19.1^{commit}"): "abc\n",
            ("git", "merge-base", "--is-ancestor", "v0.19.1", "HEAD"): "",
            (
                "git",
                "show",
                "v0.19.1:Cargo.toml",
            ): '[workspace.package]\nversion = "0.19.1"\n',
        }
        return completed(argv, outputs.get(tuple(argv), ""))

    monkeypatch.setattr(release, "run", fake_run)
    with pytest.raises(release.ReleaseError, match="parent"):
        release.preflight(workspace_release(release), allow_dirty=False)


def test_dry_run_lists_remote_actions_without_running(monkeypatch, release, capsys, cut):
    monkeypatch.setattr(release, "preflight", lambda _release, **kwargs: None)
    monkeypatch.setattr(release, "run_tests", lambda **kwargs: None)
    assert release.main(["--version", cut.version, "--dry-run", "--skip-tests"]) == 0
    output = capsys.readouterr().out
    assert f"git tag -a {cut.tag}" in output
    assert f"git push origin {cut.tag}" in output
    assert "gh release create" in output
    assert "Formula/sudo-secretspec.rb" in output


def test_dry_run_still_validates_preflight(monkeypatch, release, cut):
    seen = []
    monkeypatch.setattr(
        release, "preflight", lambda _release, **kwargs: seen.append(kwargs) or None
    )
    monkeypatch.setattr(release, "run", lambda argv, **kwargs: completed(argv))
    assert release.main(["--version", cut.version, "--dry-run", "--skip-tests"]) == 0
    assert seen == [{"allow_dirty": False}]


def test_dry_run_aborts_when_preflight_fails(monkeypatch, release, cut):
    def boom(_release, **kwargs):
        raise release.ReleaseError("fork parent must be cachix/secretspec, got None")

    monkeypatch.setattr(release, "preflight", boom)
    with pytest.raises(release.ReleaseError, match="fork parent"):
        release.main(["--version", cut.version, "--dry-run", "--skip-tests"])


def test_verify_readback_accepts_gh_parent_without_name_with_owner(
    monkeypatch, release, cut
):
    def fake_run(argv, **kwargs):
        if argv[:3] == ["gh", "repo", "view"]:
            return completed(
                argv,
                json.dumps(
                    {
                        "nameWithOwner": release.FORK_REPO,
                        "parent": {"name": "secretspec", "owner": {"login": "cachix"}},
                    }
                ),
            )
        if argv[:3] == ["gh", "release", "view"]:
            return completed(
                argv,
                json.dumps(
                    {
                        "tagName": cut.tag,
                        "name": cut.title,
                        "isDraft": False,
                    }
                ),
            )
        if argv[:2] == ["gh", "api"]:
            return completed(argv, json.dumps({"ref": f"refs/tags/{cut.tag}"}))
        if argv[-1:] == ["--version"]:
            return completed(argv, f"secretspec {cut.version}\n")
        return completed(argv)

    monkeypatch.setattr(release, "run", fake_run)
    release.verify_readback(cut, "/opt/homebrew/opt/sudo-secretspec")


def test_verify_readback_checks_repo_tag_release_and_installed_version(
    monkeypatch, release, cut
):
    calls = []

    def fake_run(argv, **kwargs):
        calls.append(argv)
        if argv[:3] == ["gh", "repo", "view"]:
            return completed(
                argv,
                json.dumps(
                    {
                        "nameWithOwner": release.FORK_REPO,
                        "parent": {"nameWithOwner": release.UPSTREAM_REPO},
                    }
                ),
            )
        if argv[:3] == ["gh", "release", "view"]:
            return completed(
                argv,
                json.dumps(
                    {
                        "tagName": cut.tag,
                        "name": cut.title,
                        "isDraft": False,
                    }
                ),
            )
        if argv[:3] == [
            "gh",
            "api",
            f"repos/{release.FORK_REPO}/git/ref/tags/{cut.tag}",
        ]:
            return completed(argv, json.dumps({"ref": f"refs/tags/{cut.tag}"}))
        if argv[-1:] == ["--version"]:
            return completed(argv, f"secretspec {cut.version}\n")
        return completed(argv)

    monkeypatch.setattr(release, "run", fake_run)
    release.verify_readback(cut, "/opt/homebrew/opt/sudo-secretspec")
    assert any(argv[:3] == ["gh", "release", "view"] for argv in calls)
    assert any(argv[-1:] == ["--version"] for argv in calls)


def test_interruption_cleanup_removes_only_created_tag_and_formula_change(
    monkeypatch, release, cut
):
    calls = []
    monkeypatch.setattr(
        release, "run", lambda argv, **kwargs: calls.append(argv) or completed(argv)
    )
    state = release.CleanupState(
        tag_created=True, tag_pushed=False, formula_changed=True
    )
    release.cleanup_interrupted(cut, state)
    assert calls == [
        ["git", "tag", "-d", cut.tag],
        [
            "git",
            "restore",
            "--staged",
            "--worktree",
            "--",
            "packaging/homebrew/sudo-secretspec.rb",
        ],
    ]


def test_interruption_cleanup_never_deletes_pushed_tag(monkeypatch, release):
    calls = []
    monkeypatch.setattr(
        release, "run", lambda argv, **kwargs: calls.append(argv) or completed(argv)
    )
    release.cleanup_interrupted(
        cut, release.CleanupState(tag_created=True, tag_pushed=True, formula_changed=False)
    )
    assert calls == []
