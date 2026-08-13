use std::path::PathBuf;

use sudo_secretspec_cli::install::InstallRequest;

#[test]
fn install_request_defaults_are_generic() {
    let req = InstallRequest::from_cli(PathBuf::from("/tmp/decl.toml"), true, false);
    assert_eq!(req.vault, PathBuf::from("/var/db/sudo-secretspec"));
    assert_eq!(req.service_user, "_sudo_secretspec");
    assert_eq!(req.service_group, "_sudo_secretspec");
    assert!(req.dry_run);
    assert!(!req.adopt_existing);
}

#[test]
fn rollback_rejects_unprotected_snapshot_path() {
    let err = sudo_secretspec_cli::rollback::run(std::path::Path::new("/tmp/not-a-snapshot"))
        .expect_err("unprotected snapshot must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("root") || msg.contains("outside") || msg.contains("invalid"),
        "{msg}"
    );
}
