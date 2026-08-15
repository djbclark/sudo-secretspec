use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sudo-secretspec"))
}

#[test]
fn version_identifies_downstream_distribution() {
    let output = binary()
        .arg("--version")
        .output()
        .expect("run sudo-secretspec");
    assert!(output.status.success());
    let reported = String::from_utf8(output.stdout).unwrap().trim().to_string();

    // Assert the shape, not a literal: the point is that the binary announces
    // itself as the downstream distribution rather than plain upstream, and
    // pinning the number here only meant editing this test on every release.
    assert_eq!(
        reported,
        format!("sudo-secretspec {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        reported.contains("-sudo."),
        "version must identify the fork: {reported}"
    );
}

#[test]
fn public_cli_rejects_control_plane_overrides() {
    for option in ["--file", "--provider", "--profile", "--direct"] {
        let output = binary()
            .args([option, "untrusted", "check"])
            .output()
            .expect("run sudo-secretspec");
        assert!(!output.status.success(), "accepted {option}");
    }
}

#[test]
fn help_exposes_typed_boundary_commands() {
    let output = binary()
        .arg("--help")
        .output()
        .expect("run sudo-secretspec");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    for command in [
        "get",
        "set",
        "add",
        "delete",
        "check",
        "export",
        "template-check",
        "run",
        "install",
        "uninstall",
        "doctor",
        "rollback",
    ] {
        assert!(stdout.contains(command), "missing {command}");
    }
}

#[test]
fn template_check_requires_a_reason() {
    // Every brokered operation is audited, and the ledger entry is keyed by the
    // reason digest. A `template-check` that could run without one would be an
    // unaudited read of the boundary's configuration.
    let output = binary()
        .arg("template-check")
        .output()
        .expect("run sudo-secretspec");
    assert!(
        !output.status.success(),
        "template-check ran without --reason"
    );
}
