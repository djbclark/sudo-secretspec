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

// --- snapshot trust decisions -------------------------------------------
//
// `plan_restore` is the half of rollback that decides what may be written.
// These cover the case that made the previous implementation an arbitrary
// root-owned write: a snapshot directory naming its own destinations.

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// Write a `<index>.prior` / `<index>.path` pair, plus a manifest row.
fn write_pair(dir: &std::path::Path, index: usize, dest: &str, body: &[u8], manifest: &mut String) {
    std::fs::write(dir.join(format!("{index}.prior")), body).unwrap();
    std::fs::write(dir.join(format!("{index}.path")), dest).unwrap();
    manifest.push_str(&format!("{}  {}\n", sha256_hex(body), dest));
}

#[test]
fn plan_restore_requires_a_manifest() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("0.prior"), b"x").unwrap();
    std::fs::write(dir.path().join("0.path"), "/usr/local/bin/sudo-secretspec").unwrap();

    let err = sudo_secretspec_cli::rollback::plan_restore(dir.path())
        .expect_err("a snapshot without a manifest must be refused");
    assert!(err.to_string().contains("manifest"), "{err}");
}

#[test]
fn plan_restore_refuses_destinations_the_installer_does_not_own() {
    let dir = tempfile::tempdir().unwrap();
    let mut manifest = String::new();
    // A hash-consistent pair is still refused: the destination is the problem.
    write_pair(
        dir.path(),
        0,
        "/etc/sudoers",
        b"root ALL=(ALL) NOPASSWD: ALL\n",
        &mut manifest,
    );
    std::fs::write(dir.path().join("MANIFEST.sha256"), &manifest).unwrap();

    let err = sudo_secretspec_cli::rollback::plan_restore(dir.path())
        .expect_err("an unowned destination must be refused");
    assert!(err.to_string().contains("does not own"), "{err}");
}

#[test]
fn plan_restore_refuses_bytes_that_do_not_match_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let mut manifest = String::new();
    write_pair(
        dir.path(),
        0,
        "/usr/local/bin/sudo-secretspec",
        b"original",
        &mut manifest,
    );
    std::fs::write(dir.path().join("MANIFEST.sha256"), &manifest).unwrap();
    // Swap the payload after the manifest was written.
    std::fs::write(dir.path().join("0.prior"), b"substituted").unwrap();

    let err = sudo_secretspec_cli::rollback::plan_restore(dir.path())
        .expect_err("a payload that does not match the manifest must be refused");
    assert!(err.to_string().contains("manifest hash"), "{err}");
}

#[test]
fn plan_restore_accepts_an_owned_verified_pair_with_its_installed_mode() {
    let dir = tempfile::tempdir().unwrap();
    let mut manifest = String::new();
    write_pair(
        dir.path(),
        0,
        "/private/etc/sudoers.d/sudo-secretspec",
        b"policy",
        &mut manifest,
    );
    std::fs::write(dir.path().join("MANIFEST.sha256"), &manifest).unwrap();

    let plan = sudo_secretspec_cli::rollback::plan_restore(dir.path()).expect("valid snapshot");
    assert_eq!(plan.len(), 1);
    let (_, dest, mode) = &plan[0];
    assert_eq!(
        dest,
        std::path::Path::new("/private/etc/sudoers.d/sudo-secretspec")
    );
    // Restored from the shared artifact table, never from the snapshot.
    assert_eq!(*mode, 0o440);
}

#[test]
fn installed_artifact_table_pins_the_sensitive_modes() {
    use sudo_secretspec_cli::install::artifact_mode;
    assert_eq!(
        artifact_mode(std::path::Path::new(
            "/private/etc/sudoers.d/sudo-secretspec"
        )),
        Some(0o440)
    );
    assert_eq!(
        artifact_mode(std::path::Path::new("/usr/local/bin/sudo-secretspec")),
        Some(0o755)
    );
    assert_eq!(
        artifact_mode(std::path::Path::new("/usr/local/etc/sudo-secretspec.toml")),
        Some(0o444)
    );
    assert_eq!(artifact_mode(std::path::Path::new("/etc/passwd")), None);
}
