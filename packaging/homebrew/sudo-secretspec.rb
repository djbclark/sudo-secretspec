# typed: strict
# frozen_string_literal: true

# Homebrew package for the upstream engine and downstream opt-in companion.
class SudoSecretspec < Formula
  desc "SecretSpec engine with an opt-in macOS privilege-boundary companion"
  homepage "https://github.com/djbclark/sudo-secretspec"
  url "https://github.com/djbclark/sudo-secretspec/archive/refs/tags/v0.19.1-djbclark.1.tar.gz"
  sha256 "29f25c0274ad8db6fd5e14b5c6db5ba2906c906e470e5504134f71a16e540200"
  license "Apache-2.0"
  head "https://github.com/djbclark/sudo-secretspec.git", branch: "sudo-main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", "--root", prefix, "--path", "secretspec"
    system "cargo", "install", "--locked", "--root", prefix, "--path", "sudo-secretspec-cli"
    (share/"sudo-secretspec").install "sudo-secretspec/AI-GUIDANCE.md"
    (share/"sudo-secretspec/skills/sudo-secretspec").install "skills/sudo-secretspec/SKILL.md"
  end

  def caveats
    <<~EOS
      Homebrew installed files only. It did not run sudo, edit sudoers, create
      users, or mutate /var/db. To opt in after reviewing the installed assets,
      explicitly run:

        sudo-secretspec install --declarations /path/to/secretspec.toml

      Existing protected stores must be adopted explicitly; see
      sudo-secretspec/AI-GUIDANCE.md in the source distribution.
    EOS
  end

  test do
    assert_match "0.19.1-djbclark.1", shell_output("#{bin}/secretspec --version")
    assert_match "sudo-secretspec 0.19.1-djbclark.1", shell_output("#{bin}/sudo-secretspec --version")
  end
end
