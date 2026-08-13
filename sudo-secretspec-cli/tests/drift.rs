/// Drift-check contract parsing tests using typed TOML configuration.
use std::path::PathBuf;
use tempfile::TempDir;

fn write_config(dir: &TempDir, content: &str) -> PathBuf {
    let path = dir.path().join("sudo-secretspec.toml");
    std::fs::write(&path, content).unwrap();
    path
}

fn minimal_toml() -> &'static str {
    r#"
engine = "/usr/local/libexec/sudo-secretspec"
audit_helper = "/usr/local/libexec/sudo-secretspec"
vault = "/var/db/sudo-secretspec"
vault_realpath = "/private/var/db/sudo-secretspec"
declarations = "/usr/local/share/sudo-secretspec/secretspec.toml"
service_user = "_sudo_secretspec"
service_group = "_sudo_secretspec"
"#
}

#[test]
fn layout_derives_single_binary_paths() {
    let dir = TempDir::new().unwrap();
    let path = write_config(&dir, minimal_toml());
    let layout = sudo_secretspec_cli::load_config(&path).unwrap();

    assert_eq!(
        layout.engine,
        PathBuf::from("/usr/local/libexec/sudo-secretspec")
    );
    assert_eq!(
        layout.broker,
        PathBuf::from("/usr/local/libexec/sudo-secretspec")
    );
    assert_eq!(
        layout.client,
        PathBuf::from("/usr/local/bin/sudo-secretspec")
    );
    assert_eq!(
        layout.checker,
        PathBuf::from("/usr/local/bin/sudo-secretspec")
    );
    assert_eq!(
        layout.retired,
        PathBuf::from("/usr/local/share/sudo-secretspec/sudo-secretspec-retired.toml")
    );
    assert_eq!(
        layout.guidance,
        PathBuf::from("/usr/local/share/sudo-secretspec/AI-GUIDANCE.md")
    );
    assert_eq!(
        layout.source_manifest,
        PathBuf::from("/usr/local/share/sudo-secretspec/MANIFEST.sha256")
    );
}

#[test]
fn layout_holds_all_config_values() {
    let dir = TempDir::new().unwrap();
    let path = write_config(&dir, minimal_toml());
    let layout = sudo_secretspec_cli::load_config(&path).unwrap();

    assert_eq!(layout.config, path);
    assert_eq!(layout.vault, PathBuf::from("/var/db/sudo-secretspec"));
    assert_eq!(
        layout.vault_realpath,
        PathBuf::from("/private/var/db/sudo-secretspec")
    );
    assert_eq!(layout.service_user, "_sudo_secretspec");
    assert_eq!(layout.service_group, "_sudo_secretspec");
    assert_eq!(
        layout.declarations,
        PathBuf::from("/usr/local/share/sudo-secretspec/secretspec.toml")
    );
    assert_eq!(
        layout.audit,
        PathBuf::from("/usr/local/libexec/sudo-secretspec")
    );
}

#[test]
fn config_rejects_unknown_keys() {
    let dir = TempDir::new().unwrap();
    let bad = format!("{}\nextra = \"nope\"\n", minimal_toml());
    let path = write_config(&dir, &bad);
    assert!(sudo_secretspec_cli::load_config(&path).is_err());
}

#[test]
fn config_rejects_missing_keys() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        &dir,
        r#"
engine = "/usr/local/libexec/sudo-secretspec"
"#,
    );
    assert!(sudo_secretspec_cli::load_config(&path).is_err());
}

#[test]
fn config_rejects_relative_paths() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        &dir,
        r#"
engine = "relative/path"
audit_helper = "/usr/local/libexec/sudo-secretspec"
vault = "/var/db/sudo-secretspec"
vault_realpath = "/private/var/db/sudo-secretspec"
declarations = "/usr/local/share/sudo-secretspec/secretspec.toml"
service_user = "_sudo_secretspec"
service_group = "_sudo_secretspec"
"#,
    );
    assert!(sudo_secretspec_cli::load_config(&path).is_err());
}

#[test]
fn config_rejects_parent_traversal_in_paths() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        &dir,
        r#"
engine = "/usr/local/libexec/../etc/passwd"
audit_helper = "/usr/local/libexec/sudo-secretspec"
vault = "/var/db/sudo-secretspec"
vault_realpath = "/private/var/db/sudo-secretspec"
declarations = "/usr/local/share/sudo-secretspec/secretspec.toml"
service_user = "_sudo_secretspec"
service_group = "_sudo_secretspec"
"#,
    );
    assert!(sudo_secretspec_cli::load_config(&path).is_err());
}

#[test]
fn config_rejects_invalid_service_user() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        &dir,
        r#"
engine = "/usr/local/libexec/sudo-secretspec"
audit_helper = "/usr/local/libexec/sudo-secretspec"
vault = "/var/db/sudo-secretspec"
vault_realpath = "/private/var/db/sudo-secretspec"
declarations = "/usr/local/share/sudo-secretspec/secretspec.toml"
service_user = "root"
service_group = "_sudo_secretspec"
"#,
    );
    assert!(sudo_secretspec_cli::load_config(&path).is_err());
}

#[test]
fn config_handles_comments_and_empty_lines() {
    let dir = TempDir::new().unwrap();
    let path = write_config(
        &dir,
        r#"
# comment

engine = "/usr/local/libexec/sudo-secretspec"
audit_helper = "/usr/local/libexec/sudo-secretspec"
vault = "/var/db/sudo-secretspec"
vault_realpath = "/private/var/db/sudo-secretspec"
declarations = "/usr/local/share/sudo-secretspec/secretspec.toml"
service_user = "_sudo_secretspec"
service_group = "_sudo_secretspec"
"#,
    );
    assert!(sudo_secretspec_cli::load_config(&path).is_ok());
}
