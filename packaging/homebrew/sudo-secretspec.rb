# typed: strict
# frozen_string_literal: true

# Homebrew package for the upstream engine and downstream opt-in companion.
class SudoSecretspec < Formula
  desc "SecretSpec engine with an opt-in macOS privilege-boundary companion"
  homepage "https://github.com/djbclark/sudo-secretspec"
  url "https://github.com/djbclark/sudo-secretspec/archive/refs/tags/v0.19.1-djbclark.2.tar.gz"
  # Homebrew parses the trailing ".1" of the tag as the whole version, which
  # breaks upgrade detection. State it explicitly.
  version "0.19.1-djbclark.2"
  sha256 "c77de60a8131b1c592ace413a1c425fe1a1e78db5776e8c8b2768a480d95bca9"
  license "Apache-2.0"
  head "https://github.com/djbclark/sudo-secretspec.git", branch: "sudo-main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", "--root", prefix, "--path", "secretspec"

    # The companion goes to libexec, which Homebrew does not link, so it never
    # lands on PATH. It is only a bootstrap: the binary you run day to day is
    # the one `sudo-secretspec install` places at /usr/local/bin, and that path
    # is what the sudoers policy and the installed manifest are pinned to. A
    # linked keg copy would shadow it with a same-version, different-hash
    # binary that `doctor` cannot currently detect.
    staging = buildpath/"companion-root"
    system "cargo", "install", "--locked", "--root", staging, "--path", "sudo-secretspec-cli"
    libexec.install staging/"bin/sudo-secretspec"

    (share/"sudo-secretspec").install "sudo-secretspec/AI-GUIDANCE.md"
    (share/"sudo-secretspec/skills/sudo-secretspec").install "skills/sudo-secretspec/SKILL.md"
  end

  def caveats
    <<~EOS
      Homebrew installed files only. It did not run sudo, edit sudoers, create
      users, or mutate /var/db. To opt in after reviewing the installed assets,
      explicitly run the bootstrap companion out of libexec:

        #{opt_libexec}/sudo-secretspec install --declarations /path/to/secretspec.toml

      That installs the client you use from then on at
      /usr/local/bin/sudo-secretspec. The libexec copy is deliberately not on
      PATH so it cannot shadow the installed client.

      Existing protected stores must be adopted explicitly; see
      sudo-secretspec/AI-GUIDANCE.md in the source distribution.
    EOS
  end

  test do
    assert_match "0.19.1-djbclark.2", shell_output("#{bin}/secretspec --version")
    assert_match "sudo-secretspec 0.19.1-djbclark.2", shell_output("#{libexec}/sudo-secretspec --version")
    # The companion must never be linked onto PATH; see the install comment.
    refute_path_exists bin/"sudo-secretspec"
  end
end
