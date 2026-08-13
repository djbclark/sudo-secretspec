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
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "sudo-secretspec 0.19.1-djbclark.1"
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
        "get", "set", "add", "delete", "check", "export", "run", "install", "doctor", "rollback",
    ] {
        assert!(stdout.contains(command), "missing {command}");
    }
}
