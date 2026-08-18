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
        Some(PathBuf::from("/usr/local/share/sudo-secretspec/secretspec.toml"))
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

/// `doctor` is supposed to answer "which binary would actually run", not only
/// "is the installed one intact". This drives the whole public entry point so
/// the `InspectOptions` plumbing is covered, not just the classifier.
#[test]
fn inspect_reports_a_client_the_callers_path_would_reach_first() {
    let dir = TempDir::new().unwrap();
    let path = write_config(&dir, minimal_toml());
    let layout = sudo_secretspec_cli::load_config(&path).unwrap();

    let shadow_dir = dir.path().join("shadow");
    std::fs::create_dir_all(&shadow_dir).unwrap();
    let shadow = shadow_dir.join("sudo-secretspec");
    std::fs::write(&shadow, b"not the installed client").unwrap();

    let caller_path = std::env::join_paths([shadow_dir.as_path()]).unwrap();
    let report = sudo_secretspec_cli::inspect(
        &layout,
        &sudo_secretspec_cli::InspectOptions {
            caller_path: Some(caller_path),
            ..Default::default()
        },
    );

    let found = report
        .findings
        .iter()
        .find(|f| f.path.as_deref() == Some(shadow.display().to_string().as_str()))
        .unwrap_or_else(|| panic!("no finding for {}: {:?}", shadow.display(), report.findings));
    assert_eq!(found.code, "CLIENT_SHADOWED");
    assert!(!found.advisory);
    assert!(!report.ok);
}

/// Without a caller `PATH` the scan still runs, and it never invents a finding
/// for a directory that holds no `sudo-secretspec`.
#[test]
fn inspect_without_a_caller_path_ignores_unrelated_directories() {
    let dir = TempDir::new().unwrap();
    let path = write_config(&dir, minimal_toml());
    let layout = sudo_secretspec_cli::load_config(&path).unwrap();

    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();

    let report =
        sudo_secretspec_cli::inspect(&layout, &sudo_secretspec_cli::InspectOptions::default());
    assert!(
        !report.findings.iter().any(|f| f
            .path
            .as_deref()
            .is_some_and(|p| p.starts_with(empty.display().to_string().as_str()))),
        "{:?}",
        report.findings
    );
}

/// `inspect` must write nothing to stdout, so `doctor --json` emits only the
/// report.
///
/// `visudo -c` prints "<path>: parsed OK" on success. Inheriting its stdout put
/// that line ahead of the JSON, and anything consuming the report as JSON —
/// which is what `AI-GUIDANCE.md` tells automation to do — failed on the very
/// first character.
///
/// Re-execs this test binary because cargo captures the parent's stdout but not
/// a child's.
#[test]
fn inspect_writes_nothing_to_stdout() {
    const MARKER: &str = "SUDO_SECRETSPEC_STDOUT_CHILD";

    if std::env::var_os(MARKER).is_some() {
        let dir = TempDir::new().unwrap();
        let policy = dir.path().join("sudo-secretspec");
        // Valid on purpose: the leak only happens when visudo *succeeds*.
        std::fs::write(
            &policy,
            "operator ALL=(root) NOPASSWD: /usr/local/libexec/sudo-secretspec doctor\n",
        )
        .unwrap();

        let mut layout = sudo_secretspec_cli::load_config(&write_config(&dir, minimal_toml()))
            .expect("minimal config");
        layout.sudoers = policy;
        let _ =
            sudo_secretspec_cli::inspect(&layout, &sudo_secretspec_cli::InspectOptions::default());
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["inspect_writes_nothing_to_stdout", "--exact", "--nocapture"])
        .env(MARKER, "1")
        .output()
        .expect("re-exec the test binary");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !stdout.contains("parsed OK"),
        "visudo output leaked into stdout: {stdout}"
    );
}
